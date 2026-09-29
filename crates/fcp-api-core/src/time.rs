//! Times in the API as RFC 3339 (docs/DEVELOPER_SURFACE.md §3, phase 2b).
//!
//! A response field keeps whatever type the handler has — a `SystemTime`,
//! or epoch seconds or milliseconds as an integer — and names one of these
//! modules in `#[serde(with = …)]`, so it is written as an RFC 3339 string
//! in UTC (`2026-09-29T14:31:44Z`, with fractional seconds only when there
//! are any). Reading accepts the RFC 3339 string and, for a client still
//! sending the old forms, the number the field used to be.

use serde::{de, Deserialize, Deserializer, Serializer};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// RFC 3339 for a moment given as a duration since the epoch.
pub fn format(since_epoch: Duration) -> String {
    let secs = since_epoch.as_secs() as i64;
    let nanos = since_epoch.subsec_nanos();
    let format = if nanos == 0 {
        chrono::SecondsFormat::Secs
    } else if nanos.is_multiple_of(1_000_000) {
        chrono::SecondsFormat::Millis
    } else {
        chrono::SecondsFormat::AutoSi
    };
    chrono::DateTime::from_timestamp(secs, nanos)
        .unwrap_or_default()
        .to_rfc3339_opts(format, true)
}

/// RFC 3339 for a `SystemTime` (a time before the epoch is the epoch).
pub fn format_system_time(time: SystemTime) -> String {
    format(time.duration_since(UNIX_EPOCH).unwrap_or_default())
}

/// A time as it may arrive: RFC 3339, or the number of `unit`s since the
/// epoch that the field used to carry.
#[derive(Deserialize)]
#[serde(untagged)]
enum Incoming {
    Text(String),
    Number(f64),
}

fn parse<E: de::Error>(incoming: Incoming, unit: Duration) -> Result<Duration, E> {
    match incoming {
        Incoming::Text(text) => {
            let dt = chrono::DateTime::parse_from_rfc3339(text.trim())
                .map_err(|e| E::custom(format!("expected an RFC 3339 time: {e}")))?;
            let secs = dt.timestamp();
            if secs < 0 {
                return Err(E::custom("a time before 1970"));
            }
            Ok(Duration::new(secs as u64, dt.timestamp_subsec_nanos()))
        }
        Incoming::Number(n) if n >= 0.0 && n.is_finite() => Ok(unit.mul_f64(n)),
        Incoming::Number(_) => Err(E::custom("a time before 1970")),
    }
}

/// A `SystemTime` field.
pub mod rfc3339 {
    use super::*;

    pub fn serialize<S: Serializer>(time: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format_system_time(*time))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SystemTime, D::Error> {
        let since = parse(Incoming::deserialize(d)?, Duration::from_secs(1))?;
        Ok(UNIX_EPOCH + since)
    }
}

/// An `Option<SystemTime>` field.
pub mod rfc3339_opt {
    use super::*;

    pub fn serialize<S: Serializer>(time: &Option<SystemTime>, s: S) -> Result<S::Ok, S::Error> {
        match time {
            Some(t) => s.serialize_str(&format_system_time(*t)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<SystemTime>, D::Error> {
        Option::<Incoming>::deserialize(d)?
            .map(|i| parse(i, Duration::from_secs(1)).map(|since| UNIX_EPOCH + since))
            .transpose()
    }
}

macro_rules! epoch_module {
    ($name:ident, $opt:ident, $ty:ty, $unit:expr, $to_duration:expr, $from_duration:expr) => {
        /// An integer since the epoch, written as RFC 3339.
        pub mod $name {
            use super::*;

            pub fn serialize<S: Serializer>(value: &$ty, s: S) -> Result<S::Ok, S::Error> {
                let to: fn($ty) -> Duration = $to_duration;
                s.serialize_str(&format(to(*value)))
            }

            pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<$ty, D::Error> {
                let from: fn(Duration) -> $ty = $from_duration;
                Ok(from(parse(Incoming::deserialize(d)?, $unit)?))
            }
        }

        /// An optional integer since the epoch, written as RFC 3339.
        pub mod $opt {
            use super::*;

            pub fn serialize<S: Serializer>(value: &Option<$ty>, s: S) -> Result<S::Ok, S::Error> {
                let to: fn($ty) -> Duration = $to_duration;
                match value {
                    Some(v) => s.serialize_str(&format(to(*v))),
                    None => s.serialize_none(),
                }
            }

            pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<$ty>, D::Error> {
                let from: fn(Duration) -> $ty = $from_duration;
                Option::<Incoming>::deserialize(d)?
                    .map(|i| parse(i, $unit).map(from))
                    .transpose()
            }
        }
    };
}

epoch_module!(
    epoch_secs,
    epoch_secs_opt,
    u64,
    Duration::from_secs(1),
    |v| Duration::from_secs(v),
    |d| d.as_secs()
);
epoch_module!(
    epoch_secs_i64,
    epoch_secs_i64_opt,
    i64,
    Duration::from_secs(1),
    |v| Duration::from_secs(v.max(0) as u64),
    |d| d.as_secs() as i64
);
epoch_module!(
    epoch_millis,
    epoch_millis_opt,
    u64,
    Duration::from_millis(1),
    |v| Duration::from_millis(v),
    |d| d.as_millis() as u64
);
epoch_module!(
    epoch_millis_i64,
    epoch_millis_i64_opt,
    i64,
    Duration::from_millis(1),
    |v| Duration::from_millis(v.max(0) as u64),
    |d| d.as_millis() as i64
);
epoch_module!(
    epoch_secs_f64,
    epoch_secs_f64_opt,
    f64,
    Duration::from_secs(1),
    |v| Duration::from_secs_f64(v.max(0.0)),
    |d| d.as_secs_f64()
);

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Row {
        #[serde(with = "rfc3339")]
        at: SystemTime,
        #[serde(with = "rfc3339_opt")]
        gone: Option<SystemTime>,
        #[serde(with = "epoch_secs")]
        secs: u64,
        #[serde(with = "epoch_millis_opt")]
        millis: Option<u64>,
    }

    #[test]
    fn times_are_written_as_rfc3339_and_read_either_way() {
        let row = Row {
            at: UNIX_EPOCH + Duration::from_secs(1_790_000_000),
            gone: None,
            secs: 1_790_000_000,
            millis: Some(1_790_000_000_250),
        };
        let json = serde_json::to_value(&row).unwrap();
        assert_eq!(json["at"], "2026-09-21T14:13:20Z");
        assert!(json["gone"].is_null());
        assert_eq!(json["secs"], "2026-09-21T14:13:20Z");
        assert_eq!(json["millis"], "2026-09-21T14:13:20.250Z");
        assert_eq!(serde_json::from_value::<Row>(json).unwrap(), row);

        // The old forms still read.
        let old = serde_json::json!({
            "at": 1_790_000_000u64, "gone": null,
            "secs": 1_790_000_000u64, "millis": 1_790_000_000_250u64,
        });
        assert_eq!(serde_json::from_value::<Row>(old).unwrap(), row);
        let offset = serde_json::json!({
            "at": "2026-09-21T16:13:20+02:00", "gone": null,
            "secs": "2026-09-21T14:13:20Z", "millis": null,
        });
        let parsed: Row = serde_json::from_value(offset).unwrap();
        assert_eq!(parsed.at, row.at);
        assert!(serde_json::from_value::<Row>(serde_json::json!({
            "at": "yesterday", "gone": null, "secs": 0, "millis": null,
        }))
        .is_err());
    }
}
