//! Query handlers: glue between the CLI surface and the attributor.
//!
//! [`QueryEnv`] is the dependency surface: where to find ctrace logs,
//! where to find Claude jsonl transcripts, and how to resolve relative
//! paths. Tests stub these to point at tempdirs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::attributor::Attributor;
use crate::event::{Actor, AttributedEvent};
use crate::sources::{ctrace, jsonl, stat};

/// Where the query handlers look for source data.
#[derive(Debug, Clone)]
pub struct QueryEnv {
    /// Root containing ctrace `*.ndjson` files (typically
    /// `~/.cache/ctrace/sessions/`). When the directory doesn't exist
    /// we degrade gracefully to stat-only.
    pub ctrace_root: PathBuf,
    /// Root containing Claude project jsonl transcripts (typically
    /// `~/.claude/projects/`).
    pub claude_projects_root: PathBuf,
}

impl QueryEnv {
    /// Default env using the canonical Joe-laptop paths.
    #[must_use]
    pub fn default_for_user() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        Self {
            ctrace_root: PathBuf::from(&home).join(".cache/ctrace/sessions"),
            claude_projects_root: PathBuf::from(&home).join(".claude/projects"),
        }
    }
}

/// A query the CLI can ask.
#[derive(Debug, Clone)]
pub enum Query {
    /// `fsstory path <path> [--since <secs>]`.
    Path {
        /// Target path (resolved absolute if possible).
        path: PathBuf,
        /// Optional minimum-age cutoff in seconds (i.e. now - since).
        since_secs: Option<i64>,
    },
    /// `fsstory who-wrote <path>`.
    WhoWrote {
        /// Target path.
        path: PathBuf,
    },
    /// `fsstory summary --root <dir> [--since <secs>]`.
    Summary {
        /// Root directory to summarize.
        root: PathBuf,
        /// Optional cutoff in seconds.
        since_secs: Option<i64>,
    },
}

/// CLI output formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// Stable JSON object, sorted by `ts_unix`.
    Json,
    /// Human-readable text (one line per event).
    Text,
}

/// What a query returns.
#[derive(Debug, Clone)]
pub enum QueryOutput {
    /// Events for a single path.
    PathEvents {
        /// Resolved path the events refer to.
        path: PathBuf,
        /// Sorted attributed events.
        events: Vec<AttributedEvent>,
    },
    /// Single latest event (who-wrote).
    Latest {
        /// Resolved path.
        path: PathBuf,
        /// The latest event, if any.
        latest: Option<AttributedEvent>,
    },
    /// By-actor summary histogram.
    Summary {
        /// Root directory.
        root: PathBuf,
        /// Counts keyed by actor label.
        counts: BTreeMap<String, usize>,
        /// Total event count surfaced.
        total: usize,
    },
}

/// Build an [`Attributor`] with events from ctrace + jsonl. Missing
/// directories silently yield empty event lists (graceful degradation).
#[must_use]
pub fn collect_events(env: &QueryEnv) -> Attributor {
    let mut a = Attributor::new();
    for (p, ev) in ctrace::read_dir(&env.ctrace_root) {
        a.push(p.display().to_string(), ev);
    }
    for (p, ev) in jsonl::read_dir(&env.claude_projects_root) {
        a.push(p.display().to_string(), ev);
    }
    a
}

/// Run a query against an environment. Pure function — no I/O after
/// the initial parse.
pub fn run(env: &QueryEnv, q: &Query) -> QueryOutput {
    let attr = collect_events(env);
    match q {
        Query::Path { path, since_secs } => {
            let mut events =
                attr.attribute_path(&path.display().to_string(), stat::read_path(path));
            if let Some(s) = since_secs {
                let cutoff = stat::now_unix() - s;
                events.retain(|e| e.ts_unix >= cutoff);
            }
            QueryOutput::PathEvents {
                path: path.clone(),
                events,
            }
        }
        Query::WhoWrote { path } => {
            let events =
                attr.attribute_path(&path.display().to_string(), stat::read_path(path));
            QueryOutput::Latest {
                path: path.clone(),
                latest: events.into_iter().last(),
            }
        }
        Query::Summary { root, since_secs } => {
            let mut events = attr.attribute_all();
            if let Some(s) = since_secs {
                let cutoff = stat::now_unix() - s;
                events.retain(|e| e.ts_unix >= cutoff);
            }
            // Filter to paths under `root`.
            let root_str = root.display().to_string();
            events.retain(|e| e.path.starts_with(&root_str));
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            for e in &events {
                let key = match &e.actor {
                    Actor::ClaudeSession { .. } | Actor::ClaudeBash { .. } => "claude",
                    Actor::UserInteractive { .. } => "user-interactive",
                    Actor::Unknown { .. } => "unknown",
                };
                *counts.entry(key.to_string()).or_default() += 1;
            }
            QueryOutput::Summary {
                root: root.clone(),
                counts,
                total: events.len(),
            }
        }
    }
}

