//! The personalized greeting (#343): "Sorry, {name} is not available.
//! Please leave a message after the tone.", spoken once by TTS with the
//! mailbox owner's name and kept as the mailbox's `Personalized` greeting,
//! so a caller never waits on TTS. It is made again when what it was made
//! from changes — the name, the name source, the recorded name, the choice
//! — and removed when the mailbox no longer plays it.
//!
//! The name is the owner's full name, their display name, or the mailbox's
//! recorded name greeting spliced between the two halves of the sentence.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use tracing::{info, warn};

use fcp_core::{
    GreetingChoice, GreetingName, GreetingType, User, VoicemailGreeting, VoicemailMailbox,
};
use fcp_media::tts::TtsService;

/// The sentence's words before the name.
pub const BEFORE_NAME: &str = "Sorry,";
/// The sentence's words after the name.
pub const AFTER_NAME: &str = "is not available. Please leave a message after the tone.";

/// The sentence with `name` in it.
pub fn sentence(name: &str) -> String {
    format!("{BEFORE_NAME} {name} {AFTER_NAME}")
}

/// What a mailbox's personalized greeting is made from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// The whole sentence spoken with this name.
    Spoken(String),
    /// The recorded name greeting between the sentence's halves, with the
    /// name to speak should the recording not be readable.
    Spliced {
        name_audio: PathBuf,
        fallback: Option<String>,
    },
}

/// What the greeting is made from, as kept on it (`made_from`): it is made
/// again when this changes.
pub fn made_from(mailbox: &VoicemailMailbox, source: &Source) -> String {
    match source {
        Source::Spoken(name) => format!("spoken:{}", sentence(name)),
        Source::Spliced { name_audio, .. } => {
            let recorded = mailbox
                .greeting(GreetingType::Name)
                .and_then(|g| g.created_at.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis())
                .unwrap_or_default();
            format!("spliced:{}@{recorded}", name_audio.display())
        }
    }
}

/// What the mailbox's personalized greeting should be made from now;
/// `None` when the mailbox does not play one (its choice is standard or its
/// own greeting, or the owner has no name to say).
pub fn source(mailbox: &VoicemailMailbox, owner: &User) -> Option<Source> {
    if mailbox.effective_choice(owner.full_name().is_some()) != GreetingChoice::Personalized {
        return None;
    }
    let spoken = owner.full_name().or_else(|| owner.shown_name());
    match mailbox.greeting_name {
        GreetingName::FullName => spoken.map(Source::Spoken),
        GreetingName::DisplayName => owner
            .shown_name()
            .or_else(|| owner.full_name())
            .map(Source::Spoken),
        GreetingName::Recorded => match mailbox.greeting(GreetingType::Name) {
            Some(name) => Some(Source::Spliced {
                name_audio: name.audio_path.clone(),
                fallback: spoken,
            }),
            None => spoken.map(Source::Spoken),
        },
    }
}

/// Bring the mailbox's personalized greeting in line with its choice and
/// its owner's name: made (or made again) when it should be there and what
/// it is made from changed, taken off when it should not be there. Whether
/// the mailbox changed — the caller stores it. The file goes in
/// `greetings_dir/<mailbox>/`, a new name each time, so the reaper removing
/// the old file never races the new one.
pub async fn refresh(
    tts: Option<&TtsService>,
    greetings_dir: &Path,
    mailbox: &mut VoicemailMailbox,
    owner: &User,
) -> Result<bool> {
    let current = mailbox.greeting(GreetingType::Personalized);
    let Some(source) = source(mailbox, owner) else {
        if current.is_none() {
            return Ok(false);
        }
        mailbox
            .greetings
            .retain(|g| g.greeting_type != GreetingType::Personalized);
        mailbox.updated_at = SystemTime::now();
        info!(mailbox_id = %mailbox.id, "Personalized greeting taken off");
        return Ok(true);
    };
    let key = made_from(mailbox, &source);
    if current.is_some_and(|g| g.made_from.as_deref() == Some(key.as_str())) {
        return Ok(false);
    }
    let tts = tts.context("no TTS service to speak the personalized greeting")?;
    let samples = match &source {
        Source::Spoken(name) => read_pcm(&tts.synthesize(&sentence(name), None, None).await?)?,
        Source::Spliced {
            name_audio,
            fallback,
        } => match spliced(tts, greetings_dir, name_audio).await {
            Ok(samples) => samples,
            Err(e) => {
                let name = fallback
                    .as_deref()
                    .context("the recorded name is unreadable and the owner has no name")?;
                warn!(mailbox_id = %mailbox.id, error = %e, "Recorded name unreadable; the greeting speaks the name");
                read_pcm(&tts.synthesize(&sentence(name), None, None).await?)?
            }
        },
    };
    let dir = greetings_dir.join(&mailbox.id);
    tokio::fs::create_dir_all(&dir)
        .await
        .context("Failed to create the mailbox's greetings directory")?;
    let path = dir.join(format!(
        "personalized-{}.wav",
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    ));
    write_pcm(&path, &samples)?;
    let mut greeting = VoicemailGreeting::new(GreetingType::Personalized, path);
    greeting.made_from = Some(key);
    mailbox.add_greeting(greeting);
    info!(mailbox_id = %mailbox.id, "Personalized greeting made");
    Ok(true)
}

