use alpine_control_plane::{Alpine, MicrobenchmarkOptions};
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
    Benchmark {
        #[arg(long, default_value_os_t = default_repository_root())]
        repository_root: PathBuf,
        #[arg(long, default_value_os_t = default_install_root())]
        install_root: PathBuf,
        #[arg(long, default_value_os_t = default_result_root())]
        result_root: PathBuf,
        #[arg(long)]
        profile: String,
        #[arg(long, default_value_t = 5)]
        runs: u32,
        #[arg(long, default_value_t = 1)]
        warmups: u32,
        #[arg(long = "workload")]
        workloads: Vec<String>,
        #[arg(long)]
        notes: Option<String>,
        #[arg(long)]
        deep_verify_artifacts: bool,
        #[arg(long, default_value_t = 100)]
        lease_timeout_ms: u64,
        #[arg(long)]
        compact: bool,
    },
    Runs {
        #[arg(long, default_value_os_t = default_database())]
        database: PathBuf,
        #[arg(long, default_value_t = 20)]
        limit: u32,
        #[arg(long)]
        compact: bool,
    },
    Evidence {
        run_id: String,
        #[arg(long, default_value_os_t = default_database())]
        database: PathBuf,
        #[arg(long)]
        compact: bool,
    },
    Resolve {
        #[arg(long, default_value_os_t = default_install_root())]
        install_root: PathBuf,
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        allow_missing_runtime: bool,
        #[arg(long)]
        compact: bool,
    },
    Session {
        #[command(subcommand)]
        command: SessionCommands,
    },
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

#[derive(Debug, Subcommand)]
enum SessionCommands {
    Status {
        #[arg(long, default_value_os_t = default_install_root())]
        install_root: PathBuf,
        #[arg(long, default_value_t = 15_000)]
        lock_timeout_ms: u64,
        #[arg(long)]
        compact: bool,
    },
    Plan {
        #[arg(long, default_value_os_t = default_install_root())]
        install_root: PathBuf,
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        vision: bool,
        #[arg(long)]
        force_fallback: bool,
        #[arg(long)]
        compact: bool,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("alpine: {error}");
            ExitCode::from(64)
        }
    }
}

fn run(cli: Cli) -> Result<u8, Box<dyn std::error::Error>> {
    match cli.command {
        Commands::Benchmark {
            repository_root,
            install_root,
            result_root,
            profile,
            runs,
            warmups,
            workloads,
            notes,
            deep_verify_artifacts,
            lease_timeout_ms,
            compact,
        } => {
            let report = Alpine::run_microbenchmark(&MicrobenchmarkOptions {
                repository_root,
                install_root,
                result_root,
                profile,
                runs,
                warmups,
                workloads,
                notes,
                deep_verify_artifacts,
                lease_timeout: Duration::from_millis(lease_timeout_ms),
            })?;
            write_json(&report, compact)?;
            Ok(if report.status == "passed" { 0 } else { 1 })
        }
        Commands::Runs {
            database,
            limit,
            compact,
        } => {
            write_json(&Alpine::list_runs(&database, limit)?, compact)?;
            Ok(0)
        }
        Commands::Evidence {
            run_id,
            database,
            compact,
        } => {
            write_json(&Alpine::run_evidence(&database, &run_id)?, compact)?;
            Ok(0)
        }
        Commands::Resolve {
            install_root,
            profile,
            allow_missing_runtime,
            compact,
        } => {
            let resolved =
                Alpine::resolve_session(&install_root, profile.as_deref(), !allow_missing_runtime)?;
            write_json(&resolved, compact)?;
            Ok(0)
        }
        Commands::Session { command } => match command {
            SessionCommands::Status {
                install_root,
                lock_timeout_ms,
                compact,
            } => {
                let status =
                    Alpine::session_status(&install_root, Duration::from_millis(lock_timeout_ms))?;
                write_json(&status, compact)?;
                Ok(if status.foreign { 2 } else { 0 })
            }
            SessionCommands::Plan {
                install_root,
                profile,
                vision,
                force_fallback,
                compact,
            } => {
                let plan = Alpine::plan_session_arguments(
                    &install_root,
                    profile.as_deref(),
                    vision,
                    force_fallback,
                )?;
                write_json(&plan, compact)?;
                Ok(0)
            }
        },
        Commands::Inspect {
            envelope,
            timeout_ms,
            compact,
        } => {
            let report = Alpine::inspect_support(&envelope, Duration::from_millis(timeout_ms))?;
            write_json(&report, compact)?;
            Ok(report.decision.exit_code())
        }
        Commands::Qualify { request, compact } => {
            let report = Alpine::qualify(&request)?;
            write_json(&report, compact)?;
            Ok(report.decision.exit_code())
        }
    }
}

fn default_install_root() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("local-models")
}

fn default_database() -> PathBuf {
    PathBuf::from("results/results.sqlite3")
}

fn default_repository_root() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn default_result_root() -> PathBuf {
    PathBuf::from("results")
}

fn write_json(value: &impl serde::Serialize, compact: bool) -> Result<(), serde_json::Error> {
    if compact {
        println!("{}", serde_json::to_string(value)?);
    } else {
        println!("{}", serde_json::to_string_pretty(value)?);
    }
    Ok(())
}
