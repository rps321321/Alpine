mod config;
mod decision;
mod evidence;
mod experiment;
mod identity;
mod process;
mod qualification;
mod support;

pub use config::{Profile, ProfileStatus, ResolvedSession, SessionConfig};
pub use decision::Decision;
pub use evidence::{RunEvidence, RunSummary, StoredIdentity};
pub use experiment::{ExperimentReport, MicrobenchmarkOptions};
pub use qualification::{QualificationReport, QualificationRequest};
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

    pub fn inspect_support(path: &Path, timeout: Duration) -> Result<SupportReport, AlpineError> {
        let (envelope, bytes) = support::read_envelope(path).map_err(AlpineError::InvalidInput)?;
        support::inspect(&envelope, &bytes, timeout).map_err(AlpineError::InvalidInput)
    }

    pub fn qualify(path: &Path) -> Result<QualificationReport, AlpineError> {
        let request = qualification::read_request(path).map_err(AlpineError::InvalidInput)?;
        qualification::qualify(&request).map_err(AlpineError::InvalidInput)
    }
}
