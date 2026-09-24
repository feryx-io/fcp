//! fcp-sbc — Session Border Controller for the Ferrous Communications Platform.
//!
//! Enforces IP ACLs, message size limits, rate limiting, and admission control
//! before delegating to the inner B2BUA SipMessageHandler. Supports trunk
//! routing, health monitoring, and topology hiding.

use anyhow::{Context, Result};
use clap::Parser;
use fcp_sip::prelude::*;
use fcp_sip::registrar::FcpRegistrar;
use fcp_sip::service::{DialogService, TransactionService};
use fcp_sip::transport::ConnectionPool;
use sip_observe::{set_transport_metrics, TracingTransportMetrics};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

mod admin;
mod cli;
mod config;
mod drain;
mod enforcement;
mod health;
mod hmr_dispatcher;
mod hooks;
mod local_capture;
mod metrics;
mod nat;
mod recording_store;
mod registration_store;
mod siprec;
mod srtp;
mod stir_shaken;
mod topology;

use cli::Cli;
use enforcement::SbcEnforcementHandler;
use fcp_sbc_core::admission::AdmissionController;
use fcp_sbc_core::health_probes;
use fcp_sbc_core::trunk_auth;
use fcp_sbc_core::trunk_health::TrunkHealthMonitor;
use fcp_sbc_core::trunk_registration::{self, TrunkRegistrar};
use fcp_sbc_core::trunk_router::TrunkRouter;
use fcp_sbc_core::CpsChecker;
use fcp_sbc_core::HeaderManipulationEngine;
use health::HealthState;
use hooks::SbcEventHooks;
use metrics::SbcRuntimeMetrics;
use registration_store::InMemoryRegistrationStore;

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Parse CLI arguments
    let cli = Cli::parse();

    // 2. Load configuration from TOML (or defaults)
    let deploy_config = fcp_core::config::check_stage(
        config::load_config(cli.config.as_deref()),
        cli.check_config,
    )?;
    let mut sbc_config = deploy_config.core.clone();

    // 3. Initialize tracing subscriber
    init_tracing(&cli.log_level, &cli.log_format);

    info!(
        version = env!("CARGO_PKG_VERSION"),
        bind = %cli.bind,
        health_bind = %cli.health_bind,
        "fcp-sbc starting"
    );

    // 4. Validate config (best-effort; warn but don't abort if external_domain empty)
    if let Err(e) = sbc_config.validate() {
        warn!(%e, "SBC configuration validation warning");
    }

    // 4a. The health listener's rule, checked before anything is bound so
    // `--check-config` sees it: a non-loopback bind needs the token.
    let health_bind: std::net::SocketAddr = fcp_core::config::check_stage(
        cli.health_bind
            .parse()
            .context("Invalid health bind address"),
        cli.check_config,
    )?;
    fcp_core::config::check_stage(
        if !health_bind.ip().is_loopback() && cli.health_api_token.is_none() {
            Err(anyhow::anyhow!(
                "FCP_SBC_HEALTH_API_TOKEN or --health-api-token is required when --health-bind is non-loopback ({})",
                health_bind
            ))
        } else {
            Ok(())
        },
        cli.check_config,
    )?;
    if cli.check_config {
        println!(
            "{}",
            fcp_core::config::masked_toml(&deploy_config).map_err(anyhow::Error::msg)?
        );
        return Ok(());
    }

    // 4b. Warn about unsupported media feature flags
    if sbc_config.nat.enable_srtp {
        info!("SRTP support enabled (SDES-SRTP, RFC 4568)");
    }
    if sbc_config.nat.enable_transcoding {
        info!("Codec transcoding enabled (forge-media pipeline)");
    }
    if sbc_config.nat.enable_dtmf {
        info!("DTMF support enabled (RFC 2833 relay + SIP INFO passthrough)");
    }

    if sbc_config.b2bua.session_timer.enabled {
        info!(
            session_expires = sbc_config.b2bua.session_timer.session_expires_secs,
            min_se = sbc_config.b2bua.session_timer.min_se_secs,
            "RFC 4028 session timer enforcement enabled"
        );
    } else {
        warn!(
            "RFC 4028 session timer enforcement is disabled — long-running \
             calls with dead endpoints will not be automatically cleaned up"
        );
    }

    // 5. Create UDP socket with SO_REUSEADDR
    let bind_addr: std::net::SocketAddr = cli.bind.parse().context("Invalid bind address")?;

    let domain = if bind_addr.is_ipv4() {
        socket2::Domain::IPV4
    } else {
        socket2::Domain::IPV6
    };

    let socket = socket2::Socket::new(domain, socket2::Type::DGRAM, Some(socket2::Protocol::UDP))
        .context("Failed to create UDP socket")?;
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;

    // For IPv6: dual-stack when binding to [::], v6-only for specific addresses
    if bind_addr.is_ipv6() {
        let v6_only = !bind_addr.ip().is_unspecified();
        socket.set_only_v6(v6_only)?;
    }

    socket.bind(&bind_addr.into())?;
    let std_socket: std::net::UdpSocket = socket.into();
    let udp_socket = tokio::net::UdpSocket::from_std(std_socket)?;

    // 6. Create TCP ConnectionPool
    let tcp_pool = Arc::new(ConnectionPool::new());
    let (client_tx, client_rx) = tokio::sync::mpsc::channel(1024);
    tcp_pool.set_inbound_tx(client_tx).await;

    // 7. Create RealTransportDispatcher
    let dispatcher = Arc::new(fcp_sip::transport::RealTransportDispatcher::new_with_pool(
        Some(Arc::new(udp_socket)),
        Arc::clone(&tcp_pool),
    ));

    // Set up transport metrics
    if !set_transport_metrics(Arc::new(TracingTransportMetrics)) {
        warn!("transport metrics already configured");
    }

    // 7b. Build HMR engine (if enabled) and wrap dispatcher for egress
    let hmr_engine = if sbc_config.header_manipulation.enabled {
        Some(Arc::new(
            HeaderManipulationEngine::new(&sbc_config.header_manipulation, &sbc_config.trunks)
                .map_err(|e| anyhow::anyhow!("Failed to compile HMR rules: {e}"))?,
        ))
    } else {
        None
    };

    let effective_dispatcher: Arc<dyn TransportDispatcher> = if let Some(ref engine) = hmr_engine {
        Arc::new(hmr_dispatcher::HmrTransportDispatcher::new(
            Arc::clone(&dispatcher) as Arc<dyn TransportDispatcher>,
            Arc::clone(engine),
            &sbc_config.trunks,
        ))
    } else {
        Arc::clone(&dispatcher) as Arc<dyn TransportDispatcher>
    };

    // 7c. STIR/SHAKEN verifier, signer, metrics (created early so signer can wrap dispatcher)
    let stir_metrics = if sbc_config.stir_shaken.enabled {
        Some(Arc::new(stir_shaken::StirMetrics::new()))
    } else {
        None
    };
    let stir_verifier = if sbc_config.stir_shaken.enabled {
        Some(Arc::new(stir_shaken::StirVerifier::new(
            &sbc_config.stir_shaken,
        )))
    } else {
        None
    };
    let stir_signer = if sbc_config.stir_shaken.enabled {
        stir_shaken::StirSigner::new(&sbc_config.stir_shaken)
            .map_err(|e| anyhow::anyhow!("Failed to load STIR/SHAKEN signing key: {e}"))?
            .map(Arc::new)
    } else {
        None
    };

    // 7d. Wrap dispatcher with STIR/SHAKEN signing (after HMR, before TransactionManager)
    let effective_dispatcher: Arc<dyn TransportDispatcher> = if let Some(ref signer) = stir_signer {
        Arc::new(stir_shaken::StirTransportDispatcher::new(
            effective_dispatcher,
            Arc::clone(signer),
            &sbc_config.trunks,
            Arc::clone(stir_metrics.as_ref().unwrap()),
        ))
    } else {
        effective_dispatcher
    };

    // 8. Create TransactionManager (uses effective dispatcher for HMR + STIR egress)
    let transaction_manager =
        TransactionManager::new_with_pool(effective_dispatcher.clone(), tcp_pool);
    let transaction_service = TransactionService::new(transaction_manager);
    let transaction_manager_ref = Arc::clone(transaction_service.manager());

    // 9. Create DialogManager
    let dialog_manager = DialogManager::new();
    let dialog_service = DialogService::new(dialog_manager);
    let dialog_manager_ref = Arc::clone(dialog_service.manager());

    // 10. Create UAC
    let local_addr = derive_local_addr(&cli.bind);
    let local_uri = create_local_uri(&cli.bind)?;

    let resolver = Arc::new(SipResolver::from_system().context("Failed to create DNS resolver")?);
    let credential_provider = Arc::new(trunk_auth::TrunkCredentialProvider::from_trunks(
        &sbc_config.trunks,
    ));
    let integrated_uac = IntegratedUAC::builder()
        .local_uri(local_uri.as_str())?
        .local_addr(&local_addr)?
        .via_advertised_addr(&local_addr)?
        .transaction_manager(Arc::clone(&transaction_manager_ref))
        .resolver(resolver)
        .dispatcher(Arc::clone(&effective_dispatcher))
        .credential_provider(credential_provider)
        .build()
        .context("Failed to build IntegratedUAC")?;
    let uac = Arc::new(SipUserAgentClient::new(Arc::new(integrated_uac)));

    // 11. Create AdmissionController, TrunkHealthMonitor, SbcRuntimeMetrics FIRST
    let admission = Arc::new(AdmissionController::new(sbc_config.admission.clone()));
    let trunk_health = Arc::new(TrunkHealthMonitor::new(&sbc_config.trunks));
    let sbc_metrics = Arc::new(SbcRuntimeMetrics::new());

    // 11b. Create TrunkRegistrar and shutdown channel for graceful de-registration
    let trunk_registrar = Arc::new(TrunkRegistrar::new());
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // 12. Compute effective media IP and create media stack for NAT/SDP rewriting
    //     (moved before hooks so LocalCaptureManager can use media_manager)
    let local_ip = derive_local_ip(&cli.bind);
    let effective_ip = nat::effective_media_ip(&sbc_config.nat, &local_ip);

    let (media_manager, media_config) = if sbc_config.nat.enabled && sbc_config.nat.rewrite_sdp {
        let fcp_media_cfg = nat::build_fcp_media_config(&sbc_config.nat, &effective_ip);
        let mgr = fcp_media::MediaManager::new(fcp_media_cfg)
            .await
            .context("Failed to create media manager for NAT/SDP rewriting")?;
        let core_cfg = nat::build_fcp_core_media_config(&sbc_config.nat, &effective_ip);
        (Some(Arc::new(mgr)), Some(core_cfg))
    } else {
        (None, None)
    };

    // 12a. Create recording store and local capture manager
    let (recording_store, local_capture) = if sbc_config.siprec.local_capture_enabled {
        let store = Arc::new(recording_store::RecordingStore::new());
        let capture = local_capture::LocalCaptureManager::new(
            sbc_config.siprec.clone(),
            Arc::clone(
                media_manager
                    .as_ref()
                    .expect("media manager required for local capture (enable nat.rewrite_sdp)"),
            ),
            Arc::clone(&store),
        );
        (Some(store), capture.map(Arc::new))
    } else {
        (None, None)
    };

    if sbc_config.siprec.local_capture_enabled {
        info!(
            recording_dir = %sbc_config.siprec.recording_dir,
            format = %sbc_config.siprec.recording_format,
            "Local call recording enabled"
        );
    }

    // 12b. Create SIPREC recorder (if configured)
    let siprec_recorder = siprec::SiprecRecorder::new(sbc_config.siprec.clone(), Arc::clone(&uac));
    let siprec_recorder = siprec_recorder.map(Arc::new);

    // 12b2. Create quality tracker early (needed by hooks AND TrunkRouter)
    let quality_tracker = if sbc_config.lcr.enabled || sbc_config.metrics.per_trunk_metrics {
        let trunk_names: Vec<String> = sbc_config.trunks.iter().map(|t| t.name.clone()).collect();
        Some(Arc::new(fcp_sbc_core::TrunkQualityTracker::new(
            &trunk_names,
            sbc_config.lcr.quality_window_secs,
        )))
    } else {
        None
    };

    // 12c. Create event hooks so call/registration lifecycle updates admission/metrics/trunk health
    let sbc_hooks: Arc<dyn fcp_events::EventPublisher> = {
        let hooks = match &siprec_recorder {
            Some(recorder) => SbcEventHooks::with_recorder(
                Arc::clone(&admission),
                Arc::clone(&sbc_metrics),
                Arc::clone(recorder),
                Some(Arc::clone(&trunk_health)),
                local_capture,
            ),
            None => SbcEventHooks::new(
                Arc::clone(&admission),
                Arc::clone(&sbc_metrics),
                Some(Arc::clone(&trunk_health)),
                local_capture,
            ),
        };
        // Wire quality tracker into hooks if available
        let hooks = if let Some(ref qt) = quality_tracker {
            hooks.with_quality(Arc::clone(qt))
        } else {
            hooks
        };
        Arc::new(hooks)
    };
    // 12d. Initialize optional storage for CDR persistence
    let (dyn_storage, config_store, replica_health): (
        Option<Arc<fcp_core::DynStorage>>,
        Option<Arc<dyn fcp_core::config_store::ConfigStore>>,
        fcp_storage_postgres::ReplicaHealth,
    ) = if sbc_config.storage.enabled {
        let pg_config = fcp_storage_postgres::PostgresConfig {
            connection_string: sbc_config.storage.database_url.clone(),
            read_replica_url: sbc_config.storage.read_replica_url.clone(),
            max_replica_lag: Duration::from_secs(sbc_config.storage.max_replica_lag_secs),
            max_connections: 10,
            connection_timeout: std::time::Duration::from_secs(30),
            encryption: None,
        };
        let pg_storage = fcp_storage_postgres::PostgresStorage::new(pg_config)
            .await
            .context("Failed to initialize PostgreSQL storage for CDR")?;
        let pg_storage = Arc::new(pg_storage);
        let storage = Arc::new(fcp_core::DynStorage::from_all_in_one(pg_storage.clone()));
        info!(db = %sbc_config.storage.database_url, "CDR storage initialized");
        (
            Some(storage),
            Some(pg_storage.clone() as Arc<dyn fcp_core::config_store::ConfigStore>),
            fcp_storage_postgres::ReplicaHealth::of(pg_storage),
        )
    } else {
        (None, None, fcp_storage_postgres::ReplicaHealth::none())
    };

    // 12d2. Seed operational config sections into database (DB primary, TOML seed)
    if let Some(ref config_store) = config_store {
        let cs = config_store.as_ref();
        let svc = "sbc";
        let tid = "_system";

        let sections: Vec<(&str, serde_json::Value)> = vec![
            (
                "admission",
                serde_json::to_value(&sbc_config.admission).unwrap_or_default(),
            ),
            (
                "rate_limit",
                serde_json::to_value(&sbc_config.rate_limit).unwrap_or_default(),
            ),
            (
                "security",
                serde_json::to_value(&sbc_config.security).unwrap_or_default(),
            ),
            (
                "nat",
                serde_json::to_value(&sbc_config.nat).unwrap_or_default(),
            ),
            (
                "siprec",
                serde_json::to_value(&sbc_config.siprec).unwrap_or_default(),
            ),
            (
                "metrics",
                serde_json::to_value(&sbc_config.metrics).unwrap_or_default(),
            ),
            (
                "lcr",
                serde_json::to_value(&sbc_config.lcr).unwrap_or_default(),
            ),
            (
                "header_manipulation",
                serde_json::to_value(&sbc_config.header_manipulation).unwrap_or_default(),
            ),
            (
                "stir_shaken",
                serde_json::to_value(&sbc_config.stir_shaken).unwrap_or_default(),
            ),
            (
                "b2bua",
                serde_json::to_value(&sbc_config.b2bua).unwrap_or_default(),
            ),
            (
                "cluster",
                serde_json::to_value(&deploy_config.cluster).unwrap_or_default(),
            ),
        ];

        for (section, toml_val) in sections {
            match cs.get_config(svc, section, tid).await {
                Ok(Some(entry)) => {
                    match section {
                        "admission" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                sbc_config.admission = v;
                            }
                        }
                        "rate_limit" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                sbc_config.rate_limit = v;
                            }
                        }
                        "security" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                sbc_config.security = v;
                            }
                        }
                        "nat" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                sbc_config.nat = v;
                            }
                        }
                        "siprec" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                sbc_config.siprec = v;
                            }
                        }
                        "metrics" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                sbc_config.metrics = v;
                            }
                        }
                        "lcr" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                sbc_config.lcr = v;
                            }
                        }
                        "header_manipulation" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                sbc_config.header_manipulation = v;
                            }
                        }
                        "stir_shaken" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                sbc_config.stir_shaken = v;
                            }
                        }
                        "b2bua" => {
                            if let Ok(v) = serde_json::from_value(entry.value) {
                                sbc_config.b2bua = v;
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

    let call_manager = match &dyn_storage {
        Some(storage) => {
            let mut manager = fcp_sip::CallManager::with_storage_and_events(
                Arc::clone(storage),
                Arc::clone(&sbc_hooks),
            );
            // The CDR pipeline's node side (docs/CDR_PIPELINE.md §4.1).
            if let Some(billing) = manager.billing().cloned() {
                let processor = Arc::new(fcp_sip::CallManager::processor(
                    storage,
                    &billing,
                    Some(Arc::clone(&sbc_hooks) as Arc<dyn fcp_events::EventPublisher>),
                ));
                // The SBC keeps no Redis: its records are stored locally.
                let writer = fcp_cdr::start_node(&sbc_config.storage.cdr, Some(processor), None)
                    .context("Failed to start the CDR spool")?;
                manager.set_cdr_writer(writer);
            }
            Arc::new(manager)
        }
        None => Arc::new(fcp_sip::CallManager::with_events(Arc::clone(&sbc_hooks))),
    };

    // 12e. Create registrar with event publishing so registration lifecycle
    //      updates admission counters (registration_added / registration_removed)
    let reg_store: Arc<dyn fcp_core::RegistrationStore> =
        Arc::new(InMemoryRegistrationStore::default());
    let registrar = Arc::new(
        FcpRegistrar::with_events(
            reg_store,
            None, // no credential store — SBC does not authenticate registrations itself
            Some(sbc_config.external_domain.clone()),
            Arc::clone(&sbc_hooks),
            false, // SBC does not authenticate registrations itself
        )
        .with_default_tenant_id(sbc_config.default_tenant_id),
    );

    // 13. Create TrunkRouter with LCR support (provides outbound routing when registrar lookup fails)
    //
    // The SBC is the platform's own edge: it routes by the trunks in its
    // configuration, not by a tenant's (`docs/TENANT_TRUNKS.md` §13.8).
    let trunk_set = Arc::new(fcp_sbc_core::TrunkSet::platform_only(
        sbc_config.trunks.clone(),
    ));
    let trunk_router = if sbc_config.lcr.enabled {
        if let Some(ref qt) = quality_tracker {
            info!(
                routes = sbc_config.lcr.routes.len(),
                strategy = ?sbc_config.lcr.load_balance_strategy,
                max_failover = sbc_config.lcr.max_failover_attempts,
                "LCR routing enabled"
            );
            Arc::new(TrunkRouter::with_trunk_set_and_lcr(
                Arc::clone(&trunk_set),
                Arc::clone(&trunk_health),
                Arc::clone(&admission) as Arc<dyn CpsChecker>,
                sbc_config.lcr.clone(),
                Arc::clone(qt),
            ))
        } else {
            Arc::new(TrunkRouter::with_trunk_set(
                Arc::clone(&trunk_set),
                Arc::clone(&trunk_health),
                Arc::clone(&admission) as Arc<dyn CpsChecker>,
            ))
        }
    } else {
        Arc::new(TrunkRouter::with_trunk_set(
            Arc::clone(&trunk_set),
            Arc::clone(&trunk_health),
            Arc::clone(&admission) as Arc<dyn CpsChecker>,
        ))
    };

    // 14. Create RateLimiterSet from sip-ratelimit config
    let rate_limiter = Arc::new(build_rate_limiter_set(&sbc_config.rate_limit));

    // 15. Create SipMessageHandler (with B2buaConfig from SbcConfig)
    let media_profile = Some(MediaProfileBuilder::audio_only());
    let handler = Arc::new(
        SipMessageHandler::new(
            Arc::clone(&transaction_manager_ref),
            Arc::clone(&dialog_manager_ref),
            local_uri,
            media_profile,
            effective_ip,
            sbc_config.nat.rtp_port_start,
            Some(registrar),
            None, // no routing service
            Arc::clone(&call_manager),
            Some(Arc::clone(&uac)),
            dyn_storage,
            media_manager,
            media_config,
            None, // no flow handler
            sbc_config.b2bua.clone(),
            Some(trunk_router as Arc<dyn fcp_sip::OutboundRouteProvider>),
            None, // No PBX handler
            None, // No conference handler
            None, // No queue handler
            None, // presence_handler
            None, // cluster_parking
            deploy_config.service.node_id.clone(),
        )?
        .with_default_tenant_id(sbc_config.default_tenant_id)
        .with_trusted_edges(fcp_sip::emergency::TrustedEdges::parse(
            &sbc_config.e911.trusted_edges,
        )?)
        // An SBC stands in front of phones: it tells the call manager where
        // each caller is (docs/E911_BLOCK_2.md §6.2).
        .with_client_address_vouching(sbc_config.e911.vouch_client_address),
    );

    // 19. Build peer-trunk IP map for ingress trunk identification (reuses shared
    //     implementation that also resolves FQDN hosts via DNS)
    let peer_trunk_map = hmr_dispatcher::build_peer_trunk_map(&sbc_config.trunks);

    // 19b. Build per-trunk STIR/SHAKEN config map for enforcement handler
    let trunk_stir_configs: HashMap<String, fcp_sbc_core::TrunkStirShakenConfig> = sbc_config
        .trunks
        .iter()
        .map(|t| (t.name.clone(), t.stir_shaken.clone()))
        .collect();

    // 19c. Create SbcEnforcementHandler (wraps SipMessageHandler)
    let enforcement_handler = Arc::new(SbcEnforcementHandler::new(
        handler as Arc<dyn InboundMessageHandler>,
        sbc_config.security.clone(),
        Arc::clone(&rate_limiter),
        Arc::clone(&admission),
        Arc::clone(&sbc_metrics),
        Arc::clone(&dispatcher),
        hmr_engine,
        peer_trunk_map,
        stir_verifier.clone(),
        trunk_stir_configs,
        stir_metrics.clone(),
    ));

    // 20. Resolve TLS config: CLI args override config file
    let tls_bind_str = cli.tls_bind.as_deref().or(sbc_config.tls_bind.as_deref());
    let tls_cert = cli.tls_cert.as_deref().or(sbc_config.tls_cert.as_deref());
    let tls_key = cli.tls_key.as_deref().or(sbc_config.tls_key.as_deref());
    let ws_bind_str = cli.ws_bind.as_deref().or(sbc_config.ws_bind.as_deref());
    let wss_bind_str = cli.wss_bind.as_deref().or(sbc_config.wss_bind.as_deref());

    let tls_bind_addr: Option<std::net::SocketAddr> = match tls_bind_str {
        Some(s) => Some(s.parse().context("Invalid TLS bind address")?),
        None => None,
    };
    let ws_bind_addr: Option<std::net::SocketAddr> = match ws_bind_str {
        Some(s) => Some(s.parse().context("Invalid WS bind address")?),
        None => None,
    };
    let wss_bind_addr: Option<std::net::SocketAddr> = match wss_bind_str {
        Some(s) => Some(s.parse().context("Invalid WSS bind address")?),
        None => None,
    };

    // Create TransportListener (wired to SbcEnforcementHandler)
    let listener_config = ListenerConfig {
        udp_bind: Some(bind_addr),
        tcp_bind: Some(bind_addr),
        #[cfg(feature = "tls")]
        tls_bind: tls_bind_addr,
        #[cfg(feature = "tls")]
        tls_cert: tls_cert.map(|s| s.to_string()),
        #[cfg(feature = "tls")]
        tls_key: tls_key.map(|s| s.to_string()),
        #[cfg(feature = "tls")]
        tls_client_ca: None,
        #[cfg(feature = "tls")]
        tls_client_auth: None,
        ws_bind: ws_bind_addr,
        wss_bind: wss_bind_addr,
    };
    let enforcement_for_tcp = Arc::clone(&enforcement_handler);
    let handler_dyn: Arc<dyn InboundMessageHandler> =
        enforcement_handler as Arc<dyn InboundMessageHandler>;
    let mut listener = TransportListener::new(listener_config, handler_dyn);
    listener
        .start()
        .await
        .context("Failed to start transport listeners")?;

    // Spawn TCP response routing task — routes through enforcement handler
    // so that TCP inbound messages get ACL/rate-limit/admission checks.
    // SIP responses bypass enforcement automatically (see enforcement.rs).
    let enforcement_clone = enforcement_for_tcp;
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
            if let Err(e) = enforcement_clone
                .handle_inbound(
                    txn_transport,
                    packet.peer(),
                    packet.payload().clone(),
                    packet.stream().cloned(),
                    packet.peer_identity().cloned(),
                )
                .await
            {
                tracing::error!("Failed to handle TCP message: {}", e);
            }
        }
    });

    // 21. Spawn health HTTP server (its bind and token were checked in 4a)

    // Initialize cluster membership (if configured)
    #[cfg(feature = "storage-redis")]
    let _cluster_manager = if deploy_config.cluster.enabled {
        if let Some(ref url) = deploy_config.redis_url {
            match fcp_storage_redis::RedisStorage::new(fcp_storage_redis::RedisConfig {
                url: url.clone(),
                ..Default::default()
            })
            .await
            {
                Ok(redis) => {
                    let member = fcp_cluster::ClusterMember {
                        node_id: deploy_config.service.node_id.clone(),
                        node_type: "sbc".to_string(),
                        http_address: fcp_cluster::advertised_address(
                            cli.health_bind
                                .parse()
                                .unwrap_or_else(|_| "0.0.0.0:8080".parse().unwrap()),
                            Some(&local_ip),
                        ),
                        sip_address: fcp_cluster::advertised_address(bind_addr, Some(&local_ip)),
                        sip_tls_address: None,
                        capabilities: vec!["sbc".to_string(), "trunking".to_string()],
                        active_calls: 0,
                        started_at: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                        version: env!("CARGO_PKG_VERSION").to_string(),
                        public_url: None,
                        capacity: None,
                        draining: false,
                        region: fcp_cluster::resolve_region(
                            deploy_config.cluster.region.as_deref(),
                        ),
                        video_budget: Vec::new(),
                        registering_trunks: Vec::new(),
                        view: None,
                    };
                    let cm = fcp_cluster::ClusterManager::new(redis, member).with_intervals(
                        deploy_config.cluster.heartbeat_interval_secs,
                        deploy_config.cluster.heartbeat_ttl_secs,
                    );
                    let _handle = cm.start();
                    info!("Cluster membership started");
                    Some(Arc::new(cm))
                }
                Err(e) => {
                    warn!(error = %e, "Failed to connect to Redis; cluster disabled");
                    None
                }
            }
        } else {
            warn!("cluster.enabled = true but redis_url is not set; cluster disabled");
            None
        }
    } else {
        None
    };

    // 21b. Leaving service without dropping calls
    // (`docs/CAPACITY_AND_SURVIVABILITY.md` §6): the same sequence the call
    // manager runs, from the same place, whether it starts at `POST /drain`
    // or at SIGTERM.
    let node_id = deploy_config.service.node_id.clone();
    let drain_target = Arc::new(drain::NodeCalls::new(
        Arc::clone(&call_manager),
        Some(Arc::clone(&uac)),
        Arc::clone(&dialog_manager_ref),
        #[cfg(feature = "storage-redis")]
        _cluster_manager.clone(),
    ));
    let drain = Arc::new(fcp_sip::drain::Drain::new(
        Arc::clone(&admission) as Arc<dyn fcp_sip::admission::NodeAdmission>,
        drain_target as Arc<dyn fcp_sip::drain::DrainTarget>,
        Arc::clone(&sbc_hooks),
        node_id,
        deploy_config.service.drain_wait_secs,
    ));

    let health_state = Arc::new(HealthState {
        admission: Arc::clone(&admission),
        health_monitor: Arc::clone(&trunk_health),
        metrics: Arc::clone(&sbc_metrics),
        call_manager: Arc::clone(&call_manager),
        trunk_configs: sbc_config.trunks.clone(),
        trunk_registrar: Some(Arc::clone(&trunk_registrar)),
        uac: Some(Arc::clone(&uac)),
        dialog_manager: Arc::clone(&dialog_manager_ref),
        recording_store,
        quality_tracker: quality_tracker.clone(),
        lcr_routes: sbc_config.lcr.routes.clone(),
        stir_metrics: stir_metrics.clone(),
        stir_verifier: stir_verifier.clone(),
        api_token: cli.health_api_token.clone(),
        drain: Arc::clone(&drain),
        replica: replica_health,
    });
    tokio::spawn(async move {
        if let Err(e) = health::run_health_server(health_bind, health_state).await {
            tracing::error!(%e, "SBC health server exited");
        }
    });

    // 22. Spawn metrics snapshot background task
    let metrics_admission = Arc::clone(&admission);
    let metrics_ref = Arc::clone(&sbc_metrics);
    let snapshot_interval = sbc_config.metrics.snapshot_interval_secs;
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(snapshot_interval));
        loop {
            interval.tick().await;
            let cps = metrics_admission.current_cps();
            let calls = metrics_admission.concurrent_calls();
            let regs = metrics_admission.concurrent_registrations();
            metrics_ref.update_snapshot(cps, calls, regs);
        }
    });

    // Spawn trunk health OPTIONS probe task
    let probe_monitor = Arc::clone(&trunk_health);
    let probe_uac = Arc::clone(&uac);
    let probe_trunks = Arc::clone(&trunk_set);
    let probe_hooks = Arc::clone(&sbc_hooks);
    tokio::spawn(async move {
        health_probes::run_health_probes(
            probe_monitor,
            probe_trunks,
            probe_uac,
            fcp_sbc_core::HealthAnnouncer::new(probe_hooks, "sbc".to_string()),
        )
        .await;
    });

    // Spawn trunk registration task (maintains REGISTER bindings for trunks
    // that require registration with their remote peers)
    {
        // The SBC is the platform's edge and routes by the platform's
        // trunks alone (`docs/TENANT_TRUNKS.md` §13.8), so it is the only
        // node that loaded them: it owns every binding it holds.
        let ctx = fcp_sbc_core::TrunkRegistrationContext {
            registrar: Arc::clone(&trunk_registrar),
            trunks: Arc::clone(&trunk_set),
            health: Arc::clone(&trunk_health),
            uac: Arc::clone(&uac),
            ownership: Arc::new(fcp_sbc_core::AloneOnThisNode),
            events: Some(Arc::clone(&sbc_hooks)),
            node_id: deploy_config.service.node_id.clone(),
        };
        let reg_shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            trunk_registration::run_trunk_registration(ctx, reg_shutdown).await;
        });
    }

    if sbc_config.stir_shaken.enabled {
        let verify_trunks = sbc_config
            .trunks
            .iter()
            .filter(|t| t.stir_shaken.verify_mode != fcp_sbc_core::StirVerifyMode::Disabled)
            .count();
        let sign_trunks = sbc_config
            .trunks
            .iter()
            .filter(|t| t.stir_shaken.sign_mode != fcp_sbc_core::StirSignMode::Disabled)
            .count();
        info!(
            signing = stir_signer.is_some(),
            verify_trunks = verify_trunks,
            sign_trunks = sign_trunks,
            "STIR/SHAKEN enabled (RFC 8224/8225/8588)"
        );
    }

    // 22b. Spawn config poll loop (checks DB every 10s for config changes)
    if let Some(ref cs) = config_store {
        let cs = Arc::clone(cs);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut last_version: i64 = 0;
            loop {
                interval.tick().await;
                match cs.get_latest_version("sbc", "_system").await {
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

    info!(
        udp = %cli.bind,
        tcp = %cli.bind,
        tls = ?tls_bind_str,
        ws = ?ws_bind_str,
        wss = ?wss_bind_str,
        health = %cli.health_bind,
        trunks = sbc_config.trunks.len(),
        "fcp-sbc ready — listening for SIP messages"
    );

    // 23. Wait for shutdown signal
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("Failed to register SIGTERM handler");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("Received SIGINT (Ctrl+C), initiating graceful shutdown...");
        }
        _ = sigterm.recv() => {
            info!("Received SIGTERM, initiating graceful shutdown...");
        }
    }

    // Leave service first (`docs/CAPACITY_AND_SURVIVABILITY.md` §6): tell
    // the cluster, stop taking calls, wait for the ones this node is
    // relaying, and end what is left with a BYE that says why. With
    // `drain_wait_secs = 0` — the default — this is what the SBC always
    // did: it stops where it stands, having said it is going.
    if deploy_config.service.drain_wait_secs > 0 || drain.is_draining() {
        fcp_sip::drain::drain_for_shutdown(&drain).await;
    } else {
        drain.start().await;
    }

    // Shutdown cluster manager
    #[cfg(feature = "storage-redis")]
    if let Some(ref cm) = _cluster_manager {
        cm.shutdown().await;
    }

    // 24. Graceful shutdown — signal registration task to de-register trunks,
    //     then wait briefly for de-registration before stopping the listener.
    let _ = shutdown_tx.send(true);
    if trunk_registrar.has_registerable_trunks() {
        tokio::time::sleep(Duration::from_secs(4)).await;
    }
    listener.stop().await?;
    info!("Shutdown complete");
    Ok(())
}

