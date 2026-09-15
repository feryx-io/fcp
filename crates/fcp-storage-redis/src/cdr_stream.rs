//! The CDR stream (`docs/CDR_PIPELINE.md` §4.2): nodes append their spooled
//! records to it, the processor in `fcp-jobs` consumes it through a
//! consumer group.
//!
//! Unlike the event stream it is never trimmed by length. The consumer
//! trims it by id: entries older than the replay window *and* older than
//! the oldest entry the group has not finished (pending, or not yet
//! delivered), so nothing is dropped before it is stored.
//!
//! Keys, under the storage's prefix: `cdrs` (the stream; one field,
//! `record`, the record's JSON) and `cdrs:dead` (entries that will never be
//! stored: fields `id`, `reason`, `record`).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use fcp_cdr::consumer::{RecordSource, SourceStatus, SourcedRecord, Undecodable};
use fcp_cdr::sink::Sink;
use fcp_core::storage::CallDetailRecord;
use tracing::{debug, info};

use crate::RedisStorage;

/// The consumer group every processor joins.
pub const DEFAULT_GROUP: &str = "cdr-processor";

/// Dead letters kept (`XADD MAXLEN ~`): a bound, not a retention policy.
const DEAD_LETTER_MAXLEN: usize = 100_000;

/// The CDR stream on one Redis.
pub struct RedisCdrStream {
    storage: RedisStorage,
    stream_key: String,
    dead_key: String,
    group: String,
    consumer: String,
    retain: Duration,
    appended: AtomicU64,
}

impl RedisCdrStream {
    /// The stream under `storage`'s prefix, read as `consumer` in the
    /// default group, keeping acknowledged entries for `retain`.
    pub fn new(storage: RedisStorage, consumer: impl Into<String>, retain: Duration) -> Self {
        let stream_key = storage.key("cdrs");
        let dead_key = storage.key("cdrs:dead");
        Self {
            storage,
            stream_key,
            dead_key,
            group: DEFAULT_GROUP.to_string(),
            consumer: consumer.into(),
            retain,
            appended: AtomicU64::new(0),
        }
    }

    /// Join a different consumer group.
    pub fn with_group(mut self, group: impl Into<String>) -> Self {
        self.group = group.into();
        self
    }

    pub fn stream_key(&self) -> &str {
        &self.stream_key
    }

    pub fn dead_key(&self) -> &str {
        &self.dead_key
    }

    /// Records appended by this handle.
    pub fn appended(&self) -> u64 {
        self.appended.load(Ordering::Relaxed)
    }

    /// Append records, all or none (one `MULTI`).
    pub async fn append(&self, records: &[CallDetailRecord]) -> anyhow::Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let mut pipe = redis::pipe();
        pipe.atomic();
        for record in records {
            let json = serde_json::to_string(record)?;
            pipe.cmd("XADD")
                .arg(&self.stream_key)
                .arg("*")
                .arg("record")
                .arg(json)
                .ignore();
        }
        let mut conn = self.storage.connection();
        let _: () = pipe
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("XADD to {} failed: {e}", self.stream_key))?;
        self.appended
            .fetch_add(records.len() as u64, Ordering::Relaxed);
        Ok(())
    }

    /// The id of the oldest entry the group has not finished with: its
    /// oldest pending entry, else the last it delivered. `None` when the
    /// group does not exist.
    async fn unfinished_boundary(&self) -> anyhow::Result<Option<String>> {
        let mut conn = self.storage.connection();
        // XPENDING key group → [count, smallest, greatest, consumers]
        let pending: redis::RedisResult<redis::Value> = redis::cmd("XPENDING")
            .arg(&self.stream_key)
            .arg(&self.group)
            .query_async(&mut conn)
            .await;
        match pending {
            Ok(redis::Value::Array(parts)) if parts.len() >= 2 => {
                if int(&parts[0]).unwrap_or(0) > 0 {
                    if let Some(id) = text(&parts[1]) {
                        return Ok(Some(id));
                    }
                }
            }
            Ok(_) => {}
            Err(e) if e.to_string().contains("NOGROUP") => return Ok(None),
            Err(e) => return Err(anyhow::anyhow!("XPENDING failed: {e}")),
        }
        // Nothing pending: everything up to the last delivered id is done.
        let groups: redis::Value = redis::cmd("XINFO")
            .arg("GROUPS")
            .arg(&self.stream_key)
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("XINFO GROUPS failed: {e}"))?;
        let redis::Value::Array(groups) = groups else {
            return Ok(None);
        };
        for group in groups {
            let fields = fields_of(&group);
            if fields.get("name").map(String::as_str) == Some(self.group.as_str()) {
                return Ok(fields.get("last-delivered-id").cloned());
            }
        }
        Ok(None)
    }

    fn decode(id: String, raw: String) -> SourcedRecord {
        let record = serde_json::from_str::<CallDetailRecord>(&raw).map_err(|e| Undecodable {
            reason: format!("record does not decode: {e}"),
            raw: raw.clone(),
        });
        SourcedRecord { id, record }
    }
}

