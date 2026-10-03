//! Configuration down (`docs/HYBRID_CONTROL_PLANE.md` §3.2, phase 3): the
//! site's Postgres kept a copy of the tenant's slice. The first pull is
//! the cloud's snapshot — every synced object, with the journal's head as
//! the cursor; from then on the link holds `GET /link/v1/changes` open
//! and applies each change in journal order, idempotently, through the
//! same storage crate the cloud writes with. A put writes the object as
//! the cloud has it now (the store seals its secrets under the site's
//! key); a delete removes the row. The cursor is kept in the state
//! directory, so a restart carries on where it stopped; a state
//! directory without one starts from a snapshot again.

use anyhow::{Context, Result};
use fcp_core::billing::BillingStore;
use fcp_core::storage::*;
use fcp_core::CallingStore;
use fcp_core::ClassOfServiceStore;
use fcp_core::ScheduleStore;
use fcp_core::{DynStorage, SiteId, SiteSync, TenantId};
use fcp_flow::FlowStore;
use fcp_storage_postgres::{PostgresConfig, PostgresStorage};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tracing::{debug, info, warn};

/// The site's stores, as the link writes them.
#[derive(Clone)]
pub struct SiteStores {
    pub storage: Arc<DynStorage>,
    pub flows: Arc<dyn FlowStore>,
}

/// Open the site's Postgres — its schema brought up to date, as every
/// binary does on start — with the stores the tenant's slice is written
/// through.
pub async fn open_site(database_url: &str, max_connections: u32) -> Result<SiteStores> {
    let storage = PostgresStorage::new(PostgresConfig {
        connection_string: database_url.to_string(),
        max_connections,
        connection_timeout: Duration::from_secs(30),
        encryption: None,
        read_replica_url: None,
        max_replica_lag: Duration::from_secs(5),
    })
    .await
    .context("opening the site's database")?;
    let storage = Arc::new(storage);
    let prompt_store = Arc::new(fcp_storage_postgres::prompt::PostgresPromptStore::new(
        storage.pool().clone(),
    ));
    let emergency_store = Arc::new(
        fcp_storage_postgres::emergency::PostgresEmergencyStore::new(storage.pool().clone()),
    );
    let recording_store = Arc::new(
        fcp_storage_postgres::recording::PostgresRecordingStore::new(storage.pool().clone()),
    );
    let compliance_store = Arc::new(
        fcp_storage_postgres::compliance::PostgresComplianceStore::new(storage.pool().clone()),
    );
    let dyn_storage = DynStorage::from_all_in_one(storage.clone())
        .with_emergency_store(emergency_store as Arc<dyn fcp_core::e911::EmergencyStore>)
        .with_recording_store(
            recording_store.clone() as Arc<dyn fcp_core::recording::RecordingStore>
        )
        .with_call_recording_store(
            recording_store as Arc<dyn fcp_core::recording::CallRecordingStore>,
        )
        .with_compliance_store(compliance_store as Arc<dyn fcp_core::compliance::ComplianceStore>)
        .with_encryption_store(storage.clone() as Arc<dyn fcp_core::encryption::EncryptionStore>)
        .with_prompt_store(prompt_store as Arc<dyn fcp_core::prompt::PromptStore>)
        .with_config_template_store(
            storage.clone() as Arc<dyn fcp_core::storage::ConfigTemplateStore>
        )
        .with_identity_store(storage.clone() as Arc<dyn fcp_core::identity::IdentityStore>)
        .with_offboarding_store(
            storage.clone() as Arc<dyn fcp_core::offboarding::OffboardingStore>
        );
    Ok(SiteStores {
        storage: Arc::new(dyn_storage),
        flows: storage as Arc<dyn FlowStore>,
    })
}

