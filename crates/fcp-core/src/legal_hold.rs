//! Legal holds (`docs/COMPLIANCE_FOLLOWUPS.md` §5): what a tenant must not
//! delete while a matter is open.
//!
//! A hold is a filter every deletion consults, not a flag on data. Its scope
//! is a time window, the subjects and calls it concerns, and the classes of
//! data it covers; an empty part of the scope covers everything. The
//! Postgres backend compiles the same rules to SQL: retention leaves held
//! rows out of its deletes, and a trigger on every governed table refuses a
//! delete that would remove a held row, whatever path it came by. A hold is
//! never deleted, only released.

use crate::storage::DynStorage;
use crate::{LegalHoldId, TenantId};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// The classes of data a hold can cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataClass {
    Cdr,
    CallRecording,
    ConferenceRecording,
    Voicemail,
    SipTrace,
    FlowTrace,
    AuditEvent,
}

impl DataClass {
    pub const ALL: [DataClass; 7] = [
        DataClass::Cdr,
        DataClass::CallRecording,
        DataClass::ConferenceRecording,
        DataClass::Voicemail,
        DataClass::SipTrace,
        DataClass::FlowTrace,
        DataClass::AuditEvent,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            DataClass::Cdr => "cdr",
            DataClass::CallRecording => "call_recording",
            DataClass::ConferenceRecording => "conference_recording",
            DataClass::Voicemail => "voicemail",
            DataClass::SipTrace => "sip_trace",
            DataClass::FlowTrace => "flow_trace",
            DataClass::AuditEvent => "audit_event",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == s)
    }
}

/// Who a hold concerns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SubjectIdentifier {
    /// A platform user of the tenant: their extension, numbers, addresses
    /// on the tenant's domains, email and mailbox, as they are when the
    /// hold is placed.
    User {
        username: String,
    },
    /// A telephone number, the tenant's or anyone's.
    Number {
        number: String,
    },
    /// A SIP address of record.
    Aor {
        aor: String,
    },
    Email {
        address: String,
    },
}

/// What a hold covers. An empty part covers everything.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldScope {
    /// Data from this instant (a record's start, a recording's start, a
    /// message's receipt, a trace's capture, an audit event).
    #[serde(default)]
    pub from: Option<SystemTime>,
    /// Data before this instant.
    #[serde(default)]
    pub to: Option<SystemTime>,
    /// Data about these subjects.
    #[serde(default)]
    pub subjects: Vec<SubjectIdentifier>,
    /// Data of these calls (`call-…`), besides the subjects'.
    #[serde(default)]
    pub call_ids: Vec<String>,
    #[serde(default)]
    pub classes: Vec<DataClass>,
}

/// Who released a hold, when and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldRelease {
    pub by: String,
    pub at: SystemTime,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegalHold {
    pub id: LegalHoldId,
    pub tenant_id: TenantId,
    /// The case or request the hold serves.
    pub matter: String,
    pub reason: Option<String>,
    pub scope: HoldScope,
    /// The subjects' canonical keys (`crate::subject::keys`), expanded when
    /// the hold was placed; what the store matches data against.
    pub subject_keys: Vec<String>,
    pub created_by: String,
    pub created_at: SystemTime,
    pub released: Option<HoldRelease>,
}

/// An item a hold may cover, for the checks made outside the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataRef {
    pub class: DataClass,
    /// The instant the item is dated by.
    pub at: SystemTime,
    /// The item's subject keys; `None` when they cannot be known (a
    /// recording whose call has no stored record yet), which a hold with
    /// subjects treats as covered.
    pub keys: Option<Vec<String>>,
    pub call_id: Option<String>,
}

impl LegalHold {
    pub fn is_active(&self) -> bool {
        self.released.is_none()
    }

