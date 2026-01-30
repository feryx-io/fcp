//! Runtime builder for configuring and constructing the FCP runtime.

use crate::Runtime;
use anyhow::{Context, Result};
use fcp_core::{FcpConfig, NodeId, ServiceManager};
use fcp_events::{EventBus, InMemoryEventBus};
use fcp_sip::prelude::*;
use fcp_sip::service::{DialogService, SubscriptionService, TransactionService};
use fcp_sip::transport::ConnectionPool;
use std::sync::Arc;
use tracing::info;

/// Builder for constructing a Runtime instance.
pub struct RuntimeBuilder {
    config: Option<FcpConfig>,
    node_id: Option<NodeId>,
    storage: Option<Arc<fcp_core::DynStorage>>,
}

impl RuntimeBuilder {
    /// Create a new runtime builder.
    pub(crate) fn new() -> Self {
        Self {
            config: None,
            node_id: None,
            storage: None,
        }
    }

    /// Set the runtime configuration.
    pub fn with_config(mut self, config: FcpConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Set a custom node ID.
    pub fn with_node_id(mut self, node_id: NodeId) -> Self {
        self.node_id = Some(node_id);
        self
    }

    /// Set a custom storage backend.
    ///
    /// If not provided, the runtime will initialize a default SQLite storage
    /// based on the storage configuration when the registrar is enabled.
    pub fn with_storage(mut self, storage: Arc<fcp_core::DynStorage>) -> Self {
        self.storage = Some(storage);
        self
    }

    /// Build the runtime.
    pub async fn build(self) -> Result<Runtime> {
        // Get or create config
        let config = self.config.unwrap_or_default();
        config.validate().context("invalid runtime configuration")?;

        // Get or generate node ID
        let node_id = self.node_id.unwrap_or_default();

        info!(
            node_id = %node_id,
            node_name = %config.node.name,
            "Building FCP runtime"
        );

        // Create SIP components
        let dialog_manager = DialogManager::new();
        let subscription_manager = SubscriptionManager::new();

        // Create a single shared TCP connection pool for efficient resource usage
        // This pool is shared between the dispatcher and transaction manager,
        // avoiding duplicate connections and reducing file descriptor usage.
        let tcp_pool = Arc::new(ConnectionPool::new());

        // Setup TCP response routing BEFORE creating components
        // This enables responses on outbound TCP connections to be routed back to the handler
        let (client_tx, client_rx) = tokio::sync::mpsc::channel(1024);
        tcp_pool.set_inbound_tx(client_tx).await;

        // Create shared transport dispatcher with the shared pool
        let dispatcher = create_transport_dispatcher(&config, Arc::clone(&tcp_pool)).await?;

        // Create transaction manager with shared dispatcher and shared pool
        // This ensures both components use the same connection pool for optimal resource sharing
        let transaction_manager = TransactionManager::new_with_pool(dispatcher.clone(), tcp_pool);

        // Create services that wrap the managers
        let transaction_service = TransactionService::new(transaction_manager);
        let dialog_service = DialogService::new(dialog_manager);
        let subscription_service = SubscriptionService::new(subscription_manager);

        // Get Arc references to the managers from the services
        let transaction_manager_ref = Arc::clone(transaction_service.manager());
        let dialog_manager_ref = Arc::clone(dialog_service.manager());
        let subscription_manager_ref = Arc::clone(subscription_service.manager());

        // Create service manager and register services
        let mut services = ServiceManager::new();
        services.register(transaction_service);
        services.register(dialog_service);
        services.register(subscription_service);

        // Create UAC for outbound calls (reuses same dispatcher)
        let uac = create_uac(
            &config,
            Arc::clone(&transaction_manager_ref),
            Arc::clone(&dialog_manager_ref),
            dispatcher,
        )
        .await?;

        // Create registrar if enabled
        let (registrar, storage, flow_store) = if config.registrar.enabled {
            let (reg, store, flow) = create_registrar(&config, self.storage).await?;
            (Some(reg), Some(store), Some(flow))
        } else {
            (None, self.storage, None)
        };

        // Create routing service if enabled
        let routing_service = if config.routing.enabled {
            Some(create_routing_service(&config).await?)
        } else {
            None
        };

        // Create media manager
        let media_manager = create_media_manager(&config).await?;

        // Create event bus for real-time event streaming
        let event_bus: Arc<dyn EventBus> = Arc::new(InMemoryEventBus::new());
        info!("Event bus initialized (in-memory)");

        // Create flow handler if flow_store is available
        let flow_handler = if let Some(flow_store) = &flow_store {
            Some(create_flow_handler(flow_store, &event_bus, &media_manager, &config).await?)
        } else {
            None
        };

        // Create transport listener
        let (listener, handler, call_manager) = create_transport_listener(
            &config,
            Arc::clone(&transaction_manager_ref),
            Arc::clone(&dialog_manager_ref),
            registrar,
            routing_service,
            Arc::clone(&uac),
            storage.clone(),
            Arc::clone(&media_manager),
            Arc::clone(&event_bus),
            flow_handler,
        )?;

        // Spawn task to route responses from outbound TCP connections back into the handler
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
                    )
                    .await
                {
                    tracing::error!("Failed to handle outbound TCP response: {}", e);
                }
            }
        });

        Ok(Runtime {
            config: Arc::new(config),
            node_id,
            services,
            transaction_manager: transaction_manager_ref,
            dialog_manager: dialog_manager_ref,
            subscription_manager: subscription_manager_ref,
            uac,
            call_manager,
            handler,
            media_manager,
            event_bus,
            listener: Some(listener),
            shutdown_tx: None,
            storage,
            flow_store,
            cleanup_task: None,
        })
    }
}

