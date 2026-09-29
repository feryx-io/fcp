//! Whether this node's view of the cluster can be trusted
//! (`docs/MULTI_REGION_FOLLOWUPS.md` §2.3, phase 3).
//!
//! A region cut off from the others cannot tell, from membership alone,
//! "the other regions went away" from "I can no longer see them". This
//! says which it looks like. It is visibility only, deliberately (§4, Q3):
//! nothing here changes placement or anything else a node does.
//!
//! Two shapes are recognised:
//!
//! - **The cluster store is unreachable.** Discovery fails, so the member
//!   list is whatever it was when discovery last worked — it does not
//!   shrink, and nothing else says it is stale. This is what the cut-off
//!   side of a single, global Redis sees.
//! - **Every other region vanished at once while this one stays.** The
//!   store answers, but every node outside this node's region has gone
//!   from it within one heartbeat window, and not by shutting down. With
//!   an active-active Redis this is what *both* sides of a split see;
//!   with one Redis it is what the side holding the Redis sees when a
//!   region is cut off or lost. Either way it is worth saying, and a node
//!   cannot tell which from inside.
//!
//! A probe of the node's database, when the binary gives one, adds the
//! distinction the survey asked for: a node that still reaches its
//! database but not the cluster is almost certainly the one cut off; one
//! that reaches neither is more likely looking at its own network.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// What this node's view of the cluster looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ViewState {
    /// Nothing suggests the view is wrong.
    Whole,
    /// Discovery is failing: the member list is as old as the last
    /// discovery that worked.
    StoreUnreachable,
    /// Every node outside this node's region went at once.
    RemoteRegionsLost,
}

impl ViewState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ViewState::Whole => "whole",
            ViewState::StoreUnreachable => "store_unreachable",
            ViewState::RemoteRegionsLost => "remote_regions_lost",
        }
    }
}

/// This node's view, as `/status`, the heartbeat record and the
/// `system.cluster_view_changed` event carry it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ClusterView {
    pub state: ViewState,
    /// When the current state began (Unix seconds); absent while whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
    /// The regions whose every node went at once.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lost_regions: Vec<String>,
    /// Seconds since discovery last worked — how old the member list is.
    /// Absent before the first discovery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_age_secs: Option<u64>,
    /// Whether the node reaches its database, when it has a probe and the
    /// view is not whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_reachable: Option<bool>,
    /// One sentence for a person.
    pub summary: String,
}

impl ClusterView {
    pub fn is_whole(&self) -> bool {
        self.state == ViewState::Whole
    }

    /// `fcp_cluster_view{state=…}` (one series per state, the current one
    /// at 1) and `fcp_cluster_view_age_seconds`.
    pub fn prometheus(&self, labels: &str) -> String {
        let sep = if labels.is_empty() { "" } else { "," };
        let mut out = String::from(
            "# HELP fcp_cluster_view Whether this node's view of the cluster looks whole, by state (docs/MULTI_REGION_FOLLOWUPS.md §2.3).\n\
             # TYPE fcp_cluster_view gauge\n",
        );
        for state in [
            ViewState::Whole,
            ViewState::StoreUnreachable,
            ViewState::RemoteRegionsLost,
        ] {
            out.push_str(&format!(
                "fcp_cluster_view{{{labels}{sep}state=\"{}\"}} {}\n",
                state.as_str(),
                u8::from(self.state == state)
            ));
        }
        if let Some(age) = self.view_age_secs {
            let braces = if labels.is_empty() {
                String::new()
            } else {
                format!("{{{labels}}}")
            };
            out.push_str(&format!(
                "# HELP fcp_cluster_view_age_seconds Seconds since cluster discovery last worked.\n\
                 # TYPE fcp_cluster_view_age_seconds gauge\n\
                 fcp_cluster_view_age_seconds{braces} {age}\n"
            ));
        }
        out
    }
}

/// A node that went from the member list since the last discovery.
#[derive(Debug, Clone)]
pub struct Departure {
    pub region: Option<String>,
    /// It said it was leaving (a shutdown), rather than going quiet.
    pub graceful: bool,
}

/// A change of state, for the event.
#[derive(Debug, Clone)]
pub struct Transition {
    pub from: ViewState,
    pub view: ClusterView,
    /// How long the state just left lasted, when it was not whole.
    pub lasted_secs: Option<u64>,
}

/// Follows discovery's outcomes and says what they add up to.
#[derive(Debug)]
pub struct ViewTracker {
    own_region: Option<String>,
    /// How close together departures must be to count as "at once": two
    /// heartbeat TTLs, since nodes that stopped together still expire up
    /// to one heartbeat interval apart.
    window: Duration,
    state: ViewState,
    since: Option<(Instant, SystemTime)>,
    lost_regions: Vec<String>,
    last_success: Option<Instant>,
    /// Regions outside this one with a node at the last discovery.
    remote_present: BTreeSet<String>,
    /// Remote nodes that went quiet recently, by region.
    recent_losses: Vec<(Instant, String)>,
    database_reachable: Option<bool>,
}

