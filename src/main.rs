use alpine_control_plane::{
    AcquireSessionOptions, Alpine, CleanRestartStabilityOptions, EvidencePhase,
    ExternalEvidenceKind, GoldenAgentOptions, MicrobenchmarkOptions, NearLimitContextOptions,
    OperatorReviewOptions, ReleaseSessionOptions, RollbackProofOptions, RunQualificationOptions,
    SameProcessStabilityOptions, SessionAcquisition, StartSessionOptions, StopSessionOptions,
    TuningDisposition, TuningOptions,
};
use clap::{Parser, Subcommand, ValueEnum};
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(name = "alpine", version, about = "Local AI inference control plane")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum BenchmarkPhase {
    Tuning,
    Final,
}

impl From<BenchmarkPhase> for EvidencePhase {
    fn from(value: BenchmarkPhase) -> Self {
        match value {
            BenchmarkPhase::Tuning => Self::Tuning,
            BenchmarkPhase::Final => Self::Final,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum QualificationStage {
    Candidate,
    Validated,
    Production,
}

impl From<QualificationStage> for alpine_control_plane::QualificationTarget {
    fn from(value: QualificationStage) -> Self {
        match value {
            QualificationStage::Candidate => Self::Candidate,
            QualificationStage::Validated => Self::Validated,
            QualificationStage::Production => Self::Production,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum EvidenceKind {
    OperatorReviewedCapabilityReport,
}

impl From<EvidenceKind> for ExternalEvidenceKind {
    fn from(value: EvidenceKind) -> Self {
        match value {
            EvidenceKind::OperatorReviewedCapabilityReport => {
                Self::OperatorReviewedCapabilityReport
            }
        }
    }
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
        #[arg(long, value_enum, default_value_t = BenchmarkPhase::Tuning)]
        phase: BenchmarkPhase,
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
    Tune {
        #[arg(long)]
        baseline_run: String,
        #[arg(long = "candidate-run", required = true)]
        candidate_runs: Vec<String>,
        #[arg(long, default_value_os_t = default_repository_root())]
        repository_root: PathBuf,
        #[arg(long, default_value_os_t = default_database())]
        database: PathBuf,
        #[arg(long)]
        compact: bool,
    },
    RecordEvidence {
        anchor_run_id: String,
        #[arg(long, value_enum)]
        kind: EvidenceKind,
        #[arg(long)]
        evidence: PathBuf,
        #[arg(long)]
        reviewed_by: Option<String>,
        #[arg(long, default_value_os_t = default_repository_root())]
        repository_root: PathBuf,
        #[arg(long, default_value_os_t = default_result_root())]
        result_root: PathBuf,
        #[arg(long, default_value_os_t = default_database())]
        database: PathBuf,
        #[arg(long)]
        compact: bool,
    },
    SameProcessStability {
        anchor_run_id: String,
        #[arg(long, default_value_os_t = default_repository_root())]
        repository_root: PathBuf,
        #[arg(long, default_value_os_t = default_install_root())]
        install_root: PathBuf,
        #[arg(long, default_value_os_t = default_result_root())]
        result_root: PathBuf,
        #[arg(long, default_value_os_t = default_database())]
        database: PathBuf,
        #[arg(long)]
        allow_legacy_identity: bool,
        #[arg(long, default_value_t = 15_000)]
        lease_timeout_ms: u64,
        #[arg(long, default_value_t = 600_000)]
        startup_timeout_ms: u64,
        #[arg(long, default_value_t = 600_000)]
        request_timeout_ms: u64,
        #[arg(long)]
        compact: bool,
    },
    CleanRestartStability {
        anchor_run_id: String,
        #[arg(long, default_value_os_t = default_repository_root())]
        repository_root: PathBuf,
        #[arg(long, default_value_os_t = default_install_root())]
        install_root: PathBuf,
        #[arg(long, default_value_os_t = default_result_root())]
        result_root: PathBuf,
        #[arg(long, default_value_os_t = default_database())]
        database: PathBuf,
        #[arg(long)]
        allow_legacy_identity: bool,
        #[arg(long, default_value_t = 15_000)]
        lease_timeout_ms: u64,
        #[arg(long, default_value_t = 600_000)]
        startup_timeout_ms: u64,
        #[arg(long, default_value_t = 600_000)]
        request_timeout_ms: u64,
        #[arg(long)]
        compact: bool,
    },
    NearLimitContext {
        anchor_run_id: String,
        #[arg(long, default_value_t = 0.85)]
        ratio: f64,
        #[arg(long, default_value_os_t = default_repository_root())]
        repository_root: PathBuf,
        #[arg(long, default_value_os_t = default_install_root())]
        install_root: PathBuf,
        #[arg(long, default_value_os_t = default_result_root())]
        result_root: PathBuf,
        #[arg(long, default_value_os_t = default_database())]
        database: PathBuf,
        #[arg(long)]
        allow_legacy_identity: bool,
        #[arg(long, default_value_t = 15_000)]
        lease_timeout_ms: u64,
        #[arg(long, default_value_t = 600_000)]
        startup_timeout_ms: u64,
        #[arg(long, default_value_t = 600_000)]
        request_timeout_ms: u64,
        #[arg(long)]
        compact: bool,
    },
    GoldenAgent {
        anchor_run_id: String,
        #[arg(long, default_value = "python-off-by-one")]
        task: String,
        #[arg(long, default_value_os_t = default_repository_root())]
        repository_root: PathBuf,
        #[arg(long, default_value_os_t = default_install_root())]
        install_root: PathBuf,
        #[arg(long, default_value_os_t = default_result_root())]
        result_root: PathBuf,
        #[arg(long, default_value_os_t = default_database())]
        database: PathBuf,
        #[arg(long)]
        allow_legacy_identity: bool,
        #[arg(long, default_value_t = 15_000)]
        lease_timeout_ms: u64,
        #[arg(long, default_value_t = 600_000)]
        startup_timeout_ms: u64,
        #[arg(long)]
        compact: bool,
    },
    RollbackProof {
        anchor_run_id: String,
        #[arg(long, default_value_os_t = default_repository_root())]
        repository_root: PathBuf,
        #[arg(long, default_value_os_t = default_install_root())]
        install_root: PathBuf,
        #[arg(long, default_value_os_t = default_result_root())]
        result_root: PathBuf,
        #[arg(long, default_value_os_t = default_database())]
        database: PathBuf,
        #[arg(long)]
        allow_legacy_identity: bool,
        #[arg(long, default_value_t = 15_000)]
        lease_timeout_ms: u64,
        #[arg(long, default_value_t = 600_000)]
        startup_timeout_ms: u64,
        #[arg(long, default_value_t = 600_000)]
        request_timeout_ms: u64,
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
        final_run_id: String,
        #[arg(long = "tuning-run", required = true)]
        tuning_run_ids: Vec<String>,
        #[arg(long, default_value_os_t = default_repository_root())]
        repository_root: PathBuf,
        #[arg(long, default_value_os_t = default_install_root())]
        install_root: PathBuf,
        #[arg(long, default_value_os_t = default_database())]
        database: PathBuf,
        #[arg(long, value_enum, default_value_t = QualificationStage::Candidate)]
        target: QualificationStage,
        #[arg(long, default_value_t = 10_000)]
        support_timeout_ms: u64,
        #[arg(long)]
        compact: bool,
    },
    QualifyRequest {
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        compact: bool,
    },
}

#[derive(Debug, Subcommand)]
enum SessionCommands {
    Acquire {
        #[arg(long, default_value_os_t = default_install_root())]
        install_root: PathBuf,
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        vision: bool,
        #[arg(long)]
        force_fallback: bool,
        #[arg(long)]
        allow_legacy_identity: bool,
        #[arg(long, default_value_t = 15_000)]
        lock_timeout_ms: u64,
        #[arg(long, default_value_t = 600_000)]
        startup_timeout_ms: u64,
        #[arg(long)]
        compact: bool,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Release {
        #[arg(long, default_value_os_t = default_install_root())]
        install_root: PathBuf,
        #[arg(long)]
        acquisition: PathBuf,
        #[arg(long)]
        keep_server: bool,
        #[arg(long, default_value_t = 15_000)]
        lock_timeout_ms: u64,
        #[arg(long, default_value_t = 600_000)]
        startup_timeout_ms: u64,
        #[arg(long)]
        compact: bool,
    },
    Start {
        #[arg(long, default_value_os_t = default_install_root())]
        install_root: PathBuf,
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        vision: bool,
        #[arg(long)]
        force_fallback: bool,
        #[arg(long, default_value_t = 15_000)]
        lock_timeout_ms: u64,
        #[arg(long, default_value_t = 600_000)]
        startup_timeout_ms: u64,
        #[arg(long)]
        compact: bool,
    },
    Stop {
        #[arg(long, default_value_os_t = default_install_root())]
        install_root: PathBuf,
        #[arg(long, default_value_t = 15_000)]
        lock_timeout_ms: u64,
        #[arg(long)]
        allow_legacy_identity: bool,
        #[arg(long)]
        compact: bool,
    },
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
            phase,
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
                phase: phase.into(),
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
        Commands::Tune {
            baseline_run,
            candidate_runs,
            repository_root,
            database,
            compact,
        } => {
            let report = Alpine::tune(&TuningOptions {
                repository_root,
                database,
                baseline_run_id: baseline_run,
                candidate_run_ids: candidate_runs,
            })?;
            let code = if report.disposition == TuningDisposition::NotProven {
                2
            } else {
                0
            };
            write_json(&report, compact)?;
            Ok(code)
        }
        Commands::RecordEvidence {
            anchor_run_id,
            kind,
            evidence,
            reviewed_by,
            repository_root,
            result_root,
            database,
            compact,
        } => {
            let kind: ExternalEvidenceKind = kind.into();
            if kind != ExternalEvidenceKind::OperatorReviewedCapabilityReport {
                return Err(
                    "automated evidence must be produced by its Rust-owned harness; record-evidence is reserved for the human capability review"
                        .into(),
                );
            }
            let metadata = std::fs::metadata(&evidence)?;
            if !metadata.is_file() || metadata.len() > 1024 * 1024 {
                return Err("evidence details must be a JSON file no larger than 1 MiB".into());
            }
            let details = serde_json::from_slice(&std::fs::read(&evidence)?)?;
            let report = Alpine::record_operator_review(&OperatorReviewOptions {
                repository_root,
                database,
                result_root,
                anchor_run_id,
                evidence: details,
                reviewed_by: reviewed_by.ok_or(
                    "record-evidence requires --reviewed-by for the human capability gate",
                )?,
            })?;
            write_json(&report, compact)?;
            Ok(0)
        }
        Commands::SameProcessStability {
            anchor_run_id,
            repository_root,
            install_root,
            result_root,
            database,
            allow_legacy_identity,
            lease_timeout_ms,
            startup_timeout_ms,
            request_timeout_ms,
            compact,
        } => {
            let report = Alpine::run_same_process_stability(&SameProcessStabilityOptions {
                repository_root,
                install_root,
                database,
                result_root,
                anchor_run_id,
                allow_legacy_identity,
                lease_timeout: Duration::from_millis(lease_timeout_ms),
                startup_timeout: Duration::from_millis(startup_timeout_ms),
                request_timeout: Duration::from_millis(request_timeout_ms),
            })?;
            write_json(&report, compact)?;
            Ok(0)
        }
        Commands::CleanRestartStability {
            anchor_run_id,
            repository_root,
            install_root,
            result_root,
            database,
            allow_legacy_identity,
            lease_timeout_ms,
            startup_timeout_ms,
            request_timeout_ms,
            compact,
        } => {
            let report = Alpine::run_clean_restart_stability(&CleanRestartStabilityOptions {
                repository_root,
                install_root,
                database,
                result_root,
                anchor_run_id,
                allow_legacy_identity,
                lease_timeout: Duration::from_millis(lease_timeout_ms),
                startup_timeout: Duration::from_millis(startup_timeout_ms),
                request_timeout: Duration::from_millis(request_timeout_ms),
            })?;
            write_json(&report, compact)?;
            Ok(0)
        }
        Commands::NearLimitContext {
            anchor_run_id,
            ratio,
            repository_root,
            install_root,
            result_root,
            database,
            allow_legacy_identity,
            lease_timeout_ms,
            startup_timeout_ms,
            request_timeout_ms,
            compact,
        } => {
            let report = Alpine::run_near_limit_context(&NearLimitContextOptions {
                repository_root,
                install_root,
                database,
                result_root,
                anchor_run_id,
                ratio,
                allow_legacy_identity,
                lease_timeout: Duration::from_millis(lease_timeout_ms),
                startup_timeout: Duration::from_millis(startup_timeout_ms),
                request_timeout: Duration::from_millis(request_timeout_ms),
            })?;
            write_json(&report, compact)?;
            Ok(0)
        }
        Commands::GoldenAgent {
            anchor_run_id,
            task,
            repository_root,
            install_root,
            result_root,
            database,
            allow_legacy_identity,
            lease_timeout_ms,
            startup_timeout_ms,
            compact,
        } => {
            let report = Alpine::run_golden_agent(&GoldenAgentOptions {
                repository_root,
                install_root,
                database,
                result_root,
                anchor_run_id,
                task_id: task,
                allow_legacy_identity,
                lease_timeout: Duration::from_millis(lease_timeout_ms),
                startup_timeout: Duration::from_millis(startup_timeout_ms),
            })?;
            write_json(&report, compact)?;
            Ok(0)
        }
        Commands::RollbackProof {
            anchor_run_id,
            repository_root,
            install_root,
            result_root,
            database,
            allow_legacy_identity,
            lease_timeout_ms,
            startup_timeout_ms,
            request_timeout_ms,
            compact,
        } => {
            let report = Alpine::prove_rollback(&RollbackProofOptions {
                repository_root,
                install_root,
                database,
                result_root,
                anchor_run_id,
                allow_legacy_identity,
                lease_timeout: Duration::from_millis(lease_timeout_ms),
                startup_timeout: Duration::from_millis(startup_timeout_ms),
                request_timeout: Duration::from_millis(request_timeout_ms),
            })?;
            write_json(&report, compact)?;
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
            SessionCommands::Acquire {
                install_root,
                profile,
                vision,
                force_fallback,
                allow_legacy_identity,
                lock_timeout_ms,
                startup_timeout_ms,
                compact,
                output,
            } => {
                let acquisition = Alpine::acquire_session(&AcquireSessionOptions {
                    install_root,
                    profile,
                    vision,
                    force_fallback,
                    allow_legacy_identity,
                    lock_timeout: Duration::from_millis(lock_timeout_ms),
                    startup_timeout: Duration::from_millis(startup_timeout_ms),
                })?;
                if let Some(path) = output {
                    write_json_file(&path, &acquisition, compact)?;
                } else {
                    write_json(&acquisition, compact)?;
                }
                Ok(0)
            }
            SessionCommands::Release {
                install_root,
                acquisition,
                keep_server,
                lock_timeout_ms,
                startup_timeout_ms,
                compact,
            } => {
                const MAX_ACQUISITION_BYTES: u64 = 1024 * 1024;
                let metadata = std::fs::metadata(&acquisition)?;
                if metadata.len() > MAX_ACQUISITION_BYTES {
                    return Err(format!(
                        "session acquisition exceeds the 1 MiB input limit: {}",
                        acquisition.display()
                    )
                    .into());
                }
                let bytes = std::fs::read(&acquisition)?;
                let acquisition: SessionAcquisition = serde_json::from_slice(&bytes)?;
                let report = Alpine::release_session(&ReleaseSessionOptions {
                    install_root,
                    acquisition,
                    keep_server,
                    lock_timeout: Duration::from_millis(lock_timeout_ms),
                    startup_timeout: Duration::from_millis(startup_timeout_ms),
                })?;
                write_json(&report, compact)?;
                Ok(0)
            }
            SessionCommands::Start {
                install_root,
                profile,
                vision,
                force_fallback,
                lock_timeout_ms,
                startup_timeout_ms,
                compact,
            } => {
                let report = Alpine::start_session(&StartSessionOptions {
                    install_root,
                    profile,
                    vision,
                    force_fallback,
                    lock_timeout: Duration::from_millis(lock_timeout_ms),
                    startup_timeout: Duration::from_millis(startup_timeout_ms),
                })?;
                write_json(&report, compact)?;
                Ok(0)
            }
            SessionCommands::Stop {
                install_root,
                lock_timeout_ms,
                allow_legacy_identity,
                compact,
            } => {
                let report = Alpine::stop_session(&StopSessionOptions {
                    install_root,
                    lock_timeout: Duration::from_millis(lock_timeout_ms),
                    allow_legacy_identity,
                })?;
                write_json(&report, compact)?;
                Ok(0)
            }
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
        Commands::Qualify {
            final_run_id,
            tuning_run_ids,
            repository_root,
            install_root,
            database,
            target,
            support_timeout_ms,
            compact,
        } => {
            let report = Alpine::qualify_run(&RunQualificationOptions {
                repository_root,
                install_root,
                database,
                final_run_id,
                tuning_run_ids,
                target: target.into(),
                support_timeout: Duration::from_millis(support_timeout_ms),
            })?;
            write_json(&report, compact)?;
            Ok(report.decision.exit_code())
        }
        Commands::QualifyRequest { request, compact } => {
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

fn write_json_file(
    path: &std::path::Path,
    value: &impl serde::Serialize,
    compact: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    if compact {
        serde_json::to_writer(&mut temporary, value)?;
    } else {
        serde_json::to_writer_pretty(&mut temporary, value)?;
    }
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}