impl Default for RuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a transport dispatcher shared by TransactionManager and UAC.
async fn create_transport_dispatcher(
    config: &FcpConfig,
    tcp_pool: Arc<ConnectionPool>,
) -> Result<Arc<RealTransportDispatcher>> {
    use anyhow::Context;
    use fcp_sip::prelude::RealTransportDispatcher;
    use socket2::{Domain, Protocol, Socket, Type};

    // Use the same UDP port as the listener for the dispatcher
    // This ensures that SIP responses come back to the correct port via rport mechanism
    // The listener uses SO_REUSEADDR, so both sockets can bind to the same port
    let bind_addr = config
        .transport
        .udp_bind
        .unwrap_or_else(|| "0.0.0.0:5060".parse().unwrap());

    // Create UDP socket with SO_REUSEADDR
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .context("Failed to create UDP socket")?;
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&bind_addr.into())?;
    let std_socket: std::net::UdpSocket = socket.into();
    let udp_socket = tokio::net::UdpSocket::from_std(std_socket)?;

    // Create dispatcher with UDP socket and shared TCP pool
    #[allow(unused_mut)]
    let mut dispatcher =
        RealTransportDispatcher::new_with_pool(Some(Arc::new(udp_socket)), tcp_pool);

    // Optionally load TLS configuration if TLS support is enabled
    #[cfg(feature = "tls")]
    {
        if config.transport.tls_cert.is_some() || config.transport.tls_key.is_some() {
            info!("TLS client configuration will use system root certificates");
        }

        // Load TLS client config for outbound SIPS connections
        match RealTransportDispatcher::load_tls_client_config() {
            Ok(tls_config) => {
                info!("TLS client configuration loaded successfully");
                dispatcher = dispatcher.with_tls_config(tls_config);
            }
            Err(e) => {
                tracing::warn!("Failed to load TLS configuration: {}", e);
            }
        }
    }

    Ok(Arc::new(dispatcher))
}

