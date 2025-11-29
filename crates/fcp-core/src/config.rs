use serde::{Deserialize, Serialize};
use std::net::IpAddr;

/// Basic runtime configuration for a single-node deployment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FcpConfig {
    pub node_name: String,
    pub bind_addr: IpAddr,
    pub sip_port: u16,
}

impl Default for FcpConfig {
    fn default() -> Self {
        Self {
            node_name: "fcp-node".to_string(),
            bind_addr: "0.0.0.0".parse().unwrap(),
            sip_port: 5060,
        }
    }
}
