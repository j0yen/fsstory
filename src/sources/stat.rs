//! stat() fallback. When ctrace is off and no jsonl tool_use mentions a
//! path, we still surface a single low-confidence event derived from the
//! file's mtime so callers never see an empty result for a path that
//! actually exists.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::event::{EventOp, RawEvent, SourceKind};

/// If `path` exists, return a single RawEvent for its mtime.
#[must_use]
pub fn read_path(path: &Path) -> Option<RawEvent> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?;
    let ts = mtime
        .duration_since(UNIX_EPOCH)
        .ok()
        .map_or(0_i64, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    Some(RawEvent {
        ts_unix: ts,
        path: path.display().to_string(),
        op: EventOp::Write,
        source: SourceKind::Stat,
        pid: None,
        ppid: None,
        comm: None,
        jsonl_basename: None,
        jsonl_turn: None,
        tool_name: None,
        size_delta_bytes: None,
    })
}

/// Convenience: now() as a UNIX second count.
#[must_use]
pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn missing_path_returns_none() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("nope");
        assert!(read_path(&p).is_none());
    }

    #[test]
    fn existing_file_returns_event() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("x");
        let mut f = std::fs::File::create(&p).unwrap();
        writeln!(f, "hi").unwrap();
        let ev = read_path(&p).unwrap();
        assert_eq!(ev.source, SourceKind::Stat);
        assert_eq!(ev.op, EventOp::Write);
    }
}