/// Create a registrar with storage and optional authentication.
/// Returns (registrar, storage) tuple.
///
/// If storage is provided, it will be used. Otherwise, a default SQLite storage
/// will be created using the storage configuration from the config.
async fn create_registrar(
    config: &FcpConfig,
    provided_storage: Option<Arc<fcp_core::DynStorage>>,
) -> Result<(
    Arc<fcp_sip::registrar::FcpRegistrar>,
    Arc<fcp_core::DynStorage>,
    Arc<dyn fcp_flow::FlowStore>,
)> {
    use fcp_core::{CredentialStore, DynStorage};
    use fcp_sip::registrar::FcpRegistrar;

    info!(
        auth_enabled = config.registrar.auth_enabled,
        realm = %config.registrar.realm,
        "Creating registrar"
    );

    // Use provided storage or initialize from config
    // We need to keep both DynStorage and FlowStore references
    let (storage, flow_store): (Arc<DynStorage>, Arc<dyn fcp_flow::FlowStore>) = if let Some(
        storage,
    ) =
        provided_storage
    {
        info!("Using provided storage backend for registrar");
        // TODO: Extract FlowStore from provided storage
        // For now, use InMemoryFlowStore as fallback
        (storage, Arc::new(fcp_flow::InMemoryFlowStore::new()))
    } else {
        info!(
            backend = ?config.storage.backend,
            connection_string = mask_connection_string(&config.storage.connection_string),
            "Initializing default storage backend for registrar"
        );

        // Initialize storage based on configuration
        match config.storage.backend {
            fcp_core::StorageBackend::Sqlite => {
                #[cfg(feature = "storage-sqlite")]
                {
                    use fcp_storage_sqlite::{SqliteConfig, SqliteStorage};

                    // Parse connection string (format: sqlite://path/to/db.sqlite or sqlite:///path/to/db.sqlite)
                    let db_path = config.storage.connection_string
                        .strip_prefix("sqlite://")
                        .or_else(|| config.storage.connection_string.strip_prefix("sqlite:///"))
                        .ok_or_else(|| anyhow::anyhow!("Invalid SQLite connection string format. Expected: sqlite://path/to/db.sqlite or sqlite:///path/to/db.sqlite"))?;

                    // Create SqliteStorage from fcp-storage-sqlite crate
                    let sqlite_config = SqliteConfig::new(db_path);
                    let sqlite_storage = Arc::new(
                        SqliteStorage::new(sqlite_config)
                            .await
                            .context("Failed to initialize SQLite storage for registrar")?,
                    );

                    // Keep reference for FlowStore before wrapping in DynStorage
                    let flow_store = Arc::clone(&sqlite_storage) as Arc<dyn fcp_flow::FlowStore>;

                    // Create DynStorage for basic storage operations
                    // Note: This will fail until we implement the missing traits in fcp-storage-sqlite
                    // For now, let's use InMemoryFlowStore and address the full migration separately
                    let dyn_storage =
                        Arc::new(fcp_core::DynStorage::from_all_in_one(sqlite_storage));

                    (dyn_storage, flow_store)
                }

                #[cfg(not(feature = "storage-sqlite"))]
                {
                    anyhow::bail!(
                        "SQLite storage backend not available. Compile with --features storage-sqlite"
                    )
                }
            }
            _ => {
                anyhow::bail!(
                    "Storage backend {:?} not yet supported for automatic initialization in registrar. Please provide storage via RuntimeBuilder::with_storage()",
                    config.storage.backend
                )
            }
        }
    };

    // If authentication is enabled, ensure at least one user exists
    let credential_store: Option<Arc<dyn CredentialStore>> = if config.registrar.auth_enabled {
        // Check if any users exist
        let users = storage.list_users().await?;
        if users.is_empty() {
            // Try to load users from file if specified
            if let Some(users_file) = &config.registrar.users_file {
                info!(path = %users_file.display(), "Loading users from file");

                match load_users_from_file(users_file, &storage, &config.registrar.realm).await {
                    Ok(count) => {
                        info!(count, "Loaded users from file");
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "Failed to load users from file, creating default test users"
                        );
                        create_default_test_users(&storage, &config.registrar.realm).await?;
                    }
                }
            } else {
                info!("No users file specified, creating default test users");
                create_default_test_users(&storage, &config.registrar.realm).await?;
            }
        } else {
            info!(
                user_count = users.len(),
                "Loaded existing users from storage"
            );
        }

        Some(storage.clone())
    } else {
        None
    };

    // Create registrar
    let registrar = FcpRegistrar::new(
        storage.clone(),
        credential_store,
        Some(config.registrar.realm.clone()),
    );

    Ok((Arc::new(registrar), storage, flow_store))
}

/// Create a routing service based on configuration.
async fn create_routing_service(config: &FcpConfig) -> Result<Arc<fcp_routing::RoutingService>> {
    use fcp_core::config::RoutingBackend;
    use fcp_routing::{RoutingService, YamlRoutingStore};

    info!(
        backend = ?config.routing.backend,
        "Creating routing service"
    );

    match &config.routing.backend {
        RoutingBackend::Yaml { path, watch } => {
            let store = YamlRoutingStore::new(path.clone(), *watch).await?;
            Ok(Arc::new(RoutingService::new(Arc::new(store))))
        }
        RoutingBackend::Database { connection_string } => {
            // TODO: Implement database backend
            anyhow::bail!(
                "Database routing backend not yet implemented: {}",
                connection_string
            )
        }
    }
}

