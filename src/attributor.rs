//! Join (PID, time-window, comm, tool_name) to produce an
//! [`AttributedEvent`] from one or more [`RawEvent`]s.
//!
//! The join model:
//!
//! 1. Bin all RawEvents by `path`.
//! 2. Within each path's bin, walk ctrace events in timestamp order.
//!    For each ctrace event:
//!    a. If a jsonl tool_use event on the same `path` exists within
//!       `MATCH_WINDOW_SECS`, attribute to that session/turn with
//!       `confidence: high`.
//!    b. Else if `comm` matches a known interactive editor, attribute
//!       to `user-interactive` with `confidence: medium`.
//!    c. Else if `comm` matches `claude` / a known claude-spawned
//!       tool, attribute to `claude-bash` with `confidence: medium`
//!       (we know it was claude, just not which turn).
//!    d. Else fall back to `unknown` (`confidence: medium` — we have
//!       a ctrace event, just no actor mapping).
//! 3. For paths with no ctrace event but a jsonl tool_use, surface the
//!    jsonl event with `confidence: medium`.
//! 4. For paths with neither, the caller's stat() fallback gives
//!    `confidence: low`.
//!
//! `MATCH_WINDOW_SECS` is 5s — the Edit/Write tool_use timestamp on the
//! jsonl is wall-clock at the point Claude emitted the tool_use; the
//! ctrace write event lands a fraction of a second later when the
//! harness actually executes it. 5s is generous enough for slow
//! tool-router round-trips without being so wide that adjacent
//! tool_uses bleed into each other.

use std::collections::HashMap;

use crate::event::{
    Actor, AttributedEvent, Confidence, Evidence, EventOp, RawEvent, SourceKind,
};
use crate::time::iso8601_utc;

const MATCH_WINDOW_SECS: i64 = 5;

/// Comm values that mean "an interactive editor". Hardcoded per PRD
/// open question #4 (a learned classifier was deemed not worth the
/// complexity for v1).
const INTERACTIVE_COMMS: &[&str] = &[
    "vim", "nvim", "vi", "code", "code-oss", "codium", "vscode", "zed", "helix",
    "hx", "emacs", "nano", "kak", "kakoune", "micro", "subl", "sublime_text",
    "gedit", "kate",
];

/// Comm values that mean "this is a Claude-controlled process". The
/// canonical case is `comm=claude`; we also accept `claude-code` and
/// a few well-known helper binaries the user runs via Bash tool_use
/// (these still attribute as `claude-bash:<turn>` once we tie the
/// PID back to a jsonl line).
const CLAUDE_COMMS: &[&str] =
    &["claude", "claude-code", "node", "python3", "uv", "cargo", "rustc"];

/// Whether `comm` is in the interactive-editor set (case-sensitive
/// basename, since that's what ctrace records).
#[must_use]
fn is_interactive_editor(comm: &str) -> bool {
    INTERACTIVE_COMMS.iter().any(|e| *e == comm)
}

/// Whether `comm` is plausibly a Claude-controlled process.
#[must_use]
fn is_claude_like(comm: &str) -> bool {
    CLAUDE_COMMS.iter().any(|e| *e == comm)
}

/// The attributor. Hold a list of RawEvents + a parallel list of the
/// source paths they came from (for evidence pointers).
#[derive(Debug, Default)]
pub struct Attributor {
    /// (source_log_path, raw_event) pairs.
    pub events: Vec<(String, RawEvent)>,
}

impl Attributor {
    /// Build an attributor with no events.
    #[must_use]
    pub const fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// Add a raw event tagged with the source file it came from.
    pub fn push(&mut self, source: impl Into<String>, ev: RawEvent) {
        self.events.push((source.into(), ev));
    }

    /// Add many.
    pub fn extend<I, S>(&mut self, iter: I)
    where
        I: IntoIterator<Item = (S, RawEvent)>,
        S: Into<String>,
    {
        for (s, e) in iter {
            self.events.push((s.into(), e));
        }
    }

