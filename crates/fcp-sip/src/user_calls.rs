//! A user's calls, counted for call waiting (`docs/ENTERPRISE_CALLING.md`
//! §3.4). Every call counts against the users that are its parties while
//! it is up — the caller, the callee the plan rang — and the call
//! manager's parties hook keeps the count: a call established, a party
//! transferred away, a call ended. The count is this node's alone, or the
//! cluster's when a [`UserCallCount`] over Redis is given
//! (`fcp_cluster::CallLeases`, subject `User`).
//!
//! Each party taken or let go is announced as `presence.user_call`, from
//! which the admin shows the person on a call (`docs/PRESENCE.md`).

use async_trait::async_trait;
use fcp_b2bua_core::{CallId, CallManager};
use fcp_core::{TenantId, UserId};
use std::collections::HashSet;
use std::sync::{Arc, Weak};
use std::time::Duration;
use tracing::{debug, warn};

/// Where a user's calls are counted.
#[async_trait]
pub trait UserCallCount: Send + Sync {
    /// The call counts against the user.
    async fn take(&self, tenant: &TenantId, user: &UserId, call_id: &str) -> anyhow::Result<()>;
    /// It no longer does.
    async fn release(&self, tenant: &TenantId, user: &UserId, call_id: &str) -> anyhow::Result<()>;
    /// How many calls count against the user.
    async fn count(&self, tenant: &TenantId, user: &UserId) -> anyhow::Result<u64>;
    /// Keep these alive (a lease that lapses would free a user on a call).
    async fn refresh(&self, held: &[(TenantId, UserId, String)]) -> anyhow::Result<()> {
        let _ = held;
        Ok(())
    }
}

/// This node's count, in memory.
#[derive(Default)]
pub struct LocalUserCalls {
    calls: dashmap::DashMap<UserId, HashSet<String>>,
}

#[async_trait]
impl UserCallCount for LocalUserCalls {
    async fn take(&self, _tenant: &TenantId, user: &UserId, call_id: &str) -> anyhow::Result<()> {
        self.calls
            .entry(*user)
            .or_default()
            .insert(call_id.to_string());
        Ok(())
    }

    async fn release(
        &self,
        _tenant: &TenantId,
        user: &UserId,
        call_id: &str,
    ) -> anyhow::Result<()> {
        if let Some(mut set) = self.calls.get_mut(user) {
            set.remove(call_id);
        }
        self.calls.remove_if(user, |_, set| set.is_empty());
        Ok(())
    }

    async fn count(&self, _tenant: &TenantId, user: &UserId) -> anyhow::Result<u64> {
        Ok(self.calls.get(user).map(|s| s.len() as u64).unwrap_or(0))
    }
}

/// Keeps a [`UserCallCount`] in step with the calls on this node.
pub struct UserCalls {
    count: Arc<dyn UserCallCount>,
    call_manager: Weak<CallManager>,
    /// What each call counts against now, as last told.
    held: dashmap::DashMap<CallId, (TenantId, Vec<UserId>)>,
}

impl UserCalls {
    pub fn new(count: Arc<dyn UserCallCount>, call_manager: &Arc<CallManager>) -> Arc<Self> {
        Arc::new(Self {
            count,
            call_manager: Arc::downgrade(call_manager),
            held: dashmap::DashMap::new(),
        })
    }

