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

/// A built-in starter template shipped with FCP.
///
/// These are instantiated into a tenant's own `config_templates` via the
/// template-library API; the tenant's copy is then theirs to customize.
/// Bodies use only variables the provisioning endpoint supplies:
/// `mac_address`, `device_name`, `model`, `sip_server`, `sip_port`, and (for
/// devices with an owner) `sip_uri`, `sip_username`, `sip_auth_username`,
/// `sip_auth_ha1_hash`, `sip_password`, `owner_username`,
/// `owner_display_name`, `owner_email`, and for a device on a shared line
/// (its second account, `docs/ENTERPRISE_CALLING.md` §3.9)
/// `shared_line_enabled` (`1` or `0`), `shared_line_yes_no` (`Yes` or
/// `No`), `shared_line_number`,
/// `shared_line_appearances`.
#[derive(Debug, Clone, Copy)]
pub struct BuiltinTemplate {
    pub vendor: PhoneVendor,
    /// Name the tenant copy is created under (unique per tenant).
    pub name: &'static str,
    pub model_pattern: Option<&'static str>,
    pub description: &'static str,
    pub body: &'static str,
}

/// The built-in vendor starter templates.
pub fn builtin_templates() -> &'static [BuiltinTemplate] {
    const YEALINK: &str = "#!version:1.0.0.1

# FCP starter template for Yealink T4x/T5x series.
# Registers account 1 against the FCP SIP server.
account.1.enable = 1
account.1.label = ${device_name}
account.1.display_name = ${sip_username}
account.1.user_name = ${sip_username}
account.1.auth_name = ${sip_auth_username}
account.1.password = ${sip_password}
account.1.sip_server.1.address = ${sip_server}
account.1.sip_server.1.port = ${sip_port}
account.1.sip_server.1.transport_type = 0

# Account 2: the shared line the device is on, if any (BroadSoft SCA).
account.2.enable = ${shared_line_enabled}
account.2.label = ${shared_line_number}
account.2.display_name = ${shared_line_number}
account.2.user_name = ${shared_line_number}
account.2.auth_name = ${sip_auth_username}
account.2.password = ${sip_password}
account.2.sip_server.1.address = ${sip_server}
account.2.sip_server.1.port = ${sip_port}
account.2.sip_server.1.transport_type = 0
account.2.shared_line = 1
account.2.line_seize.expires = 15
";

    const POLYCOM: &str = r#"<?xml version="1.0" encoding="utf-8" standalone="yes"?>
<!-- FCP starter template for Polycom VVX series. -->
<PHONE_CONFIG>
  <ALL
    reg.1.address="${sip_username}"
    reg.1.displayName="${sip_username}"
    reg.1.label="${device_name}"
    reg.1.auth.userId="${sip_auth_username}"
    reg.1.auth.password="${sip_password}"
    reg.1.server.1.address="${sip_server}"
    reg.1.server.1.port="${sip_port}"
    reg.1.server.1.transport="UDPOnly"
    reg.2.address="${shared_line_number}"
    reg.2.displayName="${shared_line_number}"
    reg.2.label="${shared_line_number}"
    reg.2.type="shared"
    reg.2.lineKeys="${shared_line_appearances}"
    reg.2.auth.userId="${sip_auth_username}"
    reg.2.auth.password="${sip_password}"
    reg.2.server.1.address="${sip_server}"
    reg.2.server.1.port="${sip_port}"
    reg.2.server.1.transport="UDPOnly"
  />
</PHONE_CONFIG>
"#;

    const GRANDSTREAM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- FCP starter template for Grandstream GXP/GRP series. -->
<gs_provision version="1">
  <config version="1">
    <!-- Account 1 active -->
    <P271>1</P271>
    <!-- Account 1 name -->
    <P270>${device_name}</P270>
    <!-- SIP server -->
    <P47>${sip_server}</P47>
    <!-- SIP user ID -->
    <P35>${sip_username}</P35>
    <!-- Authenticate ID -->
    <P36>${sip_auth_username}</P36>
    <!-- Authenticate password -->
    <P34>${sip_password}</P34>
    <!-- Account 2: the shared line the device is on, if any. Turn on
         "Shared Call Appearance" (BroadSoft SCA) for it in the account's
         SIP settings: its P-value differs between models. -->
    <P401>${shared_line_enabled}</P401>
    <P417>${shared_line_number}</P417>
    <P402>${sip_server}</P402>
    <P404>${shared_line_number}</P404>
    <P405>${sip_auth_username}</P405>
    <P406>${sip_password}</P406>
  </config>
