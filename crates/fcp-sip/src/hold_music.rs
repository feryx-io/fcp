//! Hold music (`docs/ENTERPRISE_CALLING.md` §3.3): whenever a call's held
//! leg changes — a hold the node answered, a park, a retrieve, a transfer
//! — the music stops or starts, looped into the held leg with the other
//! leg's audio held back. The music is the holder's own choice (a prompt,
//! or none), else the music of the number the call came in on, else the
//! tenant's choice (a prompt, or none), else the tenant's first
//! hold-music prompt, else the node's built-in default,
//! synthesised at startup so no licensed audio ships.

use fcp_b2bua_core::{Call, CallId, CallManager, LegSide};
use fcp_core::storage::{LineBindingStore, TenantStore};
use fcp_core::{DynStorage, PromptCategory};
use fcp_media::{AudioTarget, MediaManager};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use tracing::{debug, info, warn};

/// The hold-music player: what the call manager's hold hook drives.
pub struct HoldMusic {
    call_manager: Weak<CallManager>,
    media: Arc<MediaManager>,
    storage: Option<Arc<DynStorage>>,
    default_music: Option<PathBuf>,
    playing: dashmap::DashMap<CallId, (fcp_core::CallId, String)>,
}

impl HoldMusic {
    pub fn new(
        call_manager: &Arc<CallManager>,
        media: Arc<MediaManager>,
        storage: Option<Arc<DynStorage>>,
        default_music: Option<PathBuf>,
    ) -> Arc<Self> {
        Arc::new(Self {
            call_manager: Arc::downgrade(call_manager),
            media,
            storage,
            default_music,
            playing: dashmap::DashMap::new(),
        })
    }

    /// Install on the call manager: every change of a held leg starts or
    /// stops the music.
    pub fn install(self: &Arc<Self>, call_manager: &CallManager) {
        let me = Arc::clone(self);
        call_manager.set_on_hold_changed(Arc::new(move |call_id, held| {
            let me = Arc::clone(&me);
            tokio::spawn(async move { me.changed(call_id, held).await });
        }));
    }

    async fn changed(&self, call_id: CallId, held: Option<LegSide>) {
        if let Some((_, (session, playback))) = self.playing.remove(&call_id) {
            if let Err(e) = self.media.stop_playback(&session, playback).await {
                debug!(call_id = %call_id, error = %e, "Hold music already gone");
            }
        }
        let Some(leg) = held else { return };
        let Some(call_manager) = self.call_manager.upgrade() else {
            return;
        };
        let Some(call) = call_manager.get_call(&call_id) else {
            return;
        };
        let Some(session) = call.media_session_id.filter(|_| call.media_anchored) else {
            debug!(call_id = %call_id, "Held, but the media is not here: no music");
            return;
        };
        let Some(music) = self.music_for(&call, leg).await else {
            info!(call_id = %call_id, "Held in silence: the holder chose no music");
            return;
        };
        let target = match leg {
            LegSide::A => AudioTarget::ParticipantA,
            LegSide::B => AudioTarget::ParticipantB,
        };
        match self
            .media
            .play_file_looped(&session, music.clone(), target)
            .await
        {
            Ok(playback) => {
                info!(call_id = %call_id, leg = leg.as_str(), music = %music, "Hold music playing");
                // Released while the music was starting: stop it again.
                if call_manager.get_call(&call_id).and_then(|c| c.held) != Some(leg) {
                    let _ = self.media.stop_playback(&session, playback).await;
                } else {
                    self.playing.insert(call_id, (session, playback));
                }
            }
            Err(e) => warn!(call_id = %call_id, error = %e, "Hold music not played"),
        }
    }

