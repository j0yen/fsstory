# fsstory

> When a file on this laptop changes, the user (Joe) needs to know who changed it (claude-session vs user-interactive vs package-manager vs unknown), when, and with what confidence, so trust calibration and forensics stop requiring three Bash invocations and guesswork.

## Why

When a file on this laptop changes, the user (Joe) needs to know who changed it (claude-session vs user-interactive vs package-manager vs unknown), when, and with what confidence, so trust calibration and forensics stop requiring three Bash invocations and guesswork. The chain underneath this surface request: stat+mtime answer 'when' but not 'who/why'; ctrace knows PID+comm of writes; Claude session JSONLs know which turn/tool ran; fsstory is the read-only joiner over those sources that produces an attributed per-path timeline.

## Build

```sh
cargo build --release
```

Produces `target/release/fsstory`. Symlink into `~/.local/bin/` if you want it on `$PATH`.

## Usage

```sh
fsstory --help
```

## Audience

Joe Yen, a single user on one Arch Linux laptop, running Claude Code sessions throughout the day. Consumes fsstory both directly via CLI and indirectly via /self-review Phase A. No multi-user, no remote hosts, no real-time stream — snapshot-on-demand queries only.

## Acceptance criteria

This project was scaffolded from a PRD via the `autobuilder` pipeline. The MUST-level acceptance criteria are:

- **AC1**: `fsstory path <path>` emits a JSON object with `path` and `events[]`, where each event has `ts`, `actor`, `op`, and `confidence` fields. Schema is stable enough that a test can parse it.
- **AC2**: Given a synthetic ctrace ndjson log containing a write event on `<path>` from `comm=claude` and a matching Claude session JSONL with an Edit tool_use on the same `<path>` in the same time window, `fsstory path <path>` attributes that eve...
- **AC3**: When no ctrace log is available (graceful degradation), `fsstory path <path>` still emits an event list derived from stat mtime + path heuristics, with `confidence: low`, and exits 0. It never errors solely because ctrace was off.
- **AC4**: `fsstory who-wrote <path>` exits 0 and emits exactly one line: the latest attributed event for `<path>` formatted as `<ts>\t<actor>\t<confidence>`.
- **AC5**: Read-only invariant: across the full test suite, fsstory does not write to or modify any file under the source directories it ingests (ctrace logs, Claude project JSONLs, pacman log). Verified by a tempdir fixture + per-file mtime/size s...

Each AC has a matching integration test under `tests/acceptance_ac<n>.rs`.

## Provenance

Built via the [`autobuilder`](https://github.com/j0yen/autobuilder) pipeline (PRD intake -> intent-card -> scaffold -> iterate-and-prove). Originally consolidated as a subdir of the [`wintermute`](https://github.com/j0yen/wintermute) monorepo; this standalone repo is a fresh-init snapshot for easier consumption and distribution.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
