//! Configuration templates for device provisioning.

use serde::{Deserialize, Serialize};
use std::time::SystemTime;

use crate::identifiers::TenantId;

/// Supported phone vendors for provisioning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhoneVendor {
    Polycom,
    Yealink,
    Cisco,
    Grandstream,
    Generic,
}

impl std::fmt::Display for PhoneVendor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Polycom => write!(f, "polycom"),
            Self::Yealink => write!(f, "yealink"),
            Self::Cisco => write!(f, "cisco"),
            Self::Grandstream => write!(f, "grandstream"),
            Self::Generic => write!(f, "generic"),
        }
    }
}

impl std::str::FromStr for PhoneVendor {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "polycom" => Ok(Self::Polycom),
            "yealink" => Ok(Self::Yealink),
            "cisco" => Ok(Self::Cisco),
            "grandstream" => Ok(Self::Grandstream),
            "generic" => Ok(Self::Generic),
            other => Err(format!("unknown vendor: {}", other)),
        }
    }
}

/// A configuration template for auto-provisioning phones.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigTemplate {
    pub id: crate::ConfigTemplateId,
    pub tenant_id: TenantId,
    pub name: String,
    pub vendor: PhoneVendor,
    /// Model pattern for matching (e.g., "T4*" for Yealink T4x series)
    pub model_pattern: Option<String>,
    /// Template body with ${variable} placeholders
    pub template_body: String,
    pub description: Option<String>,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}
