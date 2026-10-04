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
    Fanvil,
    Snom,
    Generic,
}

impl std::fmt::Display for PhoneVendor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Polycom => write!(f, "polycom"),
            Self::Yealink => write!(f, "yealink"),
            Self::Cisco => write!(f, "cisco"),
            Self::Grandstream => write!(f, "grandstream"),
            Self::Fanvil => write!(f, "fanvil"),
            Self::Snom => write!(f, "snom"),
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
            "fanvil" => Ok(Self::Fanvil),
            "snom" => Ok(Self::Snom),
            "generic" => Ok(Self::Generic),
            other => Err(format!("unknown vendor: {}", other)),
        }
    }
}

impl PhoneVendor {
    /// The vendors a phone can be (all but `Generic`).
    pub const PHONES: [PhoneVendor; 6] = [
        Self::Yealink,
        Self::Polycom,
        Self::Grandstream,
        Self::Cisco,
        Self::Fanvil,
        Self::Snom,
    ];

    /// Whether the vendor's configuration is XML, so values are escaped
    /// as they are written into it.
    pub fn is_xml(self) -> bool {
        matches!(
            self,
            Self::Polycom | Self::Grandstream | Self::Cisco | Self::Snom
        )
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
    /// Template body: Jinja (`minijinja`), where an older `${variable}`
    /// still renders as `{{ variable }}` (`docs/PROVISIONING.md` §5).
    pub template_body: String,
    pub description: Option<String>,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

// ---- Which template a phone gets (`docs/PROVISIONING.md` §5) ----

/// Whether a model pattern (`T5*`, `VVX ?50`) matches a model: `*` is any
/// run of characters, `?` any one, case and the separators ` `, `-` and
/// `_` ignored (`VVX_450`, `VVX 450` and `vvx450` are one model).
pub fn model_matches(pattern: &str, model: &str) -> bool {
    fn fold(s: &str) -> Vec<char> {
        s.chars()
            .filter(|c| !matches!(c, ' ' | '-' | '_'))
            .flat_map(char::to_lowercase)
            .collect()
    }
    let (p, m) = (fold(pattern), fold(model));
    // Iterative glob with one backtrack point.
    let (mut pi, mut mi) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while mi < m.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == m[mi]) {
            pi += 1;
            mi += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, mi));
            pi += 1;
        } else if let Some((sp, sm)) = star {
            pi = sp + 1;
            mi = sm + 1;
            star = Some((sp, sm + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

/// How closely a pattern names a model: the characters it fixes.
fn specificity(pattern: &str) -> usize {
    pattern
        .chars()
        .filter(|c| !matches!(c, '*' | '?' | ' ' | '-' | '_'))
        .count()
}

/// The template a phone gets when its device names none: one of its
/// vendor's whose pattern matches its model, the most specific first, then
/// one of its vendor's with no pattern, then a `generic` one with no
/// pattern. A tie goes to the name that sorts first.
pub fn pick_template<'a>(
    templates: &'a [ConfigTemplate],
    vendor: Option<PhoneVendor>,
    model: Option<&str>,
) -> Option<&'a ConfigTemplate> {
    let score = |t: &ConfigTemplate| -> Option<(u8, usize)> {
        let pattern = t
            .model_pattern
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty());
        match (Some(t.vendor) == vendor, pattern) {
            (true, Some(p)) => model
                .filter(|m| model_matches(p, m))
                .map(|_| (2, specificity(p))),
            (true, None) => Some((1, 0)),
            (false, None) if t.vendor == PhoneVendor::Generic => Some((0, 0)),
            _ => None,
        }
    };
    templates
        .iter()
        .filter_map(|t| score(t).map(|s| (s, t)))
        .max_by(|(a, ta), (b, tb)| a.cmp(b).then_with(|| tb.name.cmp(&ta.name)))
        .map(|(_, t)| t)
}

// ---- Rendering ----

