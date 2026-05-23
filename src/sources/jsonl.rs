//! Claude project JSONL source. Each line of a Claude project transcript
//! (`~/.claude/projects/<dashed-path>/<uuid>.jsonl`) is a JSON record.
//! We only care about tool_use blocks for Edit / Write / NotebookEdit
//! and Bash invocations whose inputs reference paths.
//!
//! The Claude transcript format we observe:
//!
//! ```text
//! {"type":"assistant","timestamp":"2026-05-22T23:48:04.000Z",
//!  "message":{"content":[
//!     {"type":"tool_use","name":"Edit",
//!      "input":{"file_path":"/abs/path","old_string":"...","new_string":"..."}},
//!     {"type":"tool_use","name":"Bash",
//!      "input":{"command":"touch /tmp/x"}}
//!  ]}}
//! ```
//!
//! We're tolerant of missing fields; turn index is the 0-based line
//! number inside the jsonl.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use walkdir::WalkDir;

use crate::event::{EventOp, RawEvent, SourceKind};

#[derive(Debug, Deserialize)]
struct JsonlLine {
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    message: Option<JsonlMessage>,
}

#[derive(Debug, Deserialize)]
struct JsonlMessage {
    #[serde(default)]
    content: Vec<JsonlContentBlock>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum JsonlContentBlock {
    ToolUse {
        #[serde(default)]
        name: String,
        #[serde(default)]
        input: serde_json::Value,
    },
    #[serde(other)]
    Other,
}

/// Parse an ISO-8601 timestamp (`YYYY-MM-DDTHH:MM:SS[.fff]Z`) into a
/// UNIX second count. Returns `None` if unparseable.
#[must_use]
fn parse_iso8601(s: &str) -> Option<i64> {
    // Format we observe: "2026-05-22T23:48:04.000Z" or "...Z" without ms.
    let trimmed = s.trim_end_matches('Z');
    let (date, time) = trimmed.split_once('T')?;
    let mut date_parts = date.split('-');
    let y: i64 = date_parts.next()?.parse().ok()?;
    let mo: i64 = date_parts.next()?.parse().ok()?;
    let d: i64 = date_parts.next()?.parse().ok()?;
    let time = time.split('.').next()?;
    let mut tp = time.split(':');
    let h: i64 = tp.next()?.parse().ok()?;
    let mi: i64 = tp.next()?.parse().ok()?;
    let se: i64 = tp.next()?.parse().ok()?;
    Some(unix_from_civil(y, mo, d, h, mi, se))
}

/// Civil date → UNIX second count (Howard Hinnant's algorithm,
/// adapted for i64).
#[must_use]
#[allow(clippy::too_many_arguments)]
fn unix_from_civil(y: i64, m: i64, d: i64, h: i64, mi: i64, s: i64) -> i64 {
    let y_adj = if m <= 2 { y - 1 } else { y };
    let era = y_adj.div_euclid(400);
    let yoe = y_adj - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    days * 86_400 + h * 3600 + mi * 60 + s
}

/// Path-bearing tool inputs we know about. Returns the path and a
/// virtual operation classification.
#[must_use]
fn extract_path(tool_name: &str, input: &serde_json::Value) -> Option<(String, EventOp)> {
    match tool_name {
        "Edit" | "NotebookEdit" => input
            .get("file_path")
            .and_then(serde_json::Value::as_str)
            .map(|p| (p.to_string(), EventOp::Write)),
        "Write" => input
            .get("file_path")
            .and_then(serde_json::Value::as_str)
            .map(|p| (p.to_string(), EventOp::Write)),
        _ => None,
    }
}

/// Read one jsonl file. Yields one RawEvent per path-bearing tool_use,
/// plus one synthetic RawEvent per Bash tool_use (no path — those get
/// merged via PID windowing at the attributor).
pub fn read_jsonl(path: &Path, out: &mut Vec<(PathBuf, RawEvent)>) {
    let Ok(file) = File::open(path) else { return };
    let basename = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let reader = BufReader::new(file);
    for (turn_idx, line_result) in reader.lines().enumerate() {
        let Ok(line) = line_result else { continue };
        if line.trim().is_empty() {
            continue;
        }
        let rec: JsonlLine = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let ts_unix = rec
            .timestamp
            .as_deref()
            .and_then(parse_iso8601)
            .unwrap_or(0);
        let Some(msg) = rec.message else { continue };
        for block in msg.content {
            if let JsonlContentBlock::ToolUse { name, input } = block {
                if let Some((p, op)) = extract_path(&name, &input) {
                    let turn_u32 = u32::try_from(turn_idx).unwrap_or(u32::MAX);
                    out.push((
                        path.to_path_buf(),
                        RawEvent {
                            ts_unix,
                            path: p,
                            op,
                            source: SourceKind::ClaudeJsonl,
                            pid: None,
                            ppid: None,
                            comm: None,
                            jsonl_basename: Some(basename.clone()),
                            jsonl_turn: Some(turn_u32),
                            tool_name: Some(name),
                            size_delta_bytes: None,
                        },
                    ));
                }
            }
        }
    }
}

/// Discover and read all `*.jsonl` files under `dir` (recursive).
#[must_use]
pub fn read_dir(dir: &Path) -> Vec<(PathBuf, RawEvent)> {
    let mut out = Vec::new();
    if !dir.exists() {
        return out;
    }
    for entry in WalkDir::new(dir)
        .max_depth(4)
        .into_iter()
        .filter_map(Result::ok)
    {
        let p = entry.path();
        if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("jsonl") {
            read_jsonl(p, &mut out);
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
    fn parses_iso8601_z() {
        assert_eq!(parse_iso8601("2026-05-22T23:48:04Z"), Some(1_779_493_684));
    }

    #[test]
    fn parses_iso8601_ms() {
        assert_eq!(
            parse_iso8601("2026-05-22T23:48:04.500Z"),
            Some(1_779_493_684)
        );
    }

    #[test]
    fn rejects_garbage_iso() {
        assert!(parse_iso8601("not-a-date").is_none());
    }

    #[test]
    fn extracts_edit_path() {
        let v = serde_json::json!({"file_path": "/tmp/x", "old_string": "a", "new_string": "b"});
        assert_eq!(
            extract_path("Edit", &v),
            Some(("/tmp/x".to_string(), EventOp::Write))
        );
    }

    #[test]
    fn ignores_bash() {
        let v = serde_json::json!({"command": "ls"});
        assert!(extract_path("Bash", &v).is_none());
    }

    #[test]
    fn jsonl_yields_edit_event() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("aaaa.jsonl");
        let mut f = std::fs::File::create(&p).unwrap();
        writeln!(
            f,
            r#"{{"type":"assistant","timestamp":"2026-05-22T23:48:04Z","message":{{"content":[{{"type":"tool_use","name":"Edit","input":{{"file_path":"/tmp/x","old_string":"a","new_string":"b"}}}}]}}}}"#
        )
        .unwrap();
        let mut events = Vec::new();
        read_jsonl(&p, &mut events);
        assert_eq!(events.len(), 1);
        let (src, ev) = &events[0];
        assert_eq!(src, &p);
        assert_eq!(ev.path, "/tmp/x");
        assert_eq!(ev.jsonl_basename.as_deref(), Some("aaaa.jsonl"));
        assert_eq!(ev.tool_name.as_deref(), Some("Edit"));
    }
}
