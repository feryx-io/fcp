//! Which release every node runs (`docs/BACKUP_RESTORE_UPGRADE.md` §1.4,
//! phase 4). Every heartbeat carries its node's version; this groups
//! the members by it at each discovery, says when the cluster stops or
//! starts running one release (`system.version_skew`), and puts the
//! groups on `/status`, `/metrics` and the admin API's cluster stats.
//! Said, never acted on: a rolling upgrade is a skew on purpose, and it
//! is the operator who knows whether this one is.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Release → the node ids on it, both sorted.
pub type VersionGroups = BTreeMap<String, Vec<String>>;

/// Group `(node id, version)` pairs by version.
pub fn group_versions<'a>(nodes: impl IntoIterator<Item = (&'a str, &'a str)>) -> VersionGroups {
    let mut groups = VersionGroups::new();
    for (node, version) in nodes {
        groups
            .entry(version.to_string())
            .or_default()
            .push(node.to_string());
    }
    for nodes in groups.values_mut() {
        nodes.sort();
        nodes.dedup();
    }
    groups
}

/// More than one release is running.
pub fn is_skew(groups: &VersionGroups) -> bool {
    groups.len() > 1
}

/// One sentence for a person: `3 nodes on 0.25.261, 1 on 0.25.259
/// (cm-2)` — the minority groups name their nodes.
pub fn summary(groups: &VersionGroups) -> String {
    if groups.is_empty() {
        return "no nodes".to_string();
    }
    if groups.len() == 1 {
        let (version, nodes) = groups.iter().next().expect("one group");
        return format!(
            "every node on {version} ({} node{})",
            nodes.len(),
            if nodes.len() == 1 { "" } else { "s" }
        );
    }
    let largest = groups.values().map(Vec::len).max().unwrap_or(0);
    let mut parts: Vec<(usize, String)> = groups
        .iter()
        .map(|(version, nodes)| {
            let text = if nodes.len() == largest {
                format!("{} nodes on {version}", nodes.len())
            } else {
                format!("{} on {version} ({})", nodes.len(), nodes.join(", "))
            };
            (nodes.len(), text)
        })
        .collect();
    parts.sort_by_key(|(n, _)| std::cmp::Reverse(*n));
    parts
        .into_iter()
        .map(|(_, text)| text)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The groups as `/status` and the event carry them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterVersions {
    pub versions: VersionGroups,
    /// More than one release is running.
    pub skew: bool,
    /// One sentence for a person.
    pub summary: String,
}

impl ClusterVersions {
    pub fn from_groups(versions: VersionGroups) -> Self {
        let skew = is_skew(&versions);
        let summary = summary(&versions);
        Self {
            versions,
            skew,
            summary,
        }
    }

    /// `fcp_cluster_version_skew` (1 while more than one release runs)
    /// and `fcp_cluster_nodes_by_version{version=…}`.
    pub fn prometheus(&self, labels: &str) -> String {
        let sep = if labels.is_empty() { "" } else { "," };
        let braces = if labels.is_empty() {
            String::new()
        } else {
            format!("{{{labels}}}")
        };
        let mut out = format!(
            "# HELP fcp_cluster_version_skew Whether the cluster's nodes run more than one release (docs/BACKUP_AND_RESTORE.md §5).\n\
             # TYPE fcp_cluster_version_skew gauge\n\
             fcp_cluster_version_skew{braces} {}\n\
             # HELP fcp_cluster_nodes_by_version Nodes running each release, as their heartbeats say.\n\
             # TYPE fcp_cluster_nodes_by_version gauge\n",
            u8::from(self.skew)
        );
        for (version, nodes) in &self.versions {
            out.push_str(&format!(
                "fcp_cluster_nodes_by_version{{{labels}{sep}version=\"{version}\"}} {}\n",
                nodes.len()
            ));
        }
        out
    }
}

/// Watches the set of releases across discoveries and says when it
/// changes while more than one is, or was, running.
#[derive(Debug, Default)]
pub struct VersionTracker {
    seen: Option<BTreeSet<String>>,
}

impl VersionTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Note this discovery's groups. `Some` when the set of releases
    /// differs from the last discovery's and either has more than one:
    /// a skew appearing, changing, or resolving. The first discovery of
    /// a cluster on one release says nothing; of a cluster already
    /// skewed, it says so.
    pub fn observe(&mut self, groups: &VersionGroups) -> Option<ClusterVersions> {
        let now: BTreeSet<String> = groups.keys().cloned().collect();
        let before = self.seen.replace(now.clone());
        let changed = before.as_ref() != Some(&now);
        let was_skew = before.is_some_and(|b| b.len() > 1);
        if changed && (now.len() > 1 || was_skew) {
            Some(ClusterVersions::from_groups(groups.clone()))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn groups(pairs: &[(&str, &str)]) -> VersionGroups {
        group_versions(pairs.iter().copied())
    }

    #[test]
    fn nodes_are_grouped_and_sorted_by_release() {
        let g = groups(&[
            ("cm-2", "0.25.261"),
            ("cm-1", "0.25.261"),
            ("conf-1", "0.25.259"),
        ]);
        assert_eq!(g["0.25.261"], vec!["cm-1", "cm-2"]);
        assert_eq!(g["0.25.259"], vec!["conf-1"]);
        assert!(is_skew(&g));
        assert_eq!(summary(&g), "2 nodes on 0.25.261, 1 on 0.25.259 (conf-1)");
    }

    #[test]
    fn one_release_is_no_skew() {
        let g = groups(&[("cm-1", "0.25.261"), ("cm-2", "0.25.261")]);
        assert!(!is_skew(&g));
        assert_eq!(summary(&g), "every node on 0.25.261 (2 nodes)");
        assert_eq!(summary(&VersionGroups::new()), "no nodes");
    }

    #[test]
    fn a_skew_is_said_when_it_appears_changes_and_resolves() {
        let mut tracker = VersionTracker::new();
        let one = groups(&[("a", "1"), ("b", "1")]);
        assert!(
            tracker.observe(&one).is_none(),
            "one release, nothing to say"
        );
        assert!(tracker.observe(&one).is_none());

        let two = groups(&[("a", "2"), ("b", "1")]);
        let said = tracker.observe(&two).expect("a skew appearing is said");
        assert!(said.skew);
        assert_eq!(said.summary, "1 nodes on 1, 1 nodes on 2");
        assert!(
            tracker.observe(&two).is_none(),
            "the same skew is said once"
        );

        let three = groups(&[("a", "2"), ("b", "1"), ("c", "3")]);
        assert!(
            tracker.observe(&three).is_some(),
            "a third release is a change"
        );

        let done = groups(&[("a", "2"), ("b", "2"), ("c", "2")]);
        let said = tracker.observe(&done).expect("the skew resolving is said");
        assert!(!said.skew);
        assert_eq!(said.summary, "every node on 2 (3 nodes)");
        assert!(tracker.observe(&done).is_none());
    }

    #[test]
    fn a_cluster_found_already_skewed_is_said_at_once() {
        let mut tracker = VersionTracker::new();
        let two = groups(&[("a", "2"), ("b", "1")]);
        assert!(tracker.observe(&two).is_some());
    }

    #[test]
    fn the_metrics_name_every_release() {
        let v = ClusterVersions::from_groups(groups(&[("a", "2"), ("b", "1")]));
        let text = v.prometheus("node=\"a\"");
        assert!(text.contains("fcp_cluster_version_skew{node=\"a\"} 1\n"));
        assert!(text.contains("fcp_cluster_nodes_by_version{node=\"a\",version=\"1\"} 1\n"));
        assert!(text.contains("fcp_cluster_nodes_by_version{node=\"a\",version=\"2\"} 1\n"));
        let text = ClusterVersions::from_groups(groups(&[("a", "2")])).prometheus("");
        assert!(text.contains("fcp_cluster_version_skew 0\n"));
        assert!(text.contains("fcp_cluster_nodes_by_version{version=\"2\"} 1\n"));
    }
}