/// Create a flow handler for programmable call flows.
async fn create_flow_handler(
    flow_store: &Arc<dyn fcp_flow::FlowStore>,
    event_bus: &Arc<dyn EventBus>,
    media_manager: &Arc<fcp_media::MediaManager>,
    config: &FcpConfig,
) -> Result<Arc<fcp_sip::flow_handler::FlowCallHandler>> {
    use fcp_sip::flow_handler::FlowCallHandler;

    info!("Creating flow handler with database-backed flow store");

    // Create flow handler with event publishing and media playback
    let mut flow_handler = FlowCallHandler::new(
        Arc::clone(&flow_store),
        Some(Arc::clone(event_bus) as Arc<dyn fcp_events::EventPublisher>),
        Some(Arc::clone(media_manager)),
    );

    // Configure media profile for SDP generation in outbound calls
    if config.transport.enable_sdp {
        let local_addr = config.transport.local_ip.clone().unwrap_or_else(|| {
            config
                .transport
                .udp_bind
                .map(|addr| addr.ip().to_string())
                .unwrap_or_else(|| "127.0.0.1".to_string())
        });
        flow_handler = flow_handler.with_media_profile(
            fcp_sip::prelude::MediaProfileBuilder::audio_only(),
            local_addr,
            config.transport.rtp_port,
        );
    }

    info!("Flow handler created successfully (using database-backed store)");

    let flow_handler = Arc::new(flow_handler);

    // Load test flow for IVR testing (only in debug mode)
    // TODO: Make this configurable via fcp.toml
    #[cfg(debug_assertions)]
    {
        info!("Loading test IVR flow");
        if let Err(e) = load_test_ivr_flow(&flow_store, &flow_handler).await {
            tracing::warn!("Failed to load test IVR flow: {}", e);
        }
    }

    Ok(flow_handler)
}

/// Create a media manager based on configuration.
async fn create_media_manager(config: &FcpConfig) -> Result<Arc<fcp_media::MediaManager>> {
    use fcp_media::{BackendType, MediaConfig};

    info!(
        enabled = config.media.enabled,
        rtp_port_range = format!(
            "{}-{}",
            config.media.rtp_port_start, config.media.rtp_port_end
        ),
        "Creating media manager"
    );

    // Build media configuration from FCP config
    // Use media.public_ip if set, otherwise fall back to transport.local_ip
    let public_ip = config
        .media
        .public_ip
        .clone()
        .or(config.transport.local_ip.clone());

    let media_config = MediaConfig {
        backend: BackendType::Embedded, // Always use embedded mode for now
        rtp_port_start: config.media.rtp_port_start,
        rtp_port_end: config.media.rtp_port_end,
        enable_srtp: config.media.enable_srtp,
        enable_transcoding: config.media.enable_transcoding,
        enable_recording: config.media.enable_recording,
        enable_dtmf: config.media.enable_dtmf,
        recording_dir: "./recordings".to_string(), // Default recording directory
        recording_format: "wav".to_string(),       // Default to WAV format
        public_ip,
        stun_server: config.media.stun_server.clone(),
        turn_server: config.media.turn_server.clone(),
    };

    // Create manager
    let manager = fcp_media::MediaManager::new(media_config)
        .await
        .context("Failed to create media manager")?;

    info!(
        backend = manager.backend_name(),
        "Media manager created successfully"
    );

    Ok(Arc::new(manager))
}