    /// Whether this hold, while active, covers `item`. The Postgres
    /// function `fcp_hold_matches` applies the same rules.
    pub fn covers(&self, item: &DataRef) -> bool {
        let scope = &self.scope;
        if !self.is_active() {
            return false;
        }
        if !scope.classes.is_empty() && !scope.classes.contains(&item.class) {
            return false;
        }
        if scope.from.is_some_and(|from| item.at < from) || scope.to.is_some_and(|to| item.at >= to)
        {
            return false;
        }
        if self.subject_keys.is_empty() && scope.call_ids.is_empty() {
            return true;
        }
        let by_subject = !self.subject_keys.is_empty()
            && match &item.keys {
                Some(keys) => keys.iter().any(|k| self.subject_keys.contains(k)),
                None => true,
            };
        let by_call = item
            .call_id
            .as_ref()
            .is_some_and(|c| scope.call_ids.contains(c));
        by_subject || by_call
    }
}

/// A delete refused because a hold covers what it would remove.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("held by legal hold {hold_id}")]
pub struct LegalHoldViolation {
    pub hold_id: String,
}

/// A subject as the tenant knows them: every spelling that identifies them,
/// and the platform objects that are theirs (`docs/COMPLIANCE_FOLLOWUPS.md`
/// §6.1). Taken once, when a hold is placed or a request starts, and kept
/// with it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubjectExpansion {
    /// Usernames, extensions, their addresses on the tenant's SIP domains,
    /// numbers and email addresses.
    pub identifiers: Vec<String>,
    /// The platform users among the subjects.
    #[serde(default)]
    pub user_ids: Vec<crate::UserId>,
    /// Their mailboxes.
    #[serde(default)]
    pub mailbox_ids: Vec<String>,
}

impl SubjectExpansion {
    /// The canonical keys a hold matches data against, `mailbox:<id>` last.
    pub fn keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = Vec::new();
        for identifier in &self.identifiers {
            for k in crate::subject::keys(identifier) {
                if !keys.contains(&k) {
                    keys.push(k);
                }
            }
        }
        for mailbox in &self.mailbox_ids {
            let k = format!("mailbox:{mailbox}");
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
        keys
    }

    /// The matcher an export or an erasure acts by.
    pub fn matcher(&self) -> crate::subject::SubjectMatcher {
        crate::subject::SubjectMatcher::new(&self.identifiers)
    }
}

/// `subjects` in `tenant_id`, a user expanded to everything that identifies
/// them: username, extension, numbers (their own and their line bindings'),
/// addresses on the tenant's SIP domains, email, and their mailbox. A user
/// the tenant does not have is an error.
pub async fn expand(
    storage: &DynStorage,
    tenant_id: &TenantId,
    subjects: &[SubjectIdentifier],
) -> anyhow::Result<SubjectExpansion> {
    use crate::storage::{
        CredentialStore, LineBindingStore, PhoneNumberStore, TenantStore, VoicemailStore,
    };
    let mut out = SubjectExpansion::default();
    let add = |identifiers: &mut Vec<String>, identifier: &str| {
        let identifier = identifier.trim();
        if !identifier.is_empty() && !identifiers.iter().any(|i| i == identifier) {
            identifiers.push(identifier.to_string());
        }
    };
    for subject in subjects {
        match subject {
            SubjectIdentifier::Number { number } => add(&mut out.identifiers, number),
            SubjectIdentifier::Aor { aor } => add(&mut out.identifiers, aor),
            SubjectIdentifier::Email { address } => add(&mut out.identifiers, address),
            SubjectIdentifier::User { username } => {
                let user = storage
                    .get_user_in_tenant(username, tenant_id)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("no user {username} in the tenant"))?;
                let domains = storage
                    .get_tenant(tenant_id)
                    .await?
                    .map(|t| t.sip_domains)
                    .unwrap_or_default();
                let mut names = vec![user.username.clone()];
                names.extend(user.extension.clone());
                for name in &names {
                    add(&mut out.identifiers, name);
                    for domain in &domains {
                        add(&mut out.identifiers, &format!("{name}@{domain}"));
                    }
                }
                for identifier in [&user.phone_number, &user.mobile_number, &user.email]
                    .into_iter()
                    .flatten()
                {
                    add(&mut out.identifiers, identifier);
                }
                for binding in storage.list_line_bindings_by_user(&user.id).await? {
                    if let Some(number) = storage.get_phone_number(&binding.phone_number_id).await?
                    {
                        add(&mut out.identifiers, &number.e164);
                    }
                }
                if let Some(mailbox) = storage.get_mailbox_by_user(&user.id).await? {
                    let id = mailbox.id.to_string();
                    if !out.mailbox_ids.contains(&id) {
                        out.mailbox_ids.push(id);
                    }
                }
                if !out.user_ids.contains(&user.id) {
                    out.user_ids.push(user.id);
                }
            }
        }
    }
    Ok(out)
}

