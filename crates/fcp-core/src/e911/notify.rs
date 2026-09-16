//! On-site notification of emergency calls (Kari's Law,
//! `docs/E911_BLOCK_2.md` §8).
//!
//! A tenant lists typed contacts; a call that is placed becomes one
//! [`EmergencyNotice`], rendered per channel; every attempt to deliver it
//! is an [`EmergencyNotification`] row, so a tenant can prove afterwards
//! that someone was told — and a `933` test call proves the chain before
//! it is needed.

use super::EmergencyKind;
use crate::TenantId;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use std::time::SystemTime;

/// How an on-site notification reaches someone (§8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContactKind {
    /// An email address, delivered over SMTP.
    Email,
    /// An E.164 number, delivered through the configured SMS provider.
    Sms,
    /// An extension or SIP URI, called and spoken to.
    Call,
    /// A webhook endpoint id, delivered by the existing dispatcher.
    Webhook,
}

impl fmt::Display for ContactKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ContactKind::Email => "email",
            ContactKind::Sms => "sms",
            ContactKind::Call => "call",
            ContactKind::Webhook => "webhook",
        })
    }
}

impl FromStr for ContactKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "email" | "mail" => Ok(ContactKind::Email),
            "sms" | "text" => Ok(ContactKind::Sms),
            "call" | "phone" => Ok(ContactKind::Call),
            "webhook" => Ok(ContactKind::Webhook),
            other => Err(format!(
                "unknown contact kind {other:?}: expected email, sms, call or webhook"
            )),
        }
    }
}

/// One place an emergency notification is sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotifyContact {
    pub kind: ContactKind,
    /// The address, number, extension or webhook endpoint id.
    pub target: String,
    /// Who this is, for the delivery log and the admin UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl NotifyContact {
    pub fn new(kind: ContactKind, target: impl Into<String>) -> Self {
        Self {
            kind,
            target: target.into(),
            name: None,
        }
    }

    /// Read a contact that was written as free text, by its shape — how
    /// the contacts a tenant saved before block 2 are migrated (§8.1):
    /// a SIP URI or a bare extension is a call, an `@` without a scheme an
    /// email, a phone number an SMS, a UUID a webhook endpoint.
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.is_empty() {
            return None;
        }
        let lower = value.to_ascii_lowercase();
        if lower.starts_with("sip:") || lower.starts_with("sips:") || lower.starts_with("tel:") {
            return Some(Self::new(ContactKind::Call, value));
        }
        if let Some(rest) = lower
            .strip_prefix("mailto:")
            .or_else(|| lower.strip_prefix("email:"))
        {
            return Some(Self::new(ContactKind::Email, rest.trim()));
        }
        if let Some((kind, target)) = value.split_once(':') {
            if let Ok(kind) = kind.parse::<ContactKind>() {
                return Some(Self::new(kind, target.trim()));
            }
        }
        if value.contains('@') {
            return Some(Self::new(ContactKind::Email, value));
        }
        if uuid::Uuid::parse_str(value).is_ok() {
            return Some(Self::new(ContactKind::Webhook, value));
        }
        let digits: String = value.chars().filter(|c| c.is_ascii_digit()).collect();
        let dialable = !digits.is_empty()
            && value
                .chars()
                .all(|c| c.is_ascii_digit() || "+-() .".contains(c));
        if dialable {
            // A short number is an extension to ring; a full one is a
            // mobile to text.
            return Some(if digits.len() >= 7 {
                Self::new(ContactKind::Sms, value)
            } else {
                Self::new(ContactKind::Call, value)
            });
        }
        None
    }

    /// What is wrong with this contact, empty when it can be delivered to.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let target = self.target.trim();
        if target.is_empty() {
            problems.push(format!("{} contact has no target", self.kind));
            return problems;
        }
        if target.contains('\r') || target.contains('\n') {
            problems.push(format!("{} target must not contain CR/LF", self.kind));
        }
        match self.kind {
            ContactKind::Email => {
                let parts: Vec<&str> = target.split('@').collect();
                if parts.len() != 2 || parts.iter().any(|p| p.is_empty()) || !parts[1].contains('.')
                {
                    problems.push(format!("{target:?} is not an email address"));
                }
            }
            ContactKind::Sms => {
                let digits = target.chars().filter(|c| c.is_ascii_digit()).count();
                if digits < 7 {
                    problems.push(format!(
                        "{target:?} is not a phone number that can be texted"
                    ));
                }
            }
            ContactKind::Call => {
                let sip = target.to_ascii_lowercase().starts_with("sip:")
                    || target.to_ascii_lowercase().starts_with("sips:");
                let extension = target
                    .chars()
                    .all(|c| c.is_ascii_digit() || "+-*#".contains(c));
                if !sip && !extension {
                    problems.push(format!("{target:?} is not an extension or a SIP URI"));
                }
            }
            ContactKind::Webhook => {
                if uuid::Uuid::parse_str(target).is_err() {
                    problems.push(format!("{target:?} is not a webhook endpoint id"));
                }
            }
        }
        problems
    }

    /// How this contact reads in the delivery log and in the event's
    /// `notify_contacts`, which carried plain strings before block 2.
    pub fn display_target(&self) -> String {
        self.target.trim().to_string()
    }
}

