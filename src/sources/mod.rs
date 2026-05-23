//! Source parsers. Each module exposes a streaming reader that yields
//! [`crate::event::RawEvent`]s. Parsers are strict about IO errors
//! (missing files surface as `Ok(empty)` so graceful degradation
//! survives at the call site) but lenient about per-line schema drift
//! — a malformed line is skipped, not fatal.

pub mod ctrace;
pub mod jsonl;
pub mod stat;
