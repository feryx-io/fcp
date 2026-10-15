//! WAV files for tests that play a prompt and read back a recording: a tone
//! written as an 8 kHz prompt, and a recording's samples read without
//! trusting sizes a recorder that was never finalised left at zero.

use std::path::Path;

use crate::media::{tone_frame, ToneDetector};

/// Write `ms` of a sine at `hz` as a 16-bit mono 8 kHz WAV file.
pub fn write_tone(path: &Path, hz: f32, ms: u64) -> std::io::Result<()> {
    let mut phase = 0.0f32;
    let mut samples = Vec::new();
    for _ in 0..ms / 20 {
        samples.extend(tone_frame(hz, &mut phase));
    }
    write_pcm(path, 8000, &samples)
}

/// Write `samples` as a 16-bit mono WAV file at `rate`.
pub fn write_pcm(path: &Path, rate: u32, samples: &[i16]) -> std::io::Result<()> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, out)
}

/// A 16-bit mono WAV file's sample rate and samples. The `data` chunk is
/// read to the end of the file whatever size its header gives.
pub fn read_pcm(path: &Path) -> std::io::Result<(u32, Vec<i16>)> {
    let (rate, channels, samples) = read_interleaved(path)?;
    if channels != 1 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "not 16-bit mono",
        ));
    }
    Ok((rate, samples))
}

/// A 16-bit WAV file of any rate and channel count as 8 kHz mono: the
/// channels averaged, the rate divided down by averaging (a crude low-pass,
/// enough for the tones tests listen for). A rate not a multiple of 8 kHz
/// is refused.
pub fn read_as_8k_mono(path: &Path) -> std::io::Result<Vec<i16>> {
    let (rate, channels, samples) = read_interleaved(path)?;
    if rate % 8000 != 0 || channels == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("{rate} Hz, {channels} channels"),
        ));
    }
    let step = (rate / 8000) as usize * channels as usize;
    Ok(samples
        .chunks_exact(step)
        .map(|frame| (frame.iter().map(|&s| s as i32).sum::<i32>() / step as i32) as i16)
        .collect())
}

/// A 16-bit WAV file's rate, channel count and interleaved samples.
fn read_interleaved(path: &Path) -> std::io::Result<(u32, u16, Vec<i16>)> {
    let bytes = std::fs::read(path)?;
    let bad = |why: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, why.to_string());
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(bad("not a WAV file"));
    }
    let mut at = 12;
    let mut rate = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = at + 8;
        if id == b"fmt " && body + 16 <= bytes.len() {
            let channels = u16::from_le_bytes([bytes[body + 2], bytes[body + 3]]);
            let bits = u16::from_le_bytes([bytes[body + 14], bytes[body + 15]]);
            if bits != 16 {
                return Err(bad("not 16-bit"));
            }
            rate = Some((
                u32::from_le_bytes(bytes[body + 4..body + 8].try_into().unwrap()),
                channels,
            ));
        } else if id == b"data" {
            let (rate, channels) = rate.ok_or_else(|| bad("data before fmt"))?;
            let samples = bytes[body..]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| i16::from_le_bytes(*b))
                .collect();
            return Ok((rate, channels, samples));
        }
        at = body + len + (len & 1);
    }
    Err(bad("no data chunk"))
}

/// Whether `hz` sounds in 8 kHz `samples`, as a phone would hear it.
pub fn contains_tone(samples: &[i16], hz: u32) -> bool {
    let mut detector = ToneDetector::new(hz);
    for frame in samples.chunks(160) {
        detector.feed(frame);
    }
    detector.heard()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_48k_stereo_recording_reads_as_8k_mono() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.wav");
        // 400 ms of 1 kHz at 48 kHz, the same in both channels.
        let mono: Vec<i16> = (0..19_200)
            .map(|i| {
                ((i as f32 * 2.0 * std::f32::consts::PI * 1000.0 / 48_000.0).sin() * 8000.0) as i16
            })
            .collect();
        let stereo: Vec<i16> = mono.iter().flat_map(|&s| [s, s]).collect();
        write_pcm(&path, 48_000, &stereo).unwrap();
        // Re-mark the header as two channels: write_pcm writes mono.
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[22..24].copy_from_slice(&2u16.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        let samples = read_as_8k_mono(&path).unwrap();
        assert_eq!(samples.len(), 3200);
        assert!(contains_tone(&samples, 1000));
    }

    #[test]
    fn a_written_tone_reads_back_and_is_heard() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.wav");
        write_tone(&path, 1000.0, 400).unwrap();
        let (rate, samples) = read_pcm(&path).unwrap();
        assert_eq!(rate, 8000);
        assert_eq!(samples.len(), 3200);
        assert!(contains_tone(&samples, 1000));
        assert!(!contains_tone(&samples, 440));

        // A header whose sizes were never written still reads.
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[40..44].copy_from_slice(&0u32.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(read_pcm(&path).unwrap().1.len(), 3200);
    }
}
