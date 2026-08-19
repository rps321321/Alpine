use alpine_control_plane::{Alpine, Decision};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(name = "alpine", version, about = "Local AI inference control plane")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Inspect {
        #[arg(long, default_value = "config/support-envelope.json")]
        envelope: PathBuf,
        #[arg(long, default_value_t = 10_000)]
        timeout_ms: u64,
        #[arg(long)]
        compact: bool,
    },
    Qualify {
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        compact: bool,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(decision) => ExitCode::from(decision.exit_code()),
        Err(error) => {
            eprintln!("alpine: {error}");
            ExitCode::from(64)
        }
    }
}

fn run(cli: Cli) -> Result<Decision, Box<dyn std::error::Error>> {
    match cli.command {
        Commands::Inspect {
            envelope,
            timeout_ms,
            compact,
        } => {
            let report = Alpine::inspect_support(&envelope, Duration::from_millis(timeout_ms))?;
            write_json(&report, compact)?;
            Ok(report.decision)
        }
        Commands::Qualify { request, compact } => {
            let report = Alpine::qualify(&request)?;
            write_json(&report, compact)?;
            Ok(report.decision)
        }
    }
}

fn write_json(value: &impl serde::Serialize, compact: bool) -> Result<(), serde_json::Error> {
    if compact {
        println!("{}", serde_json::to_string(value)?);
    } else {
        println!("{}", serde_json::to_string_pretty(value)?);
    }
    Ok(())
}
