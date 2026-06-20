# fsstory

`fsstory` answers a question `stat` can't: when a file on this laptop changed, *who* changed it — a Claude session, an interactive editor, or something unattributed — when, and with what confidence. Read-only, snapshot on demand.

## Why it exists

`stat` tells you a file's mtime. It does not tell you who wrote it. On a laptop where Claude sessions edit files all day alongside your own work, "when did this change" is the easy half of the question; "who, and why" is the half that matters for trust and forensics — and answering it usually means three `Bash` invocations and a guess.

The information exists, just scattered. ctrace knows the PID and `comm` of each write. Claude session transcripts know which turn and which tool ran. `stat` knows the mtime. `fsstory` is the read-only joiner over those sources: it produces one attributed timeline per path, so the answer is a single command instead of a reconstruction.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/j0yen/fsstory/main/install.sh | bash
```

Or build it yourself — requires `cargo` and `rustc 1.85+`:

```sh
git clone --depth 1 https://github.com/j0yen/fsstory.git
cd fsstory
./install.sh        # cargo install --path . --locked → ~/.cargo/bin/fsstory
```

## Quickstart

```sh
# Full attributed timeline for one path (JSON by default)
fsstory path src/main.rs --since 24h

# Just the latest event — one tab-separated line: <ts>\t<actor>\t<confidence>
fsstory who-wrote src/main.rs

# By-actor histogram under a directory
fsstory summary --root . --since 7d
```

Each event carries a timestamp, an actor, an operation, and a confidence. When ctrace data isn't available, `fsstory` still answers from `stat` mtime plus path heuristics, marks the events `confidence: low`, and exits 0 — it degrades, it doesn't error.

## Actors

The attribution it can assign:

| Actor | Meaning |
|---|---|
| `claude-session:<jsonl>:<turn>` | A specific Claude tool-use (Edit / Write / NotebookEdit / MCP). |
| `claude-bash:<jsonl>:<turn>` | A side-effect of a Bash tool-use, not a specific edit. |
| `user-interactive:<comm>` | An interactive editor (vim, nvim, code, zed, helix) with no Claude ancestor. |
| `unknown[:<comm>]` | Couldn't attribute; carries the writer's `comm` if ctrace saw one. |

## Sources

Three, joined per query: ctrace write logs (`~/.cache/ctrace/sessions`, overridable with `--ctrace-root`), Claude session transcripts (`~/.claude/projects`, overridable with `--projects-root`), and `stat` mtime. Pacman and journald are deliberately out of scope.

## Read-only by contract

`fsstory` never writes to or modifies any file it ingests. The invariant is verified in the test suite by snapshotting mtime and size of every fixture file across a full run.

## Where it fits

A single-user tool for one laptop — no multi-user, no remote hosts, no real-time stream. Used directly from the CLI, and indirectly by `/self-review` Phase A for trust calibration.

## Status

The three subcommands (`path`, `who-wrote`, `summary`), the four-actor attribution, and graceful ctrace-absent degradation are complete and covered by integration tests, one per acceptance criterion under `tests/`.

## Provenance

Built via the [`autobuilder`](https://github.com/j0yen/autobuilder) pipeline (PRD → intent-card → scaffold → iterate-and-prove). Originally a subdir of the [`wintermute`](https://github.com/j0yen/wintermute) monorepo; this is a fresh-init standalone snapshot.

## License

Apache-2.0 OR MIT, at your option ([LICENSE-APACHE](LICENSE-APACHE), [LICENSE-MIT](LICENSE-MIT)).