/// What a phone's template is rendered with (`docs/PROVISIONING.md` §5).
/// Every field is always there, empty when it does not apply, so a
/// template can test it with `{% if %}`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TemplateContext {
    pub mac_address: String,
    pub device_name: String,
    /// The phone's vendor (`yealink`, …) as FCP knows it.
    pub vendor: String,
    pub model: String,
    pub firmware_version: String,

    /// Where the phone registers, and how (`[api.provisioning]`).
    pub sip_server: String,
    pub sip_port: u16,
    /// `udp`, `tcp` or `tls`.
    pub sip_transport: String,

    /// The phone's first account.
    pub sip_uri: String,
    pub sip_username: String,
    pub sip_auth_username: String,
    pub sip_auth_ha1_hash: String,
    pub sip_password: String,
    /// The same as `sip_password`, kept for older templates.
    pub sip_provisioning_password: String,

    pub owner_username: String,
    pub owner_display_name: String,
    pub owner_email: String,
    pub owner_user_id: String,

    /// Who the phone is for now: a hoteling guest, else its owner.
    pub effective_user_username: String,
    pub effective_user_display_name: String,
    pub effective_user_extension: String,
    pub effective_user_id: String,
    pub hoteling_guest: bool,

    /// The shared line the device is on, as its second account.
    pub shared_line_enabled: u8,
    pub shared_line_yes_no: String,
    pub shared_line_number: String,
    pub shared_line_appearances: u32,

    /// The tenant's IANA timezone (`calling.timezone`), `UTC` when unset,
    /// and its offset now: `+1`, `-5`, `+5:30`; `+01:00`; minutes.
    pub timezone: String,
    pub utc_offset: String,
    pub utc_offset_hhmm: String,
    pub utc_offset_minutes: i32,
    pub ntp_server: String,

    /// The tenant's provisioning URL (`…/provision/<key>/`), so a phone
    /// can be told to keep fetching from it; empty when unknown.
    pub provisioning_url: String,

    /// The phone's accounts in order: its own, then a shared line's.
    pub accounts: Vec<TemplateAccount>,
    /// The device's line keys, numbered from 1.
    pub line_keys: Vec<TemplateLineKey>,
}

/// One account on a phone.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TemplateAccount {
    /// 1 for the first.
    pub index: u32,
    pub label: String,
    pub display_name: String,
    pub user_name: String,
    pub auth_name: String,
    pub password: String,
    /// A shared line (BroadSoft SCA), with its appearances.
    pub shared: bool,
    pub appearances: u32,
}

/// One programmed key.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TemplateLineKey {
    pub index: u32,
    /// `blf`, `speed_dial` or `park`.
    pub kind: String,
    pub value: String,
    pub label: String,
}

impl TemplateContext {
    /// The offset fields for a timezone, now.
    pub fn set_timezone(&mut self, tz: Option<&str>) {
        use chrono::Offset;
        let tz: chrono_tz::Tz = tz.and_then(|t| t.parse().ok()).unwrap_or(chrono_tz::UTC);
        let minutes = chrono::Utc::now()
            .with_timezone(&tz)
            .offset()
            .fix()
            .local_minus_utc()
            / 60;
        let sign = if minutes < 0 { '-' } else { '+' };
        let (h, m) = (minutes.abs() / 60, minutes.abs() % 60);
        self.timezone = tz.name().to_string();
        self.utc_offset_minutes = minutes;
        self.utc_offset = if m == 0 {
            format!("{sign}{h}")
        } else {
            format!("{sign}{h}:{m:02}")
        };
        self.utc_offset_hhmm = format!("{sign}{h:02}:{m:02}");
    }

