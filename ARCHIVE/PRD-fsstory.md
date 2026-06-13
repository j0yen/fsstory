# PRD: fsstory (Attribution-Aware Filesystem Timeline)

This is the intake-derived MUST/SHOULD/MAY acceptance criteria for fsstory.
See `/home/jsy/projects/autobuilder/PRD-attribution-timeline.md` for the source PRD.

## Acceptance Criteria

- AC1 (MUST): `fsstory path <path>` emits a JSON object with `path` and `events[]` where each event has `ts`, `actor`, `op`, `confidence` fields.
- AC2 (MUST): Given a synthetic ctrace ndjson + matching Claude JSONL Edit on the same path/window, `fsstory path` attributes that event to `claude-session:<jsonl-basename>:<turn>` with `confidence: high`.
- AC3 (MUST): When ctrace is absent (graceful degradation), `fsstory path` still emits a stat-fallback event with `confidence: low` and exits 0.
- AC4 (MUST): `fsstory who-wrote <path>` exits 0 and emits one line: `<ts>\t<actor>\t<confidence>`.
- AC5 (MUST): Read-only invariant: across the full test suite, fsstory does not modify any file under the source directories it ingests.
- AC6 (SHOULD): ctrace events whose `comm` matches a known interactive editor resolve to `actor: user-interactive` with `confidence: medium`.
- AC7 (SHOULD): `fsstory summary --root <dir>` emits a by-actor histogram of file changes.
- AC8 (MAY): `--format json` output is byte-stable across runs on the same fixture.

Verification: every AC has a corresponding test in `tests/acceptance_<id>.rs`.
