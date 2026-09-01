//! Queue position announcement audio.
//!
//! Loads WAV files from a configured directory, decodes them to PCM,
//! then pre-encodes into RTP-sized frames for each supported codec
//! (PCMU, PCMA, G.722). At runtime the MOH loop can play announcements
//! without any encoding latency.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use forge_codecs::g711;
use tracing::{debug, info, warn};

/// Frame size in samples for narrowband codecs (8kHz, 20ms).
const NB_FRAME_SAMPLES: usize = 160;
/// Frame size in samples for G.722 (16kHz input, 20ms).
const G722_FRAME_SAMPLES: usize = 320;

/// Pre-encoded position and ETA announcements for a single language.
struct PerLanguageCache {
    /// Position number → pre-encoded RTP-payload frames (PCMU).
    pcmu: HashMap<usize, Vec<Vec<u8>>>,
    /// Position number → pre-encoded RTP-payload frames (PCMA).
    pcma: HashMap<usize, Vec<Vec<u8>>>,
    /// Position number → pre-encoded RTP-payload frames (G.722).
    g722: HashMap<usize, Vec<Vec<u8>>>,
    /// ETA minutes → pre-encoded RTP-payload frames (PCMU).
    eta_pcmu: HashMap<usize, Vec<Vec<u8>>>,
    /// ETA minutes → pre-encoded RTP-payload frames (PCMA).
    eta_pcma: HashMap<usize, Vec<Vec<u8>>>,
    /// ETA minutes → pre-encoded RTP-payload frames (G.722).
    eta_g722: HashMap<usize, Vec<Vec<u8>>>,
}

/// Load announcement audio from a directory and build a PerLanguageCache.
///
/// Expected files in `dir`:
/// - `position.wav` — "You are caller number"
/// - `digits/1.wav` through `digits/20.wav`
/// - `digits/30.wav`, `digits/40.wav`, `digits/50.wav`, `digits/60.wav`
/// - `eta.wav` and `minutes.wav` (optional, for ETA announcements)
fn load_language_cache(dir: &Path) -> Option<PerLanguageCache> {
    let prefix_path = dir.join("position.wav");
    let prefix_pcm = load_wav_pcm(&prefix_path)?;

    let mut digit_pcm: HashMap<usize, Vec<i16>> = HashMap::new();
    let digits_dir = dir.join("digits");
    if digits_dir.exists() {
        for n in (1..=20).chain([30, 40, 50, 60].iter().copied()) {
            let path = digits_dir.join(format!("{}.wav", n));
            if let Some(samples) = load_wav_pcm(&path) {
                digit_pcm.insert(n, samples);
            }
        }
    }

    let mut pcmu = HashMap::new();
    let mut pcma = HashMap::new();
    let mut g722 = HashMap::new();

    for pos in 1..=50 {
        let number_samples = compose_number_pcm(pos, &digit_pcm);
        if number_samples.is_empty() {
            continue;
        }
        let mut full_pcm = prefix_pcm.clone();
        full_pcm.extend_from_slice(&[0i16; 800]);
        full_pcm.extend_from_slice(&number_samples);

        pcmu.insert(pos, encode_pcmu_frames(&full_pcm));
        pcma.insert(pos, encode_pcma_frames(&full_pcm));
        g722.insert(pos, encode_g722_frames(&full_pcm));
    }

    let mut eta_pcmu = HashMap::new();
    let mut eta_pcma = HashMap::new();
    let mut eta_g722 = HashMap::new();

    let eta_prefix_pcm = load_wav_pcm(&dir.join("eta.wav"));
    let minutes_pcm = load_wav_pcm(&dir.join("minutes.wav"));

    if let (Some(eta_prefix), Some(minutes_suffix)) = (eta_prefix_pcm, minutes_pcm) {
        for mins in 1..=60 {
            let number_samples = compose_number_pcm(mins, &digit_pcm);
            if number_samples.is_empty() {
                continue;
            }
            let mut full_pcm = eta_prefix.clone();
            full_pcm.extend_from_slice(&[0i16; 800]);
            full_pcm.extend_from_slice(&number_samples);
            full_pcm.extend_from_slice(&[0i16; 400]);
            full_pcm.extend_from_slice(&minutes_suffix);

            eta_pcmu.insert(mins, encode_pcmu_frames(&full_pcm));
            eta_pcma.insert(mins, encode_pcma_frames(&full_pcm));
            eta_g722.insert(mins, encode_g722_frames(&full_pcm));
        }
    }

    if pcmu.is_empty() {
        return None;
    }

    Some(PerLanguageCache {
        pcmu,
        pcma,
        g722,
        eta_pcmu,
        eta_pcma,
        eta_g722,
    })
}

