//! fcp-conference-server — Standalone multi-party audio conference bridge.
//!
//! Accepts SIP INVITEs for configured meeting room numbers, authenticates
//! participants via PIN, mixes audio using forge-conference, and provides
//! a REST API for meeting management and runtime conference control.
//!
//! Key differences from fcp-voicemail-server:
//! - Uses ConferenceCallHandler instead of FlowCallHandler
//! - Creates ConferenceBridge (forge-conference) for multi-party audio mixing
//! - REST API manages persistent Meeting configs + runtime conference operations
//! - No flow engine — conference calls are multi-party, not sequential single-call flows

use anyhow::{Context, Result};
use clap::Parser;
use fcp_events::{EventFilter, EventSubscriber, InMemoryEventBus};
use fcp_media::{BackendType, MediaConfig};
use fcp_sbc_core::{
    CpsChecker, NoCpsLimit, TrunkCredentialProvider, TrunkHealthMonitor, TrunkRegistrar,
    TrunkRouter,
};
use fcp_sip::prelude::*;
use fcp_sip::service::{DialogService, SubscriptionService, TransactionService};
use fcp_sip::transport::ConnectionPool;
use forge_conference::ConferenceBridge;
use forge_core::{AudioCodec as ForgeAudioCodec, AudioFormat};
use forge_mixer::MixerOptions;
use forge_rtp::{PortPool, PortPoolConfig};
use sip_observe::{set_transport_metrics, TracingTransportMetrics};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, warn};

mod audio_bridge;
mod cli;
mod conference_handler;
mod control;
mod health;
mod storage;
mod trunks;

