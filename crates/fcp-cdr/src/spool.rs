//! The local spool: an append-only directory of JSON Lines segments that
//! every record goes to before the hangup path moves on.
//!
//! `append` is a synchronous write into the page cache, which survives a
//! process crash; a task syncs the open segment to disk every
//! `fsync_interval`, so a power loss can take at most one interval. The
//! shipper reads from a [`Cursor`] and commits it only after the sink has
//! acknowledged a batch; a segment is deleted once the cursor has left it.
//! A crash between the acknowledgement and the commit ships a batch twice,
//! which the record's `cdr_id` absorbs downstream.
//!
//! Layout of the directory:
//!
//! ```text
//! 00000000000000000000.jsonl   segments, one record per line
//! 00000000000000000001.jsonl
//! cursor                       "<segment> <byte offset>"
//! dead.jsonl                   lines that did not decode, kept for a human
//! ```

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use fcp_core::storage::CallDetailRecord;
use tokio::sync::Notify;
use tracing::{debug, error, warn};

use crate::Result;

const SEGMENT_SUFFIX: &str = ".jsonl";
const CURSOR_FILE: &str = "cursor";
const DEAD_FILE: &str = "dead.jsonl";

/// How the spool writes.
#[derive(Debug, Clone)]
pub struct SpoolConfig {
    /// How often the open segment is synced to disk.
    pub fsync_interval: Duration,
    /// A segment is rolled once it holds this many bytes.
    pub segment_bytes: u64,
}

impl Default for SpoolConfig {
    fn default() -> Self {
        Self {
            fsync_interval: Duration::from_millis(100),
            segment_bytes: 16 * 1024 * 1024,
        }
    }
}

/// Where the shipper is in the spool: the next record to ship starts at
/// `offset` in `segment`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Cursor {
    pub segment: u64,
    pub offset: u64,
}

/// What the spool holds that has not been shipped.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SpoolStatus {
    /// Bytes behind the cursor.
    pub pending_bytes: u64,
    /// Segments on disk.
    pub segments: usize,
    /// How long the earliest unshipped segment has waited, when something
    /// is pending.
    pub oldest_pending_age: Option<Duration>,
}

struct Writer {
    segment: u64,
    file: File,
    len: u64,
}

struct Inner {
    dir: PathBuf,
    config: SpoolConfig,
    writer: Mutex<Writer>,
    dirty: AtomicBool,
    notify: Notify,
}

/// The spool. Cheap to clone; every clone is the same spool.
#[derive(Clone)]
pub struct Spool {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Spool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Spool")
            .field("dir", &self.inner.dir)
            .finish()
    }
}

