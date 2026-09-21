//! What a run does to the node (`docs/CAPACITY_AND_SURVIVABILITY.md` §8).

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// The four shapes of load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Profile {
    /// N devices register at once, then re-REGISTER on their expiry: what
    /// a registrar meets when a subnet comes back after an outage.
    RegisterStorm,
    /// Offered calls per second climb a ladder until a stop condition is
    /// met; the report names the last rung that held.
    CallRamp,
    /// N concurrent calls with a tone each way for a while: what holding
    /// them costs in memory, CPU and media quality.
    Soak,
    /// A stated ratio of the three at once, which is what a real PBX
    /// sees.
    Mixed,
}

impl Profile {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RegisterStorm => "register-storm",
            Self::CallRamp => "call-ramp",
            Self::Soak => "soak",
            Self::Mixed => "mixed",
        }
    }
}

impl fmt::Display for Profile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Profile {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "register-storm" | "register" | "storm" => Ok(Self::RegisterStorm),
            "call-ramp" | "ramp" => Ok(Self::CallRamp),
            "soak" => Ok(Self::Soak),
            "mixed" => Ok(Self::Mixed),
            other => Err(format!(
                "unknown profile '{other}': register-storm, call-ramp, soak or mixed"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_round_trip_through_their_names() {
        for p in [
            Profile::RegisterStorm,
            Profile::CallRamp,
            Profile::Soak,
            Profile::Mixed,
        ] {
            assert_eq!(p.as_str().parse::<Profile>(), Ok(p));
        }
        assert_eq!("ramp".parse(), Ok(Profile::CallRamp));
        assert!("stampede".parse::<Profile>().is_err());
    }
}