fn unix_secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

impl ViewTracker {
    pub fn new(own_region: Option<String>, heartbeat_ttl: Duration) -> Self {
        Self {
            own_region,
            window: heartbeat_ttl * 2,
            state: ViewState::Whole,
            since: None,
            lost_regions: Vec::new(),
            last_success: None,
            remote_present: BTreeSet::new(),
            recent_losses: Vec::new(),
            database_reachable: None,
        }
    }

    pub fn state(&self) -> ViewState {
        self.state
    }

    /// Discovery worked: `member_regions` is the region of every other
    /// node now in the store, `departed` every node that went since the
    /// last discovery.
    pub fn on_discovery(
        &mut self,
        now: Instant,
        wall: SystemTime,
        member_regions: &[Option<String>],
        departed: &[Departure],
    ) -> Option<Transition> {
        self.last_success = Some(now);
        let Some(own) = self.own_region.clone() else {
            // Without a region there is nothing remote to lose.
            return self.become_whole(now);
        };

        for d in departed {
            if let Some(region) = &d.region {
                if *region != own && !d.graceful {
                    self.recent_losses.push((now, region.clone()));
                }
            }
        }
        let window = self.window;
        self.recent_losses
            .retain(|(at, _)| now.duration_since(*at) <= window);

        let remote_now: BTreeSet<String> = member_regions
            .iter()
            .flatten()
            .filter(|r| **r != own)
            .cloned()
            .collect();
        let had_remote = !self.remote_present.is_empty();
        let previously = std::mem::replace(&mut self.remote_present, remote_now.clone());

        if !remote_now.is_empty() {
            return self.become_whole(now);
        }
        if self.state == ViewState::RemoteRegionsLost {
            return None;
        }
        // Nothing remote now. It is a loss worth saying only if there was
        // something remote a moment ago and it went quiet rather than
        // shutting down.
        if !self.recent_losses.is_empty() && (had_remote || self.state != ViewState::Whole) {
            let lost: BTreeSet<String> = self
                .recent_losses
                .iter()
                .map(|(_, r)| r.clone())
                .chain(previously)
                .collect();
            return self.enter(
                ViewState::RemoteRegionsLost,
                now,
                wall,
                lost.into_iter().collect(),
            );
        }
        self.become_whole(now)
    }

    /// Discovery failed.
    pub fn on_failure(&mut self, now: Instant, wall: SystemTime) -> Option<Transition> {
        if self.state == ViewState::StoreUnreachable {
            return None;
        }
        self.enter(ViewState::StoreUnreachable, now, wall, Vec::new())
    }

    /// What the database probe said, while the view is not whole.
    pub fn set_database_reachable(&mut self, reachable: Option<bool>) {
        self.database_reachable = reachable;
    }

    /// The view now.
    pub fn view(&self, now: Instant) -> ClusterView {
        let view_age_secs = self.last_success.map(|at| now.duration_since(at).as_secs());
        let database_reachable = if self.state == ViewState::Whole {
            None
        } else {
            self.database_reachable
        };
        ClusterView {
            state: self.state,
            since: self.since.map(|(_, wall)| unix_secs(wall)),
            lost_regions: self.lost_regions.clone(),
            view_age_secs,
            database_reachable,
            summary: self.summary(view_age_secs, database_reachable),
        }
    }

    fn summary(&self, age: Option<u64>, database: Option<bool>) -> String {
        let age = age
            .map(|a| format!("{a}s old"))
            .unwrap_or_else(|| "from before any discovery".to_string());
        match (self.state, database) {
            (ViewState::Whole, _) => "the cluster view looks whole".to_string(),
            (ViewState::StoreUnreachable, Some(true)) => format!(
                "cannot reach the cluster store but reaches the database: this node is \
                 probably the one cut off; its member list is {age}"
            ),
            (ViewState::StoreUnreachable, Some(false)) => format!(
                "reaches neither the cluster store nor the database: this node's own \
                 network is the likely cause; its member list is {age}"
            ),
            (ViewState::StoreUnreachable, None) => {
                format!("cannot reach the cluster store; its member list is {age}")
            }
            (ViewState::RemoteRegionsLost, database) => {
                let own = self.own_region.as_deref().unwrap_or("-");
                let still = match database {
                    Some(true) => ", and it still reaches the database",
                    Some(false) => ", and it cannot reach the database either",
                    None => "",
                };
                format!(
                    "every node in {} went at once while {own} stays{still}: this node's \
                     view of the cluster may be what is broken",
                    self.lost_regions.join(", ")
                )
            }
        }
    }