/// The canonical keys of `subjects` in `tenant_id` ([`expand`], then
/// [`SubjectExpansion::keys`]).
pub async fn expand_subjects(
    storage: &DynStorage,
    tenant_id: &TenantId,
    subjects: &[SubjectIdentifier],
) -> anyhow::Result<Vec<String>> {
    Ok(expand(storage, tenant_id, subjects).await?.keys())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    fn hold(scope: HoldScope, keys: &[&str]) -> LegalHold {
        LegalHold {
            id: LegalHoldId::new(),
            tenant_id: TenantId::nil(),
            matter: "m".into(),
            reason: None,
            scope,
            subject_keys: keys.iter().map(|k| k.to_string()).collect(),
            created_by: "a".into(),
            created_at: at(0),
            released: None,
        }
    }

    fn item(class: DataClass, secs: u64, keys: Option<&[&str]>, call: Option<&str>) -> DataRef {
        DataRef {
            class,
            at: at(secs),
            keys: keys.map(|k| k.iter().map(|s| s.to_string()).collect()),
            call_id: call.map(str::to_string),
        }
    }

    #[test]
    fn an_empty_scope_holds_the_whole_tenant() {
        let h = hold(HoldScope::default(), &[]);
        for class in DataClass::ALL {
            assert!(h.covers(&item(class, 5, Some(&[]), None)));
        }
    }

    #[test]
    fn window_classes_subjects_and_calls_narrow_it() {
        let h = hold(
            HoldScope {
                from: Some(at(10)),
                to: Some(at(20)),
                call_ids: vec!["call-x".into()],
                classes: vec![DataClass::Cdr, DataClass::CallRecording],
                ..Default::default()
            },
            &["num:1001"],
        );
        assert!(h.covers(&item(DataClass::Cdr, 10, Some(&["num:1001"]), None)));
        assert!(
            h.covers(&item(
                DataClass::Cdr,
                15,
                Some(&["user:bob"]),
                Some("call-x")
            )),
            "by call"
        );
        assert!(
            !h.covers(&item(DataClass::Cdr, 20, Some(&["num:1001"]), None)),
            "window is [from, to)"
        );
        assert!(!h.covers(&item(DataClass::Cdr, 9, Some(&["num:1001"]), None)));
        assert!(
            !h.covers(&item(DataClass::Voicemail, 15, Some(&["num:1001"]), None)),
            "class"
        );
        assert!(
            !h.covers(&item(DataClass::Cdr, 15, Some(&["num:2002"]), None)),
            "subject"
        );
        assert!(
            h.covers(&item(DataClass::CallRecording, 15, None, None)),
            "unknown keys are held"
        );
        let mut released = h.clone();
        released.released = Some(HoldRelease {
            by: "a".into(),
            at: at(30),
            reason: "closed".into(),
        });
        assert!(!released.covers(&item(DataClass::Cdr, 15, Some(&["num:1001"]), None)));
    }

    #[test]
    fn classes_and_subjects_read_as_their_names() {
        for class in DataClass::ALL {
            assert_eq!(DataClass::parse(class.as_str()), Some(class));
        }
        let s: SubjectIdentifier =
            serde_json::from_value(serde_json::json!({"type": "number", "number": "+14155550100"}))
                .unwrap();
        assert_eq!(
            s,
            SubjectIdentifier::Number {
                number: "+14155550100".into()
            }
        );
    }
}
