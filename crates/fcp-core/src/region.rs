//! Deployment regions.
//!
//! A region is a free-form label for where a node runs (`us-east`,
//! `eu-west`, `dc-frankfurt`); it only has to agree across nodes. It names
//! infrastructure, not people: the geographic-routing [`Region`](crate::Region)
//! (country, state, city of a caller or number) is a different thing.
//! Nodes advertise it in cluster membership, events carry the region they
//! originated in, and placement prefers the caller's region. See
//! `docs/MULTI_REGION.md`.

/// The region a node should advertise: the configured value when it is
/// non-blank, else the `FCP_REGION` environment variable, else none.
/// Surrounding whitespace is dropped.
pub fn resolve_region(configured: Option<&str>) -> Option<String> {
    configured
        .map(str::to_owned)
        .or_else(|| std::env::var("FCP_REGION").ok())
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_region_is_trimmed_and_blank_means_none() {
        assert_eq!(
            resolve_region(Some(" us-east ")).as_deref(),
            Some("us-east")
        );
        assert_eq!(resolve_region(Some("   ")), None);
        // No configured value: whatever FCP_REGION says in this environment.
        match std::env::var("FCP_REGION") {
            Ok(env) if !env.trim().is_empty() => {
                assert_eq!(resolve_region(None).as_deref(), Some(env.trim()))
            }
            _ => assert_eq!(resolve_region(None), None),
        }
    }
}
