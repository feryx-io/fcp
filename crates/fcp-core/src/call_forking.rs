//! Call forking implementation for parallel and serial call distribution.
//!
//! This module provides the logic for forking calls to multiple targets based on
//! ring strategy, with support for:
//! - Parallel forking (simultaneous ringing)
//! - Serial forking (sequential ringing)
//! - Early answer handling (cancel other forks)
//! - Timeout management

use crate::{
    call_state::{ActiveCall, CallFork, CallState, CallStateManager},
    routing::{RingPlan, RoutingTarget},
    CallId, DeviceId,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;
use tracing::{debug, info, warn};

/// Result of a fork operation.
#[derive(Debug)]
pub enum ForkResult {
    /// Fork answered successfully
    Answered {
        device_id: DeviceId,
        dialog_id: String,
        fork_index: usize,
    },
    /// Fork failed (busy, rejected, timeout)
    Failed { device_id: DeviceId, reason: String },
    /// All forks failed
    AllFailed,
}

/// Fork strategy determines how calls are distributed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkStrategy {
    /// Ring all targets simultaneously
    Parallel,
    /// Ring targets one at a time
    Serial,
}

/// Call forker handles the forking logic.
pub struct CallForker {
    call_state_manager: Arc<CallStateManager>,
}

impl CallForker {
    /// Creates a new call forker.
    pub fn new(call_state_manager: Arc<CallStateManager>) -> Self {
        Self { call_state_manager }
    }

    /// Forks a call according to the ring plan.
    ///
    /// This method will:
    /// 1. Create forks for each group of targets
    /// 2. Wait for answers or timeouts
    /// 3. Cancel remaining forks when one answers
    /// 4. Return the result
    pub async fn fork_call<F, Fut>(
        &self,
        call: &mut ActiveCall,
        ring_plan: &RingPlan,
        fork_fn: F,
    ) -> ForkResult
    where
        F: Fn(RoutingTarget) -> Fut + Send + Sync + 'static + Copy,
        Fut: std::future::Future<Output = Result<String, String>> + Send + 'static,
    {
        info!(
            "Forking call {} with strategy {:?} to {} targets",
            call.call_id,
            ring_plan.strategy,
            ring_plan.total_targets()
        );

        // Process each ring group in order
        for (group_idx, ring_group) in ring_plan.groups.iter().enumerate() {
            debug!(
                "Processing ring group {} with {} targets",
                group_idx,
                ring_group.targets.len()
            );

            // Fork to all targets in this group
            let mut fork_tasks = Vec::new();
            let fork_start_index = call.forks.len();

            for (target_idx, target) in ring_group.targets.iter().enumerate() {
                let fork_index = fork_start_index + target_idx;

                // Create fork entry
                let fork = CallFork {
                    device_id: target.device_id,
                    dialog_id: String::new(), // Will be set after forking
                    state: CallState::Dialing,
                    ring_start: std::time::SystemTime::now(),
                    answer_time: None,
                    priority: target.priority,
                };

                call.forks.push(fork);
                self.call_state_manager
                    .add_fork(&call.call_id, call.forks[fork_index].clone())
                    .await;

                // Start the fork
                let target_clone = target.clone();
                let call_id = call.call_id;
                let device_id = target.device_id;
                let target_timeout = ring_group.timeout;

                let task = tokio::spawn(async move {
                    debug!("Starting fork to device {} for call {}", device_id, call_id);

                    match timeout(target_timeout, fork_fn(target_clone)).await {
                        Ok(Ok(dialog_id)) => {
                            info!("Fork to device {} answered", device_id);
                            ForkResult::Answered {
                                device_id,
                                dialog_id,
                                fork_index,
                            }
                        }
                        Ok(Err(reason)) => {
                            warn!("Fork to device {} failed: {}", device_id, reason);
                            ForkResult::Failed { device_id, reason }
                        }
                        Err(_) => {
                            warn!("Fork to device {} timed out", device_id);
                            ForkResult::Failed {
                                device_id,
                                reason: "Timeout".to_string(),
                            }
                        }
                    }
                });

                fork_tasks.push(task);
            }

            // Wait for first answer or all failures
            let result = self
                .wait_for_answer(&call.call_id, fork_tasks, ring_group.timeout)
                .await;

            match result {
                ForkResult::Answered {
                    device_id,
                    dialog_id,
                    fork_index,
                } => {
                    // Update the fork with the dialog ID
                    if let Some(fork) = call.forks.get_mut(fork_index) {
                        fork.dialog_id = dialog_id.clone();
                    }

                    // Mark this fork as answered
                    self.call_state_manager
                        .answer_fork(&call.call_id, fork_index)
                        .await;

                    return ForkResult::Answered {
                        device_id,
                        dialog_id,
                        fork_index,
                    };
                }
                ForkResult::Failed { .. } => {
                    // Continue to next group
                    continue;
                }
                ForkResult::AllFailed => {
                    // Continue to next group
                    continue;
                }
            }
        }

        // All groups failed
        warn!("All fork groups failed for call {}", call.call_id);
        self.call_state_manager.terminate_call(&call.call_id).await;

        ForkResult::AllFailed
    }

