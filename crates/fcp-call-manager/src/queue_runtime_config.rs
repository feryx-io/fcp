use anyhow::{bail, Context, Result};
use fcp_core::storage::{QueueConfigStore, TenantStore};
use fcp_core::{DynStorage, QueueConfigRecord, TenantId};
use fcp_queue::queue_manager::TenantQueueDefinition;
use fcp_queue::{DistributionStrategy, QueueConfig, QueueKey, QueueSystemConfig};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

fn parse_distribution_strategy(strategy: &str) -> DistributionStrategy {
    match strategy.trim().to_ascii_lowercase().as_str() {
        "round_robin" => DistributionStrategy::RoundRobin,
        "longest_idle" => DistributionStrategy::LongestIdle,
        "fewest_calls" => DistributionStrategy::FewestCalls,
        "random" => DistributionStrategy::Random,
        "linear" => DistributionStrategy::Linear,
        "skills_based" => DistributionStrategy::SkillsBased,
        "weighted" => DistributionStrategy::Weighted,
        _ => DistributionStrategy::RoundRobin,
    }
}

fn queue_config_from_record(record: &QueueConfigRecord) -> QueueConfig {
    QueueConfig {
        name: record.name.clone(),
        extension: record.extension.clone(),
        strategy: parse_distribution_strategy(&record.strategy),
        max_wait_time_secs: record.max_wait_time_secs.max(0) as u32,
        max_queue_depth: record.max_queue_depth.max(0) as usize,
        overflow_destination: record.overflow_destination.clone(),
        overflow_destinations: Vec::new(),
        moh_class: record.moh_class.clone(),
        announce_position: record.announce_position,
        announce_interval_secs: record.announce_interval_secs.max(0) as u32,
        announce_eta: false,
        retry_on_no_answer: record.retry_on_no_answer,
        agents: record.agents.clone(),
        agent_group_id: record.agent_group_id.map(|id| id.to_string()),
        priority: record.priority.max(0) as u32,
        required_skills: record.required_skills.clone(),
        agent_skills: HashMap::new(),
        dtmf_exit_options: HashMap::new(),
        callback_enabled: record.callback_enabled,
        callback_digit: None,
        agent_priorities: HashMap::new(),
        agent_weights: HashMap::new(),
        priority_escalation_secs: 0,
        schedule: Vec::new(),
        closed_destination: None,
        timezone: if record.timezone.trim().is_empty() {
            "UTC".to_string()
        } else {
            record.timezone.clone()
        },
        entry_prompt: None,
        connect_prompt: None,
        record_calls: record.record_calls,
        recording_format: "wav".to_string(),
        callback_max_attempts: 3,
        callback_retry_delay_secs: 60,
        sla_warning_threshold_pct: 0.0,
        announcement_language: None,
        campaign_enabled: false,
        campaign_dial_ratio: 1.0,
        campaign_abandon_timeout_secs: 15,
    }
}

fn validate_extension_uniqueness(
    definitions: impl Iterator<Item = TenantQueueDefinition>,
) -> Result<Vec<TenantQueueDefinition>> {
    let mut seen = HashSet::new();
    let mut collected = Vec::new();
    for definition in definitions {
        let extension_key = (definition.tenant_id, definition.config.extension.clone());
        if !seen.insert(extension_key) {
            bail!(
                "duplicate queue extension '{}' configured inside tenant '{}'",
                definition.config.extension,
                definition.tenant_id
            );
        }
        collected.push(definition);
    }
    Ok(collected)
}

/// Build the effective queue runtime configuration by merging file-backed queue
/// config with database-backed queue-config records from every tenant.
///
/// A queue is its name within its tenant (`fcp_queue::QueueKey`), so two
/// tenants may each have a `sales` (docs/TENANT_SCOPE.md §4b). A database
/// record in the default tenant replaces the TOML queue of the same name.
/// The configuration returned keeps the TOML's queues; the definitions are
/// every tenant's.
pub async fn build_effective_queue_runtime(
    base_config: &QueueSystemConfig,
    storage: &Arc<DynStorage>,
    default_tenant_id: TenantId,
) -> Result<(QueueSystemConfig, Vec<TenantQueueDefinition>)> {
    let mut definitions_by_key = HashMap::<QueueKey, TenantQueueDefinition>::new();

    // Seed from the TOML file (always owned by the default tenant).
    for (name, config) in &base_config.queues {
        let definition = TenantQueueDefinition {
            name: name.clone(),
            tenant_id: default_tenant_id,
            config: config.clone(),
        };
        definitions_by_key.insert(definition.key(), definition);
    }

    // Collect tenant IDs: at minimum the default, plus every tenant in the
    // store. Using a HashSet avoids loading the default tenant twice if it
    // also appears in the store.
    let mut tenant_ids = HashSet::from([default_tenant_id]);
    for tenant in storage
        .list_tenants()
        .await
        .context("failed to list tenants for queue runtime configuration")?
    {
        tenant_ids.insert(tenant.id);
    }

    // Load DB-backed queue configs for every tenant.
    for tenant_id in tenant_ids {
        let records = storage
            .list_queue_configs(&tenant_id)
            .await
            .with_context(|| format!("failed to list queue configs for tenant '{}'", tenant_id))?;

        for record in records {
            let definition = TenantQueueDefinition {
                name: record.name.clone(),
                tenant_id: record.tenant_id,
                config: queue_config_from_record(&record),
            };
            definitions_by_key.insert(definition.key(), definition);
        }
    }

    let definitions = validate_extension_uniqueness(definitions_by_key.into_values())?;
    Ok((base_config.clone(), definitions))
}

/// Bring the running queues up to date with the database: a queue made,
/// changed or removed on the admin API is answered, changed or drained
/// here, and each new one starts distributing. Run at an interval and on
/// SIGHUP (`docs/DEEP_TESTING.md` D8: a queue made on the API was never
/// answered until the node was signalled). The names of new queues.
pub async fn sync_from_store(
    queue_manager: &fcp_queue::QueueManager,
    queue_handler: Option<&crate::queue_handler::QueueCallHandler>,
    base_config: &QueueSystemConfig,
    storage: &Arc<DynStorage>,
    default_tenant_id: TenantId,
) -> Result<Vec<QueueKey>> {
    let (effective, definitions) =
        build_effective_queue_runtime(base_config, storage, default_tenant_id).await?;
    let new_queues = queue_manager.sync_queue_definitions(definitions, effective)?;
    if let Some(handler) = queue_handler {
        for name in &new_queues {
            tracing::info!(queue = %name, "Spawning distribution loop for new queue");
            handler.spawn_distribution_loop_for(name.clone());
        }
    }
    Ok(new_queues)
}
