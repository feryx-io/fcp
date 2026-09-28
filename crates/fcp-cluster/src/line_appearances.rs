//! A shared line's appearances across the cluster
//! (`docs/ENTERPRISE_CALLING.md` §3.9, Q9).
//!
//! Every node's phones must see every node's calls on a line, so the
//! appearances live here, one hash per line:
//!
//! ```text
//! fcp:sca:<tenant-id>:<line-binding-id>   index → appearance (JSON)
//! ```
//!
//! An appearance carries its own expiry: a seized index lapses in seconds
//! if no INVITE follows, and a call's lapses half an hour after its node
//! last refreshed it, so a node that dies mid-call frees its keys without
//! anyone tidying up. A claim is one Lua script — read the index, decide,
//! write — so two nodes cannot both take an index.

use fcp_core::sca::Appearance;
use fcp_storage_redis::RedisStorage;
use std::time::{SystemTime, UNIX_EPOCH};

/// How long a line's hash stands after its last write: past every entry's
/// own expiry, so an idle line leaves nothing behind.
const LINE_KEY_TTL_SECS: i64 = 2 * 60 * 60;

/// Claim an index: `ARGV` = index (0 for the lowest that will take it),
/// max, now, owner, replacing (empty for none), entry JSON without its
/// index. Returns the index taken, or 0.
const CLAIM: &str = r#"
local want = tonumber(ARGV[1])
local max = tonumber(ARGV[2])
local now = tonumber(ARGV[3])
local owner = ARGV[4]
local replacing = ARGV[5]
local first, last = want, want
if want == 0 then first, last = 1, max end
for i = first, last do
  local current = redis.call('HGET', KEYS[1], tostring(i))
  local free = true
  if current then
    local ok, entry = pcall(cjson.decode, current)
    if ok and type(entry) == 'table' then
      local lapsed = tonumber(entry['expires_at'] or 0) <= now
      local mine = entry['owner'] == owner or (replacing ~= '' and entry['owner'] == replacing)
      free = lapsed or mine
    end
  end
  if free then
    redis.call('HSET', KEYS[1], tostring(i), ARGV[6])
    redis.call('EXPIRE', KEYS[1], tonumber(ARGV[7]))
    return i
  end
end
return 0
"#;

/// Free an index if its owner is the one asking: `ARGV` = index, owner.
const RELEASE: &str = r#"
local current = redis.call('HGET', KEYS[1], ARGV[1])
if not current then return 0 end
local ok, entry = pcall(cjson.decode, current)
if (not ok) or type(entry) ~= 'table' or entry['owner'] == ARGV[2] then
  redis.call('HDEL', KEYS[1], ARGV[1])
  return 1
end
return 0
"#;

/// The cluster's appearances of its shared lines.
#[derive(Clone)]
pub struct LineAppearances {
    storage: RedisStorage,
}

impl LineAppearances {
    pub fn new(storage: RedisStorage) -> Self {
        Self { storage }
    }

    fn key(&self, tenant: &str, line: &str) -> String {
        self.storage.key(&format!("sca:{tenant}:{line}"))
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// The line's live appearances, lapsed ones left out.
    pub async fn list(&self, tenant: &str, line: &str) -> anyhow::Result<Vec<Appearance>> {
        let key = self.key(tenant, line);
        let mut conn = self.storage.connection();
        let entries: Vec<(String, String)> = redis::cmd("HGETALL")
            .arg(&key)
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("appearances: {e}"))?;
        let now = Self::now();
        let mut out = Vec::new();
        for (index, json) in entries {
            let Ok(index) = index.parse::<u8>() else {
                continue;
            };
            match serde_json::from_str::<Appearance>(&json) {
                Ok(mut a) if !a.lapsed(now) => {
                    a.index = index;
                    out.push(a);
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(key = %key, index, error = %e, "An appearance could not be read")
                }
            }
        }
        out.sort_by_key(|a| a.index);
        Ok(out)
    }

    /// Put `entry` at `index` (or the lowest of 1..=`max` that will take it
    /// when `None`) if the index is free, lapsed, `entry.owner`'s or
    /// `replacing`'s. The index taken, or `None`.
    pub async fn claim(
        &self,
        tenant: &str,
        line: &str,
        index: Option<u8>,
        max: u8,
        mut entry: Appearance,
        replacing: Option<&str>,
    ) -> anyhow::Result<Option<u8>> {
        entry.index = 0;
        let json = serde_json::to_string(&entry)?;
        let mut conn = self.storage.connection();
        let taken: i64 = redis::Script::new(CLAIM)
            .key(self.key(tenant, line))
            .arg(index.unwrap_or(0))
            .arg(max)
            .arg(Self::now())
            .arg(&entry.owner)
            .arg(replacing.unwrap_or(""))
            .arg(json)
            .arg(LINE_KEY_TTL_SECS)
            .invoke_async(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("appearance claim: {e}"))?;
        Ok(u8::try_from(taken).ok().filter(|i| *i > 0))
    }

    /// Free `index` if `owner` holds it.
    pub async fn release(
        &self,
        tenant: &str,
        line: &str,
        index: u8,
        owner: &str,
    ) -> anyhow::Result<()> {
        let mut conn = self.storage.connection();
        let _: i64 = redis::Script::new(RELEASE)
            .key(self.key(tenant, line))
            .arg(index)
            .arg(owner)
            .invoke_async(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("appearance release: {e}"))?;
        Ok(())
    }
}
