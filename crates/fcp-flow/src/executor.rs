//! Flow executor for managing concurrent flow executions.
//!
//! This module provides utilities for managing multiple concurrent flow executions,
//! lifecycle management, and execution tracking.
//!
//! NOTE: This is a placeholder for future executor functionality.
//! In the MVP, flow execution is handled directly via FlowEngine.

use crate::engine::{FlowEngine, FlowContext};
use crate::schema::FlowDefinition;
use crate::Result;
use fcp_core::CallId;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

/// Manages concurrent flow executions.
///
/// The executor tracks active flow executions and provides lifecycle management.
pub struct FlowExecutor {
    /// The flow engine for executing flows
    engine: Arc<FlowEngine>,

    /// Active flow executions, indexed by call ID
    active_executions: Arc<RwLock<HashMap<CallId, Arc<FlowContext>>>>,
}

impl FlowExecutor {
    /// Create a new flow executor.
    pub fn new(engine: Arc<FlowEngine>) -> Self {
        Self {
            engine,
            active_executions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Start a flow execution for a call.
    pub async fn start_execution(
        &self,
        flow: Arc<FlowDefinition>,
        call_id: CallId,
        initial_variables: HashMap<String, String>,
    ) -> Result<Arc<FlowContext>> {
        info!(
            call_id = %call_id,
            flow_id = %flow.id,
            "Starting flow execution"
        );

        let context = self.engine.execute(flow, call_id, initial_variables).await?;
        let context = Arc::new(context);

        // Track active execution
        let mut executions = self.active_executions.write().await;
        executions.insert(call_id, context.clone());

        Ok(context)
    }

    /// Stop a flow execution.
    pub async fn stop_execution(&self, call_id: &CallId) {
        info!(call_id = %call_id, "Stopping flow execution");

        let mut executions = self.active_executions.write().await;
        executions.remove(call_id);
    }

    /// Get an active execution by call ID.
    pub async fn get_execution(&self, call_id: &CallId) -> Option<Arc<FlowContext>> {
        let executions = self.active_executions.read().await;
        executions.get(call_id).cloned()
    }

    /// Get count of active executions.
    pub async fn active_count(&self) -> usize {
        let executions = self.active_executions.read().await;
        executions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_executor_lifecycle() {
        let engine = Arc::new(FlowEngine::new());
        let executor = FlowExecutor::new(engine);

        assert_eq!(executor.active_count().await, 0);
    }
}