/// Create a transport listener for receiving inbound SIP messages.
/// Returns (listener, handler, call_manager) tuple.
fn create_transport_listener(
    config: &FcpConfig,
    transaction_manager: Arc<TransactionManager>,
    dialog_manager: Arc<DialogManager>,
    registrar: Option<Arc<fcp_sip::registrar::FcpRegistrar>>,
    routing_service: Option<Arc<fcp_routing::RoutingService>>,
    uac: Arc<SipUserAgentClient>,
    storage: Option<Arc<fcp_core::DynStorage>>,
    media_manager: Arc<fcp_media::MediaManager>,
    event_bus: Arc<dyn EventBus>,
    flow_handler: Option<Arc<fcp_sip::flow_handler::FlowCallHandler>>,
) -> Result<(TransportListener, Arc<SipMessageHandler>, Arc<CallManager>)> {
    use fcp_sip::prelude::{ListenerConfig, SipMessageHandler};

    // Build listener config from FCP config
    let listener_config = ListenerConfig {
        udp_bind: config.transport.udp_bind,
        tcp_bind: config.transport.tcp_bind,
        #[cfg(feature = "tls")]
        tls_bind: config.transport.tls_bind,
        #[cfg(feature = "tls")]
        tls_cert: config
            .transport
            .tls_cert
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
        #[cfg(feature = "tls")]
        tls_key: config
            .transport
            .tls_key
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
    };

    // Create local URI from config
    let local_uri = create_local_uri(config)?;

    // Create SDP media profile if enabled
    let media_profile = if config.transport.enable_sdp {
        Some(fcp_sip::prelude::MediaProfileBuilder::audio_only())
    } else {
        None
    };

    // Determine local IP for SDP
    let local_addr = config.transport.local_ip.clone().unwrap_or_else(|| {
        // Derive from UDP bind address if not specified
        config
            .transport
            .udp_bind
            .map(|addr| addr.ip().to_string())
            .unwrap_or_else(|| "127.0.0.1".to_string())
    });

    // Create CallManager for B2BUA functionality with storage and events
    let call_manager = match &storage {
        Some(storage) => {
            // Both storage and events available
            Arc::new(fcp_sip::CallManager::with_storage_and_events(
                Arc::clone(storage),
                Arc::clone(&event_bus) as Arc<dyn fcp_events::EventPublisher>,
            ))
        }
        None => {
            // Only events available
            Arc::new(fcp_sip::CallManager::with_events(
                Arc::clone(&event_bus) as Arc<dyn fcp_events::EventPublisher>
            ))
        }
    };

    // Create message handler that routes to transaction and dialog managers
    let handler = Arc::new(SipMessageHandler::new(
        transaction_manager,
        dialog_manager,
        local_uri,
        media_profile,
        local_addr,
        config.transport.rtp_port,
        registrar,
        routing_service,
        Arc::clone(&call_manager),
        Some(uac),
        storage,
        Some(media_manager),
        Some(config.media.clone()),
        flow_handler,
        fcp_sip::B2buaConfig::default(),
        None, // No outbound route provider
    ));

    // Create listener (cast handler to trait object)
    let handler_dyn: Arc<dyn InboundMessageHandler> =
        Arc::clone(&handler) as Arc<dyn InboundMessageHandler>;
    let listener = TransportListener::new(listener_config, handler_dyn);

    // Return listener, handler, and call_manager
    Ok((listener, handler, call_manager))
}

/// Create a UAC for outbound calls (reuses existing dispatcher).
async fn create_uac(
    config: &FcpConfig,
    transaction_manager: Arc<TransactionManager>,
    _dialog_manager: Arc<DialogManager>,
    dispatcher: Arc<RealTransportDispatcher>,
) -> Result<Arc<SipUserAgentClient>> {
    use anyhow::Context;
    use fcp_sip::prelude::{IntegratedUAC, SipUserAgentClient};

    // Create local URI from config
    let local_uri = create_local_uri(config)?;

    // Determine local address from config
    // Use local_ip if configured, otherwise derive from UDP bind address
    let local_addr = if let Some(ref local_ip) = config.transport.local_ip {
        // If local_ip is configured, use it with the UDP port
        let port = config
            .transport
            .udp_bind
            .map(|addr| addr.port())
            .filter(|&p| p != 0) // Port 0 means OS-assigned, use default instead
            .unwrap_or(5060);
        format!("{}:{}", local_ip, port)
    } else {
        // Fallback to UDP bind address
        config
            .transport
            .udp_bind
            .map(|addr| {
                let port = if addr.port() == 0 { 5060 } else { addr.port() };
                format!("{}:{}", addr.ip(), port)
            })
            .unwrap_or_else(|| "0.0.0.0:5060".to_string())
    };

    // Create DNS resolver (system resolver)
    let resolver = Arc::new(
        fcp_sip::prelude::SipResolver::from_system().context("Failed to create DNS resolver")?,
    );

    // Build IntegratedUAC (reuses shared dispatcher)
    let integrated_uac = IntegratedUAC::builder()
        .local_uri(local_uri.as_str())
        .local_addr(&local_addr)?
        .via_advertised_addr(&local_addr)? // Explicitly set Via header address for B2BUA
        .transaction_manager(transaction_manager)
        .resolver(resolver)
        .dispatcher(dispatcher)
        .build()
        .context("Failed to build IntegratedUAC")?;

    // Wrap in SipUserAgentClient
    let uac = SipUserAgentClient::new(Arc::new(integrated_uac));

    Ok(Arc::new(uac))
}