/// Pre-encoded position and ETA announcements, keyed by language code.
///
/// Supports per-queue language selection via subdirectories:
/// `{audio_dir}/{lang}/position.wav`, `{audio_dir}/{lang}/digits/1.wav`, etc.
/// Falls back to loading flat layout as "en" for backward compatibility.
pub struct AnnouncementCache {
    languages: HashMap<String, PerLanguageCache>,
}

impl AnnouncementCache {
    /// Load WAV audio from `audio_dir` and pre-encode for all codecs.
    ///
    /// Scans for language subdirectories containing `position.wav`.
    /// Each valid subdirectory (except `moh/`) becomes a language.
    /// If no subdirectories are found but flat `position.wav` exists,
    /// loads as "en" for backward compatibility.
    pub fn load(audio_dir: &Path) -> Self {
        let mut languages = HashMap::new();

        if !audio_dir.exists() {
            warn!(path = %audio_dir.display(), "Queue audio directory not found; announcements disabled");
            return Self { languages };
        }

        // Scan for language subdirectories
        if let Ok(entries) = std::fs::read_dir(audio_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let dir_name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                // Skip moh/ directory
                if dir_name == "moh" {
                    continue;
                }
                // Check if this subdir has position.wav
                if !path.join("position.wav").exists() {
                    continue;
                }
                if let Some(cache) = load_language_cache(&path) {
                    info!(lang = %dir_name, positions = cache.pcmu.len(), "Loaded announcement language");
                    languages.insert(dir_name, cache);
                }
            }
        }

        // Backward compatibility: if no language subdirs found, try flat layout as "en"
        if languages.is_empty() && audio_dir.join("position.wav").exists() {
            if let Some(cache) = load_language_cache(audio_dir) {
                info!(
                    positions = cache.pcmu.len(),
                    "Loaded flat announcement audio as 'en'"
                );
                languages.insert("en".to_string(), cache);
            }
        }

        Self { languages }
    }

    /// Get pre-encoded frames for a position number, codec, and language.
    /// Falls back to "en" if the requested language is not available.
    pub fn get_position_frames(
        &self,
        position: usize,
        codec_name: &str,
        lang: &str,
    ) -> Option<&[Vec<u8>]> {
        let cache = self
            .languages
            .get(lang)
            .or_else(|| self.languages.get("en"))?;
        let map = match codec_name {
            "PCMU" => &cache.pcmu,
            "PCMA" => &cache.pcma,
            "G722" => &cache.g722,
            _ => return None,
        };
        map.get(&position).map(|v| v.as_slice())
    }

    /// Get pre-encoded ETA frames for a given number of minutes, codec, and language.
    /// Falls back to "en" if the requested language is not available.
    pub fn get_eta_frames(
        &self,
        minutes: usize,
        codec_name: &str,
        lang: &str,
    ) -> Option<&[Vec<u8>]> {
        let cache = self
            .languages
            .get(lang)
            .or_else(|| self.languages.get("en"))?;
        let map = match codec_name {
            "PCMU" => &cache.eta_pcmu,
            "PCMA" => &cache.eta_pcma,
            "G722" => &cache.eta_g722,
            _ => return None,
        };
        map.get(&minutes).map(|v| v.as_slice())
    }

    /// Check if ETA announcements are available for a language.
    /// Falls back to "en" if the requested language is not available.
    pub fn has_eta(&self, lang: &str) -> bool {
        self.languages
            .get(lang)
            .or_else(|| self.languages.get("en"))
            .map(|c| !c.eta_pcmu.is_empty())
            .unwrap_or(false)
    }

    /// Check if any announcements are loaded.
    pub fn is_empty(&self) -> bool {
        self.languages.is_empty()
    }
}