impl Spool {
    /// Open the spool at `dir`, creating it, and continue the last segment
    /// found there. A partial last line (a crash mid-write) is truncated
    /// away, since it was never a shipped record.
    pub fn open(dir: impl Into<PathBuf>, config: SpoolConfig) -> Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        let segment = list_segments(&dir)?.last().copied().unwrap_or(0);
        let path = segment_path(&dir, segment);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&path)?;
        let mut len = file.metadata()?.len();
        if let Some(cut) = partial_tail(&mut file, len)? {
            warn!(path = %path.display(), bytes = len - cut, "Truncating a partial record left by a crash");
            file.set_len(cut)?;
            len = cut;
        }
        Ok(Self {
            inner: Arc::new(Inner {
                dir,
                config,
                writer: Mutex::new(Writer { segment, file, len }),
                dirty: AtomicBool::new(false),
                notify: Notify::new(),
            }),
        })
    }

    /// The directory the spool lives in.
    pub fn dir(&self) -> &Path {
        &self.inner.dir
    }

    /// Append one record. Synchronous: the line is in the page cache when
    /// this returns.
    pub fn append(&self, record: &CallDetailRecord) -> Result<()> {
        let mut line = serde_json::to_vec(record)?;
        line.push(b'\n');
        let mut writer = self.inner.writer.lock().unwrap_or_else(|e| e.into_inner());
        if writer.len > 0 && writer.len + line.len() as u64 > self.inner.config.segment_bytes {
            self.roll(&mut writer)?;
        }
        writer.file.write_all(&line)?;
        writer.len += line.len() as u64;
        self.inner.dirty.store(true, Ordering::Release);
        self.inner.notify.notify_one();
        Ok(())
    }

    fn roll(&self, writer: &mut Writer) -> Result<()> {
        writer.file.sync_data()?;
        let next = writer.segment + 1;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(segment_path(&self.inner.dir, next))?;
        debug!(segment = next, "Rolled spool segment");
        *writer = Writer {
            segment: next,
            file,
            len: 0,
        };
        Ok(())
    }

    /// Sync the open segment if anything was appended since the last sync.
    pub fn sync(&self) -> Result<()> {
        if self.inner.dirty.swap(false, Ordering::AcqRel) {
            let writer = self.inner.writer.lock().unwrap_or_else(|e| e.into_inner());
            writer.file.sync_data()?;
        }
        Ok(())
    }

    /// Start the task that syncs the open segment every `fsync_interval`.
    /// Needs a tokio runtime.
    pub fn start_fsync(&self) -> tokio::task::JoinHandle<()> {
        let spool = self.clone();
        let interval = self.inner.config.fsync_interval;
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                let s = spool.clone();
                let result = tokio::task::spawn_blocking(move || s.sync()).await;
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => error!(error = %e, "Spool fsync failed"),
                    Err(e) => error!(error = %e, "Spool fsync task panicked"),
                }
            }
        })
    }

    /// Resolves when a record has been appended since the last call, or
    /// immediately if one was appended before this call.
    pub async fn notified(&self) {
        self.inner.notify.notified().await;
    }

    /// The committed cursor, or the start of the spool.
    pub fn cursor(&self) -> Result<Cursor> {
        let path = self.inner.dir.join(CURSOR_FILE);
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Cursor::default()),
            Err(e) => return Err(e.into()),
        };
        let mut parts = text.split_whitespace();
        let segment = parts.next().and_then(|s| s.parse().ok());
        let offset = parts.next().and_then(|s| s.parse().ok());
        match (segment, offset) {
            (Some(segment), Some(offset)) => Ok(Cursor { segment, offset }),
            _ => {
                warn!(path = %path.display(), "Unreadable spool cursor; starting from the beginning");
                Ok(Cursor::default())
            }
        }
    }

    /// Up to `max` records from `cursor` on, and the cursor after them.
    /// Reads across segment boundaries; stops at a partial last line. A
    /// line that does not decode goes to `dead.jsonl` and is skipped.
    pub fn read_from(&self, cursor: Cursor, max: usize) -> Result<(Vec<CallDetailRecord>, Cursor)> {
        let mut out = Vec::new();
        let mut at = cursor;
        let open_segment = self.open_segment();
        while out.len() < max {
            let path = segment_path(&self.inner.dir, at.segment);
            let file = match File::open(&path) {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound && at.segment < open_segment => {
                    // A segment removed from under us: skip to the next one.
                    at = Cursor {
                        segment: at.segment + 1,
                        offset: 0,
                    };
                    continue;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                Err(e) => return Err(e.into()),
            };
            let mut reader = BufReader::new(file);
            reader.seek(SeekFrom::Start(at.offset))?;
            let mut hit_end = false;
            while out.len() < max {
                let mut line = Vec::new();
                let n = reader.read_until(b'\n', &mut line)?;
                if n == 0 || line.last() != Some(&b'\n') {
                    // End of segment, or a record still being written.
                    hit_end = true;
                    break;
                }
                at.offset += n as u64;
                match serde_json::from_slice::<CallDetailRecord>(&line) {
                    Ok(record) => out.push(record),
                    Err(e) => {
                        error!(error = %e, segment = at.segment, "Spool line did not decode; kept in dead.jsonl");
                        self.dead_letter(&line)?;
                    }
                }
            }
            if !hit_end {
                continue;
            }
            if at.segment < open_segment {
                at = Cursor {
                    segment: at.segment + 1,
                    offset: 0,
                };
                continue;
            }
            break;
        }
        Ok((out, at))
    }

    /// Keep a record the processor refused in `dead.jsonl`, with the
    /// reason, for a person to look at.
    pub fn dead_letter_record(&self, record: &CallDetailRecord, reason: &str) -> Result<()> {
        let mut line = serde_json::to_vec(&serde_json::json!({
            "reason": reason,
            "record": record,
        }))?;
        line.push(b'\n');
        self.dead_letter(&line)
    }

    fn dead_letter(&self, line: &[u8]) -> Result<()> {
        let mut dead = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.inner.dir.join(DEAD_FILE))?;
        dead.write_all(line)?;
        Ok(())
    }

    /// Record that everything before `cursor` has been shipped, and delete
    /// the segments it has left.
    pub fn commit(&self, cursor: Cursor) -> Result<()> {
        let tmp = self.inner.dir.join("cursor.tmp");
        {
            let mut f = File::create(&tmp)?;
            writeln!(f, "{} {}", cursor.segment, cursor.offset)?;
            f.sync_data()?;
        }
        fs::rename(&tmp, self.inner.dir.join(CURSOR_FILE))?;
        for segment in list_segments(&self.inner.dir)? {
            if segment < cursor.segment {
                let path = segment_path(&self.inner.dir, segment);
                if let Err(e) = fs::remove_file(&path) {
                    warn!(path = %path.display(), error = %e, "Could not delete a shipped spool segment");
                }
            }
        }
        Ok(())
    }

    /// What is waiting to be shipped.
    pub fn status(&self) -> Result<SpoolStatus> {
        let cursor = self.cursor()?;
        let segments = list_segments(&self.inner.dir)?;
        let mut pending_bytes = 0u64;
        let mut oldest: Option<SystemTime> = None;
        for segment in &segments {
            if *segment < cursor.segment {
                continue;
            }
            let meta = match fs::metadata(segment_path(&self.inner.dir, *segment)) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let bytes = if *segment == cursor.segment {
                meta.len().saturating_sub(cursor.offset)
            } else {
                meta.len()
            };
            if bytes > 0 {
                pending_bytes += bytes;
                if let Ok(modified) = meta.modified() {
                    oldest = Some(oldest.map_or(modified, |o| o.min(modified)));
                }
            }
        }
        Ok(SpoolStatus {
            pending_bytes,
            segments: segments.len(),
            oldest_pending_age: oldest.and_then(|t| SystemTime::now().duration_since(t).ok()),
        })
    }

    fn open_segment(&self) -> u64 {
        self.inner
            .writer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .segment
    }
}

