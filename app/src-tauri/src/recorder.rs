//! Owns the active recording.
//!
//! A recording runs on its own thread for its whole life: cpal streams
//! aren't guaranteed to be movable between threads on every platform, and
//! Tauri state must be. The app talks to that thread through channels.

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Instant;

use quill_audio::capture::{self, Source, TrackRequest, TrackSummary};
use quill_core::meeting::{MIC_FILE, SYSTEM_FILE};

pub struct ActiveRecording {
    pub meeting_id: String,
    pub started: Instant,
    stop: mpsc::Sender<()>,
    thread: JoinHandle<Result<Vec<TrackSummary>, String>>,
}

impl ActiveRecording {
    /// Starts recording mic + system audio into `dir`. Blocks until the
    /// devices are open (which, on macOS, may wait on a permission prompt).
    pub fn start(meeting_id: String, dir: PathBuf) -> Result<Self, String> {
        let (started_tx, started_rx) = mpsc::channel::<Result<(), String>>();
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
                let _ = started_tx.send(Ok(()));
                // Wait for stop (or the app dropping the sender on quit).
                let _ = stop_rx.recv();
                recording.stop().map_err(|e| e.to_string())
            })
            .map_err(|e| e.to_string())?;

        match started_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                meeting_id,
                started: Instant::now(),
                stop: stop_tx,
                thread,
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("recorder thread exited unexpectedly".into()),
        }
    }

    /// Stops recording and waits for the files to be finalized.
    pub fn stop(self) -> Result<Vec<TrackSummary>, String> {
        let _ = self.stop.send(());
        self.thread
            .join()
            .map_err(|_| "recorder thread panicked".to_string())?
    }
}
