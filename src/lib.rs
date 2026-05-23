//! fsstory — read-only joiner over ctrace ndjson logs, Claude project
//! JSONL transcripts, and stat() metadata to produce an attributed
//! per-path filesystem timeline.
//!
//! Phase 0 surface (per PRD §7): `path`, `who-wrote`, and a SHOULD-level
//! `summary`. Pacman / journald sources are deliberately out of scope
//! for this run.
//!
//! The library splits into:
//! - [`event`]: the core [`RawEvent`] and [`AttributedEvent`] types.
//! - [`sources`]: streaming parsers for ctrace and Claude jsonl.
//! - [`attributor`]: the join that turns raw events into attributed
//!   events with a confidence score.
//! - [`query`]: query handlers consumed by `src/main.rs`.

#![cfg_attr(not(test), forbid(unsafe_code))]
#![allow(
    clippy::module_name_repetitions,
    clippy::doc_markdown,
    clippy::too_long_first_doc_paragraph,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cognitive_complexity,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::many_single_char_names,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::redundant_clone,
    clippy::assigning_clones,
    clippy::large_enum_variant,
    clippy::option_if_let_else,
    clippy::manual_let_else,
    clippy::needless_pass_by_value,
    clippy::must_use_candidate,
    clippy::missing_const_for_fn,
    clippy::implicit_hasher,
    clippy::struct_field_names,
    clippy::case_sensitive_file_extension_comparisons,
    clippy::option_option,
    clippy::map_unwrap_or
)]

pub mod attributor;
pub mod event;
pub mod query;
pub mod sources;
pub mod time;

pub use attributor::Attributor;
pub use event::{
    Actor, AttributedEvent, Confidence, EventOp, RawEvent, SourceKind,
};
pub use query::{Query, QueryEnv, QueryOutput, render_path_json, render_who_wrote};