use cli::Cli;
use conference_handler::ConferenceCallHandler;
use config::ConferenceServerConfig;
use fcp_conference_server::config;

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Parse CLI arguments
    let cli = Cli::parse();

    // 2. Load configuration
    let mut config = config::load_config(cli.config.as_deref())?;

    // Apply CLI overrides
    if let Some(ref level) = cli.log_level {
        config.logging.level = level.clone();
    }
    if let Some(ref format) = cli.log_format {
        config.logging.format = format.clone();
    }
    if let Some(ref bind) = cli.sip_bind {
        config.sip.udp_bind = bind.clone();
        config.sip.tcp_bind = bind.clone();
    }
    if let Some(ref bind) = cli.http_bind {
        config.http.bind_address = bind
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid HTTP bind address '{}': {}", bind, e))?;
    }
    if let Some(ref ip) = cli.public_ip {
        if !ip.is_empty() {
            config.sip.local_ip = Some(ip.clone());
            config.media.public_ip = Some(ip.clone());
        }
    }

    // Auto-detect local IP if not explicitly configured and bind is 0.0.0.0
    if config.sip.local_ip.is_none() {
        let bind_unspecified = config
            .sip
            .udp_bind
            .parse::<std::net::SocketAddr>()
            .map(|a| a.ip().is_unspecified())
            .unwrap_or(true);
        if bind_unspecified {
            if let Some(detected_ip) = auto_detect_local_ip() {
                config.sip.local_ip = Some(detected_ip.clone());
                if config.media.public_ip.is_none() {
                    config.media.public_ip = Some(detected_ip);
                }
            }
        }
    }

    // 2b. Validate configuration
    config::validate_config(&config)?;

    // 3. Initialize tracing
    init_tracing(&config);

    info!(
        version = env!("CARGO_PKG_VERSION"),
        node_id = %config.service.node_id,
        "fcp-conference starting"
    );

    if let Some(ref ip) = config.sip.local_ip {
        info!(
            local_ip = %ip,
            source = if cli.public_ip.as_ref().is_some_and(|p| !p.is_empty()) { "PUBLIC_IP env" } else { "auto-detected" },
            "Using local IP for SIP signaling and media"
        );
    }

    // Set up observability
    if !set_transport_metrics(Arc::new(TracingTransportMetrics)) {
        warn!("transport metrics already configured");
    }

    // 4. Initialize storage
    let storage_result = storage::initialize_storage(&config.storage).await?;
    let config_store = storage_result.config_store;
    let dyn_storage = storage_result.dyn_storage;

    // 4b. Seed operational config sections into database (DB primary, TOML seed)
    // Bootstrap sections (service, sip, storage, http, logging) stay TOML-only.
    {
        let cs = config_store.as_ref();
        let svc = "conference";
        let tid = "_system";

        let sections: Vec<(&str, serde_json::Value)> = vec![
            (
                "media",
                serde_json::to_value(&config.media).unwrap_or_default(),
            ),
            (
                "events",
                serde_json::to_value(&config.events).unwrap_or_default(),
            ),
            (
                "conference",
                serde_json::to_value(&config.conference).unwrap_or_default(),
            ),
            ("ai", serde_json::to_value(&config.ai).unwrap_or_default()),
            (
                "b2bua",
                serde_json::to_value(&config.b2bua).unwrap_or_default(),
            ),
            (
                "cluster",
                serde_json::to_value(&config.cluster).unwrap_or_default(),
            ),
        ];

        for (section, toml_val) in sections {
            match cs.get_config(svc, section, tid).await {
                Ok(Some(entry)) => {
                    match section {
                        "media" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                config.media = v;
                            }
                        }
                        "events" => {
                            if let Ok(mut v) =
                                serde_json::from_value::<config::EventsSection>(entry.value)
                            {
                                // Preserve auth_token from TOML
                                if v.auth_token.is_none() {
                                    v.auth_token = config.events.auth_token.clone();
                                }
                                config.events = v;
                            }
                        }
                        "conference" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                config.conference = v;
                            }
                        }
                        "ai" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                config.ai = v;
                            }
                        }
                        "b2bua" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                config.b2bua = v;
                            }
                        }
                        "cluster" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                config.cluster = v;
                            }
                        }
                        _ => {}
                    }
                    info!(
                        service = svc,
                        section,
                        version = entry.version,
                        "loaded config from database"
                    );
                }
                Ok(None) => {
                    match cs
                        .set_config(svc, section, tid, toml_val, Some("seed"))
                        .await
                    {
                        Ok(version) => info!(
                            service = svc,
                            section, version, "seeded config into database"
                        ),
                        Err(e) => {
                            warn!(service = svc, section, error = %e, "failed to seed config")
                        }
                    }
                }
                Err(e) => {
                    warn!(service = svc, section, error = %e, "failed to check DB config, using TOML");
                }
            }
        }
        info!("Operational config loading complete (DB primary, TOML seed)");
    }

    // 4c. Seed default conference prompts into the Prompt store.
    //     These are TTS-backed prompts that users can see and re-synthesize
    //     in the Audio Prompts page. The actual WAV files are generated
    //     when the admin calls POST /prompts/{id}/synthesize (Kokoro TTS).
    if let Some(prompt_store) = dyn_storage.prompt_store() {
        let tenant_id = config.service.default_tenant_id;
        let default_prompts = vec![
            (
                "conf_pin_prompt",
                "Conference PIN Prompt",
                "Please enter your conference PIN, followed by the pound key.",
                fcp_core::prompt::PromptCategory::System,
            ),
            (
                "conf_invalid_pin",
                "Conference Invalid PIN",
                "Invalid PIN. Please try again.",
                fcp_core::prompt::PromptCategory::System,
            ),
            (
                "conf_wait_announcement",
                "Conference Wait for Moderator Announcement",
                "Please wait. The conference will begin when the moderator joins.",
                fcp_core::prompt::PromptCategory::System,
            ),
            (
                "conf_welcome",
                "Conference Welcome",
                "You have joined the conference.",
                fcp_core::prompt::PromptCategory::System,
            ),
            (
                "conf_dialout_greeting",
                "Conference Dial-Out Greeting",
                "Please hold while we connect you to the conference.",
                fcp_core::prompt::PromptCategory::System,
            ),
            (
                "conf_join_tone",
                "Conference Join Tone (generated beep)",
                "",
                fcp_core::prompt::PromptCategory::System,
            ),
            (
                "conf_exit_tone",
                "Conference Exit Tone (generated beep)",
                "",
                fcp_core::prompt::PromptCategory::System,
            ),
            (
                "conf_muted",
                "Conference Muted Notification",
                "Muted.",
                fcp_core::prompt::PromptCategory::System,
            ),
            (
                "conf_unmuted",
                "Conference Unmuted Notification",
                "Unmuted.",
                fcp_core::prompt::PromptCategory::System,
            ),
            (
                "conf_locked",
                "Conference Locked Notification",
                "Conference locked.",
                fcp_core::prompt::PromptCategory::System,
            ),
            (
                "conf_ending",
                "Conference Ending Notification",
                "The moderator has ended this conference. Goodbye.",
                fcp_core::prompt::PromptCategory::System,
            ),
            (
                "conf_unlocked",
                "Conference Unlocked Notification",
                "Conference unlocked.",
                fcp_core::prompt::PromptCategory::System,
            ),
        ];
        for (name, description, tts_text, category) in default_prompts {
            match prompt_store.get_prompt_by_name(&tenant_id, name).await {
                Ok(Some(_)) => {
                    // Already exists, don't overwrite user edits
                }
                Ok(None) => {
                    let mut prompt = fcp_core::prompt::Prompt::new_tts(
                        tenant_id, name, category, "en-US", tts_text,
                    );
                    prompt.description = Some(description.to_string());
                    prompt.tts_voice = Some("af_heart".to_string());
                    prompt.is_system = true;
                    if let Err(e) = prompt_store.put_prompt(prompt).await {
                        warn!(name, error = %e, "Failed to seed conference prompt");
                    } else {
                        info!(name, "Seeded default conference prompt");
                    }
                }
                Err(e) => {
                    warn!(name, error = %e, "Failed to check for existing prompt");
                }
            }
        }
    }

    // 5. Create SIP components
    let dialog_manager = DialogManager::new();
    let subscription_manager = SubscriptionManager::new();

    // 6. Create shared TCP connection pool
    let tcp_pool = Arc::new(ConnectionPool::new());
    let (client_tx, client_rx) = tokio::sync::mpsc::channel(1024);
    tcp_pool.set_inbound_tx(client_tx).await;

    // 7. Create transport dispatcher
    let dispatcher = create_transport_dispatcher(&config, Arc::clone(&tcp_pool)).await?;

    // 8. Create transaction manager with shared pool
    let transaction_manager = TransactionManager::new_with_pool(dispatcher.clone(), tcp_pool);

    // 9. Create services
    let transaction_service = TransactionService::new(transaction_manager);
    let dialog_service = DialogService::new(dialog_manager);
    let _subscription_service = SubscriptionService::new(subscription_manager);

    let transaction_manager_ref = Arc::clone(transaction_service.manager());
    let dialog_manager_ref = Arc::clone(dialog_service.manager());

    // 10. Build credential provider from trunks (if any)
    let trunk_cred_provider: Option<Arc<TrunkCredentialProvider>> = if !config.trunks.is_empty() {
        Some(Arc::new(TrunkCredentialProvider::from_trunks(
            &config.trunks,
        )))
    } else {
        None
    };

    // 11. Create UAC for outbound calls
    let uac = create_uac(
        &config,
        Arc::clone(&transaction_manager_ref),
        dispatcher,
        trunk_cred_provider,
    )
    .await?;

    // 12. Create event bus
    let event_bus = Arc::new(InMemoryEventBus::new());
    info!("Event bus initialized (in-memory)");

    // Initialize Redis storage (if configured)
    #[cfg(feature = "storage-redis")]
    let redis_storage = if let Some(ref url) = config.storage.redis_url {
        match fcp_storage_redis::RedisStorage::new(fcp_storage_redis::RedisConfig {
            url: url.clone(),
            ..Default::default()
        })
        .await
        {
            Ok(rs) => Some(rs),
            Err(e) => {
                warn!(error = %e, "Failed to connect to Redis; cluster disabled");
                None
            }
        }
    } else {
        None
    };

    // Start cluster membership (if configured)
    #[cfg(feature = "storage-redis")]
    let _cluster_manager = if config.cluster.enabled {
        if let Some(ref redis) = redis_storage {
            let member = fcp_cluster::ClusterMember {
                node_id: config.service.node_id.clone(),
                node_type: config.service.service_type.clone(),
                http_address: config.advertised_http_address(),
                sip_address: config
                    .sip
                    .udp_bind
                    .parse()
                    .unwrap_or_else(|_| "0.0.0.0:5060".parse().unwrap()),
                sip_tls_address: config.sip.advertised_tls_address(),
                capabilities: vec!["conference".to_string(), "audio_mixing".to_string()],
                active_calls: 0,
                started_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                draining: false,
                region: fcp_cluster::resolve_region(config.cluster.region.as_deref()),
            };
            let cm =
                fcp_cluster::ClusterManager::new(redis.clone(), member)
                    .with_intervals(
                        config.cluster.heartbeat_interval_secs,
                        config.cluster.heartbeat_ttl_secs,
                    )
                    .with_event_publisher(
                        Arc::clone(&event_bus) as Arc<dyn fcp_events::EventPublisher>
                    );
            let _handle = cm.start();
            info!("Cluster membership started");
            Some((
                cm,
                fcp_cluster::ConferenceRegistry::new(redis.clone(), config.service.node_id.clone()),
            ))
        } else {
            warn!("cluster.enabled = true but redis_url is not set; cluster disabled");
            None
        }
    } else {
        None
    };

    // 13. Trunk infrastructure (optional)
    let trunk_shutdown_tx: Option<tokio::sync::watch::Sender<bool>>;
    let outbound_route_provider: Option<Arc<dyn fcp_sip::OutboundRouteProvider>> =
        if !config.trunks.is_empty() {
            let health = Arc::new(TrunkHealthMonitor::new(&config.trunks));
            let cps: Arc<dyn CpsChecker> = Arc::new(NoCpsLimit);
            let router = Arc::new(TrunkRouter::new(
                config.trunks.clone(),
                Arc::clone(&health),
                cps,
            ));

            let health_clone = Arc::clone(&health);
            let trunks_clone = config.trunks.clone();
            let uac_clone = Arc::clone(&uac);
            tokio::spawn(fcp_sbc_core::run_health_probes(
                health_clone,
                trunks_clone,
                uac_clone,
            ));

            let trunk_registrar = Arc::new(TrunkRegistrar::new(&config.trunks));
            if trunk_registrar.has_registerable_trunks() {
                let (tx, rx) = tokio::sync::watch::channel(false);
                let reg = Arc::clone(&trunk_registrar);
                let reg_health = Arc::clone(&health);
                let reg_uac = Arc::clone(&uac);
                tokio::spawn(fcp_sbc_core::run_trunk_registration(
                    reg, reg_health, reg_uac, rx,
                ));
                trunk_shutdown_tx = Some(tx);
            } else {
                trunk_shutdown_tx = None;
            }

            info!(count = config.trunks.len(), "Trunk routing enabled");
            Some(router as Arc<dyn fcp_sip::OutboundRouteProvider>)
        } else {
            trunk_shutdown_tx = None;
            None
        };

    // 14. Create media manager (kept for potential future use, but not wired
    //     into the SIP handler — the conference room handles RTP directly)
    let _media_manager = create_media_manager(&config).await?;

    // 15. Create CallManager for B2BUA call tracking
    let call_manager = Arc::new(fcp_sip::CallManager::with_storage_and_events(
        Arc::clone(&dyn_storage),
        Arc::clone(&event_bus) as Arc<dyn fcp_events::EventPublisher>,
    ));

    // 16. Create RTP port pool for conference participants
    let port_pool = Arc::new(PortPool::new(
        PortPoolConfig::new(config.media.rtp_port_start, config.media.rtp_port_end)
            .context("Invalid RTP port range")?,
    ));

    // 17. Create ConferenceBridge + ConferenceCallHandler
    // Use config sample_rate for the mixer (8kHz for G.711, 16kHz for G.722 wideband)
    let audio_format = AudioFormat::new(config.conference.sample_rate, 1, ForgeAudioCodec::PCM);
    // Frame-clock mixing: one clock per room takes a frame from every
    // participant per tick, and every reader (each participant's N-1 mix,
    // the recorder, an AI tap, each inter-node trunk's send-mix) sees that
    // same frame without consuming it from the others.
    let mixer_options = MixerOptions {
        frame_clock: true,
        ..MixerOptions::default()
    };
    let bridge = Arc::new(
        ConferenceBridge::new(audio_format, config.conference.frame_size, mixer_options)
            .context("Failed to create ConferenceBridge")?,
    );

    let local_uri = create_local_uri(&config)?;
    let local_addr = config
        .sip
        .local_ip
        .clone()
        .or_else(|| config.media.public_ip.clone())
        .unwrap_or_else(|| derive_local_ip(&config.sip.udp_bind));

    // Ensure recordings directory exists
    if let Err(e) = std::fs::create_dir_all(&config.conference.recordings_dir) {
        warn!(
            path = %config.conference.recordings_dir,
            error = %e,
            "Failed to create recordings directory"
        );
    } else {
        info!(path = %config.conference.recordings_dir, "Recordings directory ready");
    }

    // Ensure announcements directory exists (#24)
    if let Err(e) = std::fs::create_dir_all(&config.conference.announcements_dir) {
        warn!(
            path = %config.conference.announcements_dir,
            error = %e,
            "Failed to create announcements directory"
        );
    } else {
        info!(path = %config.conference.announcements_dir, "Announcements directory ready");
    }

    let mut conference_handler = ConferenceCallHandler::new(
        Arc::clone(&bridge),
        Arc::clone(&dyn_storage),
        config.service.default_tenant_id,
        config.conference.clone(),
        local_uri.as_str().to_string(),
        local_addr.clone(),
        Arc::clone(&port_pool),
        Arc::clone(&event_bus) as Arc<dyn fcp_events::EventPublisher>,
        Arc::clone(&uac),
        config.ai.clone(),
        config.service.node_id.clone(),
    );

    // Initialize TTS service if configured (supports local/Kokoro + cloud providers)
    let tts_service: Option<Arc<fcp_media::TtsService>> = if config.tts.enabled {
        let provider = match config.tts.provider.as_str() {
            "local" | "kokoro" => Some(fcp_media::TtsProviderConfig::Local {
                engine: config.tts.engine.clone(),
                executable_path: config
                    .tts
                    .python_path
                    .as_ref()
                    .map(std::path::PathBuf::from),
                device: None,
            }),
            "openai" => config
                .tts
                .api_key
                .as_ref()
                .map(|key| fcp_media::TtsProviderConfig::OpenAi {
                    api_key: key.clone(),
                    model: "tts-1".to_string(),
                })
                .or_else(|| {
                    warn!("OpenAI TTS requires api_key");
                    None
                }),
            "google_cloud" => config
                .tts
                .api_key
                .as_ref()
                .map(|key| fcp_media::TtsProviderConfig::GoogleCloud {
                    api_key: key.clone(),
                })
                .or_else(|| {
                    warn!("Google Cloud TTS requires api_key");
                    None
                }),
            other => {
                warn!(provider = %other, "Unknown TTS provider");
                None
            }
        };
        if let Some(provider) = provider {
            let tts_config = fcp_media::TtsConfig {
                provider,
                cache_dir: std::path::PathBuf::from(&config.tts.cache_dir),
                cache_max_size_mb: config.tts.cache_max_size_mb,
                default_language: config.tts.language.clone(),
                default_voice: config.tts.voice.clone(),
            };
            match fcp_media::TtsService::new(tts_config).await {
                Ok(tts) => {
                    info!(provider = %config.tts.provider, "TTS service initialized");
                    let tts = Arc::new(tts);
                    conference_handler.set_tts_service(Arc::clone(&tts));
                    Some(tts)
                }
                Err(e) => {
                    warn!(error = %e, "TTS init failed");
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    // Auto-synthesize any conference prompts that are missing audio files.
    // This runs once at startup so callers hear prompts on the first call.
    if let (Some(ref tts), Some(prompt_store)) = (&tts_service, dyn_storage.prompt_store()) {
        let tenant_id = config.service.default_tenant_id;
        let prompts_dir = std::path::PathBuf::from(&config.tts.cache_dir);
        let _ = tokio::fs::create_dir_all(&prompts_dir).await;
        for name in &[
            "conf_pin_prompt",
            "conf_invalid_pin",
            "conf_wait_announcement",
            "conf_welcome",
            "conf_dialout_greeting",
            "conf_join_tone",
            "conf_exit_tone",
            "conf_muted",
            "conf_unmuted",
            "conf_locked",
            "conf_unlocked",
            "conf_ending",
        ] {
            match prompt_store.get_prompt_by_name(&tenant_id, name).await {
                Ok(Some(mut prompt)) if prompt.audio_path.is_none() => {
                    if let Some(ref text) = prompt.tts_text {
                        let voice = prompt.tts_voice.as_deref();
                        match tts.synthesize(text, Some(&prompt.language), voice).await {
                            Ok(cached_path) => {
                                // Copy to permanent location
                                let dest = prompts_dir.join(format!("{}.wav", prompt.id));
                                if let Ok(()) =
                                    tokio::fs::copy(&cached_path, &dest).await.map(|_| ())
                                {
                                    let size = tokio::fs::metadata(&dest)
                                        .await
                                        .map(|m| m.len())
                                        .unwrap_or(0);
                                    prompt.audio_path = Some(dest);
                                    prompt.audio_format = Some("wav".to_string());
                                    prompt.file_size = Some(size);
                                    prompt.updated_at = std::time::SystemTime::now();
                                    let _ = prompt_store.put_prompt(prompt).await;
                                    info!(name, "Auto-synthesized conference prompt");
                                }
                            }
                            Err(e) => warn!(name, error = %e, "Auto-synthesis failed for prompt"),
                        }
                    }
                }
                Ok(Some(_)) => {} // Already has audio
                _ => {}
            }
        }
    }

    // Pre-synthesize welcome message variants so they're cached for instant playback.
    // The TTS cache is keyed by text, so these only need to be synthesized once.
    if let Some(ref tts) = tts_service {
        let welcome_variants = [
            "You have joined the conference. You are the only participant in this conference.",
            "You have joined the conference. There is 1 other participant in the conference.",
            "You have joined the conference. There are 2 other participants in the conference.",
            "You have joined the conference. There are 3 other participants in the conference.",
            "You have joined the conference. There are 4 other participants in the conference.",
            "You have joined the conference. There are 5 other participants in the conference.",
            "You have joined the conference. There are 6 other participants in the conference.",
            "You have joined the conference. There are 7 other participants in the conference.",
            "You have joined the conference. There are 8 other participants in the conference.",
            "You have joined the conference. There are 9 other participants in the conference.",
            "You have joined the conference.",
        ];
        for text in &welcome_variants {
            match tts.synthesize(text, None, None).await {
                Ok(_) => {}
                Err(e) => warn!(error = %e, text, "Failed to pre-synthesize welcome variant"),
            }
        }
        info!(
            "Pre-synthesized {} welcome message variants",
            welcome_variants.len()
        );
    }

    // Load pre-synthesized mute/unmute audio samples for DTMF feedback
    if let Some(prompt_store) = dyn_storage.prompt_store() {
        let tenant_id = config.service.default_tenant_id;
        let player = forge_conference::AudioFeedbackPlayer::new(config.conference.sample_rate);
        for (prompt_name, field_setter) in [
            ("conf_muted", "muted" as &str),
            ("conf_unmuted", "unmuted"),
            ("conf_locked", "locked"),
            ("conf_unlocked", "unlocked"),
            ("conf_ending", "ending"),
        ] {
            if let Ok(Some(prompt)) = prompt_store
                .get_prompt_by_name(&tenant_id, prompt_name)
                .await
            {
                if let Some(ref path) = prompt.audio_path {
                    if let Some(samples) = player.load_audio_file(path) {
                        match field_setter {
                            "muted" => {
                                conference_handler.dtmf_muted_samples = Some(Arc::new(samples))
                            }
                            "unmuted" => {
                                conference_handler.dtmf_unmuted_samples = Some(Arc::new(samples))
                            }
                            "locked" => {
                                conference_handler.dtmf_locked_samples = Some(Arc::new(samples))
                            }
                            "unlocked" => {
                                conference_handler.dtmf_unlocked_samples = Some(Arc::new(samples))
                            }
                            "ending" => {
                                conference_handler.dtmf_ending_samples = Some(Arc::new(samples))
                            }
                            _ => {}
                        }
                        info!(prompt = prompt_name, "Loaded DTMF feedback audio");
                    }
                }
            }
        }
    }

    // Generate default join/exit tone WAV files (short beeps, not TTS).
    // Join = ascending two-tone (440Hz → 580Hz), Exit = descending (580Hz → 440Hz).
    {
        let tones_dir = std::path::Path::new(&config.tts.cache_dir);
        let _ = std::fs::create_dir_all(tones_dir);
        let sample_rate = config.conference.sample_rate;
        let mut conf_write = conference_handler.conference_config.write();

        let generate_tone = |freqs: &[(f64, f64)], path: &std::path::Path| -> bool {
            // Each segment: (frequency_hz, duration_secs)
            let mut samples: Vec<i16> = Vec::new();
            for &(freq, dur) in freqs {
                let n = (sample_rate as f64 * dur) as usize;
                for i in 0..n {
                    let t = i as f64 / sample_rate as f64;
                    // Apply fade envelope (10ms fade in/out) to avoid clicks
                    let fade_samples = (sample_rate as f64 * 0.01) as usize;
                    let envelope = if i < fade_samples {
                        i as f64 / fade_samples as f64
                    } else if i > n - fade_samples {
                        (n - i) as f64 / fade_samples as f64
                    } else {
                        1.0
                    };
                    let val =
                        (12000.0 * envelope * (2.0 * std::f64::consts::PI * freq * t).sin()) as i16;
                    samples.push(val);
                }
            }
            // Write as WAV
            if let Ok(mut writer) = hound::WavWriter::create(
                path,
                hound::WavSpec {
                    channels: 1,
                    sample_rate,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                },
            ) {
                for s in &samples {
                    let _ = writer.write_sample(*s);
                }
                let _ = writer.finalize();
                true
            } else {
                false
            }
        };

        let join_path = tones_dir.join("conf_join_tone.wav");
        if conf_write.join_sound.is_none() {
            // Ascending: 440Hz 100ms → 10ms silence → 580Hz 100ms
            if generate_tone(&[(440.0, 0.1), (0.0, 0.01), (580.0, 0.1)], &join_path) {
                conf_write.join_sound = Some(join_path.display().to_string());
                info!(path = %join_path.display(), "Generated join tone");
            }
        }

        let exit_path = tones_dir.join("conf_exit_tone.wav");
        if conf_write.exit_sound.is_none() {
            // Descending: 580Hz 100ms → 10ms silence → 440Hz 150ms
            if generate_tone(&[(580.0, 0.1), (0.0, 0.01), (440.0, 0.15)], &exit_path) {
                conf_write.exit_sound = Some(exit_path.display().to_string());
                info!(path = %exit_path.display(), "Generated exit tone");
            }
        }
    }

    let conference_handler = Arc::new(conference_handler);

    #[allow(unused_mut)]
    let mut cluster_control: Option<Arc<control::ClusterControl>> = None;

    // Conference registry: publish this node's per-room participant counts
    // so the call manager can place new participants (and, later, so peer
    // nodes can cascade audio). Event-driven on join/leave plus a periodic
    // refresh that also keeps the Redis TTL alive.
    #[cfg(feature = "storage-redis")]
    if let Some((ref cm, ref registry)) = _cluster_manager {
        // Inter-node trunks (phase 2) need a TLS listener to receive on,
        // the cluster CA to authenticate peers with, and a certificate to
        // present. Without all three this node still places and hosts
        // rooms, but a room spread across nodes would not carry audio
        // between them — so say so loudly.
        let trunk_manager = if config.sip.tls_bind.is_some()
            && config.sip.tls_client_ca_path.is_some()
            && config.sip.tls_client_identity().is_some()
        {
            conference_handler.set_trunk_srtp(config.cluster.trunk_srtp);
            let rtp_timeout = match config.cluster.trunk_rtp_timeout_secs {
                0 => None,
                secs => Some(Duration::from_secs(secs)),
            };
            let manager = Arc::new(trunks::TrunkManager::new(
                config.service.node_id.clone(),
                registry.clone(),
                Arc::clone(cm.members_map()),
                Arc::clone(&conference_handler),
                rtp_timeout,
            ));
            let interval = Duration::from_secs(config.cluster.trunk_reconcile_interval_secs);
            let _reconcile = Arc::clone(&manager).spawn_reconcile_loop(interval);
            info!(
                interval_secs = config.cluster.trunk_reconcile_interval_secs,
                tls = ?config.sip.advertised_tls_address(),
                srtp = config.cluster.trunk_srtp,
                rtp_timeout_secs = config.cluster.trunk_rtp_timeout_secs,
                "Inter-node conference trunks enabled"
            );
            Some(manager)
        } else {
            warn!(
                "Inter-node conference trunks disabled: set sip.tls_bind, sip.tls_client_ca_path \
                 and a TLS certificate so rooms can span conference nodes"
            );
            None
        };

        // Cross-node control (phase 3): forward lock/kick/mute/gain/
        // announce/end to the other nodes hosting a room, and accept the
        // same from them on /internal, all under one shared secret.
        let control = match config.cluster.secret.as_deref() {
            Some(secret) if !secret.is_empty() => {
                info!(
                    http = %config.advertised_http_address(),
                    "Cross-node conference control enabled"
                );
                Some(Arc::new(control::ClusterControl::new(
                    config.service.node_id.clone(),
                    registry.clone(),
                    Arc::clone(cm.members_map()),
                    secret.to_string(),
                )))
            }
            _ => {
                warn!(
                    "Cross-node conference control disabled: set cluster.secret (the same on \
                     every conference node) so lock, kick, mute, gain, announce and end reach \
                     every node hosting a room"
                );
                None
            }
        };
        cluster_control = control;

        struct RedisRoomRegistry {
            registry: fcp_cluster::ConferenceRegistry,
            trunks: Option<Arc<trunks::TrunkManager>>,
        }
        #[async_trait::async_trait]
        impl conference_handler::RoomRegistry for RedisRoomRegistry {
            async fn room_locked(&self, tenant_id: &str, room_number: &str) -> bool {
                match self.registry.room(tenant_id, room_number).await {
                    Ok(record) => record.locked,
                    Err(e) => {
                        warn!(error = %e, room = %room_number, "Conference registry lock lookup failed; room starts unlocked");
                        false
                    }
                }
            }

            fn room_participants_changed(
                &self,
                tenant_id: String,
                room_number: String,
                participants: usize,
            ) {
                let registry = self.registry.clone();
                let trunks = self.trunks.clone();
                tokio::spawn(async move {
                    if let Err(e) = registry
                        .set_participants(&tenant_id, &room_number, participants)
                        .await
                    {
                        warn!(error = %e, room = %room_number, "Conference registry update failed");
                    }
                    // Our own count just changed: dial or drop trunks now
                    // rather than waiting for the next reconcile tick.
                    if let Some(trunks) = trunks {
                        trunks.reconcile_room(&tenant_id, &room_number).await;
                    }
                });
            }
        }
        conference_handler.set_room_registry(Arc::new(RedisRoomRegistry {
            registry: registry.clone(),
            trunks: trunk_manager,
        }));

        let refresh_handler = Arc::clone(&conference_handler);
        let refresh_registry = registry.clone();
        let refresh_secs = config.cluster.heartbeat_interval_secs.clamp(5, 30);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(refresh_secs));
            loop {
                interval.tick().await;
                for (room, _) in refresh_handler.active_rooms() {
                    let tenant = refresh_handler
                        .tenant_for_room(&room)
                        .unwrap_or(refresh_handler.default_tenant_id)
                        .to_string();
                    let count = refresh_handler.countable_participants(&room);
                    if let Err(e) = refresh_registry
                        .set_participants(&tenant, &room, count)
                        .await
                    {
                        warn!(error = %e, room = %room, "Conference registry refresh failed");
                    }
                }
            }
        });
        info!("Conference registry publishing enabled");
    }

    if config.ai.enabled {
        info!(model = %config.ai.model, "AI participant support enabled");
    }

    info!(
        sample_rate = config.conference.sample_rate,
        frame_size = config.conference.frame_size,
        "ConferenceBridge created"
    );

    // 18. Create SipMessageHandler
    //     flow_handler = None (conference doesn't use flows)
    //     The conference handler is wired via the conference_handler field
    let handler =
        Arc::new(
            SipMessageHandler::new(
                Arc::clone(&transaction_manager_ref),
                Arc::clone(&dialog_manager_ref),
                local_uri,
                None, // No media profile — conference server handles RTP directly, no B2BUA anchoring
                local_addr,
                config.media.rtp_port_start,
                None, // registrar = None
                None, // routing_service = None
                Arc::clone(&call_manager),
                Some(Arc::clone(&uac)),
                Some(Arc::clone(&dyn_storage)),
                None, // No media manager — prevents evaluate_media_anchoring from anchoring
                None, // No media config — conference handles its own RTP
                None, // flow_handler = None
                config.b2bua.clone(),
                outbound_route_provider,
                None, // No PBX handler
                Some(Arc::clone(&conference_handler)
                    as Arc<dyn fcp_sip::conference::ConferenceHandler>),
                None,          // No queue handler
                None,          // presence_handler
                None,          // cluster_parking
                String::new(), // node_id
            )?
            .with_default_tenant_id(config.service.default_tenant_id),
        );

    // 19. Create and start TransportListener
    let udp_bind: std::net::SocketAddr = config
        .sip
        .udp_bind
        .parse()
        .context("Invalid UDP bind address")?;
    let tcp_bind: std::net::SocketAddr = config
        .sip
        .tcp_bind
        .parse()
        .context("Invalid TCP bind address")?;
    let ws_bind = config
        .sip
        .ws_bind
        .as_deref()
        .map(|bind| bind.parse().context("Invalid WS bind address"))
        .transpose()?;
    let wss_bind = config
        .sip
        .wss_bind
        .as_deref()
        .map(|bind| bind.parse().context("Invalid WSS bind address"))
        .transpose()?;

    #[cfg(feature = "tls")]
    let tls_bind = config
        .sip
        .tls_bind
        .as_deref()
        .map(|bind| bind.parse().context("Invalid TLS bind address"))
        .transpose()?;
    #[cfg(feature = "tls")]
    let tls_client_auth = Some(
        config
            .sip
            .tls_client_auth
            .parse::<fcp_sip::ClientAuthMode>()
            .map_err(|e| anyhow::anyhow!("sip.tls_client_auth: {e}"))?,
    );

    let listener_config = ListenerConfig {
        udp_bind: Some(udp_bind),
        tcp_bind: Some(tcp_bind),
        #[cfg(feature = "tls")]
        tls_bind,
        #[cfg(feature = "tls")]
        tls_cert: config.sip.tls_cert_path.clone(),
        #[cfg(feature = "tls")]
        tls_key: config.sip.tls_key_path.clone(),
        #[cfg(feature = "tls")]
        tls_client_ca: config.sip.tls_client_ca_path.clone(),
        #[cfg(feature = "tls")]
        tls_client_auth,
        ws_bind,
        wss_bind,
    };

    let handler_dyn: Arc<dyn InboundMessageHandler> =
        Arc::clone(&handler) as Arc<dyn InboundMessageHandler>;
    let mut listener = TransportListener::new(listener_config, handler_dyn);
    listener
        .start()
        .await
        .context("Failed to start transport listeners")?;

    // 20. Spawn TCP response routing task
    let handler_clone = Arc::clone(&handler);
    let mut client_rx = client_rx;
    tokio::spawn(async move {
        while let Some(packet) = client_rx.recv().await {
            let txn_transport = match packet.transport() {
                SipTransportKind::Udp => SipTransactionTransportKind::Udp,
                SipTransportKind::Tcp => SipTransactionTransportKind::Tcp,
                SipTransportKind::Tls => SipTransactionTransportKind::Tls,
                SipTransportKind::Ws => SipTransactionTransportKind::Ws,
                SipTransportKind::Wss => SipTransactionTransportKind::Wss,
                SipTransportKind::Sctp => SipTransactionTransportKind::Sctp,
                SipTransportKind::TlsSctp => SipTransactionTransportKind::TlsSctp,
            };

            if let Err(e) = handler_clone
                .handle_inbound(
                    txn_transport,
                    packet.peer(),
                    packet.payload().clone(),
                    packet.stream().cloned(),
                    packet.peer_identity().cloned(),
                )
                .await
            {
                tracing::error!("Failed to handle inbound TCP response: {}", e);
            }
        }
    });

    // 21. Spawn HTTP server (conference API)
    let events_dropped = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let conf_state = Arc::new(health::ConferenceState {
        bridge: Arc::clone(&bridge),
        storage: Arc::clone(&dyn_storage),
        conference_handler: Arc::clone(&conference_handler),
        call_manager: Arc::clone(&call_manager),
        event_bus: Arc::clone(&event_bus) as Arc<dyn fcp_events::EventPublisher>,
        default_tenant_id: config.service.default_tenant_id,
        start_time: Instant::now(),
        auth_enabled: config.http.auth_enabled,
        auth_token: config.http.auth_token.clone(),
        rate_limit_rps: config.http.rate_limit_rps,
        event_subscriber: Arc::clone(&event_bus),
        events_dropped: Arc::clone(&events_dropped),
        max_body_size_bytes: config.http.max_body_size_kb * 1024,
        cluster_secret: cluster_control
            .as_ref()
            .and(config.cluster.secret.clone())
            .filter(|s| !s.is_empty()),
        control: cluster_control,
    });
    let http_bind = config.http.bind_address;
    tokio::spawn(supervise_task("http-server", async move {
        if let Err(e) = health::run_http_server(http_bind, conf_state).await {
            tracing::error!(%e, "HTTP API server exited");
        }
    }));

    // 22. Spawn recording cleanup task (if retention configured)
    if let Some(days) = config.conference.recording_retention_days {
        if days > 0 {
            let recordings_dir = config.conference.recordings_dir.clone();
            let retention_days = days;
            let cleanup_interval_secs = config.conference.recording_cleanup_interval_secs;
            tokio::spawn(supervise_task("recording-cleanup", async move {
                run_recording_cleanup(recordings_dir, retention_days, cleanup_interval_secs).await;
            }));
            info!(retention_days = days, "Recording cleanup task started");
        }
    }

    // 23. Spawn config hot-reload watcher
    {
        let config_path = cli
            .config
            .clone()
            .unwrap_or_else(|| std::path::PathBuf::from("fcp-conference-server.toml"));
        if config_path.exists() {
            let handler_ref = Arc::clone(&conference_handler);
            let reload_interval = config.service.config_reload_interval_secs;
            tokio::spawn(supervise_task("config-watcher", async move {
                run_config_watcher(config_path, handler_ref, reload_interval).await;
            }));
            info!("Config hot-reload watcher started");
        }
    }

    // 24a. Spawn session timer reaper (RFC 4028)
    if config.conference.session_expires_secs > 0 {
        let reaper_handler = Arc::clone(&conference_handler);
        let reap_interval = config.conference.session_expires_secs;
        tokio::spawn(supervise_task("session-reaper", async move {
            // Check every half the session-expires interval
            let interval = Duration::from_secs(reap_interval / 2).max(Duration::from_secs(30));
            loop {
                tokio::time::sleep(interval).await;
                reaper_handler.reap_expired_sessions().await;
            }
        }));
        info!(
            session_expires_secs = config.conference.session_expires_secs,
            "Session timer reaper started"
        );
    }

    // 24a2. Spawn DTMF dial-out request poller
    {
        let dialout_handler = Arc::clone(&conference_handler);
        let dialout_bridge = Arc::clone(&bridge);
        tokio::spawn(supervise_task("dtmf-dialout-poller", async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                // Check each active room for pending requests
                for room_id in dialout_bridge.list_rooms() {
                    if let Ok(room) = dialout_bridge.get_room(&room_id) {
                        // Check for end-conference request (host pressed *00)
                        if room.take_end_conference_request() {
                            let handler = Arc::clone(&dialout_handler);
                            let rn = room_id.clone();
                            let ending_samples = handler.dtmf_ending_samples.clone();
                            tokio::spawn(async move {
                                info!(room = %rn, "Processing end-conference request");
                                // Play ending announcement to each participant's RTP socket
                                if let Some(ref pcm) = ending_samples {
                                    handler.play_to_all_participants(&rn, pcm).await;
                                }
                                // Now send BYE to all
                                match handler.destroy_conference(&rn).await {
                                    Ok(n) => {
                                        info!(room = %rn, kicked = n, "Conference ended by host")
                                    }
                                    Err(e) => {
                                        warn!(room = %rn, error = %e, "Failed to end conference")
                                    }
                                }
                            });
                            continue;
                        }

                        // Check for dial-out requests
                        let requests = room.drain_dial_out_requests();
                        for destination in requests {
                            let handler = Arc::clone(&dialout_handler);
                            let rn = room_id.clone();
                            let dest = destination.clone();
                            tokio::spawn(async move {
                                info!(room = %rn, destination = %dest, "Processing DTMF dial-out request");
                                let Some(tenant_id) = handler.tenant_for_room(&rn) else {
                                    warn!(
                                        room = %rn,
                                        destination = %dest,
                                        "Skipping DTMF dial-out because tenant context could not be resolved"
                                    );
                                    return;
                                };
                                match handler.dial_out(&rn, &dest, tenant_id, None, None).await {
                                    Ok(pid) => {
                                        info!(room = %rn, destination = %dest, participant = %pid, "DTMF dial-out connected")
                                    }
                                    Err(e) => {
                                        warn!(room = %rn, destination = %dest, error = %e, "DTMF dial-out failed")
                                    }
                                }
                            });
                        }
                    }
                }
            }
        }));
        info!("DTMF dial-out poller started");
    }

    // 24b. Spawn recording size monitor
    if config.conference.max_recording_size_mb > 0 {
        let monitor_handler = Arc::clone(&conference_handler);
        let monitor_bridge = Arc::clone(&bridge);
        let max_bytes = config.conference.max_recording_size_mb * 1024 * 1024;
        let recordings_dir = config.conference.recordings_dir.clone();
        let monitor_event_bus = Arc::clone(&event_bus) as Arc<dyn fcp_events::EventPublisher>;
        let size_check_interval =
            Duration::from_secs(config.conference.recording_size_check_interval_secs);
        tokio::spawn(supervise_task("recording-size-monitor", async move {
            loop {
                tokio::time::sleep(size_check_interval).await;
                // Check each actively-recording room by scanning for .wav files
                // that are currently being written (modified in the last 60s)
                let mut dir = match tokio::fs::read_dir(&recordings_dir).await {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                while let Ok(Some(entry)) = dir.next_entry().await {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) != Some("wav") {
                        continue;
                    }
                    if let Ok(meta) = tokio::fs::metadata(&path).await {
                        if meta.len() <= max_bytes {
                            continue;
                        }
                        // File exceeds limit — find the room by filename prefix and stop recording
                        let filename = match path.file_stem().and_then(|s| s.to_str()) {
                            Some(f) => f,
                            None => {
                                warn!(path = %path.display(), "Skipping recording with invalid filename");
                                continue;
                            }
                        };
                        // Filename format: {room}-{timestamp}-{uuid_simple} — extract room by
                        // stripping the last two segments (uuid is 32 hex chars, timestamp is digits).
                        // Parse from right to handle dashed room names (e.g. "conf-room-1-1707000-abc123def").
                        let room_id = filename
                            .rfind('-')
                            .and_then(|i| filename[..i].rfind('-'))
                            .map(|i| &filename[..i])
                            .unwrap_or(filename);
                        if let Ok(room) = monitor_bridge.get_room(room_id) {
                            if room.is_recording() {
                                warn!(
                                    room = %room_id,
                                    size_mb = meta.len() / (1024 * 1024),
                                    max_mb = max_bytes / (1024 * 1024),
                                    "Recording size limit reached, stopping"
                                );
                                if let Err(e) = room.stop_recording() {
                                    warn!(room = %room_id, error = %e, "Failed to stop recording on size limit");
                                }
                                if let Some(tenant_id) = monitor_handler.tenant_for_room(room_id) {
                                    let event = fcp_events::PlatformEvent::Conference(
                                        fcp_events::ConferenceEvent::RecordingStopped {
                                            room_number: room_id.to_string(),
                                            tenant_id,
                                            timestamp: std::time::SystemTime::now(),
                                        },
                                    );
                                    let _ = monitor_event_bus.publish(event).await;
                                } else {
                                    warn!(
                                        room = %room_id,
                                        "Skipped RecordingStopped event because tenant context could not be resolved"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }));
        info!(
            max_mb = config.conference.max_recording_size_mb,
            "Recording size monitor started"
        );
    }

    // 24c. Spawn event forwarding task (if configured)
    if let Some(ref events_url) = config.events.server_url {
        let events_url = events_url.clone();
        let events_config = config.events.clone();
        let event_bus_clone = Arc::clone(&event_bus);
        let dropped_counter = Arc::clone(&events_dropped);
        tokio::spawn(supervise_task("event-forwarder", async move {
            if let Err(e) = run_event_forwarder(
                event_bus_clone,
                &events_url,
                &events_config,
                dropped_counter,
            )
            .await
            {
                tracing::error!(%e, "Event forwarder task failed");
            }
        }));
        info!(
            events_url = %config.events.server_url.as_deref().unwrap_or(""),
            "Event forwarding enabled"
        );
    }

    // 23. Log startup summary
    info!(
        udp = %config.sip.udp_bind,
        tcp = %config.sip.tcp_bind,
        http = %config.http.bind_address,
        rtp_ports = format!("{}-{}", config.media.rtp_port_start, config.media.rtp_port_end),
        trunks = config.trunks.len(),
        events_forwarding = config.events.server_url.is_some(),
        "fcp-conference ready — listening for SIP INVITE messages"
    );

    // 24b2. Spawn config poll loop (checks DB every 10s for config changes)
    {
        let cs = Arc::clone(&config_store);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut last_version: i64 = 0;
            loop {
                interval.tick().await;
                match cs.get_latest_version("conference", "_system").await {
                    Ok(v) if v > last_version => {
                        tracing::info!(
                            old_version = last_version,
                            new_version = v,
                            "config version changed in database"
                        );
                        last_version = v;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::debug!(error = %e, "config poll: failed to check version");
                    }
                }
            }
        });
    }

    // 25. Wait for shutdown signal
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("Failed to register SIGTERM handler")?;

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("Received SIGINT (Ctrl+C), initiating graceful shutdown...");
        }
        _ = sigterm.recv() => {
            info!("Received SIGTERM, initiating graceful shutdown...");
        }
    }

    // 25. Graceful shutdown: drain, then hang up, then leave the cluster.
    //
    // Tell the cluster first, so placement stops sending participants
    // here before this node stops taking them; then refuse new callers;
    // then, if configured, give the callers already here a while to
    // leave on their own before hanging up whoever remains.
    #[cfg(feature = "storage-redis")]
    if let Some((ref cm, _)) = _cluster_manager {
        if let Err(e) = cm.set_draining(true).await {
            warn!(error = %e, "Could not advertise draining to the cluster");
        }
    }
    conference_handler.start_draining();

    if config.service.drain_wait_secs > 0 {
        let deadline = Instant::now() + Duration::from_secs(config.service.drain_wait_secs);
        let remaining_callers = || -> usize {
            conference_handler
                .active_rooms()
                .iter()
                .map(|(room, _)| conference_handler.countable_participants(room))
                .sum()
        };
        let mut callers = remaining_callers();
        info!(
            callers,
            wait_secs = config.service.drain_wait_secs,
            "Draining: waiting for callers to leave"
        );
        while callers > 0 && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_secs(1)).await;
            callers = remaining_callers();
        }
        if callers > 0 {
            warn!(
                callers,
                "Drain wait elapsed; hanging up the remaining callers"
            );
        } else {
            info!("Drained: every caller has left");
        }
    }

    // Rooms this node hosts, for the registry once they are gone.
    #[cfg(feature = "storage-redis")]
    let hosted_rooms: Vec<(String, String)> = conference_handler
        .active_rooms()
        .into_iter()
        .map(|(room, _)| {
            let tenant = conference_handler
                .tenant_for_room(&room)
                .unwrap_or(conference_handler.default_tenant_id)
                .to_string();
            (tenant, room)
        })
        .collect();

    let shutdown_timeout = Duration::from_secs(config.service.shutdown_timeout_secs);
    info!(
        timeout_secs = config.service.shutdown_timeout_secs,
        "Shutting down conference sessions..."
    );
    if shutdown_timeout.is_zero() {
        conference_handler.shutdown().await;
    } else {
        match tokio::time::timeout(shutdown_timeout, conference_handler.shutdown()).await {
            Ok(()) => info!("All conference sessions shut down cleanly"),
            Err(_) => warn!(
                timeout_secs = config.service.shutdown_timeout_secs,
                remaining_sessions = conference_handler.session_count(),
                "Shutdown timeout reached, force-closing remaining sessions"
            ),
        }
    }

    // Leave every room in the registry now, awaited, rather than trusting
    // the per-session updates spawned during shutdown to land before the
    // process exits; then drop out of membership.
    #[cfg(feature = "storage-redis")]
    if let Some((ref cm, ref registry)) = _cluster_manager {
        for (tenant, room) in &hosted_rooms {
            if let Err(e) = registry.leave_room(tenant, room).await {
                warn!(room = %room, error = %e, "Could not leave room in conference registry");
            }
        }
        cm.shutdown().await;
    }

    if let Some(tx) = trunk_shutdown_tx {
        let _ = tx.send(true);
        tokio::time::sleep(Duration::from_secs(4)).await;
    }
    listener.stop().await?;
    info!("Shutdown complete");
    Ok(())
}