    fn enter(
        &mut self,
        state: ViewState,
        now: Instant,
        wall: SystemTime,
        lost_regions: Vec<String>,
    ) -> Option<Transition> {
        let from = self.state;
        let lasted_secs = self.lasted(now);
        self.state = state;
        self.since = Some((now, wall));
        self.lost_regions = lost_regions;
        Some(Transition {
            from,
            view: self.view(now),
            lasted_secs,
        })
    }

    fn become_whole(&mut self, now: Instant) -> Option<Transition> {
        if self.state == ViewState::Whole {
            return None;
        }
        let from = self.state;
        let lasted_secs = self.lasted(now);
        self.state = ViewState::Whole;
        self.since = None;
        self.lost_regions.clear();
        self.recent_losses.clear();
        self.database_reachable = None;
        Some(Transition {
            from,
            view: self.view(now),
            lasted_secs,
        })
    }

    fn lasted(&self, now: Instant) -> Option<u64> {
        match self.state {
            ViewState::Whole => None,
            _ => self
                .since
                .map(|(at, _)| now.saturating_duration_since(at).as_secs()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TTL: Duration = Duration::from_secs(15);

    fn regions(rs: &[&str]) -> Vec<Option<String>> {
        rs.iter().map(|r| Some(r.to_string())).collect()
    }

    fn quiet(region: &str) -> Departure {
        Departure {
            region: Some(region.to_string()),
            graceful: false,
        }
    }

    fn shutdown(region: &str) -> Departure {
        Departure {
            region: Some(region.to_string()),
            graceful: true,
        }
    }

    fn tracker() -> (ViewTracker, Instant, SystemTime) {
        (
            ViewTracker::new(Some("us-east".into()), TTL),
            Instant::now(),
            SystemTime::now(),
        )
    }

    #[test]
    fn a_healthy_cluster_is_whole() {
        let (mut t, now, wall) = tracker();
        assert!(t
            .on_discovery(now, wall, &regions(&["us-east", "eu-west"]), &[])
            .is_none());
        let view = t.view(now);
        assert_eq!(view.state, ViewState::Whole);
        assert_eq!(view.view_age_secs, Some(0));
        assert!(view.since.is_none());
    }

    #[test]
    fn every_other_region_going_quiet_at_once_is_said() {
        let (mut t, now, wall) = tracker();
        t.on_discovery(
            now,
            wall,
            &regions(&["us-east", "eu-west", "ap-south"]),
            &[],
        );
        let later = now + Duration::from_secs(5);
        let change = t
            .on_discovery(
                later,
                wall,
                &regions(&["us-east"]),
                &[quiet("eu-west"), quiet("ap-south")],
            )
            .expect("a transition");
        assert_eq!(change.from, ViewState::Whole);
        assert_eq!(change.view.state, ViewState::RemoteRegionsLost);
        assert_eq!(change.view.lost_regions, vec!["ap-south", "eu-west"]);
        assert!(
            change.view.summary.contains("us-east stays"),
            "{}",
            change.view.summary
        );
    }

    #[test]
    fn regions_expiring_a_tick_apart_still_count_as_at_once() {
        // Nodes that stop together expire up to a heartbeat apart.
        let (mut t, now, wall) = tracker();
        t.on_discovery(
            now,
            wall,
            &regions(&["eu-west", "eu-west", "ap-south"]),
            &[],
        );
        let t1 = now + Duration::from_secs(5);
        assert!(t
            .on_discovery(
                t1,
                wall,
                &regions(&["ap-south"]),
                &[quiet("eu-west"), quiet("eu-west")]
            )
            .is_none());
        let t2 = t1 + Duration::from_secs(5);
        let change = t
            .on_discovery(t2, wall, &[], &[quiet("ap-south")])
            .expect("a transition");
        assert_eq!(change.view.lost_regions, vec!["ap-south", "eu-west"]);
    }

    #[test]
    fn a_region_that_shut_down_is_not_a_partition() {
        let (mut t, now, wall) = tracker();
        t.on_discovery(now, wall, &regions(&["eu-west"]), &[]);
        let later = now + Duration::from_secs(5);
        assert!(t
            .on_discovery(later, wall, &[], &[shutdown("eu-west")])
            .is_none());
        assert_eq!(t.state(), ViewState::Whole);
    }

    #[test]
    fn a_node_in_this_region_going_quiet_is_not_a_partition() {
        let (mut t, now, wall) = tracker();
        t.on_discovery(now, wall, &regions(&["us-east", "eu-west"]), &[]);
        let later = now + Duration::from_secs(5);
        assert!(t
            .on_discovery(later, wall, &regions(&["eu-west"]), &[quiet("us-east")])
            .is_none());
    }

    #[test]
    fn one_region_of_several_going_quiet_is_not_a_partition() {
        let (mut t, now, wall) = tracker();
        t.on_discovery(now, wall, &regions(&["eu-west", "ap-south"]), &[]);
        let later = now + Duration::from_secs(5);
        assert!(t
            .on_discovery(later, wall, &regions(&["ap-south"]), &[quiet("eu-west")])
            .is_none());
    }

    #[test]
    fn a_remote_node_coming_back_heals_it() {
        let (mut t, now, wall) = tracker();
        t.on_discovery(now, wall, &regions(&["eu-west"]), &[]);
        let t1 = now + Duration::from_secs(5);
        t.on_discovery(t1, wall, &[], &[quiet("eu-west")]);
        // Still nothing remote: the state holds, and says nothing new.
        let t2 = t1 + Duration::from_secs(60);
        assert!(t.on_discovery(t2, wall, &[], &[]).is_none());
        assert_eq!(t.state(), ViewState::RemoteRegionsLost);

        let t3 = t2 + Duration::from_secs(5);
        let healed = t
            .on_discovery(t3, wall, &regions(&["eu-west"]), &[])
            .expect("healed");
        assert_eq!(healed.from, ViewState::RemoteRegionsLost);
        assert_eq!(healed.view.state, ViewState::Whole);
        assert_eq!(healed.lasted_secs, Some(65));
    }

    #[test]
    fn a_failing_store_is_said_and_the_view_ages() {
        let (mut t, now, wall) = tracker();
        t.on_discovery(now, wall, &regions(&["eu-west"]), &[]);
        let t1 = now + Duration::from_secs(5);
        let change = t.on_failure(t1, wall).expect("a transition");
        assert_eq!(change.view.state, ViewState::StoreUnreachable);
        assert!(t.on_failure(t1 + Duration::from_secs(5), wall).is_none());

        t.set_database_reachable(Some(true));
        let view = t.view(now + Duration::from_secs(40));
        assert_eq!(view.view_age_secs, Some(40));
        assert_eq!(view.database_reachable, Some(true));
        assert!(
            view.summary.contains("probably the one cut off"),
            "{}",
            view.summary
        );

        t.set_database_reachable(Some(false));
        assert!(t
            .view(now)
            .summary
            .contains("own network is the likely cause"));

        let healed = t
            .on_discovery(
                now + Duration::from_secs(45),
                wall,
                &regions(&["eu-west"]),
                &[],
            )
            .expect("healed");
        assert_eq!(healed.view.state, ViewState::Whole);
        assert_eq!(healed.lasted_secs, Some(40));
        assert_eq!(healed.view.database_reachable, None);
    }

    #[test]
    fn coming_back_from_the_store_to_find_the_other_regions_gone_is_said() {
        // Cut off from a global Redis, then back: the stale list still had
        // the other region, and now it has gone quietly.
        let (mut t, now, wall) = tracker();
        t.on_discovery(now, wall, &regions(&["eu-west"]), &[]);
        t.on_failure(now + Duration::from_secs(5), wall);
        let change = t
            .on_discovery(
                now + Duration::from_secs(10),
                wall,
                &[],
                &[quiet("eu-west")],
            )
            .expect("a transition");
        assert_eq!(change.from, ViewState::StoreUnreachable);
        assert_eq!(change.view.state, ViewState::RemoteRegionsLost);
    }

    #[test]
    fn a_node_without_a_region_only_watches_the_store() {
        let mut t = ViewTracker::new(None, TTL);
        let (now, wall) = (Instant::now(), SystemTime::now());
        t.on_discovery(now, wall, &regions(&["eu-west"]), &[]);
        assert!(t
            .on_discovery(now + Duration::from_secs(5), wall, &[], &[quiet("eu-west")])
            .is_none());
        assert!(t.on_failure(now + Duration::from_secs(10), wall).is_some());
    }

    #[test]
    fn the_series_carry_every_state() {
        let (mut t, now, wall) = tracker();
        t.on_discovery(now, wall, &[], &[]);
        t.on_failure(now, wall);
        let body = t.view(now).prometheus("node=\"cm-1\"");
        assert!(body.contains("fcp_cluster_view{node=\"cm-1\",state=\"store_unreachable\"} 1"));
        assert!(body.contains("fcp_cluster_view{node=\"cm-1\",state=\"whole\"} 0"));
        assert!(body.contains("fcp_cluster_view_age_seconds{node=\"cm-1\"} 0"));
        assert!(!ViewTracker::new(None, TTL)
            .view(now)
            .prometheus("")
            .contains("age"));
    }

    #[test]
    fn a_whole_view_serialises_small() {
        let (t, now, _) = tracker();
        let json = serde_json::to_string(&t.view(now)).unwrap();
        assert_eq!(
            json,
            r#"{"state":"whole","summary":"the cluster view looks whole"}"#
        );
    }
}
