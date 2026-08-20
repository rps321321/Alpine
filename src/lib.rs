mod clock;
mod config;
mod context;
mod decision;
mod evidence;
mod experiment;
mod external;
mod golden;
mod identity;
mod locking;
mod process;
mod qualification;
mod rollback;
mod session;
mod stability;
mod support;
mod tuning;

pub use config::{Profile, ProfileStatus, ResolvedSession, SessionConfig};
pub use context::{NearLimitContextOptions, NearLimitContextReport};
pub use decision::Decision;
pub use evidence::{RunEvidence, RunSummary, StoredIdentity};
pub use experiment::{ExperimentReport, MicrobenchmarkOptions};
pub use external::{
    ExternalEvidenceKind, ExternalEvidenceStatus, ExternalEvidenceStatusKind,
    OperatorReviewOptions, RecordedExternalEvidence,
};
pub use golden::{GoldenAgentOptions, GoldenAgentReport};
pub use identity::runtime_bundle_sha256;
pub use qualification::{
    EvidencePhase, QualificationCheck, QualificationReport, QualificationRequest,
    QualificationTarget, RunQualificationOptions, RunQualificationReport,
};
pub use rollback::{RollbackProofOptions, RollbackProofReport};
pub use session::{
    AcquireSessionOptions, InferenceArguments, ProcessIdentityStrength, ReleaseSessionOptions,
    ReleaseSessionReport, SessionAcquisition, SessionAction, SessionSnapshot, SessionStatus,
    StartSessionOptions, StartSessionReport, StopSessionOptions, StopSessionReport,
};
pub use stability::{
    CleanRestartStabilityOptions, CleanRestartStabilityReport, SameProcessStabilityOptions,
    SameProcessStabilityReport,
};
pub use support::{SupportEnvelope, SupportReport};
pub use tuning::{TuningDisposition, TuningOptions, TuningReport};

use std::path::Path;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AlpineError {
    #[error("{0}")]
    InvalidInput(String),
}

pub struct Alpine;

impl Alpine {
    pub fn run_microbenchmark(
        options: &MicrobenchmarkOptions,
    ) -> Result<ExperimentReport, AlpineError> {
        experiment::run_microbenchmark(options).map_err(AlpineError::InvalidInput)
    }

    pub fn list_runs(database: &Path, limit: u32) -> Result<Vec<RunSummary>, AlpineError> {
        let store =
            evidence::EvidenceStore::open_read_only(database).map_err(AlpineError::InvalidInput)?;
        store.list_runs(limit).map_err(AlpineError::InvalidInput)
    }

    pub fn run_evidence(database: &Path, id: &str) -> Result<RunEvidence, AlpineError> {
        let store =
            evidence::EvidenceStore::open_read_only(database).map_err(AlpineError::InvalidInput)?;
        store
            .run(id)
            .map_err(AlpineError::InvalidInput)?
            .ok_or_else(|| AlpineError::InvalidInput(format!("evidence run not found: {id}")))
    }

    pub fn resolve_session(
        install_root: &Path,
        profile: Option<&str>,
        require_runtime: bool,
    ) -> Result<ResolvedSession, AlpineError> {
        config::resolve(install_root, profile, require_runtime).map_err(AlpineError::InvalidInput)
    }

    pub fn plan_session_arguments(
        install_root: &Path,
        profile: Option<&str>,
        vision: bool,
        force_fallback: bool,
    ) -> Result<InferenceArguments, AlpineError> {
        session::plan_arguments(install_root, profile, vision, force_fallback)
            .map_err(AlpineError::InvalidInput)
    }

    pub fn session_status(
        install_root: &Path,
        lock_timeout: Duration,
    ) -> Result<SessionStatus, AlpineError> {
        session::status(install_root, lock_timeout).map_err(AlpineError::InvalidInput)
    }

    pub fn session_action(
        current: &SessionStatus,
        requested_profile: &str,
        requested_vision: bool,
    ) -> SessionAction {
        session::resolve_action(current, requested_profile, requested_vision)
    }

    pub fn start_session(options: &StartSessionOptions) -> Result<StartSessionReport, AlpineError> {
        session::start(options).map_err(AlpineError::InvalidInput)
    }

    pub fn stop_session(options: &StopSessionOptions) -> Result<StopSessionReport, AlpineError> {
        session::stop(options).map_err(AlpineError::InvalidInput)
    }

    pub fn acquire_session(
        options: &AcquireSessionOptions,
    ) -> Result<SessionAcquisition, AlpineError> {
        session::acquire(options).map_err(AlpineError::InvalidInput)
    }

    pub fn release_session(
        options: &ReleaseSessionOptions,
    ) -> Result<ReleaseSessionReport, AlpineError> {
        session::release(options).map_err(AlpineError::InvalidInput)
    }

    pub fn inspect_support(path: &Path, timeout: Duration) -> Result<SupportReport, AlpineError> {
        let (envelope, bytes) = support::read_envelope(path).map_err(AlpineError::InvalidInput)?;
        support::inspect(&envelope, &bytes, timeout).map_err(AlpineError::InvalidInput)
    }

    pub fn qualify(path: &Path) -> Result<QualificationReport, AlpineError> {
        let request = qualification::read_request(path).map_err(AlpineError::InvalidInput)?;
        qualification::qualify(&request).map_err(AlpineError::InvalidInput)
    }

    pub fn qualify_run(
        options: &RunQualificationOptions,
    ) -> Result<RunQualificationReport, AlpineError> {
        qualification::qualify_run(options).map_err(AlpineError::InvalidInput)
    }

    pub fn tune(options: &TuningOptions) -> Result<TuningReport, AlpineError> {
        tuning::tune(options).map_err(AlpineError::InvalidInput)
    }

    pub fn record_operator_review(
        options: &OperatorReviewOptions,
    ) -> Result<RecordedExternalEvidence, AlpineError> {
        external::record_operator_review(options).map_err(AlpineError::InvalidInput)
    }

    pub fn run_same_process_stability(
        options: &SameProcessStabilityOptions,
    ) -> Result<SameProcessStabilityReport, AlpineError> {
        stability::run_same_process(options).map_err(AlpineError::InvalidInput)
    }

    pub fn run_clean_restart_stability(
        options: &CleanRestartStabilityOptions,
    ) -> Result<CleanRestartStabilityReport, AlpineError> {
        stability::run_clean_restarts(options).map_err(AlpineError::InvalidInput)
    }

    pub fn run_near_limit_context(
        options: &NearLimitContextOptions,
    ) -> Result<NearLimitContextReport, AlpineError> {
        context::run(options).map_err(AlpineError::InvalidInput)
    }

    pub fn run_golden_agent(
        options: &GoldenAgentOptions,
    ) -> Result<GoldenAgentReport, AlpineError> {
        golden::run(options).map_err(AlpineError::InvalidInput)
    }

    pub fn prove_rollback(
        options: &RollbackProofOptions,
    ) -> Result<RollbackProofReport, AlpineError> {
        rollback::run(options).map_err(AlpineError::InvalidInput)
    }
}
