//! A live room is a tenant's: two tenants may both number a meeting
//! 8801, and on one node they are two rooms with two mixes. Every room
//! this node holds is found by a [`RoomKey`] — tenant and number — and
//! [`RoomBridge`] is the only way to the mixer's rooms, so a lookup by
//! number alone does not compile (`docs/DEEP_TESTING.md` D6, where two
//! tenants' rooms were found to share one mix).

use fcp_core::TenantId;
use forge_conference::{AudioFormat, ConferenceBridge, ConferenceRoom};
use std::fmt;
use std::sync::Arc;

/// A room on this node: the tenant's meeting number.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RoomKey {
    pub tenant: TenantId,
    pub room: String,
}

impl RoomKey {
    pub fn new(tenant: TenantId, room: impl Into<String>) -> Self {
        Self {
            tenant,
            room: room.into(),
        }
    }

    /// The mixer's id for the room: `<tenant>:<room>`. A room number
    /// never holds a `:` (`validate_room_number`), so the id parses back.
    pub fn bridge_id(&self) -> String {
        format!("{}:{}", self.tenant, self.room)
    }

    /// The key a mixer id was made from.
    pub fn parse(id: &str) -> Option<Self> {
        let (tenant, room) = id.split_once(':')?;
        Some(Self::new(tenant.parse().ok()?, room))
    }
}

impl fmt::Display for RoomKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} in {}", self.room, self.tenant)
    }
}

/// The mixer's rooms, by [`RoomKey`].
pub struct RoomBridge {
    inner: Arc<ConferenceBridge>,
}

impl RoomBridge {
    pub fn new(inner: Arc<ConferenceBridge>) -> Self {
        Self { inner }
    }

    pub fn get_room(&self, key: &RoomKey) -> forge_conference::Result<Arc<ConferenceRoom>> {
        self.inner.get_room(&key.bridge_id())
    }

    pub fn get_or_create_room(
        &self,
        key: &RoomKey,
        format: Option<AudioFormat>,
    ) -> forge_conference::Result<Arc<ConferenceRoom>> {
        self.inner.get_or_create_room(key.bridge_id(), format)
    }

    pub fn delete_room(&self, key: &RoomKey) -> forge_conference::Result<()> {
        self.inner.delete_room(&key.bridge_id())
    }

    /// Every room the mixer holds.
    pub fn list_rooms(&self) -> Vec<RoomKey> {
        self.inner
            .list_rooms()
            .iter()
            .filter_map(|id| RoomKey::parse(id))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_round_trips_through_the_mixers_id() {
        let key = RoomKey::new(TenantId::new(), "8801");
        assert_eq!(RoomKey::parse(&key.bridge_id()), Some(key.clone()));
        assert_ne!(
            key.bridge_id(),
            RoomKey::new(TenantId::new(), "8801").bridge_id(),
            "two tenants' 8801 are two rooms"
        );
        assert_eq!(RoomKey::parse("8801"), None);
    }

    #[test]
    fn two_tenants_with_one_number_have_two_rooms() {
        let bridge = RoomBridge::new(Arc::new(
            ConferenceBridge::new(
                AudioFormat::pcm_mono(),
                160,
                forge_mixer::MixerOptions::default(),
            )
            .unwrap(),
        ));
        let a = RoomKey::new(TenantId::new(), "8801");
        let b = RoomKey::new(TenantId::new(), "8801");
        let room_a = bridge.get_or_create_room(&a, None).unwrap();
        let room_b = bridge.get_or_create_room(&b, None).unwrap();
        assert!(!Arc::ptr_eq(&room_a, &room_b));
        let listed = bridge.list_rooms();
        assert_eq!(listed.len(), 2);
        assert!(listed.contains(&a) && listed.contains(&b));
    }
}
