//! Checks the staged Rust files of `komga-rust/` against the growth budgets.
//!
//! Prints a warning for each soft threshold reached and an error for each hard limit exceeded.
//! Exits with 1 when the commit must be blocked and 2 when the check itself cannot run.

mod budget;
mod exceptions;
mod staged;

use std::env;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};

use budget::Severity;
use exceptions::Exceptions;
use staged::Index;

enum Mode {
    Staged,
    All,
}

fn parse_mode() -> Result<Mode> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        None => Ok(Mode::Staged),
        Some("--all") if args.next().is_none() => Ok(Mode::All),
        Some(arg) => bail!("unexpected argument {arg:?} (expected optional `--all`)"),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(err) => {
            eprintln!("code-budget: {err:#}");
            ExitCode::from(2)
        }
    }
}

/// Returns whether the check passed without hard-limit errors.
fn run() -> Result<bool> {
    let mode = parse_mode()?;
    let index = Index::open(Path::new("."))?;
    let (paths, exceptions) = match mode {
        Mode::Staged => {
            let changed = index.changed_paths()?;
            let exceptions = if index.contains(exceptions::PATH)? {
                Exceptions::parse(&index.read(exceptions::PATH)?).context(exceptions::PATH)?
            } else {
                Exceptions::default()
            };
            if changed.iter().any(|path| path == exceptions::PATH) {
                println!(
                    "note: {} changed; every exception needs maintainer approval",
                    exceptions::PATH
                );
            }
            (changed, exceptions)
        }
        Mode::All => {
            let all = index.all_paths()?;
            let exceptions = if index.worktree_contains(exceptions::PATH) {
                Exceptions::parse(&index.read_worktree(exceptions::PATH)?)
                    .context(exceptions::PATH)?
            } else {
                Exceptions::default()
            };
            (all, exceptions)
        }
    };

    let mut passed = true;
    for path in paths.iter().filter(|path| path.ends_with(".rs")) {
        let content = match mode {
            Mode::Staged => index.read(path)?,
            Mode::All => index.read_worktree(path)?,
        };
        if let Some(source) = budget::generated_source(&content) {
            println!("note: {path} is generated ({source}), budgets skipped");
            continue;
        }
        match budget::check(path, &content, &exceptions) {
            Ok(findings) => {
                for finding in findings {
                    passed &= finding.severity != Severity::Error;
                    println!("{finding}");
                }
            }
            Err(err) => {
                // Function budgets cannot be measured without a syntax tree.
                passed = false;
                let start = err.span().start();
                println!(
                    "error: {path}:{}:{}: cannot parse: {err}",
                    start.line,
                    start.column + 1
                );
            }
        }
    }
    Ok(passed)
}