/// Initialize tracing subscriber.
fn init_tracing(level: &str, format: &str) {
    use tracing_subscriber::EnvFilter;

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));

    match format {
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

/// Create a local SIP URI from the bind address.
fn create_local_uri(bind_str: &str) -> Result<SipUri> {
    let bind_addr: std::net::SocketAddr = bind_str.parse().context("Invalid bind address")?;
    let ip = if bind_addr.ip().is_unspecified() {
        auto_detect_local_ip().unwrap_or_else(|| "127.0.0.1".to_string())
    } else {
        bind_addr.ip().to_string()
    };

    let uri_str = if bind_addr.port() != 5060 && bind_addr.port() != 0 {
        format!("sip:sbc@{}:{}", ip, bind_addr.port())
    } else {
        format!("sip:sbc@{}", ip)
    };

    let uri = Uri::parse(&uri_str).context("Failed to parse local URI")?;
    uri.as_sip()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Not a SIP URI"))
}

/// Derive a local address string for UAC configuration.
fn derive_local_addr(bind_str: &str) -> String {
    if let Ok(addr) = bind_str.parse::<std::net::SocketAddr>() {
        let ip = if addr.ip().is_unspecified() {
            auto_detect_local_ip().unwrap_or_else(|| "127.0.0.1".to_string())
        } else {
            addr.ip().to_string()
        };
        let port = if addr.port() == 0 { 5060 } else { addr.port() };
        format!("{}:{}", ip, port)
    } else {
        "127.0.0.1:5060".to_string()
    }
}

/// Derive local IP string from bind address.
fn derive_local_ip(bind_str: &str) -> String {
    if let Ok(addr) = bind_str.parse::<std::net::SocketAddr>() {
        if !addr.ip().is_unspecified() {
            return addr.ip().to_string();
        }
    }
    auto_detect_local_ip().unwrap_or_else(|| "127.0.0.1".to_string())
}

/// Convert SBC rate limit policy config to sip-ratelimit's RateLimiterSet.
fn build_rate_limiter_set(
    config: &fcp_sbc_core::RateLimitPolicyConfig,
) -> sip_ratelimit::RateLimiterSet {
    use sip_ratelimit::{RateLimitConfig, RateLimiterSet};

    if !config.enabled {
        return RateLimiterSet::disabled();
    }

    let make_config = |entry: &fcp_sbc_core::RateLimitEntry| -> RateLimitConfig {
        if !entry.enabled {
            return RateLimitConfig::disabled();
        }
        let mut cfg = RateLimitConfig::new(entry.max_requests, entry.window_secs)
            .expect("rate limit config already validated");
        if let Some(burst) = entry.burst_capacity {
            cfg = cfg
                .with_burst_capacity(burst)
                .expect("burst capacity already validated");
        }
        cfg = cfg.with_idle_timeout(config.idle_cleanup_secs);
        cfg.with_enabled(true)
    };

    RateLimiterSet::with_configs(
        make_config(&config.per_ip.auth_attempts),
        make_config(&config.per_ip.register),
        make_config(&config.per_ip.invite),
        make_config(&config.per_ip.options), // connections bucket → use options config
    )
}

/// Auto-detect the local LAN IP.
fn auto_detect_local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:53").ok()?;
    let addr = socket.local_addr().ok()?;
    if addr.ip().is_unspecified() || addr.ip().is_loopback() {
        return None;
    }
    Some(addr.ip().to_string())
}