/// Create a transport dispatcher with shared TCP pool.
async fn create_transport_dispatcher(
    config: &ConferenceServerConfig,
    tcp_pool: Arc<ConnectionPool>,
) -> Result<Arc<fcp_sip::transport::RealTransportDispatcher>> {
    use fcp_sip::transport::RealTransportDispatcher;

    let bind_addr: std::net::SocketAddr = config
        .sip
        .udp_bind
        .parse()
        .context("Invalid UDP bind address")?;

    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )
    .context("Failed to create UDP socket")?;
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&bind_addr.into())?;
    let std_socket: std::net::UdpSocket = socket.into();
    let udp_socket = tokio::net::UdpSocket::from_std(std_socket)?;

    #[allow(unused_mut)]
    let mut dispatcher =
        RealTransportDispatcher::new_with_pool(Some(Arc::new(udp_socket)), tcp_pool);

    // Outbound TLS: system roots plus the cluster CA (peers dialed by
    // address are accepted under the name their certificate carries), and
    // this node's certificate presented to peers that ask (mutual TLS).
    #[cfg(feature = "tls")]
    if config.sip.tls_client_ca_path.is_some() || config.sip.tls_client_identity().is_some() {
        let tls_client = RealTransportDispatcher::load_tls_client_config_with(
            config.sip.tls_client_ca_path.as_deref(),
            config.sip.tls_client_identity(),
        )
        .context("Failed to build outbound TLS client config")?;
        dispatcher = dispatcher.with_tls_config(tls_client);
        info!(
            client_ca = ?config.sip.tls_client_ca_path,
            client_cert = ?config.sip.tls_client_identity().map(|(c, _)| c.to_string()),
            "Outbound SIP/TLS configured"
        );
    }

    Ok(Arc::new(dispatcher))
}

