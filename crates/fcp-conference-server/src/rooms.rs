//! A live room is a tenant's: two tenants may both number a meeting
//! 8801, and on one node they are two rooms with two mixes. Every room
//! this node holds is found by a [`RoomKey`] — tenant and number — and
//! [`RoomBridge`] is the only way to the mixer's rooms, so a lookup by
//! number alone does not compile (`docs/DEEP_TESTING.md` D6, where two
//! tenants' rooms were found to share one mix).

use dashmap::DashMap;
use fcp_core::TenantId;
use forge_conference::{AudioFormat, ConferenceBridge, ConferenceRoom};
use std::fmt;
use std::sync::{Arc, Mutex};

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
///
/// Making a room and removing one happen one at a time: the mixer's own
/// get-or-create checks and then inserts, so two callers making one room
/// at once could each make it, and the second replaced the first's — the
/// first caller then mixed alone. And a room emptied by its last caller
/// is removed only while nobody is on their way in ([`Joining`]): the
/// sweep that removed it used to check that it was empty, read the
/// meeting, and remove it, and a caller who joined during that read was
/// left in a room nobody else could reach (`docs/DEEP_TESTING.md` D6).
pub struct RoomBridge {
    inner: Arc<ConferenceBridge>,
    lifecycle: Mutex<()>,
    joins: Arc<DashMap<RoomKey, usize>>,
}

/// A caller on the way into a room, from the moment the room is found to
/// the end of the join: while one is held the room is not removed for
/// being empty.
pub struct Joining {
    joins: Arc<DashMap<RoomKey, usize>>,
    key: RoomKey,
}

impl Drop for Joining {
    fn drop(&mut self) {
        if let Some(mut n) = self.joins.get_mut(&self.key) {
            *n = n.saturating_sub(1);
        }
        self.joins.remove_if(&self.key, |_, n| *n == 0);
    }
}

impl RoomBridge {
    pub fn new(inner: Arc<ConferenceBridge>) -> Self {
        Self {
            inner,
            lifecycle: Mutex::new(()),
            joins: Arc::new(DashMap::new()),
        }
    }

    fn one_at_a_time(&self) -> std::sync::MutexGuard<'_, ()> {
        self.lifecycle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn get_room(&self, key: &RoomKey) -> forge_conference::Result<Arc<ConferenceRoom>> {
        self.inner.get_room(&key.bridge_id())
    }

    /// The room a caller is joining, made if it is not there, whether
    /// this call made it, and the [`Joining`] that keeps it until the
    /// caller is in.
    pub fn join(
        &self,
        key: &RoomKey,
        format: Option<AudioFormat>,
    ) -> forge_conference::Result<(Arc<ConferenceRoom>, bool, Joining)> {
        let _one = self.one_at_a_time();
        let id = key.bridge_id();
        let created = self.inner.get_room(&id).is_err();
        let room = self.inner.get_or_create_room(id, format)?;
        *self.joins.entry(key.clone()).or_insert(0) += 1;
        Ok((
            room,
            created,
            Joining {
                joins: Arc::clone(&self.joins),
                key: key.clone(),
            },
        ))
    }

    pub fn delete_room(&self, key: &RoomKey) -> forge_conference::Result<()> {
        let _one = self.one_at_a_time();
        self.inner.delete_room(&key.bridge_id())
    }

    /// Remove the room if it is empty and nobody is joining it; whether
    /// it was removed.
    pub fn delete_room_if_idle(&self, key: &RoomKey) -> bool {
        let _one = self.one_at_a_time();
        if self.joins.get(key).is_some_and(|n| *n > 0) {
            return false;
        }
        let id = key.bridge_id();
        match self.inner.get_room(&id) {
            Ok(room) if room.participant_count() == 0 => self.inner.delete_room(&id).is_ok(),
            _ => false,
        }
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
        let (room_a, _, _joining_a) = bridge.join(&a, None).unwrap();
        let (room_b, _, _joining_b) = bridge.join(&b, None).unwrap();
        assert!(!Arc::ptr_eq(&room_a, &room_b));
        let listed = bridge.list_rooms();
        assert_eq!(listed.len(), 2);
        assert!(listed.contains(&a) && listed.contains(&b));
    }

    fn bridge() -> RoomBridge {
        RoomBridge::new(Arc::new(
            ConferenceBridge::new(
                AudioFormat::pcm_mono(),
                160,
                forge_mixer::MixerOptions::default(),
            )
            .unwrap(),
        ))
    }

    #[test]
    fn callers_making_one_room_at_once_get_the_same_room() {
        let bridge = Arc::new(bridge());
        let key = RoomKey::new(TenantId::new(), "8802");
        let rooms: Vec<Arc<ConferenceRoom>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| scope.spawn(|| bridge.join(&key, None).unwrap().0))
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert!(rooms.iter().all(|r| Arc::ptr_eq(r, &rooms[0])));
    }

    #[test]
    fn an_empty_room_is_kept_while_a_caller_is_joining_it() {
        let bridge = bridge();
        let key = RoomKey::new(TenantId::new(), "8802");
        let (_, created, joining) = bridge.join(&key, None).unwrap();
        assert!(created);
        assert!(
            !bridge.delete_room_if_idle(&key),
            "a caller is on the way in"
        );
        assert!(bridge.get_room(&key).is_ok());
        drop(joining);
        assert!(
            bridge.delete_room_if_idle(&key),
            "empty, and nobody joining"
        );
        assert!(bridge.get_room(&key).is_err());
        let (_, created, _joining) = bridge.join(&key, None).unwrap();
        assert!(created, "made again for the next caller");
    }
}