    /// Run the join for one specific path. Returns events in ascending
    /// timestamp order. Adds a single stat() fallback event when no
    /// non-stat source has anything for the path and `stat_fallback`
    /// is `Some`.
    #[must_use]
    pub fn attribute_path(
        &self,
        target_path: &str,
        stat_fallback: Option<RawEvent>,
    ) -> Vec<AttributedEvent> {
        let mut by_source: HashMap<SourceKind, Vec<(String, &RawEvent)>> = HashMap::new();
        for (src, ev) in &self.events {
            if ev.path == target_path {
                by_source
                    .entry(ev.source)
                    .or_default()
                    .push((src.clone(), ev));
            }
        }
        let mut out: Vec<AttributedEvent> = Vec::new();
        let ctrace_events = by_source.remove(&SourceKind::Ctrace).unwrap_or_default();
        let jsonl_events = by_source
            .remove(&SourceKind::ClaudeJsonl)
            .unwrap_or_default();
        let mut consumed_jsonl: Vec<bool> = vec![false; jsonl_events.len()];

        for (src, ev) in &ctrace_events {
            let mut attributed: Option<(Actor, Confidence, Evidence, Option<String>)> = None;

            // 1.a — try jsonl match within MATCH_WINDOW_SECS.
            let mut best_idx: Option<usize> = None;
            let mut best_dt: i64 = i64::MAX;
            for (idx, (_, jev)) in jsonl_events.iter().enumerate() {
                if consumed_jsonl.get(idx).copied().unwrap_or(false) {
                    continue;
                }
                let dt = (jev.ts_unix - ev.ts_unix).abs();
                if dt <= MATCH_WINDOW_SECS && dt < best_dt {
                    best_dt = dt;
                    best_idx = Some(idx);
                }
            }
            if let Some(idx) = best_idx {
                if let Some(slot) = consumed_jsonl.get_mut(idx) {
                    *slot = true;
                }
                if let Some((_jsrc, jev)) = jsonl_events.get(idx) {
                    let actor = Actor::ClaudeSession {
                        jsonl_basename: jev.jsonl_basename.clone().unwrap_or_default(),
                        turn: jev.jsonl_turn.unwrap_or(0),
                        tool: jev.tool_name.clone(),
                    };
                    let evidence = Evidence {
                        ctrace_log: Some(src.clone()),
                        ctrace_ts: Some(ev.ts_unix),
                        jsonl_session: jev.jsonl_basename.clone(),
                        jsonl_turn: jev.jsonl_turn,
                    };
                    attributed = Some((
                        actor,
                        Confidence::High,
                        evidence,
                        jev.tool_name.clone(),
                    ));
                }
            }

            // 1.b — interactive editor by comm.
            if attributed.is_none() {
                if let Some(comm) = ev.comm.as_deref() {
                    if is_interactive_editor(comm) {
                        attributed = Some((
                            Actor::UserInteractive {
                                comm: comm.to_string(),
                            },
                            Confidence::Medium,
                            Evidence {
                                ctrace_log: Some(src.clone()),
                                ctrace_ts: Some(ev.ts_unix),
                                ..Evidence::default()
                            },
                            None,
                        ));
                    }
                }
            }

            // 1.c — claude-like comm without a jsonl match → claude-bash, but
            // we don't know which turn. Attribute to a synthetic
            // claude-bash:<jsonl-basename>:<turn> only if exactly one
            // candidate jsonl is open. Otherwise fall through to unknown.
            if attributed.is_none() {
                if let Some(comm) = ev.comm.as_deref() {
                    if is_claude_like(comm) {
                        // Pick a "best" jsonl: the one with the closest
                        // remaining tool_use timestamp regardless of path.
                        let mut best_basename: Option<String> = None;
                        let mut best_turn: u32 = 0;
                        let mut best_dt: i64 = i64::MAX;
                        for (_, jev) in &self.events {
                            if jev.source != SourceKind::ClaudeJsonl {
                                continue;
                            }
                            let dt = (jev.ts_unix - ev.ts_unix).abs();
                            if dt <= MATCH_WINDOW_SECS && dt < best_dt {
                                best_dt = dt;
                                best_basename = jev.jsonl_basename.clone();
                                best_turn = jev.jsonl_turn.unwrap_or(0);
                            }
                        }
                        if let Some(b) = best_basename {
                            attributed = Some((
                                Actor::ClaudeBash {
                                    jsonl_basename: b,
                                    turn: best_turn,
                                },
                                Confidence::Medium,
                                Evidence {
                                    ctrace_log: Some(src.clone()),
                                    ctrace_ts: Some(ev.ts_unix),
                                    ..Evidence::default()
                                },
                                None,
                            ));
                        } else {
                            // Claude-like comm but no jsonl candidate
                            // window — still classify as claude-bash
                            // with an empty session marker so callers
                            // can bucket it as "claude" (PRD §4.2).
                            attributed = Some((
                                Actor::ClaudeBash {
                                    jsonl_basename: String::new(),
                                    turn: 0,
                                },
                                Confidence::Medium,
                                Evidence {
                                    ctrace_log: Some(src.clone()),
                                    ctrace_ts: Some(ev.ts_unix),
                                    ..Evidence::default()
                                },
                                None,
                            ));
                        }
                    }
                }
            }

            // 1.d — last resort: unknown with comm if any.
            if attributed.is_none() {
                attributed = Some((
                    Actor::Unknown {
                        comm: ev.comm.clone(),
                    },
                    Confidence::Medium,
                    Evidence {
                        ctrace_log: Some(src.clone()),
                        ctrace_ts: Some(ev.ts_unix),
                        ..Evidence::default()
                    },
                    None,
                ));
            }

            if let Some((actor, conf, evidence, via)) = attributed {
                out.push(AttributedEvent {
                    ts: iso8601_utc(ev.ts_unix),
                    ts_unix: ev.ts_unix,
                    path: ev.path.clone(),
                    op: ev.op,
                    actor,
                    confidence: conf,
                    evidence,
                    size_delta_bytes: ev.size_delta_bytes,
                    via,
                });
            }
        }

        // 2 — unconsumed jsonl tool_uses become standalone medium events.
        for (idx, (_src, jev)) in jsonl_events.iter().enumerate() {
            if consumed_jsonl.get(idx).copied().unwrap_or(true) {
                continue;
            }
            let actor = Actor::ClaudeSession {
                jsonl_basename: jev.jsonl_basename.clone().unwrap_or_default(),
                turn: jev.jsonl_turn.unwrap_or(0),
                tool: jev.tool_name.clone(),
            };
            let evidence = Evidence {
                ctrace_log: None,
                ctrace_ts: None,
                jsonl_session: jev.jsonl_basename.clone(),
                jsonl_turn: jev.jsonl_turn,
            };
            out.push(AttributedEvent {
                ts: iso8601_utc(jev.ts_unix),
                ts_unix: jev.ts_unix,
                path: jev.path.clone(),
                op: jev.op,
                actor,
                confidence: Confidence::Medium,
                evidence,
                size_delta_bytes: None,
                via: jev.tool_name.clone(),
            });
        }

        // 3 — if still empty, fall back to stat.
        if out.is_empty() {
            if let Some(s) = stat_fallback {
                out.push(AttributedEvent {
                    ts: iso8601_utc(s.ts_unix),
                    ts_unix: s.ts_unix,
                    path: s.path.clone(),
                    op: EventOp::Write,
                    actor: Actor::Unknown { comm: None },
                    confidence: Confidence::Low,
                    evidence: Evidence::default(),
                    size_delta_bytes: None,
                    via: None,
                });
            }
        }

        out.sort_by_key(|e| e.ts_unix);
        out
    }

