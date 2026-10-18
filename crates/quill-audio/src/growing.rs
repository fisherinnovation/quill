//! Reading a WAV file while [`crate::capture`] is still writing it, for
//! live meeting notes. Nothing on the capture side changes: the recorder
//! writes as usual and this follows the file from the side.
//!
//! While recording, the header's sizes aren't filled in yet (they are on
//! `finalize`), so the data runs to the end of the file. Only whole frames
//! are returned; a partly written one is picked up next time.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::{AudioError, resample_mono, to_mono};

/// Follows a growing 32-bit float WAV file.
pub struct GrowingWav {
    path: PathBuf,
    /// Set once the header has been read.
    format: Option<Format>,
    /// Bytes of sample data already returned.
    read: u64,
}

#[derive(Debug, Clone, Copy)]
struct Format {
    data_start: u64,
    channels: u16,
    sample_rate: u32,
}

impl GrowingWav {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            format: None,
            read: 0,
        }
    }

    /// The audio added since the last call, as 16 kHz mono. Empty while the
    /// file doesn't exist yet or has nothing new.
    pub fn read_new(&mut self) -> Result<Vec<f32>, AudioError> {
        let Ok(mut file) = File::open(&self.path) else {
            return Ok(Vec::new());
        };
        let format = match self.format {
            Some(f) => f,
            None => match header(&mut file)? {
                Some(f) => *self.format.insert(f),
                None => return Ok(Vec::new()),
            },
        };
        let frame = u64::from(format.channels) * 4;
        let len = file.metadata()?.len();
        let available = len.saturating_sub(format.data_start + self.read) / frame * frame;
        if available == 0 {
            return Ok(Vec::new());
        }
        file.seek(SeekFrom::Start(format.data_start + self.read))?;
        let mut bytes = vec![0_u8; usize::try_from(available).unwrap_or(0)];
        file.read_exact(&mut bytes)?;
        self.read += available;
        let samples: Vec<f32> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        let mono = to_mono(&samples, format.channels);
        resample_mono(&mono, format.sample_rate, crate::STT_SAMPLE_RATE)
    }
}

/// The format and where the samples start, once the header is complete.
fn header(file: &mut File) -> Result<Option<Format>, AudioError> {
    let mut riff = [0_u8; 12];
    if file.read_exact(&mut riff).is_err() {
        return Ok(None);
    }
    if &riff[0..4] != b"RIFF" || &riff[8..12] != b"WAVE" {
        return Err(AudioError::UnsupportedFormat("not a WAV file".into()));
    }
    let mut pos = 12_u64;
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    loop {
        let mut chunk = [0_u8; 8];
        if file.read_exact(&mut chunk).is_err() {
            return Ok(None);
        }
        let size = u64::from(u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]));
        pos += 8;
        match &chunk[0..4] {
            b"fmt " => {
                let mut body = vec![0_u8; usize::try_from(size).unwrap_or(0)];
                if file.read_exact(&mut body).is_err() || body.len() < 16 {
                    return Ok(None);
                }
                let tag = u16::from_le_bytes([body[0], body[1]]);
                let channels = u16::from_le_bytes([body[2], body[3]]);
                let rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
                let bits = u16::from_le_bytes([body[14], body[15]]);
                fmt = Some((tag, channels, rate, bits));
            }
            b"data" => {
                let Some((tag, channels, sample_rate, bits)) = fmt else {
                    return Err(AudioError::UnsupportedFormat(
                        "WAV data before its format".into(),
                    ));
                };
                // 3 is IEEE float; 0xFFFE (extensible) is how float WAVs
                // with a channel mask say so.
                if bits != 32 || !matches!(tag, 3 | 0xFFFE) || channels == 0 {
                    return Err(AudioError::UnsupportedFormat(format!(
                        "live reading needs 32-bit float WAV, not format {tag} at {bits} bits"
                    )));
                }
                return Ok(Some(Format {
                    data_start: pos,
                    channels,
                    sample_rate,
                }));
            }
            _ => {
                file.seek(SeekFrom::Current(i64::try_from(size).unwrap_or(0)))?;
            }
        }
        // Chunks are padded to even sizes.
        pos += size + size % 2;
        if size % 2 == 1 {
            file.seek(SeekFrom::Current(1))?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hound::{SampleFormat, WavSpec, WavWriter};

    #[test]
    fn follows_a_file_as_it_grows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("track.wav");
        let mut live = GrowingWav::new(&path);
        assert!(live.read_new().unwrap().is_empty(), "no file yet");

        let spec = WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: SampleFormat::Float,
        };
        let mut writer = WavWriter::create(&path, spec).unwrap();
        for _ in 0..48_000 {
            writer.write_sample(0.5_f32).unwrap();
            writer.write_sample(0.5_f32).unwrap();
        }
        writer.flush().unwrap();
        // One second, as 16 kHz mono.
        let first = live.read_new().unwrap();
        assert!((15_900..=16_100).contains(&first.len()), "{}", first.len());
        assert!(live.read_new().unwrap().is_empty(), "nothing new");

        for _ in 0..24_000 {
            writer.write_sample(0.25_f32).unwrap();
            writer.write_sample(0.25_f32).unwrap();
        }
        writer.finalize().unwrap();
        let second = live.read_new().unwrap();
        assert!((7_900..=8_100).contains(&second.len()), "{}", second.len());
    }
}