#[async_trait]
impl Sink for RedisCdrStream {
    async fn ship(&self, records: Vec<CallDetailRecord>) -> fcp_cdr::Result<()> {
        self.append(&records)
            .await
            .map_err(|e| fcp_cdr::Error::Sink(e.to_string()))
    }

    fn name(&self) -> &'static str {
        "stream"
    }
}

#[async_trait]
impl RecordSource for RedisCdrStream {
    async fn prepare(&self) -> anyhow::Result<()> {
        let mut conn = self.storage.connection();
        let result: redis::RedisResult<()> = redis::cmd("XGROUP")
            .arg("CREATE")
            .arg(&self.stream_key)
            .arg(&self.group)
            .arg("0")
            .arg("MKSTREAM")
            .query_async(&mut conn)
            .await;
        match result {
            Ok(()) => {
                info!(stream = %self.stream_key, group = %self.group, "Created the CDR consumer group");
                Ok(())
            }
            Err(e) if e.to_string().contains("BUSYGROUP") => Ok(()),
            Err(e) => Err(anyhow::anyhow!("XGROUP CREATE failed: {e}")),
        }
    }

    async fn read(&self, count: usize, block: Duration) -> anyhow::Result<Vec<SourcedRecord>> {
        let mut conn = self.storage.connection();
        let value: redis::Value = redis::cmd("XREADGROUP")
            .arg("GROUP")
            .arg(&self.group)
            .arg(&self.consumer)
            .arg("COUNT")
            .arg(count.max(1))
            .arg("BLOCK")
            // BLOCK 0 would wait forever.
            .arg(block.as_millis().max(1) as u64)
            .arg("STREAMS")
            .arg(&self.stream_key)
            .arg(">")
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("XREADGROUP failed: {e}"))?;
        // [[stream, [[id, [field, value, ...]], ...]]], or Nil on timeout.
        let redis::Value::Array(streams) = value else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for stream in streams {
            if let redis::Value::Array(parts) = stream {
                if let Some(redis::Value::Array(entries)) = parts.get(1) {
                    out.extend(entries_of(entries));
                }
            }
        }
        Ok(out)
    }

    async fn claim_stale(
        &self,
        idle: Duration,
        count: usize,
    ) -> anyhow::Result<Vec<SourcedRecord>> {
        let mut conn = self.storage.connection();
        // XAUTOCLAIM → [next start, entries, (deleted ids)]
        let value: redis::Value = redis::cmd("XAUTOCLAIM")
            .arg(&self.stream_key)
            .arg(&self.group)
            .arg(&self.consumer)
            .arg(idle.as_millis() as u64)
            .arg("0-0")
            .arg("COUNT")
            .arg(count.max(1))
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("XAUTOCLAIM failed: {e}"))?;
        let redis::Value::Array(parts) = value else {
            return Ok(Vec::new());
        };
        Ok(match parts.get(1) {
            Some(redis::Value::Array(entries)) => entries_of(entries),
            _ => Vec::new(),
        })
    }

    async fn ack(&self, ids: &[String]) -> anyhow::Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let mut conn = self.storage.connection();
        let mut cmd = redis::cmd("XACK");
        cmd.arg(&self.stream_key).arg(&self.group);
        for id in ids {
            cmd.arg(id);
        }
        let _: u64 = cmd
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("XACK failed: {e}"))?;
        Ok(())
    }

    async fn dead_letter(&self, id: &str, raw: &str, reason: &str) -> anyhow::Result<()> {
        let mut conn = self.storage.connection();
        let _: String = redis::cmd("XADD")
            .arg(&self.dead_key)
            .arg("MAXLEN")
            .arg("~")
            .arg(DEAD_LETTER_MAXLEN)
            .arg("*")
            .arg("id")
            .arg(id)
            .arg("reason")
            .arg(reason)
            .arg("record")
            .arg(raw)
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("XADD to {} failed: {e}", self.dead_key))?;
        Ok(())
    }

    async fn trim(&self) -> anyhow::Result<u64> {
        let Some(boundary) = self.unfinished_boundary().await? else {
            return Ok(0);
        };
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let cutoff = format!(
            "{}-0",
            now_ms.saturating_sub(self.retain.as_millis() as u64)
        );
        let min_id = earlier_id(&boundary, &cutoff);
        let mut conn = self.storage.connection();
        let dropped: u64 = redis::cmd("XTRIM")
            .arg(&self.stream_key)
            .arg("MINID")
            .arg(&min_id)
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("XTRIM failed: {e}"))?;
        debug!(min_id = %min_id, dropped, "CDR stream trimmed");
        Ok(dropped)
    }

    async fn status(&self) -> anyhow::Result<SourceStatus> {
        let mut conn = self.storage.connection();
        let length: u64 = redis::cmd("XLEN")
            .arg(&self.stream_key)
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("XLEN failed: {e}"))?;
        let dead_letters: u64 = redis::cmd("XLEN")
            .arg(&self.dead_key)
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("XLEN failed: {e}"))?;
        let pending = match redis::cmd("XPENDING")
            .arg(&self.stream_key)
            .arg(&self.group)
            .query_async::<redis::Value>(&mut conn)
            .await
        {
            Ok(redis::Value::Array(parts)) => parts.first().and_then(int).unwrap_or(0) as u64,
            _ => 0,
        };
        Ok(SourceStatus {
            length,
            pending,
            dead_letters,
        })
    }
}

