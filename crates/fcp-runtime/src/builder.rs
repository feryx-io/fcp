//! Runtime builder for configuring and constructing the FCP runtime.

use crate::Runtime;
use anyhow::{Context, Result};
use fcp_core::{FcpConfig, NodeId, ServiceManager};
use fcp_sip::prelude::*;
use fcp_sip::service::{DialogService, SubscriptionService, TransactionService};
use std::sync::Arc;
use tracing::info;

/// Builder for constructing a Runtime instance.
pub struct RuntimeBuilder {
    config: Option<FcpConfig>,
    node_id: Option<NodeId>,
}

impl RuntimeBuilder {
    /// Create a new runtime builder.
    pub(crate) fn new() -> Self {
        Self {
            config: None,
            node_id: None,
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

    /// Build the runtime.
    pub async fn build(self) -> Result<Runtime> {
        // Get or create config
        let config = self.config.unwrap_or_default();

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

        // Create shared transport dispatcher
        let dispatcher = create_transport_dispatcher(&config).await?;

        // Create transaction manager with shared dispatcher
        let transaction_manager = TransactionManager::new(dispatcher.clone());

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
        let (registrar, storage) = if config.registrar.enabled {
            let (reg, store) = create_registrar(&config).await?;
            (Some(reg), Some(store))
        } else {
            (None, None)
        };

        // Create routing service if enabled
        let routing_service = if config.routing.enabled {
            Some(create_routing_service(&config).await?)
        } else {
            None
        };

        // Create media manager
        let media_manager = create_media_manager(&config).await?;

        // Create transport listener
        let (listener, handler, call_manager) = create_transport_listener(
            &config,
            Arc::clone(&transaction_manager_ref),
            Arc::clone(&dialog_manager_ref),
            registrar,
            routing_service,
            Arc::clone(&uac),
            Arc::clone(&media_manager),
        )?;

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
            listener: Some(listener),
            shutdown_tx: None,
            storage,
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
async fn create_transport_dispatcher(config: &FcpConfig) -> Result<Arc<RealTransportDispatcher>> {
    use anyhow::Context;
    use fcp_sip::prelude::RealTransportDispatcher;

    // Use the same UDP port as the listener for the dispatcher
    // This ensures that SIP responses come back to the correct port via rport mechanism
    // The listener uses SO_REUSEADDR, so both sockets can bind to the same port
    let bind_addr = config
        .transport
        .udp_bind
        .unwrap_or_else(|| "0.0.0.0:5060".parse().unwrap());

    // Create dispatcher with UDP socket
    #[allow(unused_mut)]
    let mut dispatcher = RealTransportDispatcher::with_udp(bind_addr)
        .await
        .context("Failed to create transport dispatcher")?;

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
async fn create_registrar(
    config: &FcpConfig,
) -> Result<(
    Arc<fcp_sip::registrar::FcpRegistrar>,
    Arc<fcp_core::FileBackedStorage>,
)> {
    use fcp_core::{CredentialStore, FileBackedStorage};
    use fcp_sip::registrar::FcpRegistrar;

    info!(
        auth_enabled = config.registrar.auth_enabled,
        realm = %config.registrar.realm,
        "Creating registrar"
    );

    // Create storage for registrations and users
    let storage = Arc::new(
        FileBackedStorage::new(config.registrar.data_dir.clone())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create storage: {}", e))?,
    );

    // If authentication is enabled, ensure at least one user exists
    let credential_store: Option<Arc<dyn CredentialStore>> = if config.registrar.auth_enabled {
        // Check if any users exist
        let users = storage.list_users().await?;
        if users.is_empty() {
            // Try to load users from file if specified
            if let Some(users_file) = &config.registrar.users_file {
                info!(path = %users_file.display(), "Loading users from file");

                match load_users_from_file(users_file, &storage).await {
                    Ok(count) => {
                        info!(count, "Loaded users from file");
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "Failed to load users from file, creating default test users"
                        );
                        create_default_test_users(&storage).await?;
                    }
                }
            } else {
                info!("No users file specified, creating default test users");
                create_default_test_users(&storage).await?;
            }
        } else {
            info!(user_count = users.len(), "Loaded existing users from storage");
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

    Ok((Arc::new(registrar), storage))
}

/// Create a routing service based on configuration.
async fn create_routing_service(
    config: &FcpConfig,
) -> Result<Arc<fcp_routing::RoutingService>> {
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

/// Create a media manager based on configuration.
async fn create_media_manager(config: &FcpConfig) -> Result<Arc<fcp_media::MediaManager>> {
    use fcp_media::{BackendType, MediaConfig};

    info!(
        enabled = config.media.enabled,
        rtp_port_range = format!("{}-{}", config.media.rtp_port_start, config.media.rtp_port_end),
        "Creating media manager"
    );

    // Build media configuration from FCP config
    let media_config = MediaConfig {
        backend: BackendType::Embedded, // Always use embedded mode for now
        rtp_port_start: config.media.rtp_port_start,
        rtp_port_end: config.media.rtp_port_end,
        enable_srtp: config.media.enable_srtp,
        enable_transcoding: config.media.enable_transcoding,
        enable_recording: config.media.enable_recording,
        enable_dtmf: config.media.enable_dtmf,
        public_ip: config.media.public_ip.clone(),
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
    media_manager: Arc<fcp_media::MediaManager>,
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
    let local_addr = config
        .transport
        .local_ip
        .clone()
        .unwrap_or_else(|| {
            // Derive from UDP bind address if not specified
            config
                .transport
                .udp_bind
                .map(|addr| addr.ip().to_string())
                .unwrap_or_else(|| "127.0.0.1".to_string())
        });

    // Create CallManager for B2BUA functionality
    let call_manager = Arc::new(fcp_sip::call_manager::CallManager::new());

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
        Some(media_manager),
        Some(config.media.clone()),
    ));

    // Create listener (cast handler to trait object)
    let handler_dyn: Arc<dyn InboundMessageHandler> = Arc::clone(&handler) as Arc<dyn InboundMessageHandler>;
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
            .unwrap_or(5060);
        format!("{}:{}", local_ip, port)
    } else {
        // Fallback to UDP bind address
        config
            .transport
            .udp_bind
            .map(|addr| addr.to_string())
            .unwrap_or_else(|| "0.0.0.0:5060".to_string())
    };

    // Create DNS resolver (system resolver)
    let resolver = Arc::new(
        fcp_sip::prelude::SipResolver::from_system()
            .context("Failed to create DNS resolver")?,
    );

    // Build IntegratedUAC (reuses shared dispatcher)
    let integrated_uac = IntegratedUAC::builder()
        .local_uri(local_uri.as_str())
        .local_addr(&local_addr)?
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
    let bind_addr = config.transport.udp_bind
        .ok_or_else(|| anyhow::anyhow!("UDP bind address not configured"))?;

    // If bind is 0.0.0.0, use hardcoded IP for now
    let domain = if bind_addr.ip().is_unspecified() {
        // For now, use a placeholder - in production we'd detect the actual IP
        // TODO: Auto-detect actual IP address
        "192.168.1.81".to_string()
    } else {
        bind_addr.ip().to_string()
    };

    let bind_port = bind_addr.port();

    let uri_str = if bind_port != 5060 {
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
    storage: &Arc<fcp_core::FileBackedStorage>,
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
        // Hash the plaintext password
        let password_hash = password::hash_password(&entry.password)?;

        let user = User {
            username: entry.username.clone(),
            password: password_hash,
            display_name: entry.display_name.clone(),
            tenant_id: entry.tenant_id,
            enabled: entry.enabled,
            created_at: std::time::SystemTime::now(),
            updated_at: std::time::SystemTime::now(),
            media_preferences: entry.media_preferences.clone(),
        };

        storage.put_user(user).await?;
        count += 1;
        info!(username = %entry.username, tenant_id = %entry.tenant_id, "Loaded user from file");
    }

    Ok(count)
}

/// Create default test users with hashed passwords.
async fn create_default_test_users(storage: &Arc<fcp_core::FileBackedStorage>) -> Result<()> {
    use fcp_core::storage::password;
    use fcp_core::{CredentialStore, User};

    let test_users = vec![
        ("alice", "secret123", "Alice"),
        ("bob", "secret456", "Bob"),
    ];

    for (username, plaintext_password, display_name) in test_users {
        // Hash the password using bcrypt
        let password_hash = password::hash_password(plaintext_password)?;

        let user = User {
            username: username.to_string(),
            password: password_hash,
            display_name: Some(display_name.to_string()),
            tenant_id: fcp_core::TenantId::nil(), // Default tenant for test users
            enabled: true,
            created_at: std::time::SystemTime::now(),
            updated_at: std::time::SystemTime::now(),
            media_preferences: fcp_core::MediaPreferences::default(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_builder_default() {
        let mut config = FcpConfig::default();
        config.transport.udp_bind = Some("0.0.0.0:0".parse().unwrap());
        let runtime = RuntimeBuilder::new().with_config(config).build().await.unwrap();
        assert!(!runtime.node_id().is_nil());
    }

    #[tokio::test]
    async fn test_builder_with_custom_node_id() {
        let mut config = FcpConfig::default();
        config.transport.udp_bind = Some("0.0.0.0:0".parse().unwrap());
        let custom_id = NodeId::new();
        let runtime = RuntimeBuilder::new()
            .with_config(config)
            .with_node_id(custom_id)
            .build()
            .await
            .unwrap();

        assert_eq!(runtime.node_id(), custom_id);
    }

    #[tokio::test]
    async fn test_builder_with_config() {
        let mut config = FcpConfig::default();
        config.node.name = "test-node".to_string();
        config.transport.udp_bind = Some("0.0.0.0:0".parse().unwrap());

        let runtime = RuntimeBuilder::new()
            .with_config(config.clone())
            .build()
            .await
            .unwrap();

        assert_eq!(runtime.config().node.name, "test-node");
    }
}
