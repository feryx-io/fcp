//! A user's own contacts (`docs/RECORD_AND_CONTACTS.md` §3): the people
//! they keep in their app, kept by FCP so that every app they sign in to
//! has them, and synced with the tenant's sites both ways.
//!
//! A contact is written where the user is, in the cloud or at a site, and
//! reaches the other side by the journal (down) or the link's shipping
//! (up). Each side takes a contact from the other only when it is newer
//! than its own copy ([`PersonalContact::newer_than`]): the echo of a
//! change comes back no newer and is let go, and of two edits made on
//! both sides before they met, the later one wins. A delete is a
//! tombstone (`deleted_at`), so it crosses like any other change.

use std::time::SystemTime;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{TenantId, UserId};

/// The most contacts one user keeps.
pub const MAX_CONTACTS_PER_USER: u64 = 5_000;

/// The most numbers one contact has.
pub const MAX_NUMBERS: usize = 20;

/// How long a deleted contact's tombstone is kept for the other side to
/// hear of it.
pub const TOMBSTONE_DAYS: u64 = 30;

/// A new contact's id.
pub fn new_contact_id() -> String {
    format!("contact-{}", uuid::Uuid::new_v4())
}

/// One of a contact's numbers: what it is (`mobile`, `work`, …) and what
/// to dial.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ContactNumber {
    #[serde(default)]
    pub label: String,
    pub number: String,
}

/// A contact a user keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalContact {
    /// `contact-` and a UUID, made by whichever side wrote the contact
    /// first ([`new_contact_id`]).
    pub id: String,
    pub tenant_id: TenantId,
    pub owner_user_id: UserId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub company: Option<String>,
    #[serde(default)]
    pub numbers: Vec<ContactNumber>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// The ringtone the owner's apps ring with for a call from one of its
    /// numbers, by the app's id for it (`anvil`, `classic`, `file:…`);
    /// `None` rings with the app's own choice. An app that has no such
    /// ringtone rings with its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ringtone: Option<String>,
    pub created_at: SystemTime,
    /// When it last changed, on whichever side changed it: what decides
    /// which copy wins.
    pub updated_at: SystemTime,
    /// Deleted: kept as a tombstone for [`TOMBSTONE_DAYS`] so the other
    /// side deletes its copy too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<SystemTime>,
}

impl PersonalContact {
    /// Whether this copy should replace `kept`: it is newer, or there is
    /// none.
    pub fn newer_than(&self, kept: Option<&PersonalContact>) -> bool {
        kept.is_none_or(|k| self.updated_at > k.updated_at)
    }

    /// Whether `q` (lower case) is in its name, company, email or one of
    /// its numbers.
    pub fn matches(&self, q: &str) -> bool {
        let has = |s: &str| s.to_lowercase().contains(q);
        has(&self.name)
            || self.company.as_deref().is_some_and(has)
            || self.email.as_deref().is_some_and(has)
            || self.numbers.iter().any(|n| n.number.contains(q))
    }
}

/// Where contacts are kept.
#[async_trait]
pub trait ContactStore: Send + Sync {
    /// The owner's contacts that are not deleted, by name, after `after`
    /// (a contact id, in name order), at most `limit`.
    async fn list_contacts(
        &self,
        owner: &UserId,
        q: Option<&str>,
        after: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<PersonalContact>>;

    /// How many contacts (not deleted) the owner keeps.
    async fn count_contacts(&self, owner: &UserId) -> anyhow::Result<u64>;

    /// A contact by id, deleted ones (tombstones) included.
    async fn get_contact(&self, id: &str) -> anyhow::Result<Option<PersonalContact>>;

    /// Write `contact` when it is newer than the copy kept
    /// ([`PersonalContact::newer_than`]); whether it was written. One call,
    /// so two writers cannot both win.
    async fn put_contact_if_newer(&self, contact: &PersonalContact) -> anyhow::Result<bool>;

    /// Remove a contact's row outright (the cloud's delete reaching a site,
    /// a tombstone swept).
    async fn delete_contact_row(&self, id: &str) -> anyhow::Result<bool>;

    /// Remove tombstones older than `before`; how many went.
    async fn sweep_contact_tombstones(&self, before: SystemTime) -> anyhow::Result<u64>;

    /// Contacts changed after shipping sequence `after`, in order, at most
    /// `limit` (`docs/HYBRID_CONTROL_PLANE.md` §3.3): a site's link ships
    /// them to the cloud.
    async fn contacts_after(
        &self,
        after: i64,
        limit: usize,
    ) -> anyhow::Result<Vec<(i64, PersonalContact)>>;

    /// The tenant's contacts, tombstones included: what a site's snapshot
    /// carries.
    async fn tenant_contacts(&self, tenant: &TenantId) -> anyhow::Result<Vec<PersonalContact>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn contact(at: u64) -> PersonalContact {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(at);
        PersonalContact {
            id: "c".into(),
            tenant_id: TenantId::new(),
            owner_user_id: UserId::new(),
            name: "Jo Smith".into(),
            company: Some("Acme".into()),
            numbers: vec![ContactNumber {
                label: "mobile".into(),
                number: "0412345678".into(),
            }],
            email: None,
            notes: None,
            ringtone: None,
            created_at: t,
            updated_at: t,
            deleted_at: None,
        }
    }

    #[test]
    fn only_a_newer_copy_replaces_the_kept_one() {
        let kept = contact(100);
        assert!(contact(101).newer_than(Some(&kept)));
        assert!(!contact(100).newer_than(Some(&kept)), "an echo is let go");
        assert!(!contact(99).newer_than(Some(&kept)));
        assert!(contact(1).newer_than(None));
    }

    #[test]
    fn a_search_finds_the_name_company_and_number() {
        let c = contact(1);
        assert!(c.matches("jo"));
        assert!(c.matches("acme"));
        assert!(c.matches("4123"));
        assert!(!c.matches("bob"));
    }
}
