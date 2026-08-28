//! `tidycraft` — the headless check pipeline. Every verb is read-only; exit
//! codes are a contract: 0 clean, 1 findings reached --fail-on, 2 usage or
//! config error, 3 runtime error.

mod baseline;
mod check;
mod explain;
mod rules_cmd;
mod scan_cmd;
mod util;

use clap::{Parser, Subcommand};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "tidycraft",
    version,
    about = "Cross-engine asset lint: scan a game project and check it against tidycraft.toml",
    after_help = "Exit codes: 0 clean · 1 findings at or above --fail-on · 2 usage/config error · 3 runtime error.\n\
                  Every verb is read-only. Output is English; paths print project-relative with forward slashes."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scan a project, run every analyzer phase, and report issues
    Check(check::CheckArgs),
    /// List every rule id and a project's effective configuration
    Rules(rules_cmd::RulesArgs),
    /// Explain one rule: what it checks, when it fires, how to tune it
    Explain {
        /// Rule id or family, e.g. `naming.prefix`, `texture`, `duplicate`
        rule_id: String,
    },
    /// Dump the scanned asset inventory as JSON
    Scan(scan_cmd::ScanArgs),
}

/// Failures mapped onto the exit-code contract.
pub enum CliError {
    /// Bad arguments or unusable configuration → exit 2.
    Config(String),
    /// The environment failed (IO, scan) → exit 3.
    Runtime(String),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Check(args) => check::run(args),
        Command::Rules(args) => rules_cmd::run(args),
        Command::Explain { rule_id } => explain::run(&rule_id),
        Command::Scan(args) => scan_cmd::run(args),
    };
    match result {
        Ok(code) => code,
        Err(CliError::Config(msg)) => {
            eprintln!("error: {msg}");
            ExitCode::from(2)
        }
        Err(CliError::Runtime(msg)) => {
            eprintln!("error: {msg}");
            ExitCode::from(3)
        }
    }
}
