/// Marker trait for long-running services inside the FCP runtime.
pub trait Service: Send + Sync {
    /// Human-friendly name for diagnostics.
    fn name(&self) -> &'static str;
}