#[derive(Debug, Deserialize)]
struct SyncItem {
    table: String,
    id: String,
    object: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct Snapshot {
    #[allow(dead_code)]
    protocol: u32,
    tenant_id: String,
    cursor: i64,
    objects: Vec<SyncItem>,
}

#[derive(Debug, Deserialize)]
struct Change {
    seq: i64,
    table: String,
    id: String,
    op: String,
    #[serde(default)]
    object: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct Changes {
    #[allow(dead_code)]
    protocol: u32,
    cursor: i64,
    changes: Vec<Change>,
    more: bool,
}

/// What `/status` and the heartbeat show of the sync, and what the state
/// directory keeps (`sync.json`).
#[derive(Debug, Clone, serde::Serialize, Deserialize, Default)]
struct Kept {
    cursor: i64,
    #[serde(default)]
    applied_at: Option<u64>,
}

pub struct Syncer {
    stores: SiteStores,
    client: reqwest::Client,
    link_url: String,
    secret: String,
    tenant: TenantId,
    /// Which site this is: the trunks it keeps are this site's.
    site: SiteId,
    state_path: PathBuf,
    feed_wait_secs: u64,
    state: Mutex<SiteSync>,
    /// Puts and deletes applied since start, for the status page.
    applied: Mutex<u64>,
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

impl Syncer {
    pub fn new(
        stores: SiteStores,
        client: reqwest::Client,
        link_url: &str,
        secret: String,
        tenant: TenantId,
        site: SiteId,
        state_dir: &Path,
        feed_wait_secs: u64,
    ) -> Result<Self> {
        let state_path = state_dir.join("sync.json");
        let kept: Kept = match std::fs::read_to_string(&state_path) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("reading {}", state_path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Kept::default(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", state_path.display())),
        };
        if kept.cursor > 0 {
            info!(
                cursor = kept.cursor,
                "Configuration synced through the kept cursor; following from there"
            );
        }
        Ok(Self {
            stores,
            client,
            link_url: link_url.trim_end_matches('/').to_string(),
            secret,
            tenant,
            site,
            state_path,
            feed_wait_secs,
            state: Mutex::new(SiteSync {
                cursor: kept.cursor,
                applied_at: kept.applied_at,
                error: None,
            }),
            applied: Mutex::new(0),
        })
    }

    pub fn state(&self) -> SiteSync {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn applied(&self) -> u64 {
        *self.applied.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A snapshot if nothing is kept, then the feed, forever. A failure is
    /// noted for the heartbeat and retried after a pause; the cloud
    /// refusing the secret (a disabled site) is retried more slowly.
    pub async fn run(&self) {
        loop {
            let cursor = self.state().cursor;
            let step = if cursor == 0 {
                self.snapshot().await
            } else {
                self.follow(cursor).await
            };
            match step {
                Ok(()) => {}
                Err(e) => {
                    let text = format!("{e:#}");
                    warn!(error = %text, "Sync failed; trying again");
                    self.state.lock().unwrap_or_else(|e| e.into_inner()).error = Some(text.clone());
                    let pause = if text.contains("403") { 30 } else { 5 };
                    tokio::time::sleep(Duration::from_secs(pause)).await;
                }
            }
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.link_url, path)
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, url: &str, what: &str) -> Result<T> {
        let response = self
            .client
            .get(url)
            .bearer_auth(&self.secret)
            .send()
            .await
            .with_context(|| format!("reaching the cloud for {what}"))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("the cloud refused {what}: {status} {body}");
        }
        response
            .json()
            .await
            .with_context(|| format!("reading {what}"))
    }

    async fn snapshot(&self) -> Result<()> {
        info!("Taking the tenant's snapshot from the cloud");
        let snapshot: Snapshot = self
            .get(&self.url("/link/v1/snapshot"), "the snapshot")
            .await?;
        if snapshot.tenant_id != self.tenant.to_string() {
            anyhow::bail!(
                "the snapshot is tenant {}'s, not this site's {}",
                snapshot.tenant_id,
                self.tenant
            );
        }
        let count = snapshot.objects.len();
        for item in snapshot.objects {
            self.put(&item.table, &item.id, item.object)
                .await
                .with_context(|| format!("applying {} {}", item.table, item.id))?;
        }
        self.advance(snapshot.cursor)?;
        info!(
            objects = count,
            cursor = snapshot.cursor,
            "Snapshot applied"
        );
        Ok(())
    }

    /// One page of the feed from `cursor`, held open by the cloud until
    /// something changes or its wait is up.
    async fn follow(&self, cursor: i64) -> Result<()> {
        let url = self.url(&format!(
            "/link/v1/changes?since={cursor}&wait_secs={}&limit=1000",
            self.feed_wait_secs
        ));
        let page: Changes = self.get(&url, "the change feed").await?;
        for change in &page.changes {
            match change.op.as_str() {
                "put" => match &change.object {
                    Some(object) => self.put(&change.table, &change.id, object.clone()).await,
                    None => self.delete(&change.table, &change.id).await,
                },
                "delete" => self.delete(&change.table, &change.id).await,
                other => anyhow::bail!("an unknown operation {other} on the feed"),
            }
            .with_context(|| {
                format!("applying seq {} {} {}", change.seq, change.table, change.id)
            })?;
        }
        if !page.changes.is_empty() || page.cursor > cursor {
            self.advance(page.cursor.max(cursor))?;
        }
        if !page.changes.is_empty() {
            debug!(
                changes = page.changes.len(),
                cursor = page.cursor,
                more = page.more,
                "Changes applied"
            );
        }
        self.state.lock().unwrap_or_else(|e| e.into_inner()).error = None;
        Ok(())
    }

    fn advance(&self, cursor: i64) -> Result<()> {
        let now = secs(SystemTime::now());
        let kept = Kept {
            cursor,
            applied_at: Some(now),
        };
        std::fs::write(&self.state_path, serde_json::to_string_pretty(&kept)?)
            .with_context(|| format!("writing {}", self.state_path.display()))?;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.cursor = cursor;
        state.applied_at = Some(now);
        state.error = None;
        Ok(())
    }

    fn count(&self) {
        *self.applied.lock().unwrap_or_else(|e| e.into_inner()) += 1;
    }

    /// Write one object as the cloud has it.
    async fn put(&self, table: &str, id: &str, object: serde_json::Value) -> Result<()> {
        let storage = self.stores.storage.as_ref();
        macro_rules! parse {
            () => {
                serde_json::from_value(object).with_context(|| format!("a {table} object"))?
            };
        }
        match table {
            "tenants" => storage.put_tenant(parse!()).await?,
            "users" => storage.put_user(parse!()).await?,
            "user_calling_settings" => storage.put_calling_settings(parse!()).await?,
            "schedules" => storage.put_schedule(parse!()).await?,
            "classes_of_service" => storage.put_class_of_service(parse!()).await?,
            "devices" => storage.put_device(parse!()).await?,
            "extensions" => storage.put_extension(parse!()).await?,
            "line_bindings" => storage.put_line_binding(parse!()).await?,
            "phone_numbers" => storage.put_phone_number(parse!()).await?,
            "routing_rules" => storage.put_routing_rule(parse!()).await?,
            "lcr_routes" => storage.put_lcr_route(parse!()).await?,
            "trunks" => {
                // A trunk runs where its `site_id` says
                // (`docs/HYBRID_CONTROL_PLANE.md` §3.6): the site holds its
                // own — its carriers and its link, which arrives as the
                // cloud's end and is kept as the site's — and none of the
                // cloud's, which it could neither reach nor should register.
                let mut trunk: fcp_core::Trunk = parse!();
                if trunk.site_id != Some(self.site) {
                    storage.delete_trunk(&trunk.id).await?;
                    return Ok(());
                }
                if trunk.link == Some(fcp_core::LinkEnd::Cloud) {
                    trunk.link = Some(fcp_core::LinkEnd::Site);
                }
                storage.put_trunk(trunk).await?
            }
            "queue_configs" => storage.put_queue_config(parse!()).await?,
            "flows" => self.stores.flows.put_flow(parse!()).await?,
            "prompts" => {
                let prompts = storage.prompt_store().context("no prompt store")?;
                prompts.put_prompt(parse!()).await?
            }
            "config_templates" => {
                let templates = storage
                    .config_template_store()
                    .context("no config template store")?;
                templates.put_config_template(parse!()).await?
            }
            "conference_meetings" => storage.put_meeting(parse!()).await?,
            "locations" => storage.put_location(parse!()).await?,
            "emergency_networks" => {
                let emergency = storage.emergency_store().context("no emergency store")?;
                let network: fcp_core::e911::EmergencyNetwork = parse!();
                emergency.put_emergency_network(&network).await?
            }
            "emergency_configs" => {
                let emergency = storage.emergency_store().context("no emergency store")?;
                let config: fcp_core::e911::EmergencyConfig = parse!();
                emergency.put_emergency_config(&config).await?
            }
            "identity_providers" => {
                let store = storage.identity_store().context("no identity store")?;
                let provider: fcp_core::identity::IdentityProvider = parse!();
                store.put_identity_provider(&provider).await?
            }
            "user_identities" => {
                let store = storage.identity_store().context("no identity store")?;
                let identity: fcp_core::identity::UserIdentity = parse!();
                store.put_user_identity(&identity).await?
            }
            "rate_plans" => {
                #[derive(Deserialize)]
                struct PlanWithRates {
                    plan: fcp_core::billing::RatePlan,
                    #[serde(default)]
                    rates: Vec<fcp_core::billing::Rate>,
                }
                let PlanWithRates { plan, rates } = parse!();
                let tenant = plan.tenant_id;
                let plan_id = plan.id;
                if storage.get_rate_plan(&tenant, plan_id).await?.is_some() {
                    storage.update_rate_plan(plan).await?;
                } else {
                    storage.create_rate_plan(plan).await?;
                }
                let wanted: std::collections::HashSet<uuid::Uuid> =
                    rates.iter().map(|r| r.id).collect();
                for stale in storage.list_rates(&tenant, plan_id).await? {
                    if !wanted.contains(&stale.id) {
                        storage.delete_rate(&tenant, plan_id, stale.id).await?;
                    }
                }
                for rate in rates {
                    storage.upsert_rate(&tenant, rate).await?;
                }
            }
            other => {
                warn!(
                    table = other,
                    id, "The cloud sent a table this link does not know; skipped"
                );
                return Ok(());
            }
        }
        self.count();
        Ok(())
    }

    /// Remove one object the cloud no longer has.
    async fn delete(&self, table: &str, id: &str) -> Result<()> {
        let storage = self.stores.storage.as_ref();
        fn string_id<T: serde::de::DeserializeOwned>(id: &str) -> Result<T> {
            serde_json::from_value(serde_json::Value::String(id.to_string()))
                .map_err(anyhow::Error::from)
        }
        macro_rules! id {
            () => {
                match id.parse() {
                    Ok(v) => v,
                    Err(_) => {
                        warn!(table, id, "An id the link cannot parse; skipped");
                        return Ok(());
                    }
                }
            };
        }
        match table {
            "tenants" => storage.delete_tenant(&id!()).await?,
            "users" => {
                if let Some(user) = storage.get_user_by_id(&id!()).await? {
                    // The cloud purged the user (`docs/USERS.md` §4): what
                    // only the site holds goes with them — tokens, hoteling,
                    // queue memberships, registrations, the mailbox off its
                    // line — while the synced rows arrive on their own.
                    match self.stores.storage.offboarding_store() {
                        Some(offboarding) => {
                            let request = fcp_core::offboarding::PurgeRequest {
                                tenant_id: user.tenant_id,
                                user_id: user.id,
                                deprovisioned_before: None,
                                mirror: true,
                                meetings_owner: None,
                                profile_picture_path: None,
                            };
                            fcp_core::compliance::deleting_as(
                                fcp_core::compliance::Deleter::new(
                                    fcp_core::compliance::DeletionCause::Offboarding,
                                    "fcp-link",
                                ),
                                offboarding.purge_user(&request),
                            )
                            .await?;
                        }
                        None => {
                            storage
                                .delete_user_in_tenant(&user.username, &user.tenant_id)
                                .await?
                        }
                    }
                }
            }
            "schedules" => storage.delete_schedule(&id!()).await?,
            "classes_of_service" => storage.delete_class_of_service(&id!()).await?,
            "user_calling_settings" => {
                storage
                    .delete_calling_settings(&self.tenant, &id!())
                    .await?
            }
            "devices" => storage.delete_device(&id!()).await?,
            "extensions" => storage.delete_extension(&id!()).await?,
            "line_bindings" => storage.delete_line_binding(&id!()).await?,
            "phone_numbers" => storage.delete_phone_number(&id!()).await?,
            "routing_rules" => storage.delete_routing_rule(&string_id(id)?).await?,
            "lcr_routes" => storage.delete_lcr_route(&string_id(id)?).await?,
            "trunks" => storage.delete_trunk(&string_id(id)?).await?,
            "queue_configs" => storage.delete_queue_config(&string_id(id)?).await?,
            "flows" => self.stores.flows.delete_flow(id).await?,
            "prompts" => {
                let prompts = storage.prompt_store().context("no prompt store")?;
                prompts.delete_prompt(&id!()).await?
            }
            "config_templates" => {
                let templates = storage
                    .config_template_store()
                    .context("no config template store")?;
                templates.delete_config_template(&id!()).await?
            }
            "conference_meetings" => storage.delete_meeting(&id!()).await?,
            "locations" => storage.delete_location(&id!()).await?,
            "emergency_networks" => {
                let emergency = storage.emergency_store().context("no emergency store")?;
                emergency.delete_emergency_network(&id!()).await?
            }
            "emergency_configs" => {
                // A tenant's emergency configuration has no delete; the
                // cloud replaces it with the next put.
                debug!(id, "An emergency configuration's delete; nothing to remove");
            }
            "rate_plans" => {
                let plan_id: uuid::Uuid = id!();
                storage.delete_rate_plan(&self.tenant, plan_id).await?
            }
            "identity_providers" => {
                let store = storage.identity_store().context("no identity store")?;
                store.delete_identity_provider(&id!()).await?;
            }
            "user_identities" => {
                let store = storage.identity_store().context("no identity store")?;
                match fcp_core::identity::UserIdentity::parse_row_id(id) {
                    Some((provider, subject)) => {
                        store.delete_user_identity(&provider, &subject).await?;
                    }
                    None => warn!(table, id, "An id the link cannot parse; skipped"),
                }
            }
            other => {
                warn!(
                    table = other,
                    id, "The cloud sent a table this link does not know; skipped"
                );
                return Ok(());
            }
        }
        self.count();
        Ok(())
    }
}
