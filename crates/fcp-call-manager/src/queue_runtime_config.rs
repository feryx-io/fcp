use anyhow::{bail, Context, Result};
use fcp_core::storage::{QueueConfigStore, TenantStore};
use fcp_core::{DynStorage, QueueConfigRecord, TenantId};
use fcp_queue::queue_manager::TenantQueueDefinition;
use fcp_queue::{DistributionStrategy, QueueConfig, QueueSystemConfig};
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
/// config with database-backed queue-config records.
///
/// Queue names remain global runtime identifiers. A duplicate queue name across
/// different tenants is therefore rejected at build time rather than silently
/// collapsing one tenant's queue into another.
pub async fn build_effective_queue_runtime(
    base_config: &QueueSystemConfig,
    storage: &Arc<DynStorage>,
    default_tenant_id: TenantId,
) -> Result<(QueueSystemConfig, Vec<TenantQueueDefinition>)> {
    let mut definitions_by_name = HashMap::<String, TenantQueueDefinition>::new();

    for (name, config) in &base_config.queues {
        definitions_by_name.insert(
            name.clone(),
            TenantQueueDefinition {
                name: name.clone(),
                tenant_id: default_tenant_id,
                config: config.clone(),
            },
        );
    }

    let tenants = storage
        .list_tenants()
        .await
        .context("failed to list tenants for queue runtime configuration")?;

    for tenant in tenants {
        let records = storage
            .list_queue_configs(&tenant.id)
            .await
            .with_context(|| format!("failed to list queue configs for tenant '{}'", tenant.id))?;

        for record in records {
            let definition = TenantQueueDefinition {
                name: record.name.clone(),
                tenant_id: record.tenant_id,
                config: queue_config_from_record(&record),
            };

            if let Some(existing) = definitions_by_name.get(&definition.name) {
                if existing.tenant_id != definition.tenant_id {
                    bail!(
                        "queue name '{}' is configured in multiple tenants; runtime queue names must stay globally unique",
                        definition.name
                    );
                }
            }

            definitions_by_name.insert(definition.name.clone(), definition);
        }
    }

    let definitions = validate_extension_uniqueness(definitions_by_name.into_values())?;
    let mut effective_config = base_config.clone();
    effective_config.queues = definitions
        .iter()
        .map(|definition| (definition.name.clone(), definition.config.clone()))
        .collect();

    Ok((effective_config, definitions))
}