/// Create a UAC for outbound calls.
async fn create_uac(
    config: &ConferenceServerConfig,
    transaction_manager: Arc<TransactionManager>,
    dispatcher: Arc<fcp_sip::transport::RealTransportDispatcher>,
    credential_provider: Option<Arc<TrunkCredentialProvider>>,
) -> Result<Arc<SipUserAgentClient>> {
    let local_uri = create_local_uri(config)?;

    let bind_addr: std::net::SocketAddr = config
        .sip
        .udp_bind
        .parse()
        .context("Invalid UDP bind address")?;

    let local_addr = if let Some(ref local_ip) = config.sip.local_ip {
        let port = if bind_addr.port() == 0 {
            5060
        } else {
            bind_addr.port()
        };
        format!("{}:{}", local_ip, port)
    } else {
        let port = if bind_addr.port() == 0 {
            5060
        } else {
            bind_addr.port()
        };
        format!("{}:{}", bind_addr.ip(), port)
    };

    let resolver = Arc::new(SipResolver::from_system().context("Failed to create DNS resolver")?);

    let mut builder = IntegratedUAC::builder();
    builder = builder
        .local_uri(local_uri.as_str())?
        .local_addr(&local_addr)?
        .via_advertised_addr(&local_addr)?
        .transaction_manager(transaction_manager)
        .resolver(resolver)
        .dispatcher(dispatcher);

    if let Some(cred_provider) = credential_provider {
        builder = builder.credential_provider(cred_provider);
    }

    let integrated_uac = builder.build().context("Failed to build IntegratedUAC")?;

    Ok(Arc::new(SipUserAgentClient::new(Arc::new(integrated_uac))))
}

