//! Recording the microphone and system audio to WAV files.
//!
//! Both sources go through `cpal`, which hides the OS differences:
//! - **Microphone:** the default input device.
//! - **System audio:** an input stream opened on the default *output* device.
//!   On macOS 14.6+ cpal creates a Core Audio process tap for this; on
//!   Windows it uses WASAPI loopback.
//!
//! Each track gets two threads: the OS audio thread runs our callback, which
//! only converts samples to `f32` and pushes them into a lock-free ring
//! buffer; a writer thread drains that buffer into a WAV file. The audio
//! thread never locks, allocates or touches the disk, so it can't glitch
//! because the disk was slow.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, StreamConfig};
use hound::{WavSpec, WavWriter};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::AudioError;

/// What to record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The default microphone: "Me" in a meeting.
    Microphone,
    /// Everything playing through the default output device: "Others".
    SystemAudio,
}

/// One track to record and where to write it.
#[derive(Debug, Clone)]
pub struct TrackRequest {
    pub source: Source,
    pub path: PathBuf,
}

/// What happened on one track, returned by [`Recording::stop`].
#[derive(Debug, Clone)]
pub struct TrackSummary {
    pub source: Source,
    pub path: PathBuf,
    pub device: String,
    pub sample_rate: u32,
    pub channels: u16,
    /// Frames written, including the silent lead-in.
    pub frames: u64,
    /// Silence prepended so this track lines up with the session start.
    pub lead_in_ms: u64,
    /// Samples lost because the writer fell behind (should be 0).
    pub dropped_samples: u64,
    /// Loudest sample, 0.0..=1.0. Exactly 0.0 usually means a missing
    /// permission: macOS delivers silence instead of an error.
    pub peak: f32,
}

impl TrackSummary {
    pub fn duration_secs(&self) -> f64 {
        self.frames as f64 / f64::from(self.sample_rate)
    }
}

/// An in-progress recording. Call [`Recording::stop`] to finish the files;
/// dropping it without stopping leaves them truncated.
pub struct Recording {
    tracks: Vec<ActiveTrack>,
    /// While set, every track's writer discards what arrives (see
    /// [`Recording::pause_flag`]).
    paused: Arc<AtomicBool>,
}

struct ActiveTrack {
    source: Source,
    path: PathBuf,
    device: String,
    sample_rate: u32,
    channels: u16,
    // `cpal::Stream` stops the audio callbacks when dropped.
    stream: cpal::Stream,
    stop: Arc<AtomicBool>,
    dropped: Arc<AtomicU64>,
    writer: JoinHandle<Result<WriterStats, AudioError>>,
}

/// Sentinel for "no audio has arrived yet" in the first-sample timestamp.
const NOT_STARTED: u64 = u64::MAX;

/// Starts recording every requested track.
///
/// All tracks share one start instant. Each track's first buffer is stamped
/// relative to it and the writer prepends that much silence, so sample 0 of
/// every file is the same moment in time. (Device clocks can still drift
/// apart by a few ms per minute; fine for labelling who spoke when.)
pub fn start(requests: &[TrackRequest]) -> Result<Recording, AudioError> {
    let host = cpal::default_host();
    // Set just before the streams start, not now: opening a device can block
    // for as long as a macOS permission prompt is waiting, and that wait must
    // not turn into leading silence. `OnceLock::get` is a plain atomic load,
    // so the audio callback can read it without locking.
    let session_start = Arc::new(OnceLock::new());
    let paused = Arc::new(AtomicBool::new(false));

    let mut tracks = Vec::with_capacity(requests.len());
    for request in requests {
        tracks.push(open_track(
            &host,
            request,
            Arc::clone(&session_start),
            Arc::clone(&paused),
        )?);
    }
    // Start the streams only once all are built, so they begin together.
    let _ = session_start.set(Instant::now());
    for track in &tracks {
        track
            .stream
            .play()
            .map_err(|e| AudioError::device("starting stream", e))?;
    }
    Ok(Recording { tracks, paused })
}

impl Recording {
    /// Pauses (and resumes) every track. While paused the audio still
    /// arrives but isn't written, so the files skip that stretch and stay
    /// aligned with each other (to within a writer's 20 ms poll).
    pub fn pause_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.paused)
    }

    /// Stops all tracks, waits for the writers to flush and returns a summary
    /// per track.
    pub fn stop(self) -> Result<Vec<TrackSummary>, AudioError> {
        let mut summaries = Vec::with_capacity(self.tracks.len());
        for track in self.tracks {
            // Order matters: stop the callbacks first, then tell the writer it
            // can finish once the ring buffer is empty.
            drop(track.stream);
            track.stop.store(true, Ordering::Release);
            let stats = track
                .writer
                .join()
                .map_err(|_| AudioError::WriterPanicked)??;
            summaries.push(TrackSummary {
                source: track.source,
                path: track.path,
                device: track.device,
                sample_rate: track.sample_rate,
                channels: track.channels,
                frames: stats.frames,
                lead_in_ms: stats.lead_in_frames * 1000 / u64::from(track.sample_rate),
                dropped_samples: track.dropped.load(Ordering::Relaxed),
                peak: stats.peak,
            });
        }
        Ok(summaries)
    }
}