impl fmt::Display for NotifyContact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.name {
            Some(name) => write!(f, "{} {} <{}>", self.kind, name, self.target),
            None => write!(f, "{} {}", self.kind, self.target),
        }
    }
}

/// What everyone told about an emergency call is told (§8.2): that it was
/// placed, by whom, the callback number, where it said the caller was, and
/// when — plus whether it was only a test.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmergencyNotice {
    pub tenant_id: TenantId,
    /// The call's id, as on its record and its CDR.
    pub call_id: String,
    pub kind: EmergencyKind,
    pub dialed: String,
    /// The caller's SIP URI.
    pub caller: String,
    /// The caller's username or extension, when it is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback_number: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trunk_name: Option<String>,
    pub placed_at: SystemTime,
}

impl EmergencyNotice {
    pub fn is_test(&self) -> bool {
        self.kind == EmergencyKind::Test
    }

    /// `[TEST] ` on a test call's notice, so nobody runs for a drill and
    /// nobody takes a real call for one.
    fn prefix(&self) -> &'static str {
        if self.is_test() {
            "[TEST] "
        } else {
            ""
        }
    }

    /// Who placed the call, as it reads in the notice.
    pub fn caller_label(&self) -> String {
        match &self.caller_name {
            Some(name) => format!("{name} ({})", self.caller),
            None => self.caller.clone(),
        }
    }

    /// An email subject or a notification title.
    pub fn subject(&self) -> String {
        format!(
            "{}Emergency call: {} dialled {}",
            self.prefix(),
            self.caller_name.as_deref().unwrap_or(&self.caller),
            self.dialed
        )
    }

    /// The notice, line by line: the same words on every channel.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        lines.push(format!(
            "{}An emergency call was placed from your organisation.",
            self.prefix()
        ));
        lines.push(format!("Number dialled: {}", self.dialed));
        lines.push(format!("Caller: {}", self.caller_label()));
        lines.push(format!(
            "Callback number: {}",
            self.callback_number.as_deref().unwrap_or("none")
        ));
        lines.push(format!(
            "Location sent: {}",
            self.location_summary
                .as_deref()
                .unwrap_or("none — no dispatchable location was available")
        ));
        if let Some(source) = &self.location_source {
            lines.push(format!("Location source: {source}"));
        }
        if let Some(address) = &self.client_address {
            lines.push(format!("Seen at: {address}"));
        }
        lines.push(format!("Time: {}", rfc3339(self.placed_at)));
        if self.is_test() {
            lines.push(
                "This was a test call (933). No emergency services were contacted.".to_string(),
            );
        }
        lines
    }

    /// The email body.
    pub fn text(&self) -> String {
        self.lines().join("\n")
    }

    /// One line, short enough for a text message.
    pub fn sms_text(&self) -> String {
        let mut text = format!(
            "{}Emergency call: {} dialled {} at {}.",
            self.prefix(),
            self.caller_label(),
            self.dialed,
            rfc3339(self.placed_at)
        );
        if let Some(location) = &self.location_summary {
            text.push_str(&format!(" Location: {location}."));
        }
        if let Some(callback) = &self.callback_number {
            text.push_str(&format!(" Callback: {callback}."));
        }
        text
    }

    /// What a phone is told, in words a synthesiser reads well.
    pub fn spoken(&self) -> String {
        let mut text = String::new();
        if self.is_test() {
            text.push_str("This is a test. ");
        }
        text.push_str(&format!(
            "An emergency call was placed. {} dialled {}. ",
            self.caller_name.as_deref().unwrap_or("A caller"),
            spell_digits(&self.dialed)
        ));
        match &self.location_summary {
            Some(location) => text.push_str(&format!("The location sent was {location}. ")),
            None => text.push_str("No location was sent with the call. "),
        }
        if let Some(callback) = &self.callback_number {
            text.push_str(&format!(
                "The callback number is {}. ",
                spell_digits(callback)
            ));
        }
        if self.is_test() {
            text.push_str("This was only a test.");
        }
        text
    }

    /// The webhook payload.
    pub fn payload(&self) -> serde_json::Value {
        serde_json::json!({
            "notice": self,
            "subject": self.subject(),
            "text": self.text(),
            "test": self.is_test(),
        })
    }
}