/// Create a local SIP URI from configuration.
fn create_local_uri(config: &ConferenceServerConfig) -> Result<SipUri> {
    let bind_addr: std::net::SocketAddr = config
        .sip
        .udp_bind
        .parse()
        .context("Invalid UDP bind address")?;

    if let Some(ref uri_str) = config.sip.local_uri {
        let uri = Uri::parse(uri_str).context("Failed to parse configured local_uri")?;
        return uri
            .as_sip()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Configured local_uri is not a SIP URI"));
    }

    let domain = if bind_addr.ip().is_unspecified() {
        config
            .sip
            .local_ip
            .as_ref()
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| "127.0.0.1".to_string())
    } else {
        bind_addr.ip().to_string()
    };

    let uri_str = if bind_addr.port() != 0 && bind_addr.port() != 5060 {
        format!("sip:conference@{}:{}", domain, bind_addr.port())
    } else {
        format!("sip:conference@{}", domain)
    };

    let uri = Uri::parse(&uri_str).context("Failed to parse local URI")?;
    uri.as_sip()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Not a SIP URI"))
}

/// Create a media manager from configuration.
async fn create_media_manager(
    config: &ConferenceServerConfig,
) -> Result<Arc<fcp_media::MediaManager>> {
    let media_config = MediaConfig {
        backend: BackendType::Embedded,
        rtp_port_start: config.media.rtp_port_start,
        rtp_port_end: config.media.rtp_port_end,
        public_ip: config.media.public_ip.clone(),
        ..Default::default()
    };

    let manager = fcp_media::MediaManager::new(media_config)
        .await
        .context("Failed to create media manager")?;

    info!(
        backend = manager.backend_name(),
        rtp_ports = format!(
            "{}-{}",
            config.media.rtp_port_start, config.media.rtp_port_end
        ),
        "Media manager created"
    );

    Ok(Arc::new(manager))
}

