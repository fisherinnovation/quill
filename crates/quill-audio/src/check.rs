//! A short test recording that confirms Quill can hear both sources.
//!
//! On macOS a missing permission doesn't fail: the stream opens and delivers
//! silence. So the check records a few seconds from the microphone and the
//! system output while playing a quiet chime through the speakers. The system
//! tap records every process, Quill included, so it should hear the chime;
//! silence there means access was denied. A working microphone always picks
//! up some noise, so exact silence there means the same.
//!
//! Opening the devices is also what triggers the macOS permission prompts, so
//! running this during onboarding gets them out of the way before the first
//! meeting.

use std::path::Path;
use std::time::Duration;

use crate::AudioError;
use crate::capture::{self, Source, TrackRequest, TrackSummary};
use crate::cue::{self, Cue};

/// Records both sources into `dir` for `duration`, playing a chime partway
/// through, and returns what each track heard (see [`TrackSummary::peak`]).
/// The caller decides what to do with the files.
pub fn check_access(dir: &Path, duration: Duration) -> Result<Vec<TrackSummary>, AudioError> {
    std::fs::create_dir_all(dir)?;
    let requests = [
        TrackRequest {
            source: Source::Microphone,
            path: dir.join("mic.wav"),
        },
        TrackRequest {
            source: Source::SystemAudio,
            path: dir.join("system.wav"),
        },
    ];
    let recording = capture::start(&requests)?;
    // Give the tap a moment to start before the chime plays.
    std::thread::sleep(Duration::from_millis(300));
    let chime = cue::start(Cue::Chime);
    if let Err(e) = &chime {
        // Not fatal: the microphone result is still useful, and the system
        // track may pick up other audio.
        tracing::warn!("could not play the test chime: {e}");
    }
    std::thread::sleep(duration.saturating_sub(Duration::from_millis(300)));
    drop(chime);
    recording.stop()
}