    /// A context with every field filled, for previewing a template with
    /// no device and for checking the starters.
    pub fn sample() -> Self {
        let mut c = Self {
            mac_address: "805ec0a1b2c3".into(),
            device_name: "Front desk".into(),
            vendor: "yealink".into(),
            model: "T54W".into(),
            firmware_version: "96.86.0.70".into(),
            sip_server: "pbx.example.com".into(),
            sip_port: 5061,
            sip_transport: "tls".into(),
            sip_uri: "sip:alice@pbx.example.com".into(),
            sip_username: "alice".into(),
            sip_auth_username: "alice".into(),
            sip_auth_ha1_hash: "0".repeat(32),
            sip_password: "s3cret&<pass>".into(),
            sip_provisioning_password: "s3cret&<pass>".into(),
            owner_username: "alice".into(),
            owner_display_name: "Alice O'Neil & Co".into(),
            owner_email: "alice@example.com".into(),
            owner_user_id: "00000000-0000-0000-0000-000000000001".into(),
            effective_user_username: "alice".into(),
            effective_user_display_name: "Alice O'Neil & Co".into(),
            effective_user_extension: "201".into(),
            effective_user_id: "00000000-0000-0000-0000-000000000001".into(),
            hoteling_guest: false,
            shared_line_enabled: 1,
            shared_line_yes_no: "Yes".into(),
            shared_line_number: "+15551230000".into(),
            shared_line_appearances: 2,
            ntp_server: "pool.ntp.org".into(),
            provisioning_url: "https://pbx.example.com/provision/0123456789abcdef0123456789abcdef/"
                .into(),
            accounts: vec![],
            line_keys: vec![
                TemplateLineKey {
                    index: 1,
                    kind: "blf".into(),
                    value: "202".into(),
                    label: "Bob".into(),
                },
                TemplateLineKey {
                    index: 2,
                    kind: "speed_dial".into(),
                    value: "+15559870000".into(),
                    label: "Lobby".into(),
                },
                TemplateLineKey {
                    index: 3,
                    kind: "park".into(),
                    value: "701".into(),
                    label: "Park 1".into(),
                },
            ],
            ..Default::default()
        };
        c.set_timezone(Some("America/New_York"));
        c.accounts = vec![
            TemplateAccount {
                index: 1,
                label: "201".into(),
                display_name: "Alice O'Neil & Co".into(),
                user_name: "alice".into(),
                auth_name: "alice".into(),
                password: "s3cret&<pass>".into(),
                shared: false,
                appearances: 0,
            },
            TemplateAccount {
                index: 2,
                label: "+15551230000".into(),
                display_name: "+15551230000".into(),
                user_name: "+15551230000".into(),
                auth_name: "alice".into(),
                password: "s3cret&<pass>".into(),
                shared: true,
                appearances: 2,
            },
        ];
        c
    }
}

/// The work a template may do before it is stopped (minijinja's fuel):
/// far beyond any phone's configuration, short of a loop that never ends.
const TEMPLATE_FUEL: u64 = 2_000_000;

/// An older `${name}` placeholder as Jinja's `{{ name }}`; anything else
/// that starts with `$` is left as it is.
pub fn legacy_placeholders(body: &str) -> std::borrow::Cow<'_, str> {
    if !body.contains("${") {
        return std::borrow::Cow::Borrowed(body);
    }
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end)
                if end > 0
                    && after[..end]
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_') =>
            {
                out.push_str("{{ ");
                out.push_str(&after[..end]);
                out.push_str(" }}");
                rest = &after[end + 1..];
            }
            _ => {
                out.push_str("${");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    std::borrow::Cow::Owned(out)
}

fn xml_escape_into(out: &mut minijinja::Output, s: &str) -> std::fmt::Result {
    use std::fmt::Write;
    for c in s.chars() {
        match c {
            '&' => out.write_str("&amp;")?,
            '<' => out.write_str("&lt;")?,
            '>' => out.write_str("&gt;")?,
            '"' => out.write_str("&quot;")?,
            '\'' => out.write_str("&apos;")?,
            c => out.write_char(c)?,
        }
    }
    Ok(())
}

fn environment(vendor: PhoneVendor, strict: bool) -> minijinja::Environment<'static> {
    use minijinja::{AutoEscape, Environment, UndefinedBehavior};
    let mut env = Environment::new();
    env.set_keep_trailing_newline(true);
    env.set_trim_blocks(true);
    env.set_lstrip_blocks(true);
    env.set_fuel(Some(TEMPLATE_FUEL));
    env.set_undefined_behavior(if strict {
        UndefinedBehavior::Strict
    } else {
        UndefinedBehavior::Lenient
    });
    let xml = vendor.is_xml();
    env.set_auto_escape_callback(move |_| {
        if xml {
            AutoEscape::Custom("xml")
        } else {
            AutoEscape::None
        }
    });
    env.set_formatter(|out, state, value| {
        if value.is_undefined() || value.is_none() {
            return Ok(());
        }
        if state.auto_escape() == AutoEscape::Custom("xml") && !value.is_safe() {
            let s = value.to_string();
            return xml_escape_into(out, &s).map_err(minijinja::Error::from);
        }
        write!(out, "{value}").map_err(minijinja::Error::from)
    });
    env
}