fn derive_local_ip(bind_addr: &str) -> String {
    if let Ok(addr) = bind_addr.parse::<std::net::SocketAddr>() {
        if !addr.ip().is_unspecified() {
            return addr.ip().to_string();
        }
    }
    auto_detect_local_ip().unwrap_or_else(|| "127.0.0.1".to_string())
}

fn auto_detect_local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:53").ok()?;
    let addr = socket.local_addr().ok()?;
    if addr.ip().is_unspecified() || addr.ip().is_loopback() {
        return None;
    }
    Some(addr.ip().to_string())
}

fn init_tracing(config: &ConferenceServerConfig) {
    use tracing_subscriber::EnvFilter;

    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.logging.level));

    match config.logging.format.as_str() {
        "json" => {
            tracing_subscriber::fmt()
                .with_target(false)
                .with_level(true)
                .json()
                .with_env_filter(env_filter)
                .init();
        }
        "compact" => {
            tracing_subscriber::fmt()
                .with_target(false)
                .with_level(true)
                .compact()
                .with_env_filter(env_filter)
                .init();
        }
        _ => {
            tracing_subscriber::fmt()
                .with_target(false)
                .with_level(true)
                .with_env_filter(env_filter)
                .init();
        }
    }
}

/// Periodically delete recordings older than the retention period.
/// Uses exponential backoff on directory read failure (1min → 2min → 4min → ... → 1hr cap).
async fn run_recording_cleanup(
    recordings_dir: String,
    retention_days: u32,
    cleanup_interval_secs: u64,
) {
    let retention = Duration::from_secs(retention_days as u64 * 86400);
    let normal_interval = Duration::from_secs(cleanup_interval_secs);
    let mut backoff = Duration::from_secs(60);
    let max_backoff = Duration::from_secs(3600);

    loop {
        let dir = match std::fs::read_dir(&recordings_dir) {
            Ok(d) => d,
            Err(e) => {
                warn!(error = %e, backoff_secs = backoff.as_secs(), "Recording cleanup: cannot read directory, backing off");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(max_backoff);
                continue;
            }
        };

        // Success — reset backoff
        backoff = Duration::from_secs(60);

        let mut deleted = 0u32;
        for entry in dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("wav") {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                // Skip non-regular files (symlinks, directories) to avoid following
                // symlinks that point outside the recordings directory
                if !meta.is_file() {
                    continue;
                }
                let age = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .unwrap_or_default();
                if age > retention {
                    if let Err(e) = std::fs::remove_file(&path) {
                        warn!(path = %path.display(), error = %e, "Failed to delete old recording");
                    } else {
                        deleted += 1;
                    }
                }
            }
        }

        if deleted > 0 {
            info!(
                deleted = deleted,
                retention_days = retention_days,
                "Recording cleanup completed"
            );
        }

        tokio::time::sleep(normal_interval).await;
    }
}