    /// Waits for the first answer or all failures.
    async fn wait_for_answer(
        &self,
        call_id: &CallId,
        mut fork_tasks: Vec<tokio::task::JoinHandle<ForkResult>>,
        group_timeout: Duration,
    ) -> ForkResult {
        let deadline = tokio::time::Instant::now() + group_timeout;

        while !fork_tasks.is_empty() {
            let time_left = deadline.saturating_duration_since(tokio::time::Instant::now());
            if time_left.is_zero() {
                debug!("Group timeout reached for call {}", call_id);
                break;
            }

            // Wait for any task to complete using a manual poll
            // We'll check each task and wait for the first completion
            let sleep = tokio::time::sleep(time_left);
            tokio::pin!(sleep);

            let mut completed_idx = None;
            let mut completed_result = None;

            for (idx, task) in fork_tasks.iter_mut().enumerate() {
                tokio::select! {
                    result = task => {
                        completed_idx = Some(idx);
                        completed_result = Some(result);
                        break;
                    }
                    _ = &mut sleep => {
                        // Timeout reached
                        break;
                    }
                }
            }

            if let (Some(idx), Some(result)) = (completed_idx, completed_result) {
                fork_tasks.remove(idx);

                match result {
                    Ok(ForkResult::Answered {
                        device_id,
                        dialog_id,
                        fork_index,
                    }) => {
                        // Cancel remaining tasks
                        for task in fork_tasks {
                            task.abort();
                        }

                        return ForkResult::Answered {
                            device_id,
                            dialog_id,
                            fork_index,
                        };
                    }
                    Ok(ForkResult::Failed { .. }) => {
                        // Continue waiting for other forks
                        continue;
                    }
                    Ok(ForkResult::AllFailed) => {
                        // This shouldn't happen from a task
                        continue;
                    }
                    Err(_) => {
                        // Task panicked
                        continue;
                    }
                }
            } else {
                // Timeout reached
                break;
            }
        }

        // All tasks completed without answer or timeout reached
        ForkResult::AllFailed
    }

    /// Forks a call in parallel (simultaneous ringing).
    pub async fn fork_parallel<F, Fut>(
        &self,
        call: &mut ActiveCall,
        targets: Vec<RoutingTarget>,
        timeout_duration: Duration,
        fork_fn: F,
    ) -> ForkResult
    where
        F: Fn(RoutingTarget) -> Fut + Send + Sync + 'static + Copy,
        Fut: std::future::Future<Output = Result<String, String>> + Send + 'static,
    {
        use crate::routing::RingGroup;

        let ring_plan = RingPlan {
            groups: vec![RingGroup {
                targets,
                timeout: timeout_duration,
            }],
            strategy: crate::RingStrategy::Simultaneous,
            total_timeout: timeout_duration,
        };

        self.fork_call(call, &ring_plan, fork_fn).await
    }

    /// Forks a call in serial (sequential ringing).
    pub async fn fork_serial<F, Fut>(
        &self,
        call: &mut ActiveCall,
        targets: Vec<RoutingTarget>,
        timeout_per_target: Duration,
        fork_fn: F,
    ) -> ForkResult
    where
        F: Fn(RoutingTarget) -> Fut + Send + Sync + 'static + Copy,
        Fut: std::future::Future<Output = Result<String, String>> + Send + 'static,
    {
        use crate::routing::RingGroup;

        let target_count = targets.len();
        let ring_plan = RingPlan {
            groups: targets
                .into_iter()
                .map(|target| RingGroup {
                    targets: vec![target],
                    timeout: timeout_per_target,
                })
                .collect(),
            strategy: crate::RingStrategy::Sequential,
            total_timeout: timeout_per_target * target_count as u32,
        };

        self.fork_call(call, &ring_plan, fork_fn).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CallDirection, DeviceId, TenantId};

    #[tokio::test]
    async fn test_parallel_fork_first_answers() {
        let manager = Arc::new(CallStateManager::new());
        let forker = CallForker::new(manager.clone());

        let call_id = CallId::new();
        let mut call = ActiveCall::new(
            call_id,
            TenantId::nil(),
            CallDirection::Inbound,
            "sip:alice@example.com".to_string(),
            "sip:bob@example.com".to_string(),
        );

        let targets = vec![
            RoutingTarget {
                device_id: DeviceId::new(),
                sip_uri: "sip:device1@example.com".to_string(),
                priority: 1,
                line_appearance: 1,
                shared_line: false,
                enabled: true,
                timeout: Duration::from_secs(30),
            },
            RoutingTarget {
                device_id: DeviceId::new(),
                sip_uri: "sip:device2@example.com".to_string(),
                priority: 1,
                line_appearance: 1,
                shared_line: false,
                enabled: true,
                timeout: Duration::from_secs(30),
            },
        ];

        // Mock fork function that simulates first device answering
        let fork_fn = |target: RoutingTarget| async move {
            if target.sip_uri.contains("device1") {
                tokio::time::sleep(Duration::from_millis(100)).await;
                Ok("dialog-1".to_string())
            } else {
                tokio::time::sleep(Duration::from_secs(10)).await;
                Err("No answer".to_string())
            }
        };

        let result = forker
            .fork_parallel(&mut call, targets, Duration::from_secs(5), fork_fn)
            .await;

        match result {
            ForkResult::Answered { fork_index, .. } => {
                assert_eq!(fork_index, 0);
            }
            _ => panic!("Expected answered result"),
        }
    }
}