/// Serialize a [`QueryOutput::PathEvents`] as the stable JSON shape the
/// PRD §4.3 describes.
///
/// # Errors
/// Returns the underlying `serde_json` error if serialization fails.
pub fn render_path_json(path: &Path, events: &[AttributedEvent]) -> Result<String, serde_json::Error> {
    #[derive(Serialize)]
    struct EventOut<'a> {
        ts: &'a str,
        actor: String,
        via: Option<&'a str>,
        op: &'a str,
        size_delta_bytes: Option<i64>,
        confidence: &'a str,
        evidence: &'a crate::event::Evidence,
    }
    #[derive(Serialize)]
    struct Out<'a> {
        path: String,
        events: Vec<EventOut<'a>>,
    }
    let out = Out {
        path: path.display().to_string(),
        events: events
            .iter()
            .map(|e| EventOut {
                ts: &e.ts,
                actor: e.actor.to_label(),
                via: e.via.as_deref(),
                op: match e.op {
                    crate::event::EventOp::Write => "write",
                    crate::event::EventOp::Unlink => "unlink",
                    crate::event::EventOp::Access => "access",
                },
                size_delta_bytes: e.size_delta_bytes,
                confidence: e.confidence.as_str(),
                evidence: &e.evidence,
            })
            .collect(),
    };
    serde_json::to_string_pretty(&out)
}

/// One-line `who-wrote` formatting.
#[must_use]
pub fn render_who_wrote(latest: Option<&AttributedEvent>) -> String {
    match latest {
        Some(e) => format!("{}\t{}\t{}", e.ts, e.actor.to_label(), e.confidence.as_str()),
        None => String::from("-\tunknown\tlow"),
    }
}

/// Render a summary as a `key\tcount` table sorted by key.
#[must_use]
pub fn render_summary_text(root: &Path, counts: &BTreeMap<String, usize>, total: usize) -> String {
    let mut s = format!("root: {}\n", root.display());
    s.push_str(&format!("total: {total}\n"));
    for (k, v) in counts {
        s.push_str(&format!("{k}\t{v}\n"));
    }
    s
}

/// JSON form of summary (stable, sorted by key via BTreeMap).
///
/// # Errors
/// Returns the underlying `serde_json` error if serialization fails.
pub fn render_summary_json(
    root: &Path,
    counts: &BTreeMap<String, usize>,
    total: usize,
) -> Result<String, serde_json::Error> {
    #[derive(Serialize)]
    struct Out<'a> {
        root: String,
        total: usize,
        counts: &'a BTreeMap<String, usize>,
    }
    serde_json::to_string_pretty(&Out {
        root: root.display().to_string(),
        total,
        counts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Confidence, Evidence, EventOp};

    #[test]
    fn render_who_wrote_handles_none() {
        let line = render_who_wrote(None);
        let cols: Vec<&str> = line.split('\t').collect();
        assert_eq!(cols.len(), 3);
        assert_eq!(cols[2], "low");
    }

    #[test]
    fn render_who_wrote_uses_actor_label() {
        let ev = AttributedEvent {
            ts: "2026-05-22T23:48:04Z".into(),
            ts_unix: 1_779_062_884,
            path: "/tmp/x".into(),
            op: EventOp::Write,
            actor: Actor::ClaudeSession {
                jsonl_basename: "sess.jsonl".into(),
                turn: 33,
                tool: Some("Edit".into()),
            },
            confidence: Confidence::High,
            evidence: Evidence::default(),
            size_delta_bytes: None,
            via: None,
        };
        let line = render_who_wrote(Some(&ev));
        assert!(line.contains("claude-session:sess.jsonl:33"));
        assert!(line.contains("high"));
    }
}