/// The time a notice carries, in one form on every channel.
fn rfc3339(at: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(at).to_rfc3339()
}

/// `911` read as "nine one one", so a synthesiser does not say "nine
/// hundred and eleven"; a number with anything but digits in it is left
/// as it is, since it is not a number being read out.
fn spell_digits(value: &str) -> String {
    const WORDS: [&str; 10] = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
    ];
    if value.is_empty() || value.chars().any(|c| !c.is_ascii_digit() && c != '+') {
        return value.to_string();
    }
    value
        .chars()
        .filter_map(|c| match c {
            '+' => Some("plus".to_string()),
            c => c.to_digit(10).map(|d| WORDS[d as usize].to_string()),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// How one attempt at one contact ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationStatus {
    /// It reached the contact.
    Delivered,
    /// It failed and will be tried again (`next_retry_at`).
    Retrying,
    /// It failed and will not be tried again.
    Failed,
}

impl fmt::Display for NotificationStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            NotificationStatus::Delivered => "delivered",
            NotificationStatus::Retrying => "retrying",
            NotificationStatus::Failed => "failed",
        })
    }
}

impl FromStr for NotificationStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "delivered" => Ok(NotificationStatus::Delivered),
            "retrying" => Ok(NotificationStatus::Retrying),
            "failed" => Ok(NotificationStatus::Failed),
            other => Err(format!("unknown notification status {other:?}")),
        }
    }
}

/// One attempt to tell one contact about one emergency call (§8.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmergencyNotification {
    /// Database id; 0 before insertion.
    pub id: i64,
    pub tenant_id: TenantId,
    /// The emergency call's record, when it had been written by the time
    /// the notice went out.
    pub call_record_id: Option<i64>,
    /// The call's id, which always identifies it.
    pub call_id: String,
    pub contact: NotifyContact,
    /// 1 for the first attempt.
    pub attempt: i32,
    pub status: NotificationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_at: Option<SystemTime>,
    /// When the next attempt is due, on a `retrying` row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_retry_at: Option<SystemTime>,
    /// The notice itself, so a retry needs nothing but this row.
    pub notice: EmergencyNotice,
    pub created_at: SystemTime,
}

impl EmergencyNotification {
    /// The first attempt at a contact, before it is made.
    pub fn first(
        tenant_id: TenantId,
        call_record_id: Option<i64>,
        call_id: String,
        contact: NotifyContact,
        notice: EmergencyNotice,
    ) -> Self {
        Self {
            id: 0,
            tenant_id,
            call_record_id,
            call_id,
            contact,
            attempt: 1,
            status: NotificationStatus::Retrying,
            error: None,
            delivered_at: None,
            next_retry_at: None,
            notice,
            created_at: SystemTime::now(),
        }
    }