fn open_track(
    host: &cpal::Host,
    request: &TrackRequest,
    session_start: Arc<OnceLock<Instant>>,
    paused: Arc<AtomicBool>,
) -> Result<ActiveTrack, AudioError> {
    let (device, supported) = match request.source {
        Source::Microphone => {
            let device = host
                .default_input_device()
                .ok_or(AudioError::NoDevice("microphone"))?;
            let config = device
                .default_input_config()
                .map_err(|e| AudioError::device("reading microphone format", e))?;
            (device, config)
        }
        Source::SystemAudio => {
            let device = host
                .default_output_device()
                .ok_or(AudioError::NoDevice("audio output"))?;
            let config = device
                .default_output_config()
                .map_err(|e| AudioError::device("reading output format", e))?;
            (device, config)
        }
    };
    let device_name = device.to_string();
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let sample_rate = config.sample_rate;
    let channels = config.channels;

    // 10 seconds of headroom: the writer would have to stall that long
    // before we drop audio.
    let capacity = sample_rate as usize * usize::from(channels) * 10;
    let (producer, consumer) = RingBuffer::<f32>::new(capacity);
    let first_sample_at = Arc::new(AtomicU64::new(NOT_STARTED));
    let dropped = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));

    let callback = CallbackState {
        producer,
        first_sample_at: Arc::clone(&first_sample_at),
        dropped: Arc::clone(&dropped),
        session_start,
        frames_per_sec: f64::from(sample_rate),
        channels: usize::from(channels),
    };
    // cpal hands us samples in the device's native format; build the stream
    // for that type and convert to f32 inside the callback.
    let stream = match sample_format {
        SampleFormat::F32 => build_stream::<f32>(&device, &config, callback),
        SampleFormat::I16 => build_stream::<i16>(&device, &config, callback),
        SampleFormat::I32 => build_stream::<i32>(&device, &config, callback),
        SampleFormat::U16 => build_stream::<u16>(&device, &config, callback),
        other => {
            return Err(AudioError::UnsupportedFormat(format!(
                "device format {other}"
            )));
        }
    }?;

    let spec = WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let writer = {
        let path = request.path.clone();
        let stop = Arc::clone(&stop);
        std::thread::Builder::new()
            .name(format!("quill-wav-{:?}", request.source))
            .spawn(move || write_wav(&path, spec, consumer, &first_sample_at, &stop, &paused))?
    };

    tracing::info!(
        source = ?request.source,
        device = %device_name,
        sample_rate,
        channels,
        "recording"
    );
    Ok(ActiveTrack {
        source: request.source,
        path: request.path.clone(),
        device: device_name,
        sample_rate,
        channels,
        stream,
        stop,
        dropped,
        writer,
    })
}

/// Everything the audio callback owns. It is moved into the callback closure,
/// so the audio thread never shares it with anyone.
struct CallbackState {
    producer: Producer<f32>,
    first_sample_at: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
    session_start: Arc<OnceLock<Instant>>,
    frames_per_sec: f64,
    channels: usize,
}

