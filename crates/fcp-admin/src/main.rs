//! FCP Admin - Management and Administration API
//!
//! This binary provides the Admin API for FCP, focused on management and
//! administration operations. It's designed to run internally (behind an API gateway)
//! and handles CRUD operations for:
//!
//! - Tenant management
//! - User administration
//! - Device provisioning
//! - Extension configuration
//! - Phone number management
//! - System monitoring

use anyhow::{Context, Result};
use clap::Parser;
use fcp_events::{EventBus, InMemoryEventBus};
use fcp_sip::exports::{
    DialogManager, SubscriptionManager, TransactionManager, TransportContext, TransportDispatcher,
};
use fcp_sip::handler::SipMessageHandler;
use fcp_sip::{B2buaConfig, CallManager};
use std::sync::Arc;
use tokio::signal;
use tracing::{info, warn};

/// No-op transport dispatcher for admin API.
///
/// The admin API doesn't do real SIP transport — it only needs the handler
/// for API operations (e.g. hangup calls via BYE). The handler requires a
/// `TransactionManager`, which requires a dispatcher.
struct NoopDispatcher;

#[async_trait::async_trait]
impl TransportDispatcher for NoopDispatcher {
    async fn dispatch(
        &self,
        _ctx: &TransportContext,
        _payload: bytes::Bytes,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

mod api_docs;
mod call_supervision;
mod cli;
mod cluster_api;
mod config;
mod firmware_s3;
mod route_preview;
mod services;
mod site_files;
mod site_link;
mod storage;
mod subject_requests;

use cli::Cli;
use config::load_config;

/// Initialize tracing/logging for fcp-admin.
fn init_tracing(config: &config::AdminConfig) -> Result<()> {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    let log_level = &config.logging.level;
    let log_format = &config.logging.format;

    // Create the EnvFilter from the log level
    let env_filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(log_level))
        .context("Failed to create log filter")?;

    // Build the subscriber based on format
    match log_format.as_str() {
        "json" => {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt::layer().json())
                .init();
        }
        "pretty" => {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt::layer().pretty())
                .init();
        }
        "compact" => {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt::layer().compact())
                .init();
        }
        _ => {
            // Default to JSON for production
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt::layer().json())
                .init();
        }
    }

    Ok(())
}

/// API tokens' request counts in the cluster's Redis
/// (`fcp_api_core::rate_limit`), so every admin node counts together.
struct RedisRateCounter(fcp_storage_redis::RedisStorage);

#[async_trait::async_trait]
impl fcp_api_core::rate_limit::RateCounter for RedisRateCounter {
    async fn hit(&self, key: &str, window: u64) -> anyhow::Result<u64> {
        self.0
            .count_in_window(
                &format!("ratelimit:{key}:{window}"),
                fcp_api_core::rate_limit::WINDOW * 2,
            )
            .await
    }
}

/// `/v1/trunks` reading the views the nodes publish to the cluster.
struct ClusterTrunkReports(fcp_cluster::TrunkRegistry);

