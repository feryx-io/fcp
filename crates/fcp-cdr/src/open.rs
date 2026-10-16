//! The records of calls still up (`docs/DEEP_TESTING.md` D5).
//!
//! A call's record is built when the call ends. A node killed while it
//! carries calls never gets there, so without this those calls have no
//! record at all: talk time nobody is billed for and nobody can find. So
//! while a call is up its node keeps the record as it would be written
//! now — answered, still going, ended by [`SystemCause::NodeFailed`] —
//! in a file of its own beside the spool, rewritten on a tick. The call's
//! end writes the real record and removes the file; a node that starts
//! and finds files left writes them, ended when they were last written.
//!
//! The open record and the real one have the same `cdr_id` (the call's
//! own id), so a node killed between writing the real record and
//! removing the file is billed once: the store keeps the first to land.
//!
//! [`SystemCause::NodeFailed`]: crate::SystemCause::NodeFailed

use std::path::{Path, PathBuf};

use fcp_core::storage::CallDetailRecord;
use fcp_core::CallId;
use tracing::{info, warn};

/// The directory of a spool's open records.
const OPEN_DIR: &str = "open";

/// One file per call still up, under the spool.
#[derive(Debug, Clone)]
pub struct OpenRecords {
    dir: PathBuf,
}

impl OpenRecords {
    /// The open records of the spool at `spool_dir`.
    pub fn at(spool_dir: &Path) -> std::io::Result<Self> {
        let dir = spool_dir.join(OPEN_DIR);
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    fn path(&self, call_id: &CallId) -> PathBuf {
        self.dir.join(format!("{}.json", call_id.as_uuid()))
    }

    /// Keep `record` as the call's record if its node stops now. Written
    /// beside and renamed over, so a node killed mid-write leaves the
    /// last one whole.
    pub fn put(&self, record: &CallDetailRecord) {
        let path = self.path(&record.call_id);
        let tmp = path.with_extension("json.tmp");
        let written = serde_json::to_vec(record)
            .map_err(std::io::Error::other)
            .and_then(|bytes| std::fs::write(&tmp, bytes))
            .and_then(|()| std::fs::rename(&tmp, &path));
        if let Err(e) = written {
            warn!(call_id = %record.call_id, error = %e, "A call's open record could not be kept");
        }
    }

    /// The call ended and its record was written: forget the open one.
    pub fn close(&self, call_id: &CallId) {
        match std::fs::remove_file(self.path(call_id)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                warn!(call_id = %call_id, error = %e, "A call's open record could not be removed")
            }
        }
    }

    /// The records a node that stopped left open, each handed to `write`
    /// and then removed. Called once at start, before any call: whatever
    /// is here belongs to a run that has gone.
    pub fn recover(&self, mut write: impl FnMut(CallDetailRecord) -> bool) -> usize {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) => {
                warn!(dir = %self.dir.display(), error = %e, "The open records could not be read");
                return 0;
            }
        };
        let mut recovered = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            match path.extension().and_then(|e| e.to_str()) {
                Some("json") => {}
                // A write the last run did not finish: the file it was
                // replacing, if any, is whole.
                Some("tmp") => {
                    let _ = std::fs::remove_file(&path);
                    continue;
                }
                _ => continue,
            }
            let record = std::fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<CallDetailRecord>(&bytes).ok());
            let Some(record) = record else {
                warn!(path = %path.display(), "An open record could not be read; left in place");
                continue;
            };
            let call_id = record.call_id;
            if write(record) {
                let _ = std::fs::remove_file(&path);
                recovered += 1;
                info!(call_id = %call_id, "Recorded a call that was up when this node last stopped");
            }
        }
        recovered
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RecordBuilder, SystemCause, TerminationReason};
    use fcp_core::storage::CdrKind;
    use fcp_core::TenantId;

    fn record(call: CallId) -> CallDetailRecord {
        RecordBuilder::new(CdrKind::Call, call, TenantId::new())
            .cdr_id(*call.as_uuid())
            .reason(TerminationReason::system(
                SystemCause::NodeFailed,
                "stopped",
            ))
            .build()
    }

    #[test]
    fn a_closed_call_leaves_nothing_and_an_open_one_is_recovered_once() {
        let dir = tempfile::tempdir().unwrap();
        let open = OpenRecords::at(dir.path()).unwrap();
        let (ended, up) = (CallId::new(), CallId::new());
        open.put(&record(ended));
        open.put(&record(up));
        open.put(&record(up));
        open.close(&ended);
        open.close(&ended);

        let mut written = Vec::new();
        assert_eq!(
            open.recover(|r| {
                written.push(r);
                true
            }),
            1
        );
        assert_eq!(written.len(), 1);
        assert_eq!(written[0].call_id, up);
        assert_eq!(written[0].cdr_id, *up.as_uuid());
        assert_eq!(open.recover(|_| true), 0);
    }

    #[test]
    fn a_record_that_could_not_be_written_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let open = OpenRecords::at(dir.path()).unwrap();
        open.put(&record(CallId::new()));
        assert_eq!(open.recover(|_| false), 0);
        assert_eq!(open.recover(|_| true), 1);
    }
}