/// Compose the PCM samples for a number (1-50) by concatenating digit files.
fn compose_number_pcm(n: usize, digits: &HashMap<usize, Vec<i16>>) -> Vec<i16> {
    if n == 0 || n > 60 {
        return Vec::new();
    }

    // Direct match for 1-20
    if n <= 20 {
        return digits.get(&n).cloned().unwrap_or_default();
    }

    // 21-50: tens + ones (e.g., 23 = "20" + "3")
    let tens = (n / 10) * 10;
    let ones = n % 10;

    let mut result = Vec::new();
    if let Some(t) = digits.get(&tens) {
        result.extend_from_slice(t);
    } else {
        return Vec::new();
    }

    if ones > 0 {
        // Small gap between tens and ones (50ms = 400 samples)
        result.extend_from_slice(&[0i16; 400]);
        if let Some(o) = digits.get(&ones) {
            result.extend_from_slice(o);
        }
    }

    result
}

/// Load a WAV file and extract raw PCM i16 samples.
/// Expects 8kHz mono 16-bit LE PCM.
fn load_wav_pcm(path: &Path) -> Option<Vec<i16>> {
    let data = std::fs::read(path).ok()?;

    // Minimal WAV parser: find "data" chunk and read i16 LE samples
    if data.len() < 44 {
        warn!(path = %path.display(), "WAV file too small");
        return None;
    }

    // Verify RIFF header
    if &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        warn!(path = %path.display(), "Not a valid WAV file");
        return None;
    }

    // Find "data" chunk
    let mut offset = 12;
    while offset + 8 < data.len() {
        let chunk_id = &data[offset..offset + 4];
        let chunk_size = u32::from_le_bytes([
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ]) as usize;

        if chunk_id == b"data" {
            let start = offset + 8;
            let end = (start + chunk_size).min(data.len());
            let pcm_bytes = &data[start..end];

            // Convert LE i16 pairs
            let samples: Vec<i16> = pcm_bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                .collect();

            debug!(
                path = %path.display(),
                samples = samples.len(),
                duration_ms = samples.len() * 1000 / 8000,
                "Loaded WAV audio"
            );

            return Some(samples);
        }

        offset += 8 + chunk_size;
        // WAV chunks are word-aligned
        if !chunk_size.is_multiple_of(2) {
            offset += 1;
        }
    }

    warn!(path = %path.display(), "No data chunk found in WAV file");
    None
}

/// Encode PCM samples into PCMU RTP-payload frames (160 samples each).
fn encode_pcmu_frames(pcm: &[i16]) -> Vec<Vec<u8>> {
    pcm.chunks(NB_FRAME_SAMPLES)
        .map(|chunk| chunk.iter().map(|&s| g711::encode_ulaw(s)).collect())
        .collect()
}

/// Encode PCM samples into PCMA RTP-payload frames (160 samples each).
fn encode_pcma_frames(pcm: &[i16]) -> Vec<Vec<u8>> {
    pcm.chunks(NB_FRAME_SAMPLES)
        .map(|chunk| chunk.iter().map(|&s| g711::encode_alaw(s)).collect())
        .collect()
}