/// The sample rate the greeting is kept at: the telephone's.
const RATE: u32 = 8000;

/// The two halves spoken, the recorded name between them.
async fn spliced(tts: &TtsService, greetings_dir: &Path, name_audio: &Path) -> Result<Vec<i16>> {
    crate::validate_path_within_dir(name_audio, greetings_dir)
        .context("the recorded name is outside the greetings directory")?;
    let name = read_pcm(name_audio)?;
    let before = read_pcm(&tts.synthesize(BEFORE_NAME, None, None).await?)?;
    let after = read_pcm(&tts.synthesize(AFTER_NAME, None, None).await?)?;
    let gap = vec![0i16; (RATE / 8) as usize];
    Ok([before, gap.clone(), trim_silence(name), gap, after].concat())
}

/// A WAV file as 8 kHz mono 16-bit samples, whatever its rate, width or
/// channels.
pub fn read_pcm(path: &Path) -> Result<Vec<i16>> {
    let mut reader = hound::WavReader::open(path)
        .with_context(|| format!("{} is not a readable WAV file", path.display()))?;
    let spec = reader.spec();
    let samples: Vec<f32> = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Int, 8) => reader
            .samples::<i8>()
            .map(|s| s.map(|s| s as f32 / i8::MAX as f32))
            .collect::<Result<_, _>>()?,
        (hound::SampleFormat::Int, 16) => reader
            .samples::<i16>()
            .map(|s| s.map(|s| s as f32 / i16::MAX as f32))
            .collect::<Result<_, _>>()?,
        (hound::SampleFormat::Int, bits @ (24 | 32)) => {
            let full = (1i64 << (bits - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|s| s as f32 / full))
                .collect::<Result<_, _>>()?
        }
        (hound::SampleFormat::Float, 32) => reader.samples::<f32>().collect::<Result<_, _>>()?,
        (format, bits) => anyhow::bail!("unsupported WAV samples: {format:?} at {bits} bits"),
    };
    let channels = spec.channels.max(1) as usize;
    let mono: Vec<f32> = samples
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect();
    Ok(resample(&mono, spec.sample_rate, RATE)
        .into_iter()
        .map(|s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
        .collect())
}

/// Linear resampling: speech for a phone, not music.
fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.is_empty() || from == 0 {
        return samples.to_vec();
    }
    let len = (samples.len() as u64 * to as u64 / from as u64) as usize;
    (0..len)
        .map(|i| {
            let at = i as f64 * from as f64 / to as f64;
            let j = at.floor() as usize;
            let frac = (at - j as f64) as f32;
            let a = samples[j.min(samples.len() - 1)];
            let b = samples[(j + 1).min(samples.len() - 1)];
            a + (b - a) * frac
        })
        .collect()
}

/// A recorded name without the quiet before and after it.
fn trim_silence(samples: Vec<i16>) -> Vec<i16> {
    const QUIET: i16 = 300;
    let start = samples.iter().position(|s| s.unsigned_abs() > QUIET as u16);
    let end = samples
        .iter()
        .rposition(|s| s.unsigned_abs() > QUIET as u16);
    match (start, end) {
        (Some(start), Some(end)) => samples[start..=end].to_vec(),
        _ => samples,
    }
}