/// A template's syntax, checked when it is saved.
pub fn check_template(body: &str) -> Result<(), String> {
    let source = legacy_placeholders(body);
    let env = environment(PhoneVendor::Generic, false);
    env.template_from_str(&source)
        .map(|_| ())
        .map_err(|e| describe(&e))
}

/// Render a template: Jinja, with older `${name}` placeholders read as
/// `{{ name }}`, a missing value written as nothing, and values escaped
/// for XML when the vendor's configuration is XML (`|safe` opts out).
pub fn render(
    body: &str,
    vendor: PhoneVendor,
    context: &TemplateContext,
) -> Result<String, String> {
    render_with(body, vendor, context, false)
}

fn render_with(
    body: &str,
    vendor: PhoneVendor,
    context: &TemplateContext,
    strict: bool,
) -> Result<String, String> {
    let source = legacy_placeholders(body);
    environment(vendor, strict)
        .render_str(&source, context)
        .map_err(|e| describe(&e))
}

/// An error as one line: what, and where.
fn describe(e: &minijinja::Error) -> String {
    match (e.detail(), e.line()) {
        (Some(detail), Some(line)) => format!("{} on line {line}: {detail}", e.kind()),
        (Some(detail), None) => format!("{}: {detail}", e.kind()),
        (None, Some(line)) => format!("{} on line {line}", e.kind()),
        (None, None) => e.kind().to_string(),
    }
}

// ---- The starters ----

/// A built-in starter template shipped with FCP.
///
/// These are instantiated into a tenant's own `config_templates` via the
/// template-library API; the tenant's copy is then theirs to customize.
/// Bodies use only [`TemplateContext`]'s fields.
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
    const YEALINK: &str = r#"#!version:1.0.0.1
# FCP starter template for Yealink T4x/T5x (docs/PROVISIONING.md).
{% set transport = {"udp": 0, "tcp": 1, "tls": 2}[sip_transport] %}
{% for account in accounts %}
account.{{ account.index }}.enable = 1
account.{{ account.index }}.label = {{ account.label }}
account.{{ account.index }}.display_name = {{ account.display_name }}
account.{{ account.index }}.user_name = {{ account.user_name }}
account.{{ account.index }}.auth_name = {{ account.auth_name }}
account.{{ account.index }}.password = {{ account.password }}
account.{{ account.index }}.sip_server.1.address = {{ sip_server }}
account.{{ account.index }}.sip_server.1.port = {{ sip_port }}
account.{{ account.index }}.sip_server.1.transport_type = {{ transport }}
{% if sip_transport == "tls" %}
account.{{ account.index }}.srtp_encryption = 1
{% endif %}
{% if account.shared %}
account.{{ account.index }}.shared_line = 1
account.{{ account.index }}.line_seize.expires = 15
{% endif %}
{% endfor %}
{% for n in range(accounts|length + 1, 3) %}
account.{{ n }}.enable = 0
{% endfor %}

local_time.time_zone = {{ utc_offset }}
local_time.ntp_server1 = {{ ntp_server }}
{% if provisioning_url %}
static.auto_provision.server.url = {{ provisioning_url }}
{% endif %}

# Line keys after the accounts' own.
{% for key in line_keys %}
{% set n = accounts|length + key.index %}
linekey.{{ n }}.type = {{ {"blf": 16, "speed_dial": 13, "park": 10}[key.kind] }}
linekey.{{ n }}.line = 1
linekey.{{ n }}.value = {{ key.value }}
linekey.{{ n }}.label = {{ key.label }}
{% endfor %}
"#;

    const POLYCOM: &str = r#"<?xml version="1.0" encoding="utf-8" standalone="yes"?>