impl CallbackState {
    /// Runs on the OS audio thread: no locks, no allocation, no I/O.
    //
    // `f32: FromSample<T>` is a trait bound on the *target* type: it means
    // "there is a conversion from T into f32", which is what
    // `f32::from_sample` needs. cpal provides it for every sample format.
    fn on_data<T>(&mut self, data: &[T])
    where
        T: SizedSample,
        f32: FromSample<T>,
    {
        // Audio from before the session started (if a backend delivers any
        // before `play`) isn't part of the recording.
        let Some(session_start) = self.session_start.get() else {
            return;
        };
        if self.first_sample_at.load(Ordering::Relaxed) == NOT_STARTED {
            // The buffer's first sample was captured roughly one buffer
            // length before this callback ran.
            let buffer_secs = (data.len() / self.channels.max(1)) as f64 / self.frames_per_sec;
            let at = session_start
                .elapsed()
                .saturating_sub(Duration::from_secs_f64(buffer_secs));
            let nanos = u64::try_from(at.as_nanos()).unwrap_or(0);
            // Release pairs with the writer's Acquire load: the writer can't
            // see these samples without also seeing this timestamp.
            self.first_sample_at.store(nanos, Ordering::Release);
        }
        for &sample in data {
            if self.producer.push(f32::from_sample(sample)).is_err() {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    mut state: CallbackState,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device
        .build_input_stream::<T, _, _>(
            *config,
            move |data: &[T], _info| state.on_data(data),
            |err| tracing::error!("audio stream error: {err}"),
            None,
        )
        .map_err(|e| AudioError::device("opening stream", e))
}

struct WriterStats {
    frames: u64,
    lead_in_frames: u64,
    peak: f32,
}

/// Writer thread: drains the ring buffer into a 32-bit float WAV until told
/// to stop and the buffer is empty.
fn write_wav(
    path: &Path,
    spec: WavSpec,
    mut consumer: Consumer<f32>,
    first_sample_at: &AtomicU64,
    stop: &AtomicBool,
    paused: &AtomicBool,
) -> Result<WriterStats, AudioError> {
    let channels = u64::from(spec.channels);
    let mut wav = WavWriter::create(path, spec)?;
    let mut samples_written = 0_u64;
    let mut lead_in_frames = 0_u64;
    let mut started = false;
    let mut peak = 0.0_f32;

    loop {
        let available = consumer.slots();
        if available == 0 {
            if stop.load(Ordering::Acquire) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }

        if !started {
            // Pad with silence so this file starts at the session start.
            let nanos = first_sample_at.load(Ordering::Acquire);
            if nanos != NOT_STARTED {
                lead_in_frames = nanos * u64::from(spec.sample_rate) / 1_000_000_000;
                for _ in 0..lead_in_frames * channels {
                    wav.write_sample(0.0_f32)?;
                }
                samples_written += lead_in_frames * channels;
            }
            started = true;
        }

        // `read_chunk` borrows the readable region (possibly two slices,
        // because the buffer wraps around); `commit_all` frees it for the
        // audio thread to reuse.
        let chunk = consumer
            .read_chunk(available)
            .map_err(|e| AudioError::Capture(e.to_string()))?;
        // Paused: take the audio off the buffer, but don't keep it.
        if paused.load(Ordering::Acquire) {
            chunk.commit_all();
            continue;
        }
        let (first, second) = chunk.as_slices();
        for &sample in first.iter().chain(second) {
            wav.write_sample(sample)?;
            peak = peak.max(sample.abs());
        }
        samples_written += (first.len() + second.len()) as u64;
        chunk.commit_all();
    }

    wav.finalize()?;
    Ok(WriterStats {
        frames: samples_written / channels.max(1),
        lead_in_frames,
        peak,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn callback_state(capacity: usize) -> (CallbackState, Consumer<f32>) {
        let (producer, consumer) = RingBuffer::new(capacity);
        let state = CallbackState {
            producer,
            first_sample_at: Arc::new(AtomicU64::new(NOT_STARTED)),
            dropped: Arc::new(AtomicU64::new(0)),
            session_start: Arc::new(OnceLock::from(Instant::now())),
            frames_per_sec: 1000.0,
            channels: 2,
        };
        (state, consumer)
    }

    #[test]
    fn callback_converts_stamps_and_counts_drops() {
        let (mut state, mut consumer) = callback_state(4);
        state.on_data::<i16>(&[i16::MIN, 0, 16_384, 0, 1, 2]);

        // Converted to f32; the two samples that didn't fit were dropped.
        let chunk = consumer.read_chunk(4).unwrap();
        let (a, b) = chunk.as_slices();
        assert_eq!([a, b].concat(), vec![-1.0, 0.0, 0.5, 0.0]);
        assert_eq!(state.dropped.load(Ordering::Relaxed), 2);
        // The first buffer got a start time.
        assert_ne!(state.first_sample_at.load(Ordering::Relaxed), NOT_STARTED);
    }

    #[test]
    fn a_paused_writer_drains_but_keeps_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("track.wav");
        let spec = WavSpec {
            channels: 1,
            sample_rate: 1000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let (mut producer, consumer) = RingBuffer::new(16);
        for s in [0.25, -0.5, 0.75] {
            producer.push(s).unwrap();
        }
        let stats = write_wav(
            &path,
            spec,
            consumer,
            &AtomicU64::new(0),
            &AtomicBool::new(true),
            &AtomicBool::new(true),
        )
        .unwrap();
        assert_eq!(stats.frames, 0);
        assert_eq!(hound::WavReader::open(&path).unwrap().len(), 0);
        // The buffer was emptied, so the audio thread never stalls.
        assert_eq!(producer.slots(), 16);
    }

    #[test]
    fn writer_pads_lead_in_and_writes_all_samples() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("track.wav");
        let spec = WavSpec {
            channels: 2,
            sample_rate: 1000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let (mut producer, consumer) = RingBuffer::new(16);
        for s in [0.25, -0.5, 0.75, -1.0] {
            producer.push(s).unwrap();
        }
        // First sample arrived 5 ms after the session started → 5 frames of
        // silence at 1 kHz. Stop is already set, so the writer drains and exits.
        let first = AtomicU64::new(5_000_000);
        let stop = AtomicBool::new(true);

        let stats = write_wav(
            &path,
            spec,
            consumer,
            &first,
            &stop,
            &AtomicBool::new(false),
        )
        .unwrap();

        assert_eq!(stats.lead_in_frames, 5);
        assert_eq!(stats.frames, 7);
        assert_eq!(stats.peak, 1.0);
        let samples: Vec<f32> = hound::WavReader::open(&path)
            .unwrap()
            .into_samples()
            .map(Result::unwrap)
            .collect();
        assert_eq!(samples.len(), 14);
        assert!(samples[..10].iter().all(|&s| s == 0.0));
        assert_eq!(&samples[10..], &[0.25, -0.5, 0.75, -1.0]);
    }
}
