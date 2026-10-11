//! Live microphone capture into memory, for dictation.
//!
//! [`MicStream::open`] starts the default microphone. While idle it keeps the
//! last [`PRE_ROLL`] of audio, so speech that starts a moment before the
//! hotkey registers isn't cut off; [`MicStream::begin`] starts collecting
//! (pre-roll included) and [`MicStream::finish`] returns everything since as
//! 16 kHz mono.
//!
//! The audio callback only pushes samples into a lock-free ring buffer (no
//! locks or allocation on the audio thread). A collector thread drains it
//! every few milliseconds into the pre-roll or the take.
//!
//! Like `cpal::Stream`, a `MicStream` may not be movable between threads on
//! every platform: create and use it on one thread.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, StreamConfig};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::{AudioError, STT_SAMPLE_RATE, resample_mono, to_mono};

/// Audio kept from before `begin`.
pub const PRE_ROLL: Duration = Duration::from_millis(300);
const DRAIN_EVERY: Duration = Duration::from_millis(10);

pub struct MicStream {
    // Dropping the stream stops the callbacks.
    stream: cpal::Stream,
    shared: Arc<Shared>,
    collector: Option<JoinHandle<()>>,
    device: String,
    sample_rate: u32,
    channels: u16,
}

struct Shared {
    buffer: Mutex<Buffer>,
    stop: AtomicBool,
    /// Set by the stream's error callback (e.g. the device was unplugged).
    failed: AtomicBool,
}

struct Buffer {
    consumer: Consumer<f32>,
    /// Interleaved samples.
    pre_roll: VecDeque<f32>,
    pre_roll_capacity: usize,
    /// `Some` between `begin` and `finish`.
    take: Option<Vec<f32>>,
}

impl Buffer {
    fn drain(&mut self) {
        let available = self.consumer.slots();
        let Ok(chunk) = self.consumer.read_chunk(available) else {
            return;
        };
        let (a, b) = chunk.as_slices();
        match &mut self.take {
            Some(take) => {
                take.extend_from_slice(a);
                take.extend_from_slice(b);
            }
            None => {
                self.pre_roll.extend(a.iter().chain(b));
                let excess = self.pre_roll.len().saturating_sub(self.pre_roll_capacity);
                self.pre_roll.drain(..excess);
            }
        }
        chunk.commit_all();
    }
}

impl MicStream {
    /// Opens and starts the default microphone. On macOS this blocks while
    /// a permission prompt is open.
    pub fn open() -> Result<Self, AudioError> {
        let started = Instant::now();
        let device = cpal::default_host()
            .default_input_device()
            .ok_or(AudioError::NoDevice("microphone"))?;
        let supported = device
            .default_input_config()
            .map_err(|e| AudioError::device("reading microphone format", e))?;
        let format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let frame_len = usize::from(config.channels);
        // A second of headroom between drains; the collector runs every 10 ms.
        let (producer, consumer) = RingBuffer::<f32>::new(config.sample_rate as usize * frame_len);
        let pre_roll_capacity =
            (config.sample_rate as usize * PRE_ROLL.as_millis() as usize / 1000) * frame_len;
        let shared = Arc::new(Shared {
            buffer: Mutex::new(Buffer {
                consumer,
                pre_roll: VecDeque::with_capacity(pre_roll_capacity),
                pre_roll_capacity,
                take: None,
            }),
            stop: AtomicBool::new(false),
            failed: AtomicBool::new(false),
        });

        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, &config, producer, &shared),
            SampleFormat::I16 => build::<i16>(&device, &config, producer, &shared),
            SampleFormat::I32 => build::<i32>(&device, &config, producer, &shared),
            SampleFormat::U16 => build::<u16>(&device, &config, producer, &shared),
            other => Err(AudioError::UnsupportedFormat(format!(
                "device format {other}"
            ))),
        }?;
        stream
            .play()
            .map_err(|e| AudioError::device("starting microphone", e))?;

        let collector = {
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("quill-mic-collector".into())
                .spawn(move || {
                    while !shared.stop.load(Ordering::Acquire) {
                        shared.lock().drain();
                        std::thread::sleep(DRAIN_EVERY);
                    }
                })?
        };
        let device = device.to_string();
        tracing::debug!(%device, ms = started.elapsed().as_millis(), "microphone open");
        Ok(Self {
            stream,
            shared,
            collector: Some(collector),
            device,
            sample_rate: config.sample_rate,
            channels: config.channels,
        })
    }

    pub fn device(&self) -> &str {
        &self.device
    }

    /// False once the stream has reported an error; open a new one.
    pub fn is_healthy(&self) -> bool {
        !self.shared.failed.load(Ordering::Relaxed)
    }

    /// Starts collecting audio, beginning with the pre-roll.
    pub fn begin(&self) {
        let mut buffer = self.shared.lock();
        buffer.drain();
        let pre_roll: Vec<f32> = buffer.pre_roll.drain(..).collect();
        buffer.take = Some(pre_roll);
    }

    /// Stops collecting and returns the audio since [`begin`](Self::begin)
    /// as 16 kHz mono. Empty if `begin` wasn't called.
    pub fn finish(&self) -> Result<Vec<f32>, AudioError> {
        let take = {
            let mut buffer = self.shared.lock();
            buffer.drain();
            buffer.take.take().unwrap_or_default()
        };
        let mono = to_mono(&take, self.channels);
        resample_mono(&mono, self.sample_rate, STT_SAMPLE_RATE)
    }

    /// Stops collecting and discards the audio.
    pub fn cancel(&self) {
        self.shared.lock().take = None;
    }
}

impl Drop for MicStream {
    fn drop(&mut self) {
        // The stream (and its callbacks) stop when it's dropped after this;
        // the collector just needs telling.
        self.shared.stop.store(true, Ordering::Release);
        if let Some(collector) = self.collector.take() {
            let _ = collector.join();
        }
        let _ = self.stream.pause();
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Buffer> {
        self.buffer.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    mut producer: Producer<f32>,
    shared: &Arc<Shared>,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let failed = Arc::clone(shared);
    device
        .build_input_stream::<T, _, _>(
            *config,
            // Runs on the audio thread: convert and push, nothing else. If
            // the collector fell a whole second behind, drop samples rather
            // than block.
            move |data: &[T], _info| {
                for &sample in data {
                    let _ = producer.push(f32::from_sample(sample));
                }
            },
            move |err| {
                tracing::warn!("microphone stream error: {err}");
                failed.failed.store(true, Ordering::Relaxed);
            },
            None,
        )
        .map_err(|e| AudioError::device("opening microphone", e))
}
