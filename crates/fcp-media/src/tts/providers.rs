//! TTS provider implementations.

use super::{TtsProvider, TtsProviderConfig};
use anyhow::{Context, Result};
use std::sync::Arc;

pub mod aws_polly;
pub mod azure;
pub mod elevenlabs;
pub mod google_cloud;
pub mod local;
pub mod openai;
pub mod piper;
pub mod remote;

/// Create a TTS provider from configuration.
pub async fn create_provider(config: TtsProviderConfig) -> Result<Arc<dyn TtsProvider>> {
    match config {
        TtsProviderConfig::GoogleCloud { api_key } => {
            let provider = google_cloud::GoogleCloudTts::new(api_key)?;
            Ok(Arc::new(provider))
        }

        TtsProviderConfig::AwsPolly {
            region,
            access_key_id,
            secret_access_key,
            engine,
            endpoint,
        } => {
            let provider = aws_polly::AwsPollyTts::new(
                region,
                access_key_id,
                secret_access_key,
                engine,
                endpoint,
            )?;
            Ok(Arc::new(provider))
        }

        TtsProviderConfig::Azure {
            subscription_key,
            region,
            endpoint,
        } => {
            let provider = azure::AzureTts::new(subscription_key, region, endpoint)?;
            Ok(Arc::new(provider))
        }

        TtsProviderConfig::OpenAi { api_key, model } => {
            let provider = openai::OpenAiTts::new(api_key, model)?;
            Ok(Arc::new(provider))
        }

        TtsProviderConfig::ElevenLabs {
            api_key,
            model_id,
            endpoint,
        } => {
            let provider = elevenlabs::ElevenLabsTts::new(api_key, model_id, endpoint)?;
            Ok(Arc::new(provider))
        }

        TtsProviderConfig::Local {
            engine,
            executable_path,
            device,
        } => {
            let provider = local::LocalTts::new(engine, executable_path, device)?;
            // Eagerly start the worker so the model is pre-loaded and ready
            provider.warm_up().await?;
            Ok(Arc::new(provider))
        }

        TtsProviderConfig::Piper {
            binary,
            voices_dir,
            voice,
        } => {
            let provider = piper::PiperTts::new(binary, voices_dir, voice)?;
            // The default voice loaded before the first prompt.
            provider.warm_up().await?;
            Ok(Arc::new(provider))
        }

        TtsProviderConfig::Remote {
            url,
            auth_token,
            provider,
            tenant_id,
        } => {
            let provider = remote::RemoteTtsProvider::new(url, auth_token, provider, tenant_id)?;
            Ok(Arc::new(provider))
        }
    }
}

/// The rate every provider's audio is returned at.
pub(crate) const OUTPUT_RATE: u32 = 8000;

/// A WAV at any rate and channel count as 8 kHz mono 16-bit WAV.
pub(crate) fn wav_to_8k(bytes: &[u8]) -> Result<Vec<u8>> {
    let reader = hound::WavReader::new(std::io::Cursor::new(bytes)).context("reading the WAV")?;
    let spec = reader.spec();
    let channels = usize::from(spec.channels.max(1));
    let samples: Vec<i16> = reader
        .into_samples::<i16>()
        .collect::<std::result::Result<_, _>>()
        .context("reading the WAV's samples")?;
    let mono: Vec<i16> = samples
        .chunks(channels)
        .map(|c| (c.iter().map(|&s| i32::from(s)).sum::<i32>() / c.len() as i32) as i16)
        .collect();
    mono_to_8k_wav(mono, spec.sample_rate)
}

/// Raw 16-bit little-endian mono PCM at `rate` (what Polly and
/// ElevenLabs send) as 8 kHz WAV.
pub(crate) fn pcm_to_8k_wav(bytes: &[u8], rate: u32) -> Result<Vec<u8>> {
    let (samples, rest) = bytes.as_chunks::<2>();
    if !rest.is_empty() {
        anyhow::bail!("{} bytes of 16-bit PCM is not a whole sample", bytes.len());
    }
    let mono = samples.iter().map(|b| i16::from_le_bytes(*b)).collect();
    mono_to_8k_wav(mono, rate)
}

fn mono_to_8k_wav(mono: Vec<i16>, rate: u32) -> Result<Vec<u8>> {
    let pcm = if rate == OUTPUT_RATE {
        mono
    } else {
        forge_resampler::resample_all(&mono, rate, OUTPUT_RATE, 1)
            .map_err(|e| anyhow::anyhow!("resampling {rate} Hz: {e}"))?
    };
    let mut out = std::io::Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(
        &mut out,
        hound::WavSpec {
            channels: 1,
            sample_rate: OUTPUT_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    for s in pcm {
        writer.write_sample(s)?;
    }
    writer.finalize()?;
    Ok(out.into_inner())
}

/// A vendor's error, its status and body, for the log and the caller.
pub(crate) async fn vendor_error(vendor: &str, response: reqwest::Response) -> anyhow::Error {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let body: String = body.chars().take(500).collect();
    anyhow::anyhow!("{vendor} answered {status}: {body}")
}

/// `en-US` → `en`.
pub(crate) fn language_part(language: &str) -> &str {
    language.split(['-', '_']).next().unwrap_or(language)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_at_16k_comes_back_as_a_wav_at_8k() {
        let pcm: Vec<u8> = (0..16_000)
            .flat_map(|i| (((i as f32 * 0.05).sin() * 8000.0) as i16).to_le_bytes())
            .collect();
        let wav = pcm_to_8k_wav(&pcm, 16_000).unwrap();
        let reader = hound::WavReader::new(std::io::Cursor::new(wav)).unwrap();
        assert_eq!(reader.spec().sample_rate, 8000);
        assert_eq!(reader.spec().channels, 1);
        let n = reader.len() as i64;
        assert!((n - 8000).abs() < 200, "a second at 8 kHz: {n} samples");
        assert!(pcm_to_8k_wav(&[0, 0, 0], 8000).is_err());
    }

    #[test]
    fn the_language_part_of_a_tag() {
        assert_eq!(language_part("en-US"), "en");
        assert_eq!(language_part("pt_BR"), "pt");
        assert_eq!(language_part("fr"), "fr");
    }
}
