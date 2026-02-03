//! Pickup group resolution.
//!
//! Maps extensions to pickup groups so the handler can determine which
//! ringing calls a caller is allowed to pick up.

use crate::config::PickupGroupConfig;
use std::collections::HashMap;

/// Lookup structure for pickup groups.
///
/// Built once from configuration at startup. Thread-safe and immutable.
pub struct PickupGroups {
    /// Extension → group name (each extension can belong to at most one group).
    member_to_group: HashMap<String, String>,

    /// Group name → list of member extensions.
    group_members: HashMap<String, Vec<String>>,
}

impl PickupGroups {
    /// Build the lookup tables from configuration.
    pub fn from_config(groups: &HashMap<String, PickupGroupConfig>) -> Self {
        let mut member_to_group = HashMap::new();
        let mut group_members = HashMap::new();

        for (group_name, config) in groups {
            let mut members = Vec::with_capacity(config.members.len());
            for member in &config.members {
                member_to_group.insert(member.clone(), group_name.clone());
                members.push(member.clone());
            }
            group_members.insert(group_name.clone(), members);
        }

        Self {
            member_to_group,
            group_members,
        }
    }

    /// Get all members in the same pickup group as the given extension.
    ///
    /// Returns `None` if the extension is not in any pickup group.
    pub fn group_members_for(&self, extension: &str) -> Option<&[String]> {
        let group_name = self.member_to_group.get(extension)?;
        self.group_members.get(group_name).map(|v| v.as_slice())
    }

    /// Get the group name for an extension.
    pub fn group_for(&self, extension: &str) -> Option<&str> {
        self.member_to_group.get(extension).map(|s| s.as_str())
    }

    /// Check whether the lookup has any groups defined.
    pub fn is_empty(&self) -> bool {
        self.group_members.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PickupGroupConfig;

    fn test_groups() -> HashMap<String, PickupGroupConfig> {
        let mut groups = HashMap::new();
        groups.insert(
            "floor1".to_string(),
            PickupGroupConfig {
                members: vec!["1001".to_string(), "1002".to_string(), "1003".to_string()],
            },
        );
        groups.insert(
            "floor2".to_string(),
            PickupGroupConfig {
                members: vec!["2001".to_string(), "2002".to_string()],
            },
        );
        groups
    }

    #[test]
    fn test_group_members_for() {
        let pg = PickupGroups::from_config(&test_groups());
        let members = pg.group_members_for("1001").unwrap();
        assert_eq!(members.len(), 3);
        assert!(members.contains(&"1001".to_string()));
        assert!(members.contains(&"1002".to_string()));
        assert!(members.contains(&"1003".to_string()));
    }

    #[test]
    fn test_different_groups() {
        let pg = PickupGroups::from_config(&test_groups());
        let floor1 = pg.group_members_for("1001").unwrap();
        let floor2 = pg.group_members_for("2001").unwrap();
        assert_eq!(floor1.len(), 3);
        assert_eq!(floor2.len(), 2);
    }

    #[test]
    fn test_unknown_extension() {
        let pg = PickupGroups::from_config(&test_groups());
        assert!(pg.group_members_for("9999").is_none());
    }

    #[test]
    fn test_group_for() {
        let pg = PickupGroups::from_config(&test_groups());
        assert_eq!(pg.group_for("1001"), Some("floor1"));
        assert_eq!(pg.group_for("2001"), Some("floor2"));
        assert_eq!(pg.group_for("9999"), None);
    }

    #[test]
    fn test_empty_groups() {
        let pg = PickupGroups::from_config(&HashMap::new());
        assert!(pg.is_empty());
        assert!(pg.group_members_for("1001").is_none());
    }
}