#[async_trait::async_trait]
impl fcp_api_core::TrunkNodeReports for ClusterTrunkReports {
    async fn reports(&self) -> Vec<fcp_core::trunk_report::TrunkNodeReport> {
        match self.0.reports().await {
            Ok(reports) => reports,
            Err(e) => {
                warn!(error = %e, "Could not read what the nodes see of their trunks");
                Vec::new()
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command-line arguments
    let cli = Cli::parse();

    // Load configuration
    let mut config =
        fcp_core::config::check_stage(load_config(cli.config.as_deref()), cli.check_config)?;
    // Secrets from the environment win over the file
    // (`docs/SHIP_AND_OPERATE.md` phase 2).
    if let Some(v) = &cli.database_url {
        config.storage.database_url = v.clone();
    }
    if let Some(v) = &cli.events_auth_token {
        config.events.auth_token = Some(v.clone());
    }
    if let Some(v) = &cli.call_manager_auth_token {
        config.call_manager.auth_token = Some(v.clone());
    }
    if let Some(v) = &cli.conference_auth_token {
        config.conference.auth_token = Some(v.clone());
    }
    if let Some(v) = &cli.conference_join_secret {
        config.conference.join_secret = Some(v.clone());
    }
    if let (Some(v), Some(tts)) = (&cli.tts_auth_token, config.tts.as_mut()) {
        tts.auth_token = Some(v.clone());
    }

    // Apply CLI overrides
    if let Some(log_level) = cli.log_level {
        config.logging.level = log_level;
    }
    if let Some(log_format) = cli.log_format {
        config.logging.format = log_format;
    }
    if let Some(bind_addr) = cli.bind_addr {
        config.api.bind_address = bind_addr.parse().context("Invalid bind address format")?;
    }
    if let Some(key) = cli.session_signing_key {
        config.api.session.signing_key = Some(key);
    }
    if let Some(auth_token) = cli.auth_token {
        config.api.auth_token = Some(auth_token);
    }
    if cli.provisioning_no_https {
        config.api.provisioning_require_https = false;
    }

    fcp_core::config::check_stage(config::validate_config(&config), cli.check_config)?;

    // Initialize tracing/logging
    if cli.check_config {
        println!(
            "{}",
            fcp_core::config::masked_toml(&config).map_err(anyhow::Error::msg)?
        );
        return Ok(());
    }
    init_tracing(&config).context("Failed to initialize tracing")?;

    info!(
        version = env!("CARGO_PKG_VERSION"),
        service = "fcp-admin",
        "Starting FCP Admin API"
    );
    if config.api.provisioning_auth_token.is_some() {
        warn!(
            "[api] provisioning_auth_token is read by nothing: phones fetch from the tenant's \
             provisioning URL (docs/PROVISIONING.md); remove the key, which a later release refuses"
        );
    }

    // Initialize storage backend
    let storage_result = storage::initialize_storage(&config.storage)
        .await
        .context("Failed to initialize storage backend")?;
    let storage = storage_result.dyn_storage;
    let flow_store = storage_result.flow_store;
    let config_store = storage_result.config_store;
    let postgres = storage_result.postgres;

    info!("Storage backend initialized successfully");

    // The record table's upkeep: the coming months' partitions and the
    // hourly usage rollups (docs/CDR_PIPELINE.md §8-9).
    fcp_cdr::spawn_maintenance(
        Arc::clone(&storage) as Arc<dyn fcp_core::storage::CdrStore>,
        fcp_cdr::MaintenanceConfig::default(),
    );

    // Sign-in sessions that ended a month ago are deleted, once an hour
    // (docs/IDENTITY_AND_DEVICES.md phase 1).
    if let Some(sessions) = storage.login_session_store().cloned() {
        tokio::spawn(async move {
            let keep = std::time::Duration::from_secs(30 * 86_400);
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
            loop {
                interval.tick().await;
                match sessions.sweep_ended_sessions(keep).await {
                    Ok(0) => {}
                    Ok(n) => info!(sessions = n, "Ended sign-in sessions deleted"),
                    Err(e) => warn!(error = %e, "Sign-in session sweep failed"),
                }
            }
        });
    }

    // Retention sweep: apply every tenant's compliance policy once an hour,
    // then the platform's upkeep (docs/COMPLIANCE_FOLLOWUPS.md §4).
    // Tenants without a policy (or with no retention windows) are skipped.
    // What the cloud asks of sites and their answers
    // (docs/HYBRID_CONTROL_PLANE.md §3.3): files fetched through a site's
    // link, and files the site is asked to remove.
    let site_requests = Arc::new(site_files::SiteRequests::new(Arc::clone(&storage)));
    {
        let sweep_storage = Arc::clone(&storage);
        let actor = format!("fcp-admin:{}", config.service.node_id);
        let tenant_trace_days = config.retention.tenant_trace_days;
        let upkeep = fcp_core::compliance::UpkeepConfig {
            site_files: Some(
                Arc::clone(&site_requests) as Arc<dyn fcp_core::compliance::SiteFileRemover>
            ),
            file_roots: [
                config.call_recordings_dir.as_deref(),
                config.voicemail_recordings_dir.as_deref(),
                config.voicemail_greetings_dir.as_deref(),
                config.profile_pictures_dir.as_deref(),
            ]
            .into_iter()
            .flatten()
            .map(String::from)
            .chain(config.retention.extra_file_roots.iter().cloned())
            .map(std::path::PathBuf::from)
            .collect(),
            orphan_files_after: config
                .retention
                .orphan_files_days
                .map(|d| std::time::Duration::from_secs(u64::from(d) * 86_400)),
            system_traces_after: config
                .retention
                .system_trace_days
                .filter(|d| *d > 0)
                .map(|d| std::time::Duration::from_secs(u64::from(d) * 86_400)),
            deleted_voicemail_grace: std::time::Duration::from_secs(
                u64::from(config.retention.deleted_voicemail_grace_days) * 86_400,
            ),
            ..Default::default()
        };
        tokio::spawn(async move {
            use fcp_core::TenantStore;
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
            loop {
                interval.tick().await;
                let Some(store) = sweep_storage.compliance_store().cloned() else {
                    continue;
                };
                // One admin replica sweeps at a time.
                let lock = {
                    use fcp_core::storage::CdrStore;
                    match sweep_storage.try_maintenance_lock("fcp:retention").await {
                        Ok(Some(lock)) => lock,
                        Ok(None) => continue,
                        Err(e) => {
                            warn!(error = %e, "Retention sweep: could not take the lock");
                            continue;
                        }
                    }
                };
                let tenants = match sweep_storage.list_tenants().await {
                    Ok(t) => t,
                    Err(e) => {
                        warn!(error = %e, "Retention sweep: failed to list tenants");
                        lock.release().await;
                        continue;
                    }
                };
                // The oldest start time every tenant lets go of: whole months
                // before it can be dropped once their rows are deleted.
                let now = std::time::SystemTime::now();
                let mut partition_cutoff = (!tenants.is_empty()).then_some(now);
                for tenant in tenants {
                    let policy = match store.get_compliance_policy(&tenant.id).await {
                        Ok(p) => p
                            .unwrap_or_else(|| {
                                fcp_core::compliance::CompliancePolicy::defaults(tenant.id)
                            })
                            .with_trace_default(tenant_trace_days),
                        Err(e) => {
                            warn!(tenant_id = %tenant.id, error = %e, "Retention sweep: failed to load policy");
                            continue;
                        }
                    };
                    let policy = match policy {
                        p if p.has_retention() => p,
                        _ => {
                            partition_cutoff = None;
                            continue;
                        }
                    };
                    // A tenant whose policy requires the CDR archive lets
                    // go only of what the archive has confirmed.
                    let cdr_cutoff = match fcp_core::compliance::cdr_retention_cutoff(
                        &*sweep_storage,
                        &policy,
                        now,
                    )
                    .await
                    {
                        Ok((cutoff, _)) => cutoff,
                        Err(e) => {
                            warn!(tenant_id = %tenant.id, error = %e, "Retention sweep: no CDR archive frontier");
                            None
                        }
                    };
                    partition_cutoff = match (partition_cutoff, cdr_cutoff) {
                        (Some(c), Some(t)) => Some(c.min(t)),
                        _ => None,
                    };
                    let report =
                        fcp_core::compliance::run_retention(&sweep_storage, &policy, &actor).await;
                    if report.total_deleted() > 0 || !report.errors.is_empty() {
                        info!(
                            tenant_id = %tenant.id,
                            deleted = report.total_deleted(),
                            errors = ?report.errors,
                            "Retention sweep completed"
                        );
                    }
                    if let Err(e) = store.record_retention_run(&report).await {
                        warn!(tenant_id = %tenant.id, error = %e, "Retention sweep: failed to record run");
                    }
                }
                let upkept =
                    fcp_core::compliance::run_upkeep(&sweep_storage, &upkeep, &actor).await;
                if upkept != fcp_core::compliance::UpkeepReport::default() {
                    info!(report = ?upkept, "Platform upkeep completed");
                }
                if let Some(cutoff) = partition_cutoff {
                    use fcp_core::storage::CdrStore;
                    match sweep_storage.drop_cdr_partitions_before(cutoff).await {
                        Ok(dropped) if !dropped.is_empty() => {
                            info!(partitions = ?dropped, "Retention sweep: dropped CDR partitions")
                        }
                        Ok(_) => {}
                        Err(e) => {
                            warn!(error = %e, "Retention sweep: could not drop CDR partitions")
                        }
                    }
                }
                lock.release().await;
            }
        });
    }

    // Build lightweight SIP components (no real transport).
    // The admin API only needs the component graph for REST API operations
    // (e.g. hangup calls via BYE), not actual SIP traffic.
    let local_bus = Arc::new(InMemoryEventBus::new());
    let event_bus: Arc<dyn EventBus> = local_bus.clone();

    let dispatcher = Arc::new(NoopDispatcher);
    let transaction_manager = Arc::new(TransactionManager::new(
        dispatcher as Arc<dyn TransportDispatcher>,
    ));
    let dialog_manager = Arc::new(DialogManager::new());
    let subscription_manager = Arc::new(SubscriptionManager::new());
    let call_manager = Arc::new(CallManager::with_storage_and_events(
        storage.clone(),
        Arc::clone(&event_bus) as Arc<dyn fcp_events::EventPublisher>,
    ));

    let local_uri = fcp_sip::exports::SipUri::parse("sip:admin@127.0.0.1").unwrap();
    let handler = Arc::new(
        SipMessageHandler::new(
            transaction_manager,
            dialog_manager.clone(),
            local_uri,
            None,                    // media_profile
            "127.0.0.1".to_string(), // local_addr
            10000,                   // rtp_port
            None,                    // registrar
            None,                    // routing_service
            call_manager.clone(),
            None, // uac (no outbound SIP needed)
            Some(storage.clone()),
            None, // media_manager
            None, // media_config
            None, // flow_handler
            B2buaConfig::default(),
            None,          // outbound_route_provider
            None,          // pbx_handler
            None,          // conference_handler
            None,          // queue_handler
            None,          // presence_handler
            None,          // cluster_parking
            String::new(), // node_id
        )?
        .with_default_tenant_id(config.service.default_tenant_id),
    );

    let api_config = fcp_core::ApiConfig {
        enabled: true,
        bind: config.api.bind_address,
        auth_enabled: config.api.auth_enabled,
        auth_token: config.api.auth_token.clone(),
        provisioning_require_https: config.api.provisioning_require_https,
        allow_public_signup: config.api.allow_public_signup,
        metrics_enabled: false,
        metrics_path: "/metrics".to_string(),
        deployment_mode: config.api.deployment_mode,
        rate_limit: config.api.rate_limit.clone(),
        session: config.api.session.clone(),
        sso: config.api.sso.clone(),
        provisioning: config.api.provisioning.clone(),
        redirection: config.api.redirection.clone(),
        firmware: config.api.firmware.clone(),
        captures: config.api.captures.clone(),
    };
    // The realm digest hashes are computed in, the registrars' (`[sip]
    // realm`, else the call managers' from the operational configuration);
    // on a change, the stored hashes made again in it. A site's users carry
    // the cloud's hashes, so a site's admin leaves them be.
    let (realm, realm_source) =
        fcp_api_core::sip_realm::resolve(config.sip.realm.as_deref(), Some(config_store.as_ref()))
            .await;
    info!(realm = %realm, source = ?realm_source, "SIP realm");
    if config.api.managed_by.is_none() {
        if let Err(e) =
            fcp_api_core::sip_realm::adopt(&storage, config_store.as_ref(), &realm).await
        {
            warn!(error = %e, "The stored digest hashes could not be brought into the realm");
        }
    }
    let default_tenant_id = config.service.default_tenant_id;

    info!(node_id = %config.service.node_id, "SIP components initialized (lightweight, no transport)");

    // Clone event_bus for event forwarding, subscribing, and presence listener
    let event_bus_for_forward = Arc::clone(&event_bus);
    let event_bus_for_subscribe: Arc<dyn EventBus> = Arc::clone(&event_bus);
    let event_bus_for_presence: Arc<dyn EventBus> = Arc::clone(&event_bus);

    // Derive events server base URL from ingest URL (strip /ingest suffix)
    let events_server_url = config.events.server_url.as_ref().map(|url| {
        url.trim_end_matches("/ingest")
            .trim_end_matches('/')
            .to_string()
    });

    // Create read-only PresenceManager so the admin UI can display presence data.
    // Keep event publishing disabled here: the admin only mirrors presence from
    // cluster events and must not emit those updates back onto its local bus.
    let presence_manager = {
        let presence_config = fcp_presence::PresenceConfig {
            enabled: true,
            ..Default::default()
        };
        // `set_pubsub` takes `&mut self`, but is only called under the `redis` feature.
        #[cfg_attr(not(feature = "redis"), allow(unused_mut))]
        let mut pm = fcp_presence::PresenceManager::new(presence_config, None);

        // Wire up Redis Pub/Sub for cross-node presence sync
        #[cfg(feature = "redis")]
        if let Some(ref redis_url) = config.cluster.redis_url {
            match fcp_presence::redis_pubsub::RedisPresencePubSub::new(
                redis_url,
                Some(config.service.node_id.clone()),
            ) {
                Ok(pubsub) => {
                    info!(
                        node_id = %pubsub.node_id(),
                        "Admin presence Redis Pub/Sub initialized"
                    );
                    pm.set_pubsub(std::sync::Arc::new(pubsub));
                    match pm.run_pubsub_listener().await {
                        Ok(_handle) => info!("Admin presence Redis Pub/Sub listener started"),
                        Err(e) => {
                            warn!(error = %e, "Failed to start admin presence Pub/Sub listener")
                        }
                    }
                }
                Err(e) => warn!(error = %e, "Failed to initialize admin presence Redis Pub/Sub"),
            }
        }

        pm
    };

    // Keep the admin's per-address presence as the call managers tell
    // their phones, from the events the events server sends.
    {
        let pm_clone = presence_manager.clone();
        tokio::spawn(async move {
            run_presence_listener(event_bus_for_presence, pm_clone).await;
        });
        info!("Admin presence event listener started");
    }

    // Spawn trunk health probe background task
    let trunk_health_map = fcp_api_core::trunk_health::new_health_map();
    {
        let storage_for_health = Arc::clone(&storage);
        let health_map_clone = Arc::clone(&trunk_health_map);
        let tenant_id = default_tenant_id;
        tokio::spawn(fcp_api_core::trunk_health::run_trunk_health_probes(
            storage_for_health,
            health_map_clone,
            tenant_id,
        ));
        info!("Trunk health probe task started");
    }

    // The validator behind POST …/e911/validate (`docs/E911_BLOCK_2.md` §7).
    let e911_validator = build_e911_validator(&config.e911.validator, &storage).await?;

    // The link: what sites enroll to and heartbeat at
    // (docs/HYBRID_CONTROL_PLANE.md phase 1), when configured.
    // Firmware images: in a bucket ([api.firmware.s3]) or a directory
    // ([api.firmware] directory); the link serves them to sites too.
    let firmware_files: Option<Arc<dyn fcp_api_core::firmware_files::FirmwareFiles>> = match (
        &config.api.firmware.s3,
        config.api.firmware.directory.as_deref(),
    ) {
        (Some(s3), _) => {
            let files = firmware_s3::S3Files::new(s3, "api.firmware.s3", "firmware/")?;
            info!(
                location = %fcp_api_core::firmware_files::FirmwareFiles::location(&files),
                "Firmware images in a bucket"
            );
            Some(Arc::new(files))
        }
        (None, Some(dir)) => Some(Arc::new(fcp_api_core::firmware_files::DirectoryFiles::new(
            dir,
        ))),
        (None, None) => None,
    };
    // Apps' captures in a bucket ([api.captures.s3]); a directory is
    // made the store below (docs/CALL_DIAGNOSTICS.md §3).
    let capture_bucket: Option<Arc<dyn fcp_api_core::firmware_files::FirmwareFiles>> =
        match &config.api.captures.s3 {
            Some(s3) => {
                let files = firmware_s3::S3Files::new(s3, "api.captures.s3", "captures/")?;
                info!(
                    location = %fcp_api_core::firmware_files::FirmwareFiles::location(&files),
                    "Captures in a bucket"
                );
                Some(Arc::new(files))
            }
            None => None,
        };
    let link_state = if config.link.enabled {
        site_link::ensure_crypto_provider();
        let ca = site_link::LinkCa::load_or_create(&site_link::ca_dir(&config.link))
            .context("the link CA")?;
        // The site's events and the records the cloud writes for it are
        // announced through the events server's ingest, under the site's
        // name; without one they are logged and dropped.
        let ingest = config.events.server_url.clone().map(|url| {
            site_files::IngestPublisher::new(
                url,
                config.events.auth_token.clone(),
                config.service.node_id.clone(),
            )
        });
        if ingest.is_none() {
            warn!("No [events] server_url: sites' events are not forwarded, and their records are not announced");
        }
        // The cloud's processor for the sites' records: rated with the
        // cloud's plans, charged to the tenant, announced as cdr.written.
        let cache = std::time::Duration::from_secs(60);
        let billing = fcp_billing::BillingEngine::new(Arc::clone(&storage)).with_plan_cache(cache);
        let mut processor = fcp_cdr::Processor::new(
            Arc::clone(&storage) as Arc<dyn fcp_core::storage::CdrStore>,
            Some(Arc::new(billing)),
        )
        .with_enricher(Arc::new(fcp_cdr::DirectionEnricher::new(
            Arc::clone(&storage) as Arc<dyn fcp_core::storage::PhoneNumberStore>,
            cache,
        )));
        if let Some(ingest) = &ingest {
            processor = processor
                .with_events(Arc::new(ingest.clone()) as Arc<dyn fcp_events::EventPublisher>);
        }
        Some(site_link::LinkState {
            storage: storage.clone(),
            flows: Arc::clone(&flow_store),
            ca: Arc::new(ca),
            events: Arc::clone(&event_bus) as Arc<dyn fcp_events::EventPublisher>,
            config: config.link.clone(),
            processor: Some(Arc::new(processor)),
            ingest,
            requests: Arc::clone(&site_requests),
            firmware: firmware_files.clone(),
            pushes: None,
            services: None,
        })
    } else {
        info!("Sites cannot enroll here ([link] enabled is false)");
        None
    };
    let cluster_storage = storage.clone();
    if config.webhooks.allow_private_targets {
        warn!("[webhooks] allow_private_targets: a tenant's webhook may reach loopback and private addresses");
    }
    let webhook_dispatcher = Arc::new(fcp_webhooks::WebhookDispatcher::new(
        Arc::clone(storage.webhook_store()),
        fcp_webhooks::WebhookDispatcherConfig {
            request_timeout: std::time::Duration::from_secs(config.webhooks.request_timeout_secs),
            allow_private_targets: config.webhooks.allow_private_targets,
            ..Default::default()
        },
    ));

    // Each person's presence (docs/PRESENCE.md), from the events the
    // events server sends this admin's bus, announced on it and posted to
    // the events server once, for the call managers' phones and tenant
    // webhooks; the copy the events server sends back is let go
    // (`run_events_subscriber`).
    let presence_events: Arc<dyn fcp_events::EventPublisher> = match &config.events.server_url {
        Some(url) => Arc::new(fcp_events::LocalAndCluster::new(
            Arc::clone(&local_bus),
            Arc::new(site_files::IngestPublisher::new(
                url.clone(),
                config.events.auth_token.clone(),
                config.service.node_id.clone(),
            )),
        )),
        None => Arc::clone(&local_bus) as Arc<dyn fcp_events::EventPublisher>,
    };
    let people_presence =
        fcp_api_core::people_presence::PeoplePresence::new(Arc::clone(&storage), presence_events);
    people_presence.clone().run(Arc::clone(&event_bus));

    // Create API state
    let api_state = fcp_api_core::ApiState::new(
        Some(Arc::clone(&storage)),
        Some(flow_store),
        dialog_manager,
        subscription_manager,
        call_manager,
        handler,
        None, // media_manager (admin doesn't need media)
        event_bus,
        None, // queue_manager (admin doesn't use queues)
        Some(presence_manager),
        realm,
        default_tenant_id,
    )
    .with_people_presence(people_presence)
    .with_tenant_trace_days(config.retention.tenant_trace_days)
    .with_events_server_url(events_server_url.clone())
    .with_events_server_auth_token(config.events.auth_token.clone())
    .with_events_node_id(config.service.node_id.clone())
    .with_conference_server_url(config.conference.server_url.clone())
    .with_conference_server_auth_token(config.conference.auth_token.clone())
    .with_conference_join_secret(config.conference.join_secret.clone())
    .with_conference_gateway_url(config.conference.gateway_url.clone())
    .with_config_store(config_store)
    .with_trunk_health(trunk_health_map)
    .with_deployment_mode(config.api.deployment_mode)
    .with_managed_by(config.api.managed_by.clone())
    .with_site_files(Some(
        Arc::clone(&site_requests) as Arc<dyn fcp_api_core::site_files::SiteFiles>
    ))
    .with_e911_validator(e911_validator)
    .with_webhook_dispatcher(Some(webhook_dispatcher));

    // The firmware store, the API's and the link's (docs/PROVISIONING.md §11).
    let api_state = match &firmware_files {
        Some(files) => api_state.with_firmware_files(Arc::clone(files)),
        None => api_state,
    };
    // The captures' store: the bucket, else the directory; this state's
    // too, for the sweep, not only the router's.
    let api_state = match capture_bucket {
        Some(files) => api_state.with_capture_files(files, None),
        None => api_state,
    }
    .with_captures_config(&config.api.captures);

    // Set security policy from config
    let api_state = api_state.with_security_policy(fcp_api_core::SecurityPolicy {
        max_failed_attempts: config.security.max_failed_attempts,
        lockout_duration_secs: config.security.lockout_duration_secs,
        password_expiration_days: config.security.password_expiration_days,
    });

    // Set voicemail greetings directory (default: /tmp/fcp/voicemail/greetings)
    let api_state = if let Some(ref dir) = config.voicemail_greetings_dir {
        api_state.with_voicemail_greetings_dir(std::path::PathBuf::from(dir))
    } else {
        api_state
            .with_voicemail_greetings_dir(std::path::PathBuf::from("/tmp/fcp/voicemail/greetings"))
    };

    // Set profile pictures directory (default: /tmp/fcp/profile-pictures)
    let api_state = if let Some(ref dir) = config.profile_pictures_dir {
        api_state.with_profile_pictures_dir(std::path::PathBuf::from(dir))
    } else {
        api_state.with_profile_pictures_dir(std::path::PathBuf::from("/tmp/fcp/profile-pictures"))
    };

    // Set voicemail recordings directory (default: /tmp/fcp/voicemail/recordings)
    let api_state = if let Some(ref dir) = config.voicemail_recordings_dir {
        api_state.with_voicemail_recordings_dir(std::path::PathBuf::from(dir))
    } else {
        api_state.with_voicemail_recordings_dir(std::path::PathBuf::from(
            "/tmp/fcp/voicemail/recordings",
        ))
    };

    // Set audio prompts directory (default: /tmp/fcp/audio/prompts)
    let api_state = if let Some(ref dir) = config.prompts_dir {
        api_state.with_prompts_dir(std::path::PathBuf::from(dir))
    } else {
        api_state.with_prompts_dir(std::path::PathBuf::from("/tmp/fcp/audio/prompts"))
    };
    let api_state = match config.call_recordings_dir.as_deref() {
        Some(dir) => api_state.with_call_recordings_dir(std::path::PathBuf::from(dir)),
        None => api_state,
    };

    // Initialize TTS service for prompt audio synthesis (optional)
    let tts_provider = config
        .tts
        .as_ref()
        .and_then(|tts| match (&tts.remote_url, &tts.engine) {
            (Some(url), _) => Some(fcp_media::tts::TtsProviderConfig::Remote {
                url: url.clone(),
                auth_token: tts.auth_token.clone().unwrap_or_default(),
                provider: None,
                tenant_id: None,
            }),
            (None, Some(engine)) => Some(fcp_media::tts::TtsProviderConfig::Local {
                engine: engine.clone(),
                executable_path: tts.python_path.as_ref().map(std::path::PathBuf::from),
                device: None,
            }),
            (None, None) => {
                warn!("[tts] names neither remote_url nor engine — prompt synthesis unavailable");
                None
            }
        });
    let api_state = if let (Some(tts_config), Some(provider)) = (&config.tts, tts_provider) {
        let described = match &provider {
            fcp_media::tts::TtsProviderConfig::Remote { url, .. } => url.clone(),
            _ => tts_config.engine.clone().unwrap_or_default(),
        };
        match fcp_media::tts::TtsService::new(fcp_media::tts::TtsConfig {
            provider,
            cache_dir: std::path::PathBuf::from(&tts_config.cache_dir),
            cache_max_size_mb: 500,
            default_language: "en-US".to_string(),
            default_voice: tts_config.voice.clone(),
        })
        .await
        {
            Ok(tts_service) => {
                info!(tts = %described, "TTS service initialized for prompt synthesis");
                api_state.with_tts_service(std::sync::Arc::new(tts_service))
            }
            Err(e) => {
                warn!(error = %e, "TTS service initialization failed — prompt synthesis unavailable");
                api_state
            }
        }
    } else {
        api_state
    };

    // Subject requests: exports run here, beside retention
    // (docs/COMPLIANCE_FOLLOWUPS.md §6.2).
    if let Some(storage) = api_state.storage.clone() {
        subject_requests::spawn(
            storage,
            &config,
            api_state.clone(),
            Some(Arc::clone(&site_requests)),
        );
    }

    // Deleted users purged when their tenant's grace period ends
    // (docs/USERS.md §4), once an hour.
    fcp_api_core::offboarding::spawn_sweep(
        api_state.clone(),
        format!("fcp-admin:{}", config.service.node_id),
    );

    // App devices nobody signs in as any more removed, once an hour
    // (docs/APP_SIGN_IN.md).
    fcp_api_core::v1::app_auth::spawn_device_sweep(api_state.clone());

    // Apps' captures removed when they expire, and their files, once an
    // hour (docs/CALL_DIAGNOSTICS.md §3).
    fcp_api_core::v1::device_captures::spawn_sweep(api_state.clone());

    // SAML providers' metadata fetched again from its URL once a day, so a
    // rotated certificate is followed (docs/SSO.md §8).
    fcp_api_core::v1::sso::spawn_metadata_refresh(api_state.clone(), &api_config.sso);

    // Initialize cluster reader (optional — only when Redis is configured)
    let mut trunk_reports: Option<std::sync::Arc<dyn fcp_api_core::TrunkNodeReports>> = None;
    let mut rate_counter: Option<std::sync::Arc<dyn fcp_api_core::rate_limit::RateCounter>> = None;
    let cluster_reader = if let Some(ref redis_url) = config.cluster.redis_url {
        match fcp_storage_redis::RedisStorage::new(fcp_storage_redis::RedisConfig {
            url: redis_url.clone(),
            key_prefix: config.cluster.key_prefix.clone(),
        })
        .await
        {
            Ok(redis_storage) => {
                // The same Redis carries what each node sees of its trunks
                // (`docs/TENANT_TRUNKS.md` §9); this server reads them, and
                // publishes none of its own — it routes no calls.
                trunk_reports = Some(std::sync::Arc::new(ClusterTrunkReports(
                    fcp_cluster::TrunkRegistry::new(redis_storage.clone(), "fcp-admin".to_string()),
                ))
                    as std::sync::Arc<dyn fcp_api_core::TrunkNodeReports>);
                // API tokens' request counts, shared by every admin node
                // on this Redis.
                rate_counter = Some(std::sync::Arc::new(RedisRateCounter(redis_storage.clone())));
                let reader = fcp_cluster::ClusterReader::new(redis_storage, 15);
                info!(redis_url = %redis_url, "Cluster monitoring enabled via Redis");
                Some(std::sync::Arc::new(reader))
            }
            Err(e) => {
                warn!(error = %e, "Failed to connect to Redis for cluster monitoring — disabled");
                None
            }
        }
    } else {
        info!("Cluster monitoring disabled (no [cluster] redis_url configured)");
        None
    };

    // Say this process is there (`docs/SERVICES.md`).
    let beacon = config.cluster.redis_url.as_ref().map(|url| {
        let mut beacon = fcp_cluster::ServiceBeacon::new(
            fcp_cluster::ServiceIdentity::new(&config.service.node_id, "admin")
                .http(config.api.bind_address),
        )
        .check(fcp_cluster::Check::tenants(
            Arc::clone(&storage) as Arc<dyn fcp_core::storage::TenantStore>
        ));
        if let Some(events) = &events_server_url {
            beacon = beacon.check(fcp_cluster::Check::health("events", events));
        }
        beacon.start(fcp_storage_redis::RedisConfig {
            url: url.clone(),
            key_prefix: config.cluster.key_prefix.clone(),
        })
    });

    // The record of every service, kept from the beacons
    // (`docs/SERVICES.md`): up and down said once, on this bus and to the
    // events server for webhooks.
    let services_registry = config.cluster.redis_url.as_ref().map(|url| {
        let events: Arc<dyn fcp_events::EventPublisher> = match &config.events.server_url {
            Some(events_url) => Arc::new(fcp_events::LocalAndCluster::new(
                Arc::clone(&local_bus),
                Arc::new(site_files::IngestPublisher::new(
                    events_url.clone(),
                    config.events.auth_token.clone(),
                    config.service.node_id.clone(),
                )),
            )),
            None => Arc::clone(&local_bus) as Arc<dyn fcp_events::EventPublisher>,
        };
        let registry = Arc::new(services::ServiceRegistry::new(
            Arc::clone(&postgres) as Arc<dyn fcp_core::service_registry::ServiceInstanceStore>,
            events,
        ));
        Arc::clone(&registry).spawn_sweep(
            fcp_storage_redis::RedisConfig {
                url: url.clone(),
                key_prefix: config.cluster.key_prefix.clone(),
            },
            config.service.node_id.clone(),
            std::time::Duration::from_secs(config.services.forget_after_days * 86_400),
        );
        registry
    });
    let api_state = match &services_registry {
        Some(registry) => api_state
            .with_metrics_source(Arc::clone(registry) as Arc<dyn fcp_api_core::MetricsSource>),
        None => api_state,
    };

    // What every node sees of its trunks, once there is somewhere to read
    // it from.
    let api_state = match trunk_reports {
        Some(reports) => api_state.with_trunk_reports(reports),
        None => api_state,
    };
    let api_state = match rate_counter {
        Some(counter) => api_state.with_rate_counter(counter),
        None => api_state,
    };

    // A route preview is asked of a live call manager
    // (`docs/TENANT_TRUNKS.md` §12, phase 6d).
    let api_state = match route_preview::CallManagerPreviewer::new(
        config.call_manager.clone(),
        cluster_reader.clone(),
    ) {
        Some(previewer) => api_state.with_route_previewer(std::sync::Arc::new(previewer)),
        None => {
            info!("No call manager to ask for route previews ([call_manager] url or [cluster])");
            api_state
        }
    };

    // A call's supervisor is asked of the call manager holding the call
    // (`docs/ENTERPRISE_CALLING.md` §3.11).
    let api_state = match call_supervision::NodeCallSupervisor::new(
        config.call_manager.clone(),
        cluster_reader.clone(),
    ) {
        Some(supervisor) => api_state.with_call_supervisor(std::sync::Arc::new(supervisor)),
        None => api_state,
    };

    let services_state = services::ServicesApiState {
        registry: services_registry.clone(),
        storage: Arc::clone(&storage),
    };
    let cluster_state = cluster_api::ClusterApiState {
        reader: cluster_reader,
        storage: Some(cluster_storage),
    };

    // A site's pushes are sent by a call manager with the publisher's keys
    // (`docs/PUSH.md` §2.4).
    let link_state = link_state.map(|mut state| {
        state.services = services_registry.clone();
        state.pushes = call_supervision::NodeCallSupervisor::new(
            config.call_manager.clone(),
            cluster_state.reader.clone(),
        )
        .map(Arc::new);
        state
    });
    if let Some(state) = link_state {
        tokio::spawn(async move {
            if let Err(e) = site_link::serve(state).await {
                tracing::error!(error = %e, "The link listeners stopped");
            }
        });
    }

    // Start API server with admin UI
    info!(
        bind = %config.api.bind_address,
        auth_enabled = config.api.auth_enabled,
        "Starting Admin API server (with web UI at /admin/)"
    );

    let bind_addr = config.api.bind_address;
    let api_handle = tokio::spawn(async move {
        // Cluster routes live inside the /api/v1 nest next to the core API
        // (routes merged outside the nest would be shadowed by axum's prefix
        // matching); create_router_with does the auth/CORS/pre-auth wiring.
        let cluster_routes: axum::Router<fcp_api_core::ApiState> =
                // The platform's cluster and every tenant's sites: a
                // tenant's administrator sees their own sites at `/sites`,
                // and none of this (`docs/SERVICES.md` §4.3).
                cluster_api::cluster_router(cluster_state).layer(axum::middleware::from_fn(
                    fcp_api_core::auth::require_platform_admin,
                ))
                    // The platform's services: a tenant's administrator sees
                    // none of them (`docs/SERVICES.md` Q5).
                    .merge(services::services_router(services_state).layer(
                        axum::middleware::from_fn(fcp_api_core::auth::require_platform_admin),
                    ));
        let mut app = fcp_api_core::create_router_with(api_state, &api_config, cluster_routes);
        app = app.merge(fcp_admin_ui::admin_ui_router());
        // The contract and its reference page (docs/DEVELOPER_SURFACE.md
        // §3, phase 3), outside the bearer layer.
        app = app.merge(api_docs::router());

        let listener = tokio::net::TcpListener::bind(bind_addr).await?;
        tracing::info!("API server listening on {}", bind_addr);
        axum::serve(listener, app).await?;
        Ok::<(), anyhow::Error>(())
    });

    // Event forwarding from admin is handled by ApiState::publish_event(),
    // which POSTs directly to the events server ingest endpoint. This avoids
    // the infinite loop that would occur if we forwarded all local bus events
    // (since the WebSocket subscriber also republishes to the local bus).
    let _ = event_bus_for_forward;

    // Start events server subscriber (receives cluster-wide events for the admin UI WebSocket)
    if let Some(ref events_url) = events_server_url {
        let ws_url = events_url
            .replace("http://", "ws://")
            .replace("https://", "wss://");
        let tenant_id = default_tenant_id.to_string();
        let ws_url = events_ws_url(&ws_url, &tenant_id);
        let auth_token = config.events.auth_token.clone();
        let event_bus_sub = Arc::clone(&event_bus_for_subscribe);
        tokio::spawn(async move {
            run_events_subscriber(event_bus_sub, &ws_url, auth_token).await;
        });
        info!("Events server subscriber started");
    }

    // Wait for shutdown signal
    info!("FCP Admin API running. Press Ctrl+C or send SIGTERM to shutdown.");
    shutdown_signal().await;

    // Shutdown API server
    api_handle.abort();
    if let Some(beacon) = &beacon {
        beacon.shutdown().await;
    }

    info!("FCP Admin API shutdown complete");
    Ok(())
}

/// The address validator this deployment answers `…/e911/validate` with
/// (`docs/E911_BLOCK_2.md` §7). `basic` needs nothing; `http` takes its
/// bearer token from `token_file` or `token`, either of which may be a
/// value sealed with encryption at rest (`enc:v1:…`), opened here.
async fn build_e911_validator(
    config: &config::ValidatorConfig,
    storage: &Arc<fcp_core::DynStorage>,
) -> Result<Arc<dyn fcp_core::e911::E911AddressValidator>> {
    if config.provider != "http" {
        return Ok(Arc::new(fcp_core::e911::BasicAddressValidator));
    }
    let url = config
        .url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .context("e911.validator.url must be set when provider = \"http\"")?;

    let token = match &config.token_file {
        Some(path) => Some(
            std::fs::read_to_string(path)
                .with_context(|| format!("Failed to read e911.validator.token_file {path}"))?
                .trim()
                .to_string(),
        ),
        None => config.token.clone(),
    };
    let token = match token.filter(|t| !t.is_empty()) {
        Some(token) if token.starts_with("enc:") => {
            let store = storage.encryption_store().context(
                "e911.validator token is sealed but this deployment has no encryption at rest",
            )?;
            Some(
                store
                    .open_secret(&token)
                    .await
                    .context("Failed to open the sealed e911.validator token")?,
            )
        }
        other => other,
    };

    let name = config.name.clone().unwrap_or_else(|| "http".to_string());
    info!(validator = %name, url = %url, sealed_token = token.is_some(), "E911 address validator configured");
    Ok(Arc::new(
        fcp_api_core::e911_validator::HttpAddressValidator::new(
            name,
            url,
            token,
            std::time::Duration::from_secs(config.timeout_secs),
        )?,
    ))
}

/// Subscribe to the events server's WebSocket and republish events to the local event bus.
///
/// This allows the admin UI's WebSocket to see cluster-wide events (calls, registrations, etc.)
/// that originate from other services (e.g. fcp-call-manager).
async fn run_events_subscriber(
    event_bus: Arc<dyn fcp_events::EventBus>,
    ws_url: &str,
    auth_token: Option<String>,
) {
    use futures_util::StreamExt;

    loop {
        info!(url = %ws_url, "Connecting to events server WebSocket");
        let request = match events_ws_request(ws_url, auth_token.as_deref()) {
            Ok(request) => request,
            Err(e) => {
                warn!(url = %ws_url, error = %e, "The events server WebSocket request could not be built");
                return;
            }
        };
        match tokio_tungstenite::connect_async(request).await {
            Ok((ws_stream, _)) => {
                info!("Connected to events server WebSocket");
                let (_write, mut read) = ws_stream.split();

                while let Some(msg) = read.next().await {
                    match msg {
                        Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                            // Events server sends batch-wrapped messages:
                            //   {"type":"connected", ...}
                            //   {"type":"events", "count":N, "events":[...]}
                            // Parse the wrapper and extract individual PlatformEvents.
                            if let Ok(wrapper) = serde_json::from_str::<serde_json::Value>(&text) {
                                match wrapper.get("type").and_then(|t| t.as_str()) {
                                    Some("events") => {
                                        if let Some(events) =
                                            wrapper.get("events").and_then(|e| e.as_array())
                                        {
                                            for event_val in events {
                                                match serde_json::from_value::<
                                                    fcp_events::PlatformEvent,
                                                >(
                                                    event_val.clone()
                                                ) {
                                                    // This admin worked
                                                    // it out and announced
                                                    // it here already.
                                                    Ok(fcp_events::PlatformEvent::Presence(
                                                        fcp_events::PresenceEvent::UserChanged {
                                                            ..
                                                        },
                                                    )) => {}
                                                    Ok(event) => {
                                                        if let Err(e) =
                                                            event_bus.publish(event).await
                                                        {
                                                            tracing::debug!(%e, "Failed to republish event to local bus");
                                                        }
                                                    }
                                                    Err(e) => {
                                                        tracing::debug!(%e, "Failed to parse event from batch");
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Some("connected") => {
                                        tracing::debug!("Events server confirmed connection");
                                    }
                                    other => {
                                        tracing::debug!(msg_type = ?other, "Unknown message type from events server");
                                    }
                                }
                            }
                        }
                        Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => {
                            info!("Events server WebSocket closed");
                            break;
                        }
                        Err(e) => {
                            warn!(%e, "Events server WebSocket error");
                            break;
                        }
                        _ => {} // Ignore ping/pong/binary
                    }
                }
            }
            Err(e) => {
                warn!(%e, "Failed to connect to events server WebSocket");
            }
        }

        // Reconnect after delay
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
}

/// The events server's WebSocket for `tenant_id`. The token goes in an
/// `Authorization` header (`events_ws_request`), never the URL, which is
/// logged (#340).
fn events_ws_url(base_url: &str, tenant_id: &str) -> String {
    format!("{}/ws/{}", base_url, tenant_id)
}

/// A connect request for `url` carrying `auth_token` as a bearer token.
fn events_ws_request(
    url: &str,
    auth_token: Option<&str>,
) -> anyhow::Result<tokio_tungstenite::tungstenite::handshake::client::Request> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut request = url.into_client_request()?;
    if let Some(token) = auth_token {
        let value = format!("Bearer {token}")
            .parse()
            .map_err(|_| anyhow::anyhow!("the events token is not a valid header value"))?;
        request.headers_mut().insert("authorization", value);
    }
    Ok(request)
}

/// Extract the IP:port from a SIP contact URI like "sip:alice@192.168.1.80:5060"
// Kept as part of the module's intended API; not currently called from this binary.
#[allow(dead_code)]
fn extract_ip_from_contact(contact: &str) -> Option<String> {
    let stripped = contact
        .trim_start_matches("sip:")
        .trim_start_matches("sips:");
    // After stripping scheme, format is "user@host:port;params" or "host:port"
    if let Some(at_pos) = stripped.find('@') {
        let host_part = &stripped[at_pos + 1..];
        // Take up to first semicolon (strip params)
        let host_port = host_part.split(';').next().unwrap_or(host_part);
        // Strip angle brackets
        let host_port = host_port.trim_end_matches('>');
        Some(host_port.to_string())
    } else {
        None
    }
}

/// Keep the admin's per-address presence (`/api/v1/presence/*`) as the
/// call managers tell their phones: each `presence.status_changed`, in its
/// own tenant. What an address shows is the call managers' to work out,
/// from each person's presence (`docs/PRESENCE.md` §3); this admin's
/// manager has no publisher, so taking a change in announces nothing.
async fn run_presence_listener(
    event_bus: Arc<dyn EventBus>,
    presence: fcp_presence::PresenceManager,
) {
    use fcp_events::filter::EventType;
    use fcp_events::{EventFilter, PlatformEvent, PresenceEvent};

    let filter = EventFilter::new().types(vec![EventType::Presence]);
    loop {
        let mut stream = match event_bus.subscribe(filter.clone()).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(%e, "Admin presence listener could not subscribe");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }
        };
        while let Some(event) = stream.next().await {
            let PlatformEvent::Presence(PresenceEvent::StatusChanged {
                aor,
                new_state,
                tenant_id,
                ..
            }) = &event
            else {
                continue;
            };
            let Ok(state) = new_state.parse::<fcp_presence::PresenceState>() else {
                continue;
            };
            // The event carries no note.
            let update = fcp_presence::PresenceUpdate {
                aor: aor.clone(),
                state,
                note: None,
            };
            let _ = presence.update_presence(update, tenant_id).await;
        }
        tracing::warn!("Admin presence event stream ended, resubscribing...");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// Resolve on Ctrl+C or SIGTERM so container orchestrators can stop the
/// service gracefully (`docker stop`, Kubernetes pod termination).
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => info!("Received Ctrl+C"),
        _ = terminate => info!("Received SIGTERM"),
    }
}

#[cfg(test)]
mod events_ws_tests {
    use super::*;

    /// The token rides in a header; the URL, which is logged, has none (#340).
    #[test]
    fn the_events_token_is_a_header_not_in_the_url() {
        let url = events_ws_url("ws://events:8090", "tenant-1");
        assert_eq!(url, "ws://events:8090/ws/tenant-1");
        let request = events_ws_request(&url, Some("s3cret")).unwrap();
        assert!(!request.uri().to_string().contains("s3cret"));
        assert_eq!(request.headers()["authorization"], "Bearer s3cret");
        let request = events_ws_request(&url, None).unwrap();
        assert!(request.headers().get("authorization").is_none());
    }
}