fn write_pcm(path: &Path, samples: &[i16]) -> Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)
        .with_context(|| format!("Failed to write {}", path.display()))?;
    for sample in samples {
        writer.write_sample(*sample)?;
    }
    writer.finalize()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_core::{LineBindingId, TenantId, UserId};

    fn owner(first: Option<&str>, last: Option<&str>, display: Option<&str>) -> User {
        let mut user: User = serde_json::from_value(serde_json::json!({
            "username": "alice", "password": "", "digest_password": null,
            "sip_ha1_hash": null, "display_name": display, "department_id": null,
            "enabled": true, "created_at": 0, "updated_at": 0, "email": null,
            "phone_number": null, "mobile_number": null, "pin": null, "extension": null,
            "department": null, "job_title": null, "location": null, "location_id": null,
            "primary_phone_number_id": null, "voicemail_box_id": null, "profile_picture": null,
        }))
        .expect("a user");
        user.first_name = first.map(str::to_string);
        user.last_name = last.map(str::to_string);
        user
    }

    fn mailbox() -> VoicemailMailbox {
        VoicemailMailbox::new(
            TenantId::nil(),
            UserId::new(),
            LineBindingId::new(),
            "1001".into(),
            "1234".into(),
        )
    }

    #[test]
    fn the_default_says_the_full_name_and_is_standard_without_one() {
        let mailbox = mailbox();
        assert_eq!(
            source(&mailbox, &owner(Some("Alice"), Some("Smith"), Some("Ali"))),
            Some(Source::Spoken("Alice Smith".into()))
        );
        assert_eq!(source(&mailbox, &owner(None, None, Some("Ali"))), None);
        assert_eq!(source(&mailbox, &owner(Some("Alice"), None, None)), None);
    }

    #[test]
    fn a_chosen_personalized_greeting_says_what_name_there_is() {
        let mut mailbox = mailbox();
        mailbox.greeting_choice = Some(GreetingChoice::Personalized);
        assert_eq!(
            source(&mailbox, &owner(None, None, Some("Ali"))),
            Some(Source::Spoken("Ali".into()))
        );
        mailbox.greeting_name = GreetingName::DisplayName;
        assert_eq!(
            source(&mailbox, &owner(Some("Alice"), Some("Smith"), Some("Ali"))),
            Some(Source::Spoken("Ali".into()))
        );
        // Neither name: the display name reads as the first and last.
        assert_eq!(
            source(&mailbox, &owner(Some("Alice"), Some("Smith"), None)),
            Some(Source::Spoken("Alice Smith".into()))
        );
        mailbox.greeting_choice = Some(GreetingChoice::Own);
        assert_eq!(source(&mailbox, &owner(Some("A"), Some("S"), None)), None);
    }

    #[test]
    fn a_recorded_name_is_spliced_and_its_rerecording_makes_it_again() {
        let mut mailbox = mailbox();
        mailbox.greeting_choice = Some(GreetingChoice::Personalized);
        mailbox.greeting_name = GreetingName::Recorded;
        let alice = owner(Some("Alice"), Some("Smith"), None);
        // No recording yet: the name is spoken.
        assert_eq!(
            source(&mailbox, &alice),
            Some(Source::Spoken("Alice Smith".into()))
        );
        mailbox.add_greeting(VoicemailGreeting::new(
            GreetingType::Name,
            PathBuf::from("/g/m/name.wav"),
        ));
        let spliced = source(&mailbox, &alice).unwrap();
        assert!(matches!(spliced, Source::Spliced { .. }));
        let first = made_from(&mailbox, &spliced);
        let mut again = VoicemailGreeting::new(GreetingType::Name, PathBuf::from("/g/m/name.wav"));
        again.created_at = SystemTime::now() + std::time::Duration::from_secs(5);
        mailbox.add_greeting(again);
        assert_ne!(made_from(&mailbox, &spliced), first);
    }

    #[test]
    fn any_wav_is_read_as_telephone_audio() {
        let dir = std::env::temp_dir().join(format!("fcp-vm-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("stereo.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..16000 {
            let s = ((i as f32 / 10.0).sin() * 10000.0) as i16;
            writer.write_sample(s).unwrap();
            writer.write_sample(s).unwrap();
        }
        writer.finalize().unwrap();
        let samples = read_pcm(&path).unwrap();
        assert_eq!(samples.len(), 8000, "a second at 8 kHz");
        assert!(samples.iter().any(|s| s.unsigned_abs() > 5000));
        let _ = std::fs::remove_dir_all(dir);
    }
}