<!-- FCP starter template for Polycom VVX (docs/PROVISIONING.md). -->
<PHONE_CONFIG>
  <ALL
{% for account in accounts %}
    reg.{{ account.index }}.address="{{ account.user_name }}"
    reg.{{ account.index }}.displayName="{{ account.display_name }}"
    reg.{{ account.index }}.label="{{ account.label }}"
    reg.{{ account.index }}.auth.userId="{{ account.auth_name }}"
    reg.{{ account.index }}.auth.password="{{ account.password }}"
    reg.{{ account.index }}.server.1.address="{{ sip_server }}"
    reg.{{ account.index }}.server.1.port="{{ sip_port }}"
    reg.{{ account.index }}.server.1.transport="{{ {"udp": "UDPOnly", "tcp": "TCPOnly", "tls": "TLS"}[sip_transport] }}"
{% if account.shared %}
    reg.{{ account.index }}.type="shared"
    reg.{{ account.index }}.lineKeys="{{ account.appearances }}"
{% endif %}
{% endfor %}
    tcpIpApp.sntp.address="{{ ntp_server }}"
    tcpIpApp.sntp.gmtOffset="{{ utc_offset_minutes * 60 }}"
{% for key in line_keys %}
    attendant.resourceList.{{ key.index }}.address="{{ key.value }}"
    attendant.resourceList.{{ key.index }}.label="{{ key.label }}"
    attendant.resourceList.{{ key.index }}.type="{{ "normal" if key.kind == "blf" else "automata" }}"
{% endfor %}
  />
</PHONE_CONFIG>
"#;

    const GRANDSTREAM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- FCP starter template for Grandstream GXP/GRP (docs/PROVISIONING.md). -->
<gs_provision version="1">
  <config version="1">
    <!-- Account 1 -->
    <P271>1</P271>
    <P270>{{ device_name }}</P270>
    <P47>{{ sip_server }}:{{ sip_port }}</P47>
    <P35>{{ sip_username }}</P35>
    <P36>{{ sip_auth_username }}</P36>
    <P34>{{ sip_password }}</P34>
    <P3>{{ effective_user_display_name }}</P3>
    <!-- SIP transport: 0 UDP, 1 TCP, 2 TLS -->
    <P130>{{ {"udp": 0, "tcp": 1, "tls": 2}[sip_transport] }}</P130>
    <!-- NTP server -->
    <P30>{{ ntp_server }}</P30>
    <!-- Account 2: the shared line the device is on, if any. Turn on
         "Shared Call Appearance" (BroadSoft SCA) for it in the account's
         SIP settings: its P-value differs between models. -->
    <P401>{{ shared_line_enabled }}</P401>
    <P417>{{ shared_line_number }}</P417>
    <P402>{{ sip_server }}:{{ sip_port }}</P402>
    <P404>{{ shared_line_number }}</P404>
    <P405>{{ sip_auth_username }}</P405>
    <P406>{{ sip_password }}</P406>
    <!-- Line keys' P-values differ between models: add them per model. -->
  </config>
</gs_provision>
"#;

    const CISCO: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- FCP starter template for Cisco SPA/ATA, flat-profile (docs/PROVISIONING.md). -->
<flat-profile>
{% for account in accounts %}
  <Line_Enable_{{ account.index }}_ ua="na">Yes</Line_Enable_{{ account.index }}_>
  <Display_Name_{{ account.index }}_ ua="na">{{ account.display_name }}</Display_Name_{{ account.index }}_>
  <User_ID_{{ account.index }}_ ua="na">{{ account.user_name }}</User_ID_{{ account.index }}_>
  <Use_Auth_ID_{{ account.index }}_ ua="na">Yes</Use_Auth_ID_{{ account.index }}_>
  <Auth_ID_{{ account.index }}_ ua="na">{{ account.auth_name }}</Auth_ID_{{ account.index }}_>
  <Password_{{ account.index }}_ ua="na">{{ account.password }}</Password_{{ account.index }}_>
  <Proxy_{{ account.index }}_ ua="na">{{ sip_server }}:{{ sip_port }}</Proxy_{{ account.index }}_>
  <SIP_Transport_{{ account.index }}_ ua="na">{{ sip_transport|upper }}</SIP_Transport_{{ account.index }}_>
  <Register_{{ account.index }}_ ua="na">Yes</Register_{{ account.index }}_>
{% if account.shared %}
  <Shared_User_ID_{{ account.index }}_ ua="na">{{ account.user_name }}</Shared_User_ID_{{ account.index }}_>
  <Share_Ext_{{ account.index }}_ ua="na">shared</Share_Ext_{{ account.index }}_>
{% endif %}
{% endfor %}
  <Primary_NTP_Server ua="na">{{ ntp_server }}</Primary_NTP_Server>
  <Time_Zone ua="na">GMT{{ utc_offset_hhmm }}</Time_Zone>
{% if provisioning_url %}
  <Profile_Rule ua="na">{{ provisioning_url }}spa$MA.xml</Profile_Rule>
{% endif %}
{% for key in line_keys %}
  <Extended_Function_{{ key.index }}_ ua="na">fnc={{ {"blf": "blf+sd+cp", "speed_dial": "sd", "park": "prk"}[key.kind] }};sub={{ key.value }}@{{ sip_server }};nme={{ key.label }}</Extended_Function_{{ key.index }}_>
{% endfor %}
</flat-profile>
"#;

    const FANVIL: &str = r#"<<VOIP CONFIG FILE>>Version:2.0000000000
