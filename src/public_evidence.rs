use crate::decision::Decision;
use crate::external::{ExternalEvidenceStatusKind, PublicCapabilityReviewFacts};
use crate::qualification::{
    EvidenceIdentity, QualificationTarget, RunQualificationOptions, RunQualificationReport,
};
use serde::Serialize;
use std::path::PathBuf;
use std::time::Duration;

const PUBLIC_EVIDENCE_SCHEMA: u32 = 1;
const CAPABILITY_REVIEW_KIND: &str = "operator-reviewed-capability-report";

#[derive(Debug, Clone)]
pub struct PublicEvidenceOptions {
    pub repository_root: PathBuf,
    pub install_root: PathBuf,
    pub database: PathBuf,
    pub final_run_id: String,
    pub tuning_run_ids: Vec<String>,
    pub support_timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PublicQualificationFacts {
    pub target: QualificationTarget,
    pub decision: Decision,
    pub identity: EvidenceIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct PublicArtifactDigest {
    pub kind: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublicEvidenceBundle {
    pub schema: u32,
    pub qualification: PublicQualificationFacts,
    pub capability_review: PublicCapabilityReviewFacts,
    pub supporting_artifacts: Vec<PublicArtifactDigest>,
}

pub fn generate(options: &PublicEvidenceOptions) -> Result<PublicEvidenceBundle, String> {
    let qualification = crate::qualification::qualify_run(&RunQualificationOptions {
        repository_root: options.repository_root.clone(),
        install_root: options.install_root.clone(),
        database: options.database.clone(),
        final_run_id: options.final_run_id.clone(),
        tuning_run_ids: options.tuning_run_ids.clone(),
        target: QualificationTarget::Production,
        support_timeout: options.support_timeout,
    })?;
    generate_from_qualified_report(&qualification)
}

fn generate_from_qualified_report(
    qualification: &RunQualificationReport,
) -> Result<PublicEvidenceBundle, String> {
    if qualification.target != QualificationTarget::Production
        || qualification.decision != Decision::Qualified
        || !qualification.missing_external_evidence.is_empty()
    {
        return Err(
            "Public Evidence requires a current, complete production Qualification".to_owned(),
        );
    }
    let identity = qualification
        .identity
        .clone()
        .ok_or_else(|| "qualified evidence has no complete Evidence Identity".to_owned())?;
    let mut supporting_artifacts = Vec::new();
    let mut capability = None;
    for artifact in &qualification.external_evidence {
        if artifact.status != ExternalEvidenceStatusKind::Satisfied {
            return Err(format!(
                "Public Evidence cannot include unsatisfied artifact {}",
                artifact.name
            ));
        }
        let path = artifact
            .path
            .as_ref()
            .ok_or_else(|| format!("artifact {} has no canonical path", artifact.name))?;
        let sha256 = artifact
            .sha256
            .as_ref()
            .ok_or_else(|| format!("artifact {} has no verified digest", artifact.name))?;
        if artifact.name == CAPABILITY_REVIEW_KIND {
            capability = Some(crate::external::public_capability_review_facts(
                path,
                sha256,
                &qualification.final_run_id,
                &identity,
            )?);
        }
        supporting_artifacts.push(PublicArtifactDigest {
            kind: artifact.name.clone(),
            sha256: sha256.clone(),
        });
    }
    supporting_artifacts.sort();
    Ok(PublicEvidenceBundle {
        schema: PUBLIC_EVIDENCE_SCHEMA,
        qualification: PublicQualificationFacts {
            target: qualification.target,
            decision: qualification.decision,
            identity,
        },
        capability_review: capability.ok_or_else(|| {
            "production Qualification has no satisfied Capability Review artifact".to_owned()
        })?,
        supporting_artifacts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external::{CapabilityCategory, CapabilityReviewDecision, ExternalEvidenceStatus};
    use crate::support::{Platform, SupportReport};

    #[test]
    fn public_projection_rejects_nonqualified_reports_before_reading_private_artifacts() {
        let report = RunQualificationReport {
            schema: 1,
            decision: Decision::NotProven,
            target: QualificationTarget::Production,
            final_run_id: "final".to_owned(),
            tuning_run_ids: vec!["tuning".to_owned()],
            identity: None,
            support: SupportReport {
                schema: 1,
                envelope_id: "fixture".to_owned(),
                envelope_sha256: "0".repeat(64),
                host: Platform {
                    os: "windows".to_owned(),
                    architecture: "x86_64".to_owned(),
                },
                decision: Decision::Unsupported,
                reasons: Vec::new(),
                probes: Vec::new(),
            },
            checks: Vec::new(),
            external_evidence: vec![ExternalEvidenceStatus {
                name: CAPABILITY_REVIEW_KIND.to_owned(),
                status: ExternalEvidenceStatusKind::Missing,
                path: None,
                sha256: None,
                reason: None,
            }],
            missing_external_evidence: vec![CAPABILITY_REVIEW_KIND.to_owned()],
            reasons: Vec::new(),
        };
        assert!(generate_from_qualified_report(&report).is_err());
        let _type_contract = (
            CapabilityCategory::Diagnosis,
            CapabilityReviewDecision::Approved,
        );
    }
}