</gs_provision>
"#;

    const CISCO: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- FCP starter template for Cisco SPA/ATA series (flat-profile format). -->
<flat-profile>
  <Line_Enable_1_ ua="na">Yes</Line_Enable_1_>
  <Display_Name_1_ ua="na">${device_name}</Display_Name_1_>
  <User_ID_1_ ua="na">${sip_username}</User_ID_1_>
  <Use_Auth_ID_1_ ua="na">Yes</Use_Auth_ID_1_>
  <Auth_ID_1_ ua="na">${sip_auth_username}</Auth_ID_1_>
  <Password_1_ ua="na">${sip_password}</Password_1_>
  <Proxy_1_ ua="na">${sip_server}</Proxy_1_>
  <Register_1_ ua="na">Yes</Register_1_>
  <!-- Line 2: the shared line the device is on, if any. -->
  <Line_Enable_2_ ua="na">${shared_line_yes_no}</Line_Enable_2_>
  <Display_Name_2_ ua="na">${shared_line_number}</Display_Name_2_>
  <User_ID_2_ ua="na">${shared_line_number}</User_ID_2_>
  <Shared_User_ID_2_ ua="na">${shared_line_number}</Shared_User_ID_2_>
  <Share_Ext_2_ ua="na">shared</Share_Ext_2_>
  <Use_Auth_ID_2_ ua="na">Yes</Use_Auth_ID_2_>
  <Auth_ID_2_ ua="na">${sip_auth_username}</Auth_ID_2_>
  <Password_2_ ua="na">${sip_password}</Password_2_>
  <Proxy_2_ ua="na">${sip_server}</Proxy_2_>
</flat-profile>
"#;

    &[
        BuiltinTemplate {
            vendor: PhoneVendor::Yealink,
            name: "yealink-starter",
            model_pattern: Some("T4*"),
            description: "Starter template for Yealink T4x/T5x: registers account 1. Customize per deployment.",
            body: YEALINK,
        },
        BuiltinTemplate {
            vendor: PhoneVendor::Polycom,
            name: "polycom-starter",
            model_pattern: Some("VVX*"),
            description: "Starter template for Polycom VVX: registers line 1. Customize per deployment.",
            body: POLYCOM,
        },
        BuiltinTemplate {
            vendor: PhoneVendor::Grandstream,
            name: "grandstream-starter",
            model_pattern: Some("GXP*"),
            description: "Starter template for Grandstream GXP/GRP: registers account 1. Customize per deployment.",
            body: GRANDSTREAM,
        },
        BuiltinTemplate {
            vendor: PhoneVendor::Cisco,
            name: "cisco-starter",
            model_pattern: Some("SPA*"),
            description: "Starter template for Cisco SPA/ATA (flat-profile): registers line 1. Customize per deployment.",
            body: CISCO,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every ${variable} in a built-in body must be one the provisioning
    /// endpoint actually supplies, or it would survive rendering as a
    /// literal placeholder in a phone's config.
    #[test]
    fn test_builtin_templates_use_only_supported_variables() {
        const SUPPORTED: &[&str] = &[
            "mac_address",
            "device_name",
            "model",
            "sip_server",
            "sip_port",
            "sip_uri",
            "sip_username",
            "sip_auth_username",
            "sip_auth_ha1_hash",
            "sip_password",
            "sip_provisioning_password",
            "owner_username",
            "owner_display_name",
            "owner_email",
            "owner_user_id",
            "shared_line_enabled",
            "shared_line_yes_no",
            "shared_line_number",
            "shared_line_appearances",
        ];
        for template in builtin_templates() {
            let body = template.body;
            let mut rest = body;
            while let Some(start) = rest.find("${") {
                let after = &rest[start + 2..];
                let end = after.find('}').unwrap_or_else(|| {
                    panic!("unterminated placeholder in {} template", template.name)
                });
                let var = &after[..end];
                assert!(
                    SUPPORTED.contains(&var),
                    "{} template uses unsupported variable ${{{var}}}",
                    template.name
                );
                rest = &after[end + 1..];
            }
        }
    }

    #[test]
    fn test_builtin_templates_cover_four_vendors() {
        let vendors: Vec<PhoneVendor> = builtin_templates().iter().map(|t| t.vendor).collect();
        assert!(vendors.contains(&PhoneVendor::Yealink));
        assert!(vendors.contains(&PhoneVendor::Polycom));
        assert!(vendors.contains(&PhoneVendor::Grandstream));
        assert!(vendors.contains(&PhoneVendor::Cisco));
    }
}