/// Create a local SIP URI from configuration.
fn create_local_uri(config: &FcpConfig) -> Result<SipUri> {
    use anyhow::Context;

    // Always use the bind address IP for Contact URI (not .local domain)
    // This ensures the Contact header is routable
    let bind_addr = config
        .transport
        .udp_bind
        .ok_or_else(|| anyhow::anyhow!("UDP bind address not configured"))?;

    // If bind is 0.0.0.0 (wildcard), use configured local_ip or fallback
    let domain = if bind_addr.ip().is_unspecified() {
        // Use local_ip from config if available, otherwise use placeholder
        // In production, set transport.local_ip in configuration to your actual IP/hostname
        // Or use local_domain for DNS-resolvable names
        config
            .transport
            .local_ip
            .clone()
            .or_else(|| Some(config.node.local_domain.clone()))
            .unwrap_or_else(|| "127.0.0.1".to_string())
    } else {
        bind_addr.ip().to_string()
    };

    let bind_port = bind_addr.port();

    // Don't include port in URI if it's 0 (OS-assigned) or 5060 (default)
    let uri_str = if bind_port != 0 && bind_port != 5060 {
        format!("sip:{}@{}:{}", config.node.name, domain, bind_port)
    } else {
        format!("sip:{}@{}", config.node.name, domain)
    };

    let uri = Uri::parse(&uri_str).context("Failed to parse URI")?;

    uri.as_sip()
        .map(|sip| sip.clone())
        .ok_or_else(|| anyhow::anyhow!("Not a SIP URI"))
}

/// Load users from a JSON file.
async fn load_users_from_file(
    path: &std::path::Path,
    storage: &Arc<fcp_core::DynStorage>,
    realm: &str,
) -> Result<usize> {
    use fcp_core::storage::password;
    use fcp_core::{CredentialStore, User, UserFileEntry};

    // Read file contents
    let contents = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read users file: {}", e))?;

    // Parse JSON
    let entries: Vec<UserFileEntry> = serde_json::from_str(&contents)
        .map_err(|e| anyhow::anyhow!("Failed to parse users file: {}", e))?;

    let mut count = 0;
    for entry in entries {
        // Hash the plaintext password for API/web authentication
        let password_hash = password::hash_password(&entry.password)?;

        // Hash the digest password if provided (Cisco-style separate credentials)
        let digest_password_hash = if let Some(ref digest_pw) = entry.digest_password {
            Some(password::hash_password(digest_pw)?)
        } else {
            None
        };

        // Compute SIP HA1 hash from digest_password (if set) or password (fallback)
        let password_for_ha1 = entry.digest_password.as_deref().unwrap_or(&entry.password);
        let sip_ha1 =
            fcp_core::storage::password::compute_ha1(&entry.username, realm, password_for_ha1);

        let user = User {
            id: fcp_core::UserId::new(),
            username: entry.username.clone(),
            password: password_hash,
            digest_password: digest_password_hash,
            sip_ha1_hash: Some(sip_ha1),
            display_name: entry.display_name.clone(),
            tenant_id: entry.tenant_id,
            enabled: entry.enabled,
            role: entry.role.unwrap_or(fcp_core::UserRole::EndUser),
            department_id: entry.department_id.clone(),
            created_at: std::time::SystemTime::now(),
            updated_at: std::time::SystemTime::now(),
            media_preferences: entry.media_preferences.clone(),
            email: None,
            password_last_reset: None,
            phone_number: None,
            mobile_number: None,
            pin: None,
            extension: None,
            department: None,
            job_title: None,
            location: None,
            language: "en-US".to_string(),
            timezone: "UTC".to_string(),
            account_status: fcp_core::AccountStatus::Active,
            last_login: None,
            emergency_contact: fcp_core::EmergencyContact::default(),
            primary_phone_number_id: None,
            voicemail_box_id: None,
        };

        storage.put_user(user).await?;
        count += 1;
        info!(username = %entry.username, tenant_id = %entry.tenant_id, "Loaded user from file");
    }

    Ok(count)
}

