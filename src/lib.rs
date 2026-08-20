mod config;
mod decision;
mod evidence;
mod experiment;
mod identity;
mod locking;
mod process;
mod qualification;
mod session;
mod support;

pub use config::{Profile, ProfileStatus, ResolvedSession, SessionConfig};
pub use decision::Decision;
pub use evidence::{RunEvidence, RunSummary, StoredIdentity};
pub use experiment::{ExperimentReport, MicrobenchmarkOptions};
pub use qualification::{QualificationReport, QualificationRequest};
pub use session::{InferenceArguments, ProcessIdentityStrength, SessionAction, SessionStatus};
pub use support::{SupportEnvelope, SupportReport};

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

    pub fn inspect_support(path: &Path, timeout: Duration) -> Result<SupportReport, AlpineError> {
        let (envelope, bytes) = support::read_envelope(path).map_err(AlpineError::InvalidInput)?;
        support::inspect(&envelope, &bytes, timeout).map_err(AlpineError::InvalidInput)
    }

    pub fn qualify(path: &Path) -> Result<QualificationReport, AlpineError> {
        let request = qualification::read_request(path).map_err(AlpineError::InvalidInput)?;
        qualification::qualify(&request).map_err(AlpineError::InvalidInput)
    }
}