/// Pre-encoded MOH (Music On Hold) audio cache.
///
/// Loads WAV files from `{audio_dir}/moh/{class_name}/` subdirectories,
/// concatenates all WAVs in each class, and pre-encodes per codec.
pub struct MohCache {
    /// class_name → codec_name → Vec<frames>
    classes: HashMap<String, HashMap<String, Vec<Vec<u8>>>>,
}

impl MohCache {
    /// Load MOH audio from `{audio_dir}/moh/` directory.
    ///
    /// Each subdirectory name is a MOH class (e.g., `default/`, `jazz/`).
    /// Within each, all `*.wav` files are loaded in alphabetical order,
    /// concatenated, and pre-encoded for each codec.
    pub fn load(audio_dir: &Path) -> Self {
        let moh_dir = audio_dir.join("moh");
        let mut classes = HashMap::new();

        if !moh_dir.exists() {
            debug!(path = %moh_dir.display(), "No MOH directory found");
            return Self { classes };
        }

        let mut class_dirs: Vec<PathBuf> = match std::fs::read_dir(&moh_dir) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .map(|e| e.path())
                .collect(),
            Err(e) => {
                warn!(error = %e, "Failed to read MOH directory");
                return Self { classes };
            }
        };
        class_dirs.sort();

        for class_path in class_dirs {
            let class_name = class_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("default")
                .to_string();

            // Collect and sort WAV files in this class directory
            let mut wav_files: Vec<PathBuf> = match std::fs::read_dir(&class_path) {
                Ok(entries) => entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| {
                        p.extension()
                            .map(|ext| ext.eq_ignore_ascii_case("wav"))
                            .unwrap_or(false)
                    })
                    .collect(),
                Err(_) => continue,
            };
            wav_files.sort();

            if wav_files.is_empty() {
                continue;
            }

            // Concatenate all WAV PCM data
            let mut combined_pcm: Vec<i16> = Vec::new();
            for wav_path in &wav_files {
                if let Some(samples) = load_wav_pcm(wav_path) {
                    combined_pcm.extend_from_slice(&samples);
                }
            }

            if combined_pcm.is_empty() {
                continue;
            }

            let mut codec_map = HashMap::new();
            codec_map.insert("PCMU".to_string(), encode_pcmu_frames(&combined_pcm));
            codec_map.insert("PCMA".to_string(), encode_pcma_frames(&combined_pcm));
            codec_map.insert("G722".to_string(), encode_g722_frames(&combined_pcm));

            info!(
                class = %class_name,
                wav_files = wav_files.len(),
                duration_ms = combined_pcm.len() * 1000 / 8000,
                "Loaded MOH class"
            );

            classes.insert(class_name, codec_map);
        }

        Self { classes }
    }

    /// Get the pre-encoded frames for a MOH class and codec.
    pub fn get_frames(&self, class: &str, codec: &str) -> Option<&[Vec<u8>]> {
        self.classes
            .get(class)
            .and_then(|codecs| codecs.get(codec))
            .map(|v| v.as_slice())
    }

    /// Returns true if no MOH classes were loaded.
    pub fn is_empty(&self) -> bool {
        self.classes.is_empty()
    }
}

/// Pre-encoded prompt audio cache for entry/connect prompts.
///
/// Maps prompt filename → codec name → pre-encoded RTP frames.
pub struct PromptCache {
    prompts: HashMap<String, HashMap<String, Vec<Vec<u8>>>>,
}