/// Create default test users with hashed passwords.
async fn create_default_test_users(storage: &Arc<fcp_core::DynStorage>, realm: &str) -> Result<()> {
    use fcp_core::storage::password;
    use fcp_core::{CredentialStore, User};

    let test_users = vec![
        ("alice", "Secret123!", "Alice"),
        ("bob", "Secret456!", "Bob"),
    ];

    for (username, plaintext_password, display_name) in test_users {
        // Hash the password using bcrypt
        let password_hash = password::hash_password(plaintext_password)?;

        // For test users, we use the same password for both API and SIP (no separate digest credential)
        // Compute SIP HA1 hash for digest authentication
        let sip_ha1 = password::compute_ha1(username, realm, plaintext_password);

        let user = User {
            id: fcp_core::UserId::new(),
            username: username.to_string(),
            password: password_hash,
            digest_password: None, // Test users use same password for API and SIP
            sip_ha1_hash: Some(sip_ha1),
            display_name: Some(display_name.to_string()),
            tenant_id: fcp_core::TenantId::nil(), // Default tenant for test users
            enabled: true,
            role: fcp_core::UserRole::EndUser, // Test users default to EndUser role
            department_id: None,
            created_at: std::time::SystemTime::now(),
            updated_at: std::time::SystemTime::now(),
            media_preferences: fcp_core::MediaPreferences::default(),
            email: None,
            password_last_reset: None,
            phone_number: None,
            mobile_number: None,
            pin: None,
            extension: None,
            department: None,
            job_title: None,
            location: None,
            language: "en-US".to_string(),
            timezone: "UTC".to_string(),
            account_status: fcp_core::AccountStatus::Active,
            last_login: None,
            emergency_contact: fcp_core::EmergencyContact::default(),
            primary_phone_number_id: None,
            voicemail_box_id: None,
        };

        storage.put_user(user.clone()).await?;
        info!(
            username = %user.username,
            tenant_id = %user.tenant_id,
            "Created test user with hashed password"
        );
    }

    Ok(())
}

/// Mask sensitive information in connection strings for logging.
///
/// This function replaces passwords in connection strings with asterisks to prevent
/// leaking credentials in logs.
fn mask_connection_string(conn_str: &str) -> String {
    if conn_str.starts_with("sqlite:") {
        // SQLite connection strings don't contain credentials
        return conn_str.to_string();
    }

    // For database URLs like: postgres://user:password@host/db
    if let Some(at_pos) = conn_str.rfind('@') {
        if let Some(scheme_end) = conn_str.find("://") {
            let scheme = &conn_str[..scheme_end + 3];
            let after_at = &conn_str[at_pos..];

            // Check if there's a colon (indicating user:password format)
            let middle_part = &conn_str[scheme_end + 3..at_pos];
            if middle_part.contains(':') {
                // Split on the first colon to get username
                if let Some(colon_pos) = middle_part.find(':') {
                    let username = &middle_part[..colon_pos];
                    return format!("{}{}:***{}", scheme, username, after_at);
                }
            }
        }
    }

    // For MongoDB URIs like: mongodb://user:password@host/db
    if conn_str.starts_with("mongodb://") || conn_str.starts_with("mongodb+srv://") {
        let scheme = if conn_str.starts_with("mongodb+srv://") {
            "mongodb+srv://"
        } else {
            "mongodb://"
        };

        if let Some(at_pos) = conn_str.rfind('@') {
            let after_scheme = &conn_str[scheme.len()..at_pos];
            let after_at = &conn_str[at_pos..];

            if after_scheme.contains(':') {
                if let Some(colon_pos) = after_scheme.find(':') {
                    let username = &after_scheme[..colon_pos];
                    return format!("{}{}:***{}", scheme, username, after_at);
                }
            }
        }
    }

    // If we can't parse it, mask the entire string after the scheme
    if let Some(scheme_end) = conn_str.find("://") {
        format!("{}://***", &conn_str[..scheme_end])
    } else {
        "***".to_string()
    }
}

