mod decision;
mod process;
mod qualification;
mod support;

pub use decision::Decision;
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
    pub fn inspect_support(path: &Path, timeout: Duration) -> Result<SupportReport, AlpineError> {
        let (envelope, bytes) = support::read_envelope(path).map_err(AlpineError::InvalidInput)?;
        support::inspect(&envelope, &bytes, timeout).map_err(AlpineError::InvalidInput)
    }

    pub fn qualify(path: &Path) -> Result<QualificationReport, AlpineError> {
        let request = qualification::read_request(path).map_err(AlpineError::InvalidInput)?;
        qualification::qualify(&request).map_err(AlpineError::InvalidInput)
    }
}