fn segment_path(dir: &Path, segment: u64) -> PathBuf {
    dir.join(format!("{segment:020}{SEGMENT_SUFFIX}"))
}

fn list_segments(dir: &Path) -> Result<Vec<u64>> {
    let mut segments: Vec<u64> = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            let stem = name.strip_suffix(SEGMENT_SUFFIX)?;
            stem.parse::<u64>().ok()
        })
        .collect();
    segments.sort_unstable();
    Ok(segments)
}

/// The byte offset of the start of an unterminated last line, if the file
/// has one.
fn partial_tail(file: &mut File, len: u64) -> Result<Option<u64>> {
    if len == 0 {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(len - 1))?;
    let mut last = [0u8; 1];
    file.read_exact(&mut last)?;
    if last[0] == b'\n' {
        return Ok(None);
    }
    // Walk back in chunks to the previous newline.
    let chunk = 4096u64;
    let mut end = len;
    while end > 0 {
        let start = end.saturating_sub(chunk);
        file.seek(SeekFrom::Start(start))?;
        let mut buf = vec![0u8; (end - start) as usize];
        file.read_exact(&mut buf)?;
        if let Some(pos) = buf.iter().rposition(|b| *b == b'\n') {
            return Ok(Some(start + pos as u64 + 1));
        }
        end = start;
    }
    Ok(Some(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::RecordBuilder;
    use fcp_core::storage::CdrKind;
    use fcp_core::{CallId, TenantId};

    fn record() -> CallDetailRecord {
        RecordBuilder::new(CdrKind::Call, CallId::new(), TenantId::new())
            .from("sip:a@x", None)
            .to("sip:b@x", None)
            .build()
    }

    fn small_config() -> SpoolConfig {
        SpoolConfig {
            fsync_interval: Duration::from_millis(10),
            segment_bytes: 2048,
        }
    }

    #[test]
    fn records_come_back_in_order_and_the_cursor_moves_only_on_commit() {
        let dir = tempfile::tempdir().unwrap();
        let spool = Spool::open(dir.path(), SpoolConfig::default()).unwrap();
        let ids: Vec<_> = (0..5)
            .map(|_| {
                let r = record();
                spool.append(&r).unwrap();
                r.cdr_id
            })
            .collect();

        let (batch, next) = spool.read_from(spool.cursor().unwrap(), 3).unwrap();
        assert_eq!(batch.iter().map(|r| r.cdr_id).collect::<Vec<_>>(), ids[..3]);
        // Not committed: the same three come back.
        let (again, _) = spool.read_from(spool.cursor().unwrap(), 3).unwrap();
        assert_eq!(again.iter().map(|r| r.cdr_id).collect::<Vec<_>>(), ids[..3]);

        spool.commit(next).unwrap();
        let (rest, next) = spool.read_from(spool.cursor().unwrap(), 10).unwrap();
        assert_eq!(rest.iter().map(|r| r.cdr_id).collect::<Vec<_>>(), ids[3..]);
        spool.commit(next).unwrap();
        let (empty, _) = spool.read_from(spool.cursor().unwrap(), 10).unwrap();
        assert!(empty.is_empty());
        assert_eq!(spool.status().unwrap().pending_bytes, 0);
    }

    #[test]
    fn segments_roll_and_shipped_ones_are_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let spool = Spool::open(dir.path(), small_config()).unwrap();
        let mut ids = Vec::new();
        for _ in 0..40 {
            let r = record();
            spool.append(&r).unwrap();
            ids.push(r.cdr_id);
        }
        let segments = list_segments(dir.path()).unwrap();
        assert!(segments.len() > 2, "{segments:?}");
        assert!(spool.status().unwrap().pending_bytes > 0);

        let mut got = Vec::new();
        loop {
            let (batch, next) = spool.read_from(spool.cursor().unwrap(), 7).unwrap();
            if batch.is_empty() {
                break;
            }
            got.extend(batch.iter().map(|r| r.cdr_id));
            spool.commit(next).unwrap();
        }
        assert_eq!(got, ids);
        let left = list_segments(dir.path()).unwrap();
        assert_eq!(left.len(), 1, "only the open segment stays: {left:?}");
    }

    #[test]
    fn a_partial_last_line_is_skipped_by_the_reader_and_truncated_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let spool = Spool::open(dir.path(), SpoolConfig::default()).unwrap();
        let r = record();
        spool.append(&r).unwrap();
        let path = segment_path(dir.path(), 0);
        {
            let mut f = OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(b"{\"cdr_id\":\"half").unwrap();
        }
        let (batch, next) = spool.read_from(Cursor::default(), 10).unwrap();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].cdr_id, r.cdr_id);
        drop(spool);

        // Reopened, the partial tail is gone and a new record appends cleanly.
        let spool = Spool::open(dir.path(), SpoolConfig::default()).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().len(), next.offset);
        let r2 = record();
        spool.append(&r2).unwrap();
        let (batch, _) = spool.read_from(Cursor::default(), 10).unwrap();
        assert_eq!(
            batch.iter().map(|r| r.cdr_id).collect::<Vec<_>>(),
            vec![r.cdr_id, r2.cdr_id]
        );
    }

    #[test]
    fn a_line_that_does_not_decode_is_dead_lettered_not_lost_or_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let spool = Spool::open(dir.path(), SpoolConfig::default()).unwrap();
        let a = record();
        spool.append(&a).unwrap();
        {
            let mut f = OpenOptions::new()
                .append(true)
                .open(segment_path(dir.path(), 0))
                .unwrap();
            f.write_all(b"{\"not\":\"a record\"}\n").unwrap();
        }
        let b = record();
        spool.append(&b).unwrap();
        let (batch, _) = spool.read_from(Cursor::default(), 10).unwrap();
        assert_eq!(
            batch.iter().map(|r| r.cdr_id).collect::<Vec<_>>(),
            vec![a.cdr_id, b.cdr_id]
        );
        let dead = fs::read_to_string(dir.path().join(DEAD_FILE)).unwrap();
        assert_eq!(dead, "{\"not\":\"a record\"}\n");
    }

    #[test]
    fn the_spool_continues_after_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let first = record();
        {
            let spool = Spool::open(dir.path(), small_config()).unwrap();
            spool.append(&first).unwrap();
            spool.sync().unwrap();
        }
        let spool = Spool::open(dir.path(), small_config()).unwrap();
        let second = record();
        spool.append(&second).unwrap();
        let (batch, next) = spool.read_from(spool.cursor().unwrap(), 10).unwrap();
        assert_eq!(
            batch.iter().map(|r| r.cdr_id).collect::<Vec<_>>(),
            vec![first.cdr_id, second.cdr_id]
        );
        spool.commit(next).unwrap();
        assert_eq!(spool.cursor().unwrap(), next);
        let status = spool.status().unwrap();
        assert_eq!(status.pending_bytes, 0);
        assert_eq!(status.oldest_pending_age, None);
    }

    #[tokio::test]
    async fn notified_wakes_for_an_append() {
        let dir = tempfile::tempdir().unwrap();
        let spool = Spool::open(dir.path(), SpoolConfig::default()).unwrap();
        let waiter = spool.clone();
        let handle = tokio::spawn(async move { waiter.notified().await });
        spool.append(&record()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("woken")
            .unwrap();
    }
}
