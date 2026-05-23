//! fsstory CLI entry point.

#![cfg_attr(not(test), forbid(unsafe_code))]
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::doc_markdown,
    clippy::too_long_first_doc_paragraph,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::struct_field_names,
    clippy::module_name_repetitions
)]

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};

use fsstory::query::{
    Query, QueryEnv, QueryOutput, render_path_json, render_summary_json,
    render_summary_text, render_who_wrote, run as run_query,
};

/// Output format flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
    /// Stable JSON output (default for path/ls).
    Json,
    /// Human-readable text.
    Text,
}

impl Default for Format {
    fn default() -> Self {
        Self::Json
    }
}

#[derive(Debug, Parser)]
#[command(name = "fsstory", version, about = "Attribution-aware filesystem timeline (read-only)")]
struct Cli {
    /// Override the ctrace sessions directory (defaults to ~/.cache/ctrace/sessions).
    #[arg(long, global = true)]
    ctrace_root: Option<PathBuf>,
    /// Override the Claude projects directory (defaults to ~/.claude/projects).
    #[arg(long, global = true)]
    claude_root: Option<PathBuf>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Per-path timeline (JSON by default).
    Path {
        /// Path to inspect.
        path: PathBuf,
        /// Look-back window (e.g. `24h`, `30m`, `7d`).
        #[arg(long, default_value = "24h")]
        since: String,
        /// Output format.
        #[arg(long, value_enum, default_value = "json")]
        format: Format,
    },
    /// Latest event for `<path>`, single tab-separated line.
    WhoWrote {
        /// Path to inspect.
        path: PathBuf,
    },
    /// By-actor histogram under `<root>`.
    Summary {
        /// Root directory.
        #[arg(long)]
        root: PathBuf,
        /// Look-back window.
        #[arg(long, default_value = "24h")]
        since: String,
        /// Output format.
        #[arg(long, value_enum, default_value = "text")]
        format: Format,
    },
}

fn build_env(cli: &Cli) -> QueryEnv {
    let mut env = QueryEnv::default_for_user();
    if let Some(p) = cli.ctrace_root.clone() {
        env.ctrace_root = p;
    }
    if let Some(p) = cli.claude_root.clone() {
        env.claude_projects_root = p;
    }
    env
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let env = build_env(&cli);
    match &cli.cmd {
        Cmd::Path { path, since, format } => {
            let since_secs = fsstory::time::parse_since(since);
            let q = Query::Path {
                path: path.clone(),
                since_secs,
            };
            let out = run_query(&env, &q);
            if let QueryOutput::PathEvents { path, events } = out {
                match format {
                    Format::Json => match render_path_json(&path, &events) {
                        Ok(s) => {
                            println!("{s}");
                            ExitCode::SUCCESS
                        }
                        Err(err) => {
                            eprintln!("fsstory: serialize error: {err}");
                            ExitCode::from(2)
                        }
                    },
                    Format::Text => {
                        for e in &events {
                            println!(
                                "{}\t{}\t{}",
                                e.ts,
                                e.actor.to_label(),
                                e.confidence.as_str()
                            );
                        }
                        ExitCode::SUCCESS
                    }
                }
            } else {
                ExitCode::from(2)
            }
        }
        Cmd::WhoWrote { path } => {
            let q = Query::WhoWrote { path: path.clone() };
            let out = run_query(&env, &q);
            if let QueryOutput::Latest { latest, .. } = out {
                println!("{}", render_who_wrote(latest.as_ref()));
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            }
        }
        Cmd::Summary {
            root,
            since,
            format,
        } => {
            let since_secs = fsstory::time::parse_since(since);
            let q = Query::Summary {
                root: root.clone(),
                since_secs,
            };
            let out = run_query(&env, &q);
            if let QueryOutput::Summary { root, counts, total } = out {
                let s = match format {
                    Format::Json => match render_summary_json(&root, &counts, total) {
                        Ok(j) => j,
                        Err(err) => {
                            eprintln!("fsstory: serialize error: {err}");
                            return ExitCode::from(2);
                        }
                    },
                    Format::Text => render_summary_text(&root, &counts, total),
                };
                println!("{s}");
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            }
        }
    }
}
