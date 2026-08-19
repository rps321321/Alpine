use crate::decision::Decision;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceIdentity {
    pub hardware: String,
    pub software: String,
    pub model: String,
    pub runtime: String,
    pub workload: String,
    pub configuration: String,
    pub policy: String,
}

impl EvidenceIdentity {
    fn complete(&self) -> bool {
        [
            &self.hardware,
            &self.software,
            &self.model,
            &self.runtime,
            &self.workload,
            &self.configuration,
            &self.policy,
        ]
        .into_iter()
        .all(|value| !value.trim().is_empty())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidencePhase {
    Tuning,
    Final,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub id: String,
    pub phase: EvidencePhase,
    pub identity: EvidenceIdentity,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualificationRequest {
    pub schema: u32,
    pub support: Decision,
    pub claim_identity: EvidenceIdentity,
    #[serde(default)]
    pub tuning_evidence_ids: Vec<String>,
    #[serde(default)]
    pub final_evidence_ids: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<EvidenceRecord>,
    pub regression_detected: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QualificationReport {
    pub schema: u32,
    pub decision: Decision,
    pub reasons: Vec<String>,
    pub accepted_final_evidence: Vec<String>,
}

pub fn qualify(request: &QualificationRequest) -> Result<QualificationReport, String> {
    if request.schema != 1 {
        return Err(format!(
            "unsupported Qualification Request schema {}; expected 1",
            request.schema
        ));
    }
    if !request.claim_identity.complete() {
        return Ok(report(
            Decision::NotProven,
            "claim identity is incomplete",
            vec![],
        ));
    }
    if request.support == Decision::Unsupported {
        return Ok(report(
            Decision::Unsupported,
            "Support Envelope rejected this configuration",
            vec![],
        ));
    }
    if request.support == Decision::Inconclusive {
        return Ok(report(
            Decision::Inconclusive,
            "support could not be established",
            vec![],
        ));
    }
    if request.support == Decision::Regressed {
        return Ok(report(
            Decision::Regressed,
            "support evidence already records a regression",
            vec![],
        ));
    }
    if request.regression_detected == Some(true) {
        return Ok(report(
            Decision::Regressed,
            "a policy-defined regression was measured",
            vec![],
        ));
    }

    let tuning: BTreeSet<_> = request.tuning_evidence_ids.iter().collect();
    let final_ids: BTreeSet<_> = request.final_evidence_ids.iter().collect();
    if tuning.len() != request.tuning_evidence_ids.len()
        || final_ids.len() != request.final_evidence_ids.len()
    {
        return Ok(report(
            Decision::NotProven,
            "qualification evidence references contain duplicate ids",
            vec![],
        ));
    }
    if final_ids.is_empty() {
        return Ok(report(
            Decision::NotProven,
            "final qualification evidence is missing",
            vec![],
        ));
    }
    if tuning.iter().any(|id| final_ids.contains(id)) {
        return Ok(report(
            Decision::NotProven,
            "final qualification evidence overlaps tuning or selection evidence",
            vec![],
        ));
    }

    let records: BTreeMap<_, _> = request
        .evidence
        .iter()
        .map(|record| (&record.id, record))
        .collect();
    if records.len() != request.evidence.len() {
        return Ok(report(
            Decision::NotProven,
            "qualification evidence contains duplicate record ids",
            vec![],
        ));
    }
    for id in tuning {
        let Some(record) = records.get(id) else {
            return Ok(report(
                Decision::NotProven,
                "referenced tuning evidence is missing",
                vec![],
            ));
        };
        if record.phase != EvidencePhase::Tuning || record.identity != request.claim_identity {
            return Ok(report(
                Decision::NotProven,
                "tuning evidence phase or identity does not match the qualification claim",
                vec![],
            ));
        }
    }
    let mut accepted = Vec::new();
    for id in final_ids {
        let Some(record) = records.get(id) else {
            return Ok(report(
                Decision::NotProven,
                "referenced final evidence is missing",
                vec![],
            ));
        };
        if record.phase != EvidencePhase::Final {
            return Ok(report(
                Decision::NotProven,
                "final evidence has the wrong phase",
                vec![],
            ));
        }
        if record.identity != request.claim_identity {
            return Ok(report(
                Decision::NotProven,
                "final evidence identity does not match the qualification claim",
                vec![],
            ));
        }
        if !record.passed {
            return Ok(report(
                Decision::Unsupported,
                "final correctness evidence failed",
                vec![],
            ));
        }
        accepted.push(record.id.clone());
    }
    if request.regression_detected.is_none() {
        return Ok(report(
            Decision::Inconclusive,
            "regression comparison was not established",
            accepted,
        ));
    }
    Ok(report(
        Decision::Qualified,
        "all required independent evidence passed",
        accepted,
    ))
}

pub fn read_request(path: &Path) -> Result<QualificationRequest, String> {
    let bytes = std::fs::read(path).map_err(|error| {
        format!(
            "failed to read Qualification Request {}: {error}",
            path.display()
        )
    })?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid Qualification Request {}: {error}", path.display()))
}

fn report(
    decision: Decision,
    reason: &str,
    accepted_final_evidence: Vec<String>,
) -> QualificationReport {
    QualificationReport {
        schema: 1,
        decision,
        reasons: vec![reason.to_owned()],
        accepted_final_evidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(value: &str) -> EvidenceIdentity {
        EvidenceIdentity {
            hardware: value.to_owned(),
            software: value.to_owned(),
            model: value.to_owned(),
            runtime: value.to_owned(),
            workload: value.to_owned(),
            configuration: value.to_owned(),
            policy: value.to_owned(),
        }
    }

    fn request() -> QualificationRequest {
        QualificationRequest {
            schema: 1,
            support: Decision::NotProven,
            claim_identity: identity("claim"),
            tuning_evidence_ids: vec!["tune".to_owned()],
            final_evidence_ids: vec!["final".to_owned()],
            evidence: vec![
                EvidenceRecord {
                    id: "tune".to_owned(),
                    phase: EvidencePhase::Tuning,
                    identity: identity("claim"),
                    passed: true,
                },
                EvidenceRecord {
                    id: "final".to_owned(),
                    phase: EvidencePhase::Final,
                    identity: identity("claim"),
                    passed: true,
                },
            ],
            regression_detected: Some(false),
        }
    }

    #[test]
    fn independent_identity_bound_final_evidence_can_qualify() {
        assert_eq!(qualify(&request()).unwrap().decision, Decision::Qualified);
    }

    #[test]
    fn missing_or_reused_final_evidence_is_not_proven() {
        let mut missing = request();
        missing.final_evidence_ids.clear();
        assert_eq!(qualify(&missing).unwrap().decision, Decision::NotProven);

        let mut reused = request();
        reused.tuning_evidence_ids = vec!["final".to_owned()];
        assert_eq!(qualify(&reused).unwrap().decision, Decision::NotProven);
    }

    #[test]
    fn material_identity_change_stales_final_evidence() {
        let mut changed = request();
        changed.claim_identity.runtime = "changed-runtime".to_owned();
        assert_eq!(qualify(&changed).unwrap().decision, Decision::NotProven);
    }

    #[test]
    fn regression_and_uncertainty_are_not_reported_as_qualified() {
        let mut regressed = request();
        regressed.regression_detected = Some(true);
        assert_eq!(qualify(&regressed).unwrap().decision, Decision::Regressed);

        let mut inconclusive = request();
        inconclusive.regression_detected = None;
        assert_eq!(
            qualify(&inconclusive).unwrap().decision,
            Decision::Inconclusive
        );
    }

    #[test]
    fn rejected_support_or_failed_final_evidence_is_unsupported() {
        let mut support = request();
        support.support = Decision::Unsupported;
        assert_eq!(qualify(&support).unwrap().decision, Decision::Unsupported);

        let mut correctness = request();
        correctness.evidence[1].passed = false;
        assert_eq!(
            qualify(&correctness).unwrap().decision,
            Decision::Unsupported
        );
    }

    #[test]
    fn missing_or_stale_tuning_evidence_cannot_support_a_claim() {
        let mut missing = request();
        missing.evidence.remove(0);
        assert_eq!(qualify(&missing).unwrap().decision, Decision::NotProven);

        let mut stale = request();
        stale.evidence[0].identity.policy = "old-policy".to_owned();
        assert_eq!(qualify(&stale).unwrap().decision, Decision::NotProven);
    }
}