    /// The music for a leg held in `call`: the holder's choice, the music
    /// of the number the call came in on, the tenant's choice, the
    /// tenant's first hold-music prompt, the node's default. `None` for
    /// silence.
    async fn music_for(&self, call: &Call, held: LegSide) -> Option<String> {
        if let Some(storage) = &self.storage {
            // The lot a parked call waits in.
            if call.state == fcp_b2bua_core::CallState::Parked {
                if let Some(id) = call.parked_music.as_deref() {
                    if let Some(path) = self.prompt_audio(storage, call, id).await {
                        return Some(path);
                    }
                    warn!(call_id = %call.id, prompt = %id, "The parking lot's music has no audio; using the parker's");
                }
            }
            // The holder: the parker of a parked call, the callee the plan
            // rang (phase 1 keeps it on the call), or whoever the leg's
            // address names — an extension or a username.
            let holder = if call.state == fcp_b2bua_core::CallState::Parked {
                match call.parker_extension.as_deref() {
                    Some(name) => crate::ring_plan::user_of(storage, &call.tenant_id, name).await,
                    None => None,
                }
            } else {
                match held.other() {
                    LegSide::B if call.callee_user_id.is_some() => call.callee_user_id,
                    side => {
                        let uri = match side {
                            LegSide::A => Some(call.a_leg.remote_uri.clone()),
                            LegSide::B => call.b_leg.as_ref().map(|b| b.remote_uri.clone()),
                        };
                        match uri.as_deref().and_then(user_part) {
                            Some(name) => {
                                crate::ring_plan::user_of(storage, &call.tenant_id, &name).await
                            }
                            None => None,
                        }
                    }
                }
            };
            if let Some(user_id) = holder {
                let settings =
                    crate::ring_plan::settings_for(storage, &call.tenant_id, &user_id).await;
                match settings.hold_music.as_deref() {
                    Some(fcp_core::calling::HOLD_MUSIC_NONE) => return None,
                    Some(id) => {
                        if let Some(path) = self.prompt_audio(storage, call, id).await {
                            return Some(path);
                        }
                        warn!(call_id = %call.id, prompt = %id, "The holder's hold music has no audio; using the tenant's");
                    }
                    None => {}
                }
            }
            // The number the call came in on.
            if let Some(binding_id) = call.number_binding_id {
                match storage.get_line_binding(&binding_id).await {
                    Ok(Some(binding)) => match binding.hold_music.as_deref() {
                        Some(fcp_core::calling::HOLD_MUSIC_NONE) => return None,
                        Some(id) => {
                            if let Some(path) = self.prompt_audio(storage, call, id).await {
                                return Some(path);
                            }
                            warn!(call_id = %call.id, prompt = %id, "The number's hold music has no audio; using the tenant's");
                        }
                        None => {}
                    },
                    Ok(None) => {}
                    Err(e) => {
                        warn!(call_id = %call.id, error = %e, "Could not read the number's hold music")
                    }
                }
            }
            // The tenant's choice.
            match storage.get_tenant(&call.tenant_id).await {
                Ok(Some(tenant)) => match tenant.calling.hold_music.as_deref() {
                    Some(fcp_core::calling::HOLD_MUSIC_NONE) => return None,
                    Some(id) => {
                        if let Some(path) = self.prompt_audio(storage, call, id).await {
                            return Some(path);
                        }
                        warn!(call_id = %call.id, prompt = %id, "The tenant's hold music has no audio; using its first hold-music prompt");
                    }
                    None => {}
                },
                Ok(None) => {}
                Err(e) => {
                    warn!(call_id = %call.id, error = %e, "Could not read the tenant's hold music choice")
                }
            }
            if let Some(prompts) = storage.prompt_store() {
                match prompts
                    .list_prompts(Some(&call.tenant_id), Some(PromptCategory::Moh), None)
                    .await
                {
                    Ok(list) => {
                        if let Some(path) = list
                            .into_iter()
                            .filter_map(|p| p.audio_path)
                            .find(|p| p.is_file())
                        {
                            return Some(path.to_string_lossy().into_owned());
                        }
                    }
                    Err(e) => {
                        warn!(call_id = %call.id, error = %e, "Could not read the tenant's hold music")
                    }
                }
            }
        }
        self.default_music
            .as_ref()
            .filter(|p| p.is_file())
            .map(|p| p.to_string_lossy().into_owned())
    }

    async fn prompt_audio(&self, storage: &DynStorage, call: &Call, id: &str) -> Option<String> {
        let prompt_id = id.parse::<fcp_core::PromptId>().ok()?;
        let store = storage.prompt_store()?;
        let prompt =
            fcp_core::prompt::effective_prompt_by_id(store.as_ref(), &call.tenant_id, &prompt_id)
                .await
                .ok()
                .flatten()?;
        prompt
            .audio_path
            .filter(|p| p.is_file())
            .map(|p| p.to_string_lossy().into_owned())
    }
}

fn user_part(uri: &str) -> Option<String> {
    let uri = uri.trim().trim_start_matches('<');
    let rest = uri
        .strip_prefix("sips:")
        .or_else(|| uri.strip_prefix("sip:"))
        .unwrap_or(uri);
    let (user, _) = rest.split_once('@')?;
    (!user.is_empty()).then(|| user.to_string())
}

/// The notes of the built-in hold music, each held for [`NOTE_MS`]: a slow
/// C-major arpeggio, up and down.
pub const DEFAULT_MUSIC_NOTES: [f32; 6] = [523.25, 659.25, 783.99, 1046.5, 783.99, 659.25];
const NOTE_MS: u32 = 400;
const RATE: u32 = 8000;

/// Write the built-in hold music to `path` (a 16-bit 8 kHz mono WAV of
/// [`DEFAULT_MUSIC_NOTES`], each note faded in and out), unless it is
/// there already.
pub fn write_default_music(path: &Path) -> std::io::Result<()> {
    if path.is_file() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let per_note = (RATE * NOTE_MS / 1000) as usize;
    let fade = per_note / 8;
    let mut samples: Vec<i16> = Vec::with_capacity(per_note * DEFAULT_MUSIC_NOTES.len());
    for &hz in &DEFAULT_MUSIC_NOTES {
        for n in 0..per_note {
            let envelope = if n < fade {
                n as f32 / fade as f32
            } else if n > per_note - fade {
                (per_note - n) as f32 / fade as f32
            } else {
                1.0
            };
            let t = n as f32 / RATE as f32;
            let s = (2.0 * std::f32::consts::PI * hz * t).sin() * envelope * 0.35;
            samples.push((s * i16::MAX as f32) as i16);
        }
    }
    let data_len = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        wav.extend_from_slice(&s.to_le_bytes());
    }
    let tmp = path.with_extension("wav.tmp");
    std::fs::write(&tmp, wav)?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_music_is_a_wav_of_its_notes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("hold-music.wav");
        write_default_music(&path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        let data = (bytes.len() - 44) / 2;
        assert_eq!(data, 3200 * DEFAULT_MUSIC_NOTES.len());
        // Written once: a second call leaves it.
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        write_default_music(&path).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            before
        );
    }

    #[test]
    fn a_user_part_is_read_from_a_uri() {
        assert_eq!(user_part("sip:bob@127.0.0.1:5060").as_deref(), Some("bob"));
        assert_eq!(user_part("<sips:1002@pbx>").as_deref(), Some("1002"));
        assert_eq!(user_part("sip:pbx"), None);
    }
}
