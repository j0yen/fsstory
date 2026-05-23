//! Core event types for fsstory.

use serde::{Deserialize, Serialize};

/// What kind of source produced a raw event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceKind {
    /// ctrace ndjson session log.
    Ctrace,
    /// Claude project transcript JSONL (`~/.claude/projects/*/[uuid].jsonl`).
    ClaudeJsonl,
    /// stat()-derived fallback (no other source attributes it).
    Stat,
}

/// Filesystem operation captured by the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventOp {
    /// Write or create.
    Write,
    /// Unlink.
    Unlink,
    /// Read-only access we surfaced (rare; mainly stat).
    Access,
}

/// A raw event from a single source, before attribution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawEvent {
    /// Absolute UNIX timestamp in seconds (UTC).
    pub ts_unix: i64,
    /// Path the event refers to (absolute, canonicalized where possible).
    pub path: String,
    /// Operation kind.
    pub op: EventOp,
    /// Source that produced this raw event.
    pub source: SourceKind,
    /// PID of the writing process, if known.
    pub pid: Option<u32>,
    /// Parent PID of the writing process, if known.
    pub ppid: Option<u32>,
    /// `comm` (process basename) of the writing process, if known.
    pub comm: Option<String>,
    /// For [`SourceKind::ClaudeJsonl`]: jsonl basename (`<uuid>.jsonl`).
    pub jsonl_basename: Option<String>,
    /// For [`SourceKind::ClaudeJsonl`]: turn index (0-based).
    pub jsonl_turn: Option<u32>,
    /// For [`SourceKind::ClaudeJsonl`]: tool name (`Edit`, `Write`, `Bash`, ...).
    pub tool_name: Option<String>,
    /// Size delta in bytes (write_size - prior_size), if known.
    pub size_delta_bytes: Option<i64>,
}

/// Resolved actor for an attributed event. The variants are kept
/// deliberately coarse — PRD §4.2 lists the precise output strings; we
/// produce those via [`Actor::to_label`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum Actor {
    /// A specific Claude tool_use (Edit / Write / NotebookEdit / MCP).
    ClaudeSession {
        /// jsonl basename (e.g. `df04d4-....jsonl`).
        jsonl_basename: String,
        /// Turn index inside that jsonl.
        turn: u32,
        /// Optional tool name (`Edit`, `Write`, ...).
        tool: Option<String>,
    },
    /// A Bash tool_use side-effect (the bash invocation, not a specific
    /// Edit/Write tool call).
    ClaudeBash {
        /// jsonl basename.
        jsonl_basename: String,
        /// Turn index.
        turn: u32,
    },
    /// An interactive editor (vim/nvim/code/zed/helix) with no claude
    /// ancestor PID.
    UserInteractive {
        /// The `comm` we matched against.
        comm: String,
    },
    /// Couldn't attribute. Carries the `comm` (if any) for diagnostics.
    Unknown {
        /// `comm` of the writer if known; `None` if even ctrace had no event.
        comm: Option<String>,
    },
}

impl Actor {
    /// Render the actor as the human-readable label the PRD shows
    /// in §4.2 (e.g. `claude-session:df04d4...:33`).
    #[must_use]
    pub fn to_label(&self) -> String {
        match self {
            Self::ClaudeSession {
                jsonl_basename,
                turn,
                ..
            } => format!("claude-session:{jsonl_basename}:{turn}"),
            Self::ClaudeBash {
                jsonl_basename,
                turn,
            } => format!("claude-bash:{jsonl_basename}:{turn}"),
            Self::UserInteractive { comm } => format!("user-interactive:{comm}"),
            Self::Unknown { comm: Some(c) } => format!("unknown:{c}"),
            Self::Unknown { comm: None } => "unknown".to_string(),
        }
    }
}

/// Confidence score for an attribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    /// ctrace event + matching JSONL tool_use.
    High,
    /// ctrace event with known-actor `comm` and plausible window, or
    /// JSONL match with weak time signal.
    Medium,
    /// stat() fallback only.
    Low,
}

impl Confidence {
    /// Render as a stable lowercase token (`high` | `medium` | `low`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }
}

/// An attributed event: a RawEvent plus the resolved actor and confidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttributedEvent {
    /// ISO-8601 UTC timestamp (e.g. `2026-05-22T23:48:04Z`).
    pub ts: String,
    /// UNIX timestamp in seconds, retained for sorting and diffs.
    pub ts_unix: i64,
    /// Path the event refers to.
    pub path: String,
    /// Operation kind.
    pub op: EventOp,
    /// Resolved actor.
    pub actor: Actor,
    /// Confidence score.
    pub confidence: Confidence,
    /// PRD §4.3 evidence field — pointers back to the source records.
    pub evidence: Evidence,
    /// Size delta in bytes, if known.
    pub size_delta_bytes: Option<i64>,
    /// Tool name (`Edit`/`Write`/`Bash`/...) when known.
    pub via: Option<String>,
}

/// Pointers from the attributed event back to the source records that
/// support it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Evidence {
    /// ctrace session log path, when applicable.
    pub ctrace_log: Option<String>,
    /// ctrace timestamp from the raw event.
    pub ctrace_ts: Option<i64>,
    /// Claude jsonl basename, when applicable.
    pub jsonl_session: Option<String>,
    /// Claude jsonl turn index, when applicable.
    pub jsonl_turn: Option<u32>,
}