    /// Install on the call manager, and refresh what this node holds every
    /// `refresh_every`.
    pub fn install(self: &Arc<Self>, call_manager: &CallManager, refresh_every: Duration) {
        let me = Arc::clone(self);
        call_manager.set_on_parties_changed(Arc::new(move |call_id, users| {
            let me = Arc::clone(&me);
            tokio::spawn(async move { me.changed(call_id, users).await });
        }));
        let me = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(refresh_every);
            tick.tick().await;
            loop {
                tick.tick().await;
                let Some(me) = me.upgrade() else { return };
                me.refresh().await;
            }
        });
    }

    /// How many calls count against the user, the cluster's when counted
    /// there. An unreadable count is none: a call is not refused because
    /// Redis is away.
    pub async fn count(&self, tenant: &TenantId, user: &UserId) -> u64 {
        match self.count.count(tenant, user).await {
            Ok(n) => n,
            Err(e) => {
                warn!(user_id = %user, error = %e, "A user's calls could not be counted");
                0
            }
        }
    }

    async fn changed(&self, call_id: CallId, users: Vec<UserId>) {
        let tenant = self
            .call_manager
            .upgrade()
            .and_then(|cm| cm.get_call(&call_id))
            .map(|c| c.tenant_id)
            .or_else(|| self.held.get(&call_id).map(|h| h.0));
        let Some(tenant) = tenant else { return };
        let before = self
            .held
            .get(&call_id)
            .map(|h| h.1.clone())
            .unwrap_or_default();
        let key = call_id.to_string();
        for user in before.iter().filter(|u| !users.contains(u)) {
            if let Err(e) = self.count.release(&tenant, user, &key).await {
                warn!(call_id = %call_id, user_id = %user, error = %e, "A user's call not released");
            }
        }
        for user in users.iter().filter(|u| !before.contains(u)) {
            if let Err(e) = self.count.take(&tenant, user, &key).await {
                warn!(call_id = %call_id, user_id = %user, error = %e, "A user's call not counted");
            }
        }
        if let Some(cm) = self.call_manager.upgrade() {
            let gone = before.iter().filter(|u| !users.contains(u));
            let came = users.iter().filter(|u| !before.contains(u));
            for (user, on_call) in gone.map(|u| (u, false)).chain(came.map(|u| (u, true))) {
                cm.publish(fcp_events::PlatformEvent::Presence(
                    fcp_events::PresenceEvent::UserCall {
                        user_id: user.to_string(),
                        call_id: key.clone(),
                        on_call,
                        tenant_id: tenant,
                        timestamp: std::time::SystemTime::now(),
                    },
                ));
            }
        }
        debug!(call_id = %call_id, users = users.len(), "A call's parties counted");
        if users.is_empty() {
            self.held.remove(&call_id);
        } else {
            self.held.insert(call_id, (tenant, users));
        }
    }

    /// Push out the leases of this node's calls; forget calls this node no
    /// longer has (one discarded without ending).
    async fn refresh(&self) {
        let live = self.call_manager.upgrade();
        let gone: Vec<CallId> = self
            .held
            .iter()
            .filter(|e| {
                live.as_ref()
                    .is_none_or(|cm| cm.get_call(e.key()).is_none())
            })
            .map(|e| *e.key())
            .collect();
        for call_id in gone {
            self.changed(call_id, Vec::new()).await;
        }
        let held: Vec<(TenantId, UserId, String)> = self
            .held
            .iter()
            .flat_map(|e| {
                let (tenant, users) = e.value().clone();
                let call = e.key().to_string();
                users.into_iter().map(move |u| (tenant, u, call.clone()))
            })
            .collect();
        if let Err(e) = self.count.refresh(&held).await {
            warn!(error = %e, "Users' calls not refreshed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_local_count_takes_and_releases() {
        let local = LocalUserCalls::default();
        let (t, u) = (TenantId::new(), UserId::new());
        local.take(&t, &u, "c1").await.unwrap();
        local.take(&t, &u, "c2").await.unwrap();
        local.take(&t, &u, "c2").await.unwrap();
        assert_eq!(local.count(&t, &u).await.unwrap(), 2);
        local.release(&t, &u, "c1").await.unwrap();
        local.release(&t, &u, "c9").await.unwrap();
        assert_eq!(local.count(&t, &u).await.unwrap(), 1);
        local.release(&t, &u, "c2").await.unwrap();
        assert_eq!(local.count(&t, &u).await.unwrap(), 0);
    }
}
