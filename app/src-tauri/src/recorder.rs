//! Owns the active recording.
//!
//! A recording runs on its own thread for its whole life: cpal streams
//! aren't guaranteed to be movable between threads on every platform, and
//! Tauri state must be. The app talks to that thread through channels.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use quill_audio::capture::{self, Source, TrackRequest, TrackSummary};
use quill_core::meeting::{MIC_FILE, SYSTEM_FILE};

pub struct ActiveRecording {
    pub meeting_id: String,
    pub started: Instant,
    stop: mpsc::Sender<()>,
    thread: JoinHandle<Result<Vec<TrackSummary>, String>>,
    /// Shared with the track writers: set, they discard what arrives.
    paused: Arc<AtomicBool>,
    /// When the current pause began, and how long earlier ones lasted.
    paused_since: Option<Instant>,
    paused_for: Duration,
}

impl ActiveRecording {
    /// Starts recording mic + system audio into `dir`. Blocks until the
    /// devices are open (which, on macOS, may wait on a permission prompt).
    pub fn start(meeting_id: String, dir: PathBuf) -> Result<Self, String> {
        let (started_tx, started_rx) = mpsc::channel::<Result<Arc<AtomicBool>, String>>();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();

        let thread = std::thread::Builder::new()
            .name("quill-recorder".into())
            .spawn(move || {
                let requests = [
                    TrackRequest {
                        source: Source::Microphone,
                        path: dir.join(MIC_FILE),
                    },
                    TrackRequest {
                        source: Source::SystemAudio,
                        path: dir.join(SYSTEM_FILE),
                    },
                ];
                let recording = match capture::start(&requests) {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = started_tx.send(Err(e.to_string()));
                        return Err(e.to_string());
                    }
                };
                let _ = started_tx.send(Ok(recording.pause_flag()));
                // Wait for stop (or the app dropping the sender on quit).
                let _ = stop_rx.recv();
                recording.stop().map_err(|e| e.to_string())
            })
            .map_err(|e| e.to_string())?;

        match started_rx.recv() {
            Ok(Ok(paused)) => Ok(Self {
                meeting_id,
                started: Instant::now(),
                stop: stop_tx,
                thread,
                paused,
                paused_since: None,
                paused_for: Duration::ZERO,
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("recorder thread exited unexpectedly".into()),
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused_since.is_some()
    }

    /// Pauses or resumes: while paused nothing is written and the clock
    /// stops.
    pub fn set_paused(&mut self, paused: bool) {
        match (paused, self.paused_since) {
            (true, None) => {
                self.paused.store(true, Ordering::Release);
                self.paused_since = Some(Instant::now());
            }
            (false, Some(since)) => {
                self.paused.store(false, Ordering::Release);
                self.paused_for += since.elapsed();
                self.paused_since = None;
            }
            _ => {}
        }
    }

    /// Time recorded, pauses left out.
    pub fn elapsed(&self) -> Duration {
        let paused = self.paused_for + self.paused_since.map_or(Duration::ZERO, |s| s.elapsed());
        self.started.elapsed().saturating_sub(paused)
    }

    /// Stops recording and waits for the files to be finalized.
    pub fn stop(self) -> Result<Vec<TrackSummary>, String> {
        let _ = self.stop.send(());
        self.thread
            .join()
            .map_err(|_| "recorder thread panicked".to_string())?
    }
}