/// Load a test IVR flow for development and testing.
///
/// This function creates a simple welcome IVR flow that:
/// 1. Answers the call
/// 2. Plays welcome.wav audio file
/// 3. Hangs up
///
/// The flow is accessible by calling +15551234567
#[allow(dead_code)] // TODO: Enable for development/testing
async fn load_test_ivr_flow(
    flow_store: &Arc<dyn fcp_flow::FlowStore>,
    handler: &Arc<fcp_sip::flow_handler::FlowCallHandler>,
) -> Result<()> {
    use fcp_flow::{FlowDefinition, FlowNode, NodeType};
    use std::collections::HashMap;

    info!("Creating test IVR flow nodes");

    // Create nodes
    let mut nodes = HashMap::new();

    nodes.insert(
        "answer".to_string(),
        FlowNode {
            id: "answer".to_string(),
            name: Some("Answer Call".to_string()),
            node_type: NodeType::Answer {
                next: "play_welcome".to_string(),
            },
        },
    );

    nodes.insert(
        "play_welcome".to_string(),
        FlowNode {
            id: "play_welcome".to_string(),
            name: Some("Play Welcome".to_string()),
            node_type: NodeType::Play {
                url: "/var/lib/fcp/audio/welcome.wav".to_string(),
                loop_count: 0,
                next: "play_goodbye".to_string(),
                on_error: None,
            },
        },
    );

    nodes.insert(
        "play_goodbye".to_string(),
        FlowNode {
            id: "play_goodbye".to_string(),
            name: Some("Play Goodbye".to_string()),
            node_type: NodeType::Play {
                url: "/var/lib/fcp/audio/goodbye.wav".to_string(),
                loop_count: 0,
                next: "hangup".to_string(),
                on_error: None,
            },
        },
    );

    nodes.insert(
        "hangup".to_string(),
        FlowNode {
            id: "hangup".to_string(),
            name: Some("Hangup".to_string()),
            node_type: NodeType::Hangup { reason: None },
        },
    );

    // Create flow definition
    let flow = FlowDefinition {
        id: "welcome-test".to_string(),
        name: "Welcome Test IVR".to_string(),
        description: Some("Simple IVR to test audio playback with welcome.wav".to_string()),
        version: 1,
        initial_node: "answer".to_string(),
        nodes,
        variables: HashMap::new(),
        tenant_id: None,
        enabled: true,
        webhook_url: None,
        created_at: None,
        updated_at: None,
    };

    info!(
        flow_id = %flow.id,
        node_count = flow.nodes.len(),
        "Test IVR flow definition created"
    );

    // Insert flow into store
    flow_store
        .insert_flow(flow)
        .await
        .context("Failed to insert test IVR flow into store")?;

    info!("Test IVR flow inserted into flow store");

    // Configure DID routing: +15551234567 -> welcome-test
    handler
        .add_route("+15551234567".to_string(), "welcome-test".to_string())
        .await;

    info!(
        did = "+15551234567",
        flow_id = "welcome-test",
        "Test IVR DID routing configured - call +15551234567 to test audio playback"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_builder_default() {
        let temp_dir =
            std::env::temp_dir().join(format!("fcp-test-{:?}", std::time::SystemTime::now()));
        let db_path = temp_dir.join("test.db");

        let mut config = FcpConfig::default();
        config.transport.udp_bind = Some("0.0.0.0:0".parse().unwrap());
        config.storage.connection_string = format!("sqlite://{}", db_path.display());

        let runtime = RuntimeBuilder::new()
            .with_config(config)
            .build()
            .await
            .unwrap();
        assert!(!runtime.node_id().is_nil());

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_builder_with_custom_node_id() {
        let temp_dir =
            std::env::temp_dir().join(format!("fcp-test-{:?}", std::time::SystemTime::now()));
        let db_path = temp_dir.join("test.db");

        let mut config = FcpConfig::default();
        config.transport.udp_bind = Some("0.0.0.0:0".parse().unwrap());
        config.storage.connection_string = format!("sqlite://{}", db_path.display());

        let custom_id = NodeId::new();
        let runtime = RuntimeBuilder::new()
            .with_config(config)
            .with_node_id(custom_id)
            .build()
            .await
            .unwrap();

        assert_eq!(runtime.node_id(), custom_id);

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_builder_with_config() {
        let temp_dir =
            std::env::temp_dir().join(format!("fcp-test-{:?}", std::time::SystemTime::now()));
        let db_path = temp_dir.join("test.db");

        let mut config = FcpConfig::default();
        config.node.name = "test-node".to_string();
        config.transport.udp_bind = Some("0.0.0.0:0".parse().unwrap());
        config.storage.connection_string = format!("sqlite://{}", db_path.display());

        let runtime = RuntimeBuilder::new()
            .with_config(config.clone())
            .build()
            .await
            .unwrap();

        assert_eq!(runtime.config().node.name, "test-node");

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