/// The dead letters, for an erasure or a tenant's deletion to find what
/// names a subject (`docs/COMPLIANCE_FOLLOWUPS.md` §6.3).
#[async_trait]
impl fcp_core::subject_erasure::DeadLetterStore for RedisCdrStream {
    async fn dead_letter_page(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<(Vec<(String, String)>, Option<String>)> {
        let mut conn = self.storage.connection();
        let start = match after {
            Some(id) => format!("({id}"),
            None => "-".to_string(),
        };
        let reply: redis::Value = redis::cmd("XRANGE")
            .arg(&self.dead_key)
            .arg(start)
            .arg("+")
            .arg("COUNT")
            .arg(limit.max(1))
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("XRANGE on {} failed: {e}", self.dead_key))?;
        let redis::Value::Array(entries) = reply else {
            return Ok((Vec::new(), None));
        };
        let mut out = Vec::with_capacity(entries.len());
        for entry in &entries {
            let redis::Value::Array(parts) = entry else {
                continue;
            };
            let Some(id) = parts.first().and_then(text) else {
                continue;
            };
            let record = parts
                .get(1)
                .map(fields_of)
                .and_then(|mut fields| fields.remove("record"))
                .unwrap_or_default();
            out.push((id, record));
        }
        let next = (entries.len() >= limit.max(1))
            .then(|| out.last().map(|(id, _)| id.clone()))
            .flatten();
        Ok((out, next))
    }

    async fn delete_dead_letters(&self, ids: &[String]) -> anyhow::Result<u64> {
        if ids.is_empty() {
            return Ok(0);
        }
        let mut conn = self.storage.connection();
        let mut cmd = redis::cmd("XDEL");
        cmd.arg(&self.dead_key);
        for id in ids {
            cmd.arg(id);
        }
        let removed: u64 = cmd
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("XDEL on {} failed: {e}", self.dead_key))?;
        Ok(removed)
    }
}

/// `[[id, [field, value, ...]], ...]` as records, by their `record` field.
fn entries_of(entries: &[redis::Value]) -> Vec<SourcedRecord> {
    let mut out = Vec::new();
    for entry in entries {
        let redis::Value::Array(parts) = entry else {
            continue;
        };
        let Some(id) = parts.first().and_then(text) else {
            continue;
        };
        let raw = match parts.get(1) {
            Some(redis::Value::Array(fields)) => fields
                .chunks(2)
                .find(|pair| text(&pair[0]).as_deref() == Some("record"))
                .and_then(|pair| pair.get(1))
                .and_then(text),
            // An entry deleted from under the group comes back without fields.
            _ => None,
        };
        out.push(match raw {
            Some(raw) => RedisCdrStream::decode(id, raw),
            None => SourcedRecord {
                id,
                record: Err(Undecodable {
                    raw: String::new(),
                    reason: "entry has no record field".into(),
                }),
            },
        });
    }
    out
}

/// A flat `[name, value, ...]` reply (or a RESP3 map) as a map of strings.
fn fields_of(value: &redis::Value) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    match value {
        redis::Value::Array(items) => {
            for pair in items.chunks(2) {
                if let (Some(k), Some(v)) = (text(&pair[0]), pair.get(1).and_then(text_or_int)) {
                    map.insert(k, v);
                }
            }
        }
        redis::Value::Map(items) => {
            for (k, v) in items {
                if let (Some(k), Some(v)) = (text(k), text_or_int(v)) {
                    map.insert(k, v);
                }
            }
        }
        _ => {}
    }
    map
}

fn text(value: &redis::Value) -> Option<String> {
    match value {
        redis::Value::BulkString(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
        redis::Value::SimpleString(s) => Some(s.clone()),
        _ => None,
    }
}

fn text_or_int(value: &redis::Value) -> Option<String> {
    text(value).or_else(|| int(value).map(|i| i.to_string()))
}

fn int(value: &redis::Value) -> Option<i64> {
    match value {
        redis::Value::Int(i) => Some(*i),
        other => text(other).and_then(|s| s.parse().ok()),
    }
}

/// The earlier of two stream ids (`<ms>-<seq>`).
fn earlier_id(a: &str, b: &str) -> String {
    let key = |id: &str| -> (u64, u64) {
        let (ms, seq) = id.split_once('-').unwrap_or((id, "0"));
        (ms.parse().unwrap_or(0), seq.parse().unwrap_or(0))
    };
    if key(a) <= key(b) {
        a.to_string()
    } else {
        b.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_earlier_stream_id_compares_by_milliseconds_then_sequence() {
        assert_eq!(
            earlier_id("1700000000000-5", "1700000000001-0"),
            "1700000000000-5"
        );
        assert_eq!(
            earlier_id("1700000000000-10", "1700000000000-9"),
            "1700000000000-9"
        );
        assert_eq!(earlier_id("99-0", "100-0"), "99-0");
    }
}