async fn run_event_forwarder(
    event_bus: Arc<InMemoryEventBus>,
    events_url: &str,
    events_config: &config::EventsSection,
    dropped_metric: Arc<std::sync::atomic::AtomicU64>,
) -> Result<()> {
    use std::sync::atomic::{AtomicBool, AtomicU64};

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(events_config.timeout_secs))
        .build()
        .context("Failed to create HTTP client for event forwarding")?;

    let mut stream = event_bus
        .subscribe(EventFilter::new())
        .await
        .context("Failed to subscribe to event bus")?;

    info!(url = %events_url, "Event forwarder started");

    // Circuit breaker state (shared with spawned delivery tasks)
    let consecutive_failures = Arc::new(AtomicU64::new(0));
    let circuit_open = Arc::new(AtomicBool::new(false));
    let circuit_opened_at = Arc::new(tokio::sync::Mutex::new(Instant::now()));
    let failure_threshold = events_config.circuit_breaker_threshold;
    let cooldown = Duration::from_secs(events_config.circuit_breaker_cooldown_secs);

    // Concurrency limiter
    let semaphore = Arc::new(tokio::sync::Semaphore::new(events_config.max_concurrent));
    let max_retries = events_config.max_retries;
    let auth_header = events_config
        .auth_token
        .as_deref()
        .map(|token| format!("Bearer {}", token));

    let mut delivered_events: u64 = 0;

    loop {
        match stream.next().await {
            Some(event) => {
                // Circuit breaker check
                if circuit_open.load(std::sync::atomic::Ordering::Relaxed) {
                    let opened = *circuit_opened_at.lock().await;
                    if opened.elapsed() < cooldown {
                        // Circuit open — increment shared dropped metric (visible via /metrics)
                        let total_dropped =
                            dropped_metric.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                        if total_dropped % 100 == 1 {
                            warn!(
                                dropped = total_dropped,
                                cooldown_remaining_secs = (cooldown - opened.elapsed()).as_secs(),
                                "Circuit breaker OPEN — dropping events"
                            );
                        }
                        continue;
                    }
                    // Cooldown expired — transition to half-open (allow one probe)
                    info!("Circuit breaker half-open, probing...");
                    circuit_open.store(false, std::sync::atomic::Ordering::Relaxed);
                    consecutive_failures.store(0, std::sync::atomic::Ordering::Relaxed);
                }

                tracing::debug!(?event, "Forwarding event to events server");
                let client = client.clone();
                let url = events_url.to_string();
                let failures = Arc::clone(&consecutive_failures);
                let cb_open = Arc::clone(&circuit_open);
                let cb_opened_at = Arc::clone(&circuit_opened_at);
                let permit = Arc::clone(&semaphore);
                let drop_counter = Arc::clone(&dropped_metric);
                let auth_header = auth_header.clone();
                tokio::spawn(async move {
                    // Acquire semaphore permit to bound concurrency
                    let _permit = match permit.acquire().await {
                        Ok(p) => p,
                        Err(_) => return, // Semaphore closed (shutdown)
                    };
                    if forward_event_with_retry(
                        &client,
                        &url,
                        &event,
                        auth_header.as_deref(),
                        max_retries,
                    )
                    .await
                    {
                        // Success — reset failure counter
                        failures.store(0, std::sync::atomic::Ordering::Relaxed);
                    } else {
                        // Failure — increment dropped metric and maybe trip circuit
                        drop_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let prev = failures.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if prev + 1 >= failure_threshold
                            && !cb_open.load(std::sync::atomic::Ordering::Relaxed)
                        {
                            cb_open.store(true, std::sync::atomic::Ordering::Relaxed);
                            *cb_opened_at.lock().await = Instant::now();
                            tracing::error!(
                                consecutive_failures = prev + 1,
                                cooldown_secs = cooldown.as_secs(),
                                "Circuit breaker OPENED — event forwarding paused"
                            );
                        }
                    }
                });
                delivered_events += 1;
            }
            None => {
                warn!(
                    delivered = delivered_events,
                    dropped = dropped_metric.load(std::sync::atomic::Ordering::Relaxed),
                    "Event stream ended unexpectedly, resubscribing..."
                );
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                match event_bus.subscribe(EventFilter::new()).await {
                    Ok(new_stream) => {
                        stream = new_stream;
                        info!("Resubscribed to event bus");
                    }
                    Err(e) => {
                        warn!(%e, "Failed to resubscribe to event bus");
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                }
            }
        }
    }
}