impl PromptCache {
    /// Load prompt WAV files referenced by queue configs.
    ///
    /// `prompt_files` is the set of unique prompt filenames (relative to `audio_dir`).
    /// `languages` is the set of language codes to try. For each prompt, tries
    /// `{audio_dir}/{lang}/{filename}` first, falling back to `{audio_dir}/{filename}`.
    pub fn load(audio_dir: &Path, prompt_files: &[String], languages: &[String]) -> Self {
        let mut prompts = HashMap::new();

        for filename in prompt_files {
            // Try language-specific paths first, then flat path
            let mut loaded = false;
            for lang in languages {
                let lang_path = audio_dir.join(lang).join(filename);
                if let Some(pcm) = load_wav_pcm(&lang_path) {
                    let key = format!("{}/{}", lang, filename);
                    let mut codec_map = HashMap::new();
                    codec_map.insert("PCMU".to_string(), encode_pcmu_frames(&pcm));
                    codec_map.insert("PCMA".to_string(), encode_pcma_frames(&pcm));
                    codec_map.insert("G722".to_string(), encode_g722_frames(&pcm));

                    info!(
                        prompt = %key,
                        duration_ms = pcm.len() * 1000 / 8000,
                        "Loaded prompt audio (i18n)"
                    );
                    prompts.insert(key, codec_map);
                    loaded = true;
                }
            }
            // Fall back to flat path
            let path = audio_dir.join(filename);
            if let Some(pcm) = load_wav_pcm(&path) {
                let mut codec_map = HashMap::new();
                codec_map.insert("PCMU".to_string(), encode_pcmu_frames(&pcm));
                codec_map.insert("PCMA".to_string(), encode_pcma_frames(&pcm));
                codec_map.insert("G722".to_string(), encode_g722_frames(&pcm));

                info!(
                    prompt = %filename,
                    duration_ms = pcm.len() * 1000 / 8000,
                    "Loaded prompt audio"
                );
                prompts.insert(filename.clone(), codec_map);
                loaded = true;
            }
            if !loaded {
                warn!(prompt = %filename, "Failed to load prompt WAV file");
            }
        }

        Self { prompts }
    }

    /// Get pre-encoded frames for a prompt and codec, with language support.
    /// Tries `{lang}/{prompt_name}` first, falls back to `{prompt_name}`.
    // Kept as part of the module's intended API; not currently called from this binary.
    #[allow(dead_code)]
    pub fn get_frames_i18n(
        &self,
        prompt_name: &str,
        codec: &str,
        lang: &str,
    ) -> Option<&[Vec<u8>]> {
        let lang_key = format!("{}/{}", lang, prompt_name);
        self.prompts
            .get(&lang_key)
            .or_else(|| self.prompts.get(prompt_name))
            .and_then(|codecs| codecs.get(codec))
            .map(|v| v.as_slice())
    }

    /// Get pre-encoded frames for a prompt and codec.
    pub fn get_frames(&self, prompt_name: &str, codec: &str) -> Option<&[Vec<u8>]> {
        self.prompts
            .get(prompt_name)
            .and_then(|codecs| codecs.get(codec))
            .map(|v| v.as_slice())
    }

    /// Returns true if no prompts were loaded.
    pub fn is_empty(&self) -> bool {
        self.prompts.is_empty()
    }
}

/// Encode PCM samples into G.722 RTP-payload frames.
/// G.722 expects 16kHz input but is typically used with 8kHz PCM
/// that is upsampled. For simplicity, we duplicate each sample to
/// simulate 16kHz from 8kHz source.
fn encode_g722_frames(pcm: &[i16]) -> Vec<Vec<u8>> {
    use forge_codecs::g722::{G722BitRate, G722Encoder};

    // Upsample 8kHz → 16kHz by sample duplication
    let pcm_16k: Vec<i16> = pcm.iter().flat_map(|&s| [s, s]).collect();

    let mut encoder = G722Encoder::new(G722BitRate::Rate64k);
    let mut frames = Vec::new();

    for chunk in pcm_16k.chunks(G722_FRAME_SAMPLES) {
        match encoder.encode(chunk) {
            Ok(encoded) => frames.push(encoded),
            Err(e) => {
                warn!(error = %e, "G.722 encode error in announcement");
                // Push silence frame on error
                frames.push(vec![0u8; 160]);
            }
        }
    }

    frames
}
