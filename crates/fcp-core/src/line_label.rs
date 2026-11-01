//! A user's own names for their lines (`docs/DEVICE_LINES.md` §2): kept by
//! the line itself (`DeviceLine::key`), so every device showing the line
//! shows the name, and used only while the tenant lets users rename.

use crate::{TenantId, UserId};
use async_trait::async_trait;
use std::collections::HashMap;

/// The longest name a user may give a line.
pub const MAX_LABEL_LEN: usize = 64;

#[async_trait]
pub trait LineLabelStore: Send + Sync {
    /// The user's names, by line.
    async fn line_labels(&self, user: &UserId) -> anyhow::Result<HashMap<String, String>>;
    /// Name a line; an empty name clears it.
    async fn set_line_label(
        &self,
        tenant: &TenantId,
        user: &UserId,
        line: &str,
        label: &str,
    ) -> anyhow::Result<()>;
}
