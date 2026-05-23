//! ctrace ndjson source. Each line in a ctrace session log is a JSON
//! object — we only consume the subset relevant to fsstory:
//!
//! ```text
//! { "ts": 1779062884, "syscall": "openat", "flags": "WRONLY|CREAT",
//!   "file": "/abs/path", "pid": 1234, "ppid": 100, "comm": "claude" }
//! ```
//!
//! Real ctrace emits more keys (the cookie, raw flag bits, etc.); we
//! tolerate them by using serde's default deserializer with
//! `#[serde(default)]` on every optional field.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use walkdir::WalkDir;

use crate::event::{EventOp, RawEvent, SourceKind};

/// Per-line ndjson record we consume.
#[derive(Debug, Deserialize)]
struct CtraceLine {
    #[serde(default)]
    ts: Option<i64>,
    /// Some ctrace builds use `ts_unix` instead of `ts`.
    #[serde(default)]
    ts_unix: Option<i64>,
    #[serde(default)]
    syscall: Option<String>,
    #[serde(default)]
    flags: Option<String>,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    pid: Option<u32>,
    #[serde(default)]
    ppid: Option<u32>,
    #[serde(default)]
    comm: Option<String>,
    /// Optional size delta if ctrace recorded it.
    #[serde(default)]
    size_delta: Option<i64>,
}

/// Map a syscall name + flags string to an [`EventOp`]. Returns `None`
/// for syscalls fsstory doesn't surface (reads without `O_WRONLY`,
/// fstat-only, etc.).
fn classify(syscall: Option<&str>, flags: Option<&str>) -> Option<EventOp> {
    match syscall.unwrap_or("") {
        "openat" | "open" | "creat" | "write" | "pwrite" | "writev" | "renameat"
        | "renameat2" | "linkat" => {
            let f = flags.unwrap_or("");
            if f.contains("WRONLY") || f.contains("RDWR") || f.contains("CREAT")
                || f.contains("APPEND") || syscall == Some("write")
                || syscall == Some("pwrite") || syscall == Some("writev")
                || syscall == Some("renameat") || syscall == Some("renameat2")
                || syscall == Some("linkat") || syscall == Some("creat")
            {
                Some(EventOp::Write)
            } else {
                None
            }
        }
        "unlinkat" | "unlink" | "rmdir" => Some(EventOp::Unlink),
        _ => None,
    }
}

/// Parse a single ndjson line into a [`RawEvent`], if it's a write/unlink
/// we surface. Bogus lines return `None`.
#[must_use]
fn line_to_event(line: &str) -> Option<RawEvent> {
    let rec: CtraceLine = serde_json::from_str(line).ok()?;
    let op = classify(rec.syscall.as_deref(), rec.flags.as_deref())?;
    let path = rec.file.or(rec.path)?;
    let ts = rec.ts.or(rec.ts_unix)?;
    Some(RawEvent {
        ts_unix: ts,
        path,
        op,
        source: SourceKind::Ctrace,
        pid: rec.pid,
        ppid: rec.ppid,
        comm: rec.comm,
        jsonl_basename: None,
        jsonl_turn: None,
        tool_name: None,
        size_delta_bytes: rec.size_delta,
    })
}

/// Read one ndjson file and append matching events to `out`. Returns the
/// path that was read (for evidence pointers). I/O errors are silently
/// skipped — the PRD demands graceful degradation when ctrace is off.
pub fn read_ndjson(path: &Path, out: &mut Vec<(PathBuf, RawEvent)>) {
    let Ok(file) = File::open(path) else { return };
    let reader = BufReader::new(file);
    for line_result in reader.lines() {
        let Ok(line) = line_result else { continue };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(ev) = line_to_event(&line) {
            out.push((path.to_path_buf(), ev));
        }
    }
}

/// Discover and read all `*.ndjson` files under `dir`. Returns a flat
/// list of (ctrace_log_path, RawEvent).
#[must_use]
pub fn read_dir(dir: &Path) -> Vec<(PathBuf, RawEvent)> {
    let mut out = Vec::new();
    if !dir.exists() {
        return out;
    }
    for entry in WalkDir::new(dir)
        .max_depth(2)
        .into_iter()
        .filter_map(Result::ok)
    {
        let p = entry.path();
        if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("ndjson") {
            read_ndjson(p, &mut out);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn classifies_openat_wronly_as_write() {
        assert_eq!(
            classify(Some("openat"), Some("O_WRONLY|O_CREAT")),
            Some(EventOp::Write)
        );
    }

    #[test]
    fn classifies_unlink() {
        assert_eq!(classify(Some("unlinkat"), None), Some(EventOp::Unlink));
    }

    #[test]
    fn ignores_read_only_openat() {
        assert_eq!(classify(Some("openat"), Some("O_RDONLY")), None);
    }

    #[test]
    fn parses_minimal_line() {
        let line = r#"{"ts": 100, "syscall": "openat", "flags": "WRONLY|CREAT", "file": "/tmp/x", "pid": 7, "comm": "claude"}"#;
        let ev = line_to_event(line).unwrap();
        assert_eq!(ev.path, "/tmp/x");
        assert_eq!(ev.op, EventOp::Write);
        assert_eq!(ev.comm.as_deref(), Some("claude"));
    }

    #[test]
    fn skips_malformed_line() {
        assert!(line_to_event("not json").is_none());
        assert!(line_to_event(r#"{"syscall":"openat"}"#).is_none());
    }

    #[test]
    fn read_dir_handles_missing() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("nope");
        assert!(read_dir(&missing).is_empty());
    }

    #[test]
    fn read_dir_walks_ndjson() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("a.ndjson");
        let mut f = std::fs::File::create(&p).unwrap();
        writeln!(
            f,
            r#"{{"ts":100,"syscall":"openat","flags":"WRONLY","file":"/tmp/x","pid":1,"comm":"claude"}}"#
        )
        .unwrap();
        let out = read_dir(dir.path());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1.path, "/tmp/x");
    }
}