    /// Same as [`attribute_path`] but returns every path the attributor
    /// has events for. Used by `ls` and `summary`.
    #[must_use]
    pub fn attribute_all(&self) -> Vec<AttributedEvent> {
        let mut paths: Vec<&str> = self.events.iter().map(|(_, e)| e.path.as_str()).collect();
        paths.sort_unstable();
        paths.dedup();
        let mut out = Vec::new();
        for p in paths {
            out.extend(self.attribute_path(p, None));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrace_ev(ts: i64, path: &str, comm: &str) -> RawEvent {
        RawEvent {
            ts_unix: ts,
            path: path.to_string(),
            op: EventOp::Write,
            source: SourceKind::Ctrace,
            pid: Some(1),
            ppid: Some(0),
            comm: Some(comm.to_string()),
            jsonl_basename: None,
            jsonl_turn: None,
            tool_name: None,
            size_delta_bytes: Some(10),
        }
    }

    fn jsonl_ev(ts: i64, path: &str, base: &str, turn: u32, tool: &str) -> RawEvent {
        RawEvent {
            ts_unix: ts,
            path: path.to_string(),
            op: EventOp::Write,
            source: SourceKind::ClaudeJsonl,
            pid: None,
            ppid: None,
            comm: None,
            jsonl_basename: Some(base.to_string()),
            jsonl_turn: Some(turn),
            tool_name: Some(tool.to_string()),
            size_delta_bytes: None,
        }
    }

    #[test]
    fn ctrace_plus_jsonl_yields_high_confidence_session() {
        let mut a = Attributor::new();
        a.push("/tmp/ctrace.ndjson", ctrace_ev(100, "/tmp/x", "claude"));
        a.push(
            "/tmp/sess.jsonl",
            jsonl_ev(101, "/tmp/x", "sess.jsonl", 33, "Edit"),
        );
        let events = a.attribute_path("/tmp/x", None);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].confidence, Confidence::High);
        assert_eq!(
            events[0].actor.to_label(),
            "claude-session:sess.jsonl:33"
        );
    }

    #[test]
    fn interactive_editor_attribution() {
        let mut a = Attributor::new();
        a.push("/tmp/ctrace.ndjson", ctrace_ev(100, "/tmp/x", "nvim"));
        let events = a.attribute_path("/tmp/x", None);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].confidence, Confidence::Medium);
        match &events[0].actor {
            Actor::UserInteractive { comm } => assert_eq!(comm, "nvim"),
            _ => panic!("not user-interactive"),
        }
    }

    #[test]
    fn stat_fallback_only_when_no_other_source() {
        let a = Attributor::new();
        let stat = RawEvent {
            ts_unix: 50,
            path: "/tmp/x".to_string(),
            op: EventOp::Write,
            source: SourceKind::Stat,
            pid: None,
            ppid: None,
            comm: None,
            jsonl_basename: None,
            jsonl_turn: None,
            tool_name: None,
            size_delta_bytes: None,
        };
        let events = a.attribute_path("/tmp/x", Some(stat));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].confidence, Confidence::Low);
    }

    #[test]
    fn ctrace_only_resolves_unknown_when_comm_unmapped() {
        let mut a = Attributor::new();
        a.push(
            "/tmp/ctrace.ndjson",
            ctrace_ev(100, "/tmp/x", "weirdtool"),
        );
        let events = a.attribute_path("/tmp/x", None);
        assert_eq!(events.len(), 1);
        match &events[0].actor {
            Actor::Unknown { comm } => assert_eq!(comm.as_deref(), Some("weirdtool")),
            _ => panic!("expected Unknown"),
        }
        assert_eq!(events[0].confidence, Confidence::Medium);
    }
}