/// Forward a single event with exponential backoff retry.
/// Returns `true` on success, `false` if all retries exhausted.
async fn forward_event_with_retry(
    client: &reqwest::Client,
    url: &str,
    event: &fcp_events::PlatformEvent,
    auth_header: Option<&str>,
    max_retries: u32,
) -> bool {
    let mut delay = Duration::from_millis(200);

    let json_body = match serde_json::to_vec(event) {
        Ok(body) => body,
        Err(e) => {
            warn!(error = %e, "Failed to serialize event for forwarding");
            return false;
        }
    };

    for attempt in 1..=max_retries {
        let mut request = client
            .post(url)
            .header("content-type", "application/json")
            .body(json_body.clone());
        if let Some(auth_header) = auth_header {
            request = request.header("Authorization", auth_header);
        }

        match request.send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    return true;
                }
                tracing::warn!(
                    status = %resp.status(),
                    attempt = attempt,
                    "Event forward received non-2xx response"
                );
            }
            Err(e) => {
                tracing::warn!(
                    %e,
                    attempt = attempt,
                    max = max_retries,
                    "Failed to forward event"
                );
            }
        }

        if attempt < max_retries {
            tokio::time::sleep(delay).await;
            delay *= 2; // Exponential backoff
        }
    }

    tracing::error!("Event dropped after {} failed attempts", max_retries);
    false
}

/// Supervisor wrapper for long-running background tasks.
///
/// If the inner future panics, catches the panic, logs it, and does NOT restart
/// (the task is considered dead). This prevents silent task disappearance.
async fn supervise_task(
    name: &'static str,
    task: impl std::future::Future<Output = ()> + Send + 'static,
) {
    match tokio::task::spawn(task).await {
        Ok(()) => {
            warn!(task = name, "Background task exited normally");
        }
        Err(e) => {
            if e.is_panic() {
                tracing::error!(task = name, "Background task PANICKED: {}", e);
            } else {
                tracing::error!(task = name, "Background task cancelled: {}", e);
            }
        }
    }
}

/// Watch config file for changes and hot-reload conference/AI settings.
/// Polls file mtime at a configurable interval. Only reloads [conference] and [ai] sections.
async fn run_config_watcher(
    config_path: std::path::PathBuf,
    handler: Arc<ConferenceCallHandler>,
    poll_interval_secs: u64,
) {
    let mut last_mtime = std::fs::metadata(&config_path)
        .and_then(|m| m.modified())
        .ok();

    loop {
        tokio::time::sleep(Duration::from_secs(poll_interval_secs)).await;

        let current_mtime = match std::fs::metadata(&config_path).and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(e) => {
                warn!(error = %e, "Config watcher: cannot stat config file");
                continue;
            }
        };

        if last_mtime == Some(current_mtime) {
            continue;
        }
        last_mtime = Some(current_mtime);

        info!(path = %config_path.display(), "Config file changed, reloading...");

        match config::load_config(Some(&config_path)) {
            Ok(new_config) => {
                // Validate before applying
                if let Err(e) = config::validate_config(&new_config) {
                    warn!(error = %e, "Config reload rejected: validation failed, keeping current config");
                    continue;
                }
                handler.reload_config(new_config.conference, new_config.ai);
            }
            Err(e) => {
                warn!(error = %e, "Config reload failed, keeping current config");
            }
        }
    }
}
