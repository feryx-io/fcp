//! Files a site holds, fetched through its link on demand
//! (`docs/HYBRID_CONTROL_PLANE.md` §3.3). A site opens no port: the cloud
//! queues a request, the site's link collects it and sends the bytes up,
//! and the route that asked answers with them.

use fcp_core::{SiteId, TenantId};

#[async_trait::async_trait]
pub trait SiteFiles: Send + Sync {
    /// The bytes of `path` as `site` holds it, or why not — the site not
    /// linked, the request unanswered in time, the file gone.
    async fn fetch(
        &self,
        site: SiteId,
        tenant: TenantId,
        class: &str,
        path: &str,
    ) -> Result<Vec<u8>, String>;

    /// Ask `site` to remove `path`; `Ok` once the request is queued for
    /// the link, which removes the file on its own time.
    async fn remove(
        &self,
        site: SiteId,
        tenant: TenantId,
        class: &str,
        path: &str,
    ) -> Result<(), String>;
}