# FCP starter template for Fanvil X-series (docs/PROVISIONING.md).
# Check each key against your model's configuration guide.

<SIP CONFIG MODULE>
--SIP Line List--  :
{% for account in accounts %}
SIP{{ account.index }} Phone Number       :{{ account.user_name }}
SIP{{ account.index }} Display Name       :{{ account.display_name }}
SIP{{ account.index }} Register Addr      :{{ sip_server }}
SIP{{ account.index }} Register Port      :{{ sip_port }}
SIP{{ account.index }} Register User      :{{ account.auth_name }}
SIP{{ account.index }} Register Pswd      :{{ account.password }}
SIP{{ account.index }} Enable Reg         :1
SIP{{ account.index }} Transport          :{{ {"udp": 0, "tcp": 1, "tls": 3}[sip_transport] }}
{% endfor %}

<DSS CONFIG MODULE>
{% for key in line_keys %}
Fkey{{ key.index }} Type                  :{{ {"blf": 1, "speed_dial": 1, "park": 1}[key.kind] }}
Fkey{{ key.index }} Value                 :{{ key.value }}@1/{{ {"blf": "b", "speed_dial": "f", "park": "f"}[key.kind] }}
Fkey{{ key.index }} Title                 :{{ key.label }}
{% endfor %}
<<END OF FILE>>
"#;

    const SNOM: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<!-- FCP starter template for Snom D-series (docs/PROVISIONING.md). -->
<settings>
  <phone-settings>
{% for account in accounts %}
    <user_active idx="{{ account.index }}" perm="">on</user_active>
    <user_realname idx="{{ account.index }}" perm="">{{ account.display_name }}</user_realname>
    <user_name idx="{{ account.index }}" perm="">{{ account.user_name }}</user_name>
    <user_pname idx="{{ account.index }}" perm="">{{ account.auth_name }}</user_pname>
    <user_pass idx="{{ account.index }}" perm="">{{ account.password }}</user_pass>
    <user_host idx="{{ account.index }}" perm="">{{ sip_server }}:{{ sip_port }};transport={{ sip_transport }}</user_host>
{% if account.shared %}
    <user_shared_line idx="{{ account.index }}" perm="">on</user_shared_line>
{% endif %}
{% endfor %}
    <ntp_server perm="">{{ ntp_server }}</ntp_server>
{% if provisioning_url %}
    <setting_server perm="">{{ provisioning_url }}snom{model}-{mac}.htm</setting_server>
{% endif %}
  </phone-settings>
  <functionKeys>
{% for key in line_keys %}
    <fkey idx="{{ key.index - 1 }}" context="active" label="{{ key.label }}" perm="">{{ {"blf": "blf <sip:", "speed_dial": "speed ", "park": "speed "}[key.kind] }}{{ key.value }}{% if key.kind == "blf" %}@{{ sip_server }}>{% endif %}</fkey>
{% endfor %}
  </functionKeys>
