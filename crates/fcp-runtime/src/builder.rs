//! Runtime builder for configuring and constructing the FCP runtime.

use crate::Runtime;
use anyhow::Result;
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

        // Create transport listener
        let listener = create_transport_listener(
            &config,
            Arc::clone(&transaction_manager_ref),
            Arc::clone(&dialog_manager_ref),
        )?;

        Ok(Runtime {
            config: Arc::new(config),
            node_id,
            services,
            transaction_manager: transaction_manager_ref,
            dialog_manager: dialog_manager_ref,
            subscription_manager: subscription_manager_ref,
            uac,
            listener: Some(listener),
            shutdown_tx: None,
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

    // Get bind address from config, or use port 0 (OS assigns random port)
    // This is especially useful for tests where multiple instances may run simultaneously
    let bind_addr = config
        .transport
        .udp_bind
        .unwrap_or_else(|| "0.0.0.0:0".parse().unwrap());

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

/// Create a transport listener for receiving inbound SIP messages.
fn create_transport_listener(
    config: &FcpConfig,
    transaction_manager: Arc<TransactionManager>,
    dialog_manager: Arc<DialogManager>,
) -> Result<TransportListener> {
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

    // Create message handler that routes to transaction and dialog managers
    let handler = Arc::new(SipMessageHandler::new(
        transaction_manager,
        dialog_manager,
        local_uri,
        media_profile,
        local_addr,
        config.transport.rtp_port,
    ));

    // Create and return the listener
    Ok(TransportListener::new(listener_config, handler))
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

    // Determine local address from config or use UDP bind address
    let local_addr = config
        .transport
        .udp_bind
        .unwrap_or_else(|| "0.0.0.0:0".parse().unwrap());

    // Create DNS resolver (system resolver)
    let resolver = Arc::new(
        fcp_sip::prelude::SipResolver::from_system()
            .context("Failed to create DNS resolver")?,
    );

    // Build IntegratedUAC (reuses shared dispatcher)
    let integrated_uac = IntegratedUAC::builder()
        .local_uri(local_uri.as_str())
        .local_addr(local_addr.to_string())?
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

    // Try to use configured domain, or fallback to node name
    let domain = if !config.node.local_domain.is_empty() {
        config.node.local_domain.clone()
    } else {
        format!("{}.local", config.node.name)
    };

    // Create SIP URI: sip:node@domain
    let uri_str = format!("sip:{}@{}", config.node.name, domain);

    let uri = Uri::parse(&uri_str).context("Failed to parse URI")?;

    uri.as_sip()
        .map(|sip| sip.clone())
        .ok_or_else(|| anyhow::anyhow!("Not a SIP URI"))
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