    /// The attempt after this one.
    pub fn next_attempt(&self) -> Self {
        Self {
            id: 0,
            attempt: self.attempt + 1,
            status: NotificationStatus::Retrying,
            error: None,
            delivered_at: None,
            next_retry_at: None,
            created_at: SystemTime::now(),
            ..self.clone()
        }
    }

    pub fn is_test(&self) -> bool {
        self.notice.is_test()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(kind: EmergencyKind) -> EmergencyNotice {
        EmergencyNotice {
            tenant_id: TenantId::nil(),
            call_id: "call-1".to_string(),
            kind,
            dialed: "911".to_string(),
            caller: "sip:alice@pbx.example.com".to_string(),
            caller_name: Some("alice".to_string()),
            callback_number: Some("+15185550100".to_string()),
            location_summary: Some("HQ: 1 State St, Albany, NY 12207".to_string()),
            location_source: Some("user_location".to_string()),
            client_address: Some("10.0.0.5".to_string()),
            trunk_name: Some("carrier-a".to_string()),
            placed_at: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_789_000_000),
        }
    }

    #[test]
    fn contacts_are_read_by_shape() {
        let cases = [
            ("security@example.com", ContactKind::Email),
            ("mailto:ops@example.com", ContactKind::Email),
            ("sip:frontdesk@pbx.example.com", ContactKind::Call),
            ("+1 518 555 0100", ContactKind::Sms),
            ("2001", ContactKind::Call),
            ("f81d4fae-7dec-11d0-a765-00a0c91e6bf6", ContactKind::Webhook),
            ("sms:+15185550100", ContactKind::Sms),
        ];
        for (value, kind) in cases {
            let contact = NotifyContact::parse(value).unwrap_or_else(|| panic!("{value}"));
            assert_eq!(contact.kind, kind, "{value}");
            assert!(
                contact.problems().is_empty(),
                "{value}: {:?}",
                contact.problems()
            );
        }
        assert_eq!(NotifyContact::parse("  "), None);
        assert_eq!(NotifyContact::parse("who knows"), None);
        assert_eq!(
            NotifyContact::parse("mailto:ops@example.com")
                .unwrap()
                .target,
            "ops@example.com"
        );
    }

    #[test]
    fn a_contact_that_cannot_be_delivered_to_says_so() {
        assert!(!NotifyContact::new(ContactKind::Email, "nobody")
            .problems()
            .is_empty());
        assert!(!NotifyContact::new(ContactKind::Sms, "12")
            .problems()
            .is_empty());
        assert!(!NotifyContact::new(ContactKind::Call, "down the hall")
            .problems()
            .is_empty());
        assert!(!NotifyContact::new(ContactKind::Webhook, "an-endpoint")
            .problems()
            .is_empty());
        assert!(
            NotifyContact::new(ContactKind::Call, "sip:desk@example.com")
                .problems()
                .is_empty()
        );
    }

    /// Kari's Law asks for who, where and when; a test says it is a test
    /// in the subject and the first line (§8.4).
    #[test]
    fn the_notice_says_who_where_and_when() {
        let live = notice(EmergencyKind::Live);
        assert!(live.subject().contains("alice"), "{}", live.subject());
        assert!(live.subject().contains("911"));
        assert!(!live.subject().contains("TEST"));
        let text = live.text();
        for expected in [
            "911",
            "alice",
            "+15185550100",
            "1 State St",
            "user_location",
            "10.0.0.5",
        ] {
            assert!(text.contains(expected), "{expected} missing from {text}");
        }
        assert!(live.sms_text().len() < 320, "{}", live.sms_text());
        assert!(live.spoken().contains("nine one one"), "{}", live.spoken());

        let test = notice(EmergencyKind::Test);
        assert!(test.subject().starts_with("[TEST] "), "{}", test.subject());
        assert!(test.lines()[0].starts_with("[TEST] "));
        assert!(test.text().contains("No emergency services were contacted"));
        assert!(test.spoken().starts_with("This is a test"));
    }
}