</settings>
"#;

    &[
        BuiltinTemplate {
            vendor: PhoneVendor::Yealink,
            name: "yealink-starter",
            model_pattern: Some("T*"),
            description: "Starter template for Yealink T4x/T5x: accounts, time, line keys. Customize per deployment.",
            body: YEALINK,
        },
        BuiltinTemplate {
            vendor: PhoneVendor::Polycom,
            name: "polycom-starter",
            model_pattern: Some("VVX*"),
            description: "Starter template for Polycom VVX: lines, time, BLF. Customize per deployment.",
            body: POLYCOM,
        },
        BuiltinTemplate {
            vendor: PhoneVendor::Grandstream,
            name: "grandstream-starter",
            model_pattern: Some("G*"),
            description: "Starter template for Grandstream GXP/GRP: accounts 1 and 2, time. Customize per deployment.",
            body: GRANDSTREAM,
        },
        BuiltinTemplate {
            vendor: PhoneVendor::Cisco,
            name: "cisco-starter",
            model_pattern: Some("SPA*"),
            description: "Starter template for Cisco SPA/ATA (flat-profile): lines, time, extended functions. Customize per deployment.",
            body: CISCO,
        },
        BuiltinTemplate {
            vendor: PhoneVendor::Fanvil,
            name: "fanvil-starter",
            model_pattern: None,
            description: "Starter template for Fanvil X-series: lines and DSS keys. Customize per deployment.",
            body: FANVIL,
        },
        BuiltinTemplate {
            vendor: PhoneVendor::Snom,
            name: "snom-starter",
            model_pattern: Some("D*"),
            description: "Starter template for Snom D-series: identities, time, function keys. Customize per deployment.",
            body: SNOM,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template(name: &str, vendor: PhoneVendor, pattern: Option<&str>) -> ConfigTemplate {
        ConfigTemplate {
            id: crate::ConfigTemplateId::new(),
            tenant_id: TenantId::new(),
            name: name.into(),
            vendor,
            model_pattern: pattern.map(String::from),
            template_body: String::new(),
            description: None,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
        }
    }

    #[test]
    fn model_patterns_match_as_globs_ignoring_case_and_separators() {
        assert!(model_matches("T5*", "T54W"));
        assert!(model_matches("t5*", "T54W"));
        assert!(!model_matches("T4*", "T54W"));
        assert!(model_matches("VVX*", "VVX 450"));
        assert!(model_matches("VVX_4?0", "vvx450"));
        assert!(model_matches("*450", "VVX-450"));
        assert!(model_matches("*", "anything"));
        assert!(model_matches("T54W", "T54W"));
        assert!(!model_matches("T54W", "T54"));
        assert!(model_matches("G*X*0", "GXP2170"));
    }

    #[test]
    fn a_phone_gets_its_vendors_most_specific_template() {
        let ts = vec![
            template("generic", PhoneVendor::Generic, None),
            template("yealink-any", PhoneVendor::Yealink, None),
            template("yealink-t4", PhoneVendor::Yealink, Some("T4*")),
            template("yealink-t5", PhoneVendor::Yealink, Some("T5*")),
            template("yealink-t54w", PhoneVendor::Yealink, Some("T54W")),
            template("poly", PhoneVendor::Polycom, Some("VVX*")),
        ];
        let pick = |v, m| pick_template(&ts, v, m).map(|t| t.name.as_str());
        assert_eq!(
            pick(Some(PhoneVendor::Yealink), Some("T54W")),
            Some("yealink-t54w")
        );
        assert_eq!(
            pick(Some(PhoneVendor::Yealink), Some("T57W")),
            Some("yealink-t5")
        );
        assert_eq!(
            pick(Some(PhoneVendor::Yealink), Some("T46U")),
            Some("yealink-t4")
        );
        assert_eq!(
            pick(Some(PhoneVendor::Yealink), Some("W60B")),
            Some("yealink-any")
        );
        assert_eq!(pick(Some(PhoneVendor::Yealink), None), Some("yealink-any"));
        assert_eq!(
            pick(Some(PhoneVendor::Polycom), Some("VVX 450")),
            Some("poly")
        );
        // A Polycom of a model no pattern names, and a phone of an unknown
        // vendor, fall to the generic template.
        assert_eq!(
            pick(Some(PhoneVendor::Polycom), Some("CCX 500")),
            Some("generic")
        );
        assert_eq!(pick(None, Some("T54W")), Some("generic"));
        assert_eq!(
            pick_template(&ts[1..], Some(PhoneVendor::Snom), Some("D785")).map(|t| t.name.as_str()),
            None
        );
    }

    #[test]
    fn older_placeholders_still_render() {
        let mut c = TemplateContext::sample();
        c.sip_password = "pw".into();
        let out = render(
            "server=${sip_server}\nuser=${sip_username}\npw=${sip_password}\nunknown=${nope}\ncost=$5 ${ not a name}\n",
            PhoneVendor::Yealink,
            &c,
        )
        .unwrap();
        assert_eq!(
            out,
            "server=pbx.example.com\nuser=alice\npw=pw\nunknown=\ncost=$5 ${ not a name}\n"
        );
        assert_eq!(legacy_placeholders("a=${b}"), "a={{ b }}");
    }

    #[test]
    fn values_are_escaped_in_xml_and_not_in_text() {
        let c = TemplateContext::sample();
        let body = "<a n=\"{{ owner_display_name }}\">{{ sip_password }}</a>{{ \"<b/>\"|safe }}\n";
        assert_eq!(
            render(body, PhoneVendor::Polycom, &c).unwrap(),
            "<a n=\"Alice O&apos;Neil &amp; Co\">s3cret&amp;&lt;pass&gt;</a><b/>\n"
        );
        assert_eq!(
            render("pw={{ sip_password }}\n", PhoneVendor::Yealink, &c).unwrap(),
            "pw=s3cret&<pass>\n"
        );
    }

    #[test]
    fn a_broken_template_is_refused_with_its_line() {
        assert!(check_template("ok {{ sip_server }}").is_ok());
        assert!(check_template("${sip_server}").is_ok());
        let e = check_template("a\n{% for x in accounts %}\nno end").unwrap_err();
        assert!(e.contains("line"), "{e}");
        assert!(check_template("{{ unclosed").is_err());
    }

    #[test]
    fn a_template_that_never_ends_is_stopped() {
        let e = render(
            "{% for a in range(100000) %}{% for b in range(100000) %}x{% endfor %}{% endfor %}",
            PhoneVendor::Generic,
            &TemplateContext::sample(),
        )
        .unwrap_err();
        assert!(e.to_lowercase().contains("fuel"), "{e}");
    }

    #[test]
    fn timezone_offsets_are_written_three_ways() {
        let mut c = TemplateContext::default();
        c.set_timezone(Some("Asia/Kolkata"));
        assert_eq!(
            (
                c.utc_offset.as_str(),
                c.utc_offset_hhmm.as_str(),
                c.utc_offset_minutes
            ),
            ("+5:30", "+05:30", 330)
        );
        c.set_timezone(None);
        assert_eq!((c.timezone.as_str(), c.utc_offset.as_str()), ("UTC", "+0"));
        c.set_timezone(Some("not/a zone"));
        assert_eq!(c.timezone, "UTC");
    }

    /// Every starter renders against a full context with nothing undefined,
    /// the XML ones are well-formed, and every vendor has one.
    #[test]
    fn the_starters_render_with_every_value_and_xml_stays_well_formed() {
        let c = TemplateContext::sample();
        for t in builtin_templates() {
            assert!(check_template(t.body).is_ok(), "{}", t.name);
            let out = render_with(t.body, t.vendor, &c, true)
                .unwrap_or_else(|e| panic!("{}: {e}", t.name));
            assert!(
                !out.contains("{{") && !out.contains("{%"),
                "{}: {out}",
                t.name
            );
            assert!(out.contains("pbx.example.com"), "{}: {out}", t.name);
            if t.vendor.is_xml() {
                let mut reader = quick_xml::Reader::from_str(&out);
                loop {
                    match reader.read_event() {
                        Ok(quick_xml::events::Event::Eof) => break,
                        Ok(_) => {}
                        Err(e) => panic!("{} is not well-formed: {e}\n{out}", t.name),
                    }
                }
                assert!(!out.contains("s3cret&<"), "{}: unescaped", t.name);
            }
        }
        for vendor in PhoneVendor::PHONES {
            assert!(
                builtin_templates().iter().any(|t| t.vendor == vendor),
                "no starter for {vendor}"
            );
        }
    }

    #[test]
    fn the_yealink_starter_writes_accounts_and_keys() {
        let c = TemplateContext::sample();
        let yealink = builtin_templates()
            .iter()
            .find(|t| t.vendor == PhoneVendor::Yealink)
            .unwrap();
        let out = render(yealink.body, yealink.vendor, &c).unwrap();
        for want in [
            "account.1.user_name = alice\n",
            "account.1.sip_server.1.transport_type = 2\n",
            "account.2.shared_line = 1\n",
            "local_time.time_zone = ",
            "linekey.3.type = 16\n",
            "linekey.3.value = 202\n",
            "linekey.5.type = 10\n",
            "static.auto_provision.server.url = https://pbx.example.com/provision/",
        ] {
            assert!(out.contains(want), "{want:?} in\n{out}");
        }
        assert!(!out.contains("account.2.enable = 0"), "{out}");
    }
}
