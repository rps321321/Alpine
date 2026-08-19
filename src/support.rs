use crate::decision::Decision;
use crate::process::{resolve_executable, run_bounded};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt::Write;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupportEnvelope {
    pub schema: u32,
    pub id: String,
    pub platforms: Vec<Platform>,
    #[serde(default)]
    pub required_probes: Vec<ProbeId>,
    #[serde(default)]
    pub optional_probes: Vec<ProbeId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Platform {
    pub os: String,
    pub architecture: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProbeId {
    Cargo,
    Cmake,
    Git,
    NvidiaSmi,
    PowerShell,
    Python,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProbeStatus {
    Passed,
    Missing,
    Failed,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProbeResult {
    pub id: ProbeId,
    pub required: bool,
    pub status: ProbeStatus,
    pub executable: Option<String>,
    pub output_sha256: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SupportReport {
    pub schema: u32,
    pub envelope_id: String,
    pub envelope_sha256: String,
    pub host: Platform,
    pub decision: Decision,
    pub reasons: Vec<String>,
    pub probes: Vec<ProbeResult>,
}

pub fn inspect(
    envelope: &SupportEnvelope,
    envelope_bytes: &[u8],
    timeout: Duration,
) -> Result<SupportReport, String> {
    if envelope.schema != 1 {
        return Err(format!(
            "unsupported Support Envelope schema {}; expected 1",
            envelope.schema
        ));
    }
    if envelope.id.trim().is_empty() {
        return Err("Support Envelope id must not be empty".to_owned());
    }

    let host = Platform {
        os: std::env::consts::OS.to_owned(),
        architecture: std::env::consts::ARCH.to_owned(),
    };
    let platform_supported = envelope.platforms.contains(&host);
    let mut probes = BTreeMap::new();
    for id in &envelope.required_probes {
        probes.insert(*id, probe(*id, true, timeout));
    }
    for id in &envelope.optional_probes {
        probes
            .entry(*id)
            .or_insert_with(|| probe(*id, false, timeout));
    }

    let required_missing = probes
        .values()
        .any(|probe| probe.required && probe.status == ProbeStatus::Missing);
    let required_uncertain = probes.values().any(|probe| {
        probe.required && matches!(probe.status, ProbeStatus::Failed | ProbeStatus::TimedOut)
    });
    let (decision, reasons) = if !platform_supported {
        (
            Decision::Unsupported,
            vec![format!(
                "host platform {}/{} is outside this versioned Support Envelope",
                host.os, host.architecture
            )],
        )
    } else if required_missing {
        (
            Decision::Unsupported,
            vec!["one or more required capabilities are absent".to_owned()],
        )
    } else if required_uncertain {
        (
            Decision::Inconclusive,
            vec!["one or more required capability probes did not complete successfully".to_owned()],
        )
    } else {
        (
            Decision::NotProven,
            vec![
                "host is eligible for measurement, but support is not proven without identity-bound qualification evidence"
                    .to_owned(),
            ],
        )
    };

    Ok(SupportReport {
        schema: 1,
        envelope_id: envelope.id.clone(),
        envelope_sha256: hex_sha256(envelope_bytes),
        host,
        decision,
        reasons,
        probes: probes.into_values().collect(),
    })
}

fn probe(id: ProbeId, required: bool, timeout: Duration) -> ProbeResult {
    let (name, arguments): (&str, &[&OsStr]) = match id {
        ProbeId::Cargo => ("cargo", &[OsStr::new("--version")]),
        ProbeId::Cmake => ("cmake", &[OsStr::new("--version")]),
        ProbeId::Git => ("git", &[OsStr::new("--version")]),
        ProbeId::NvidiaSmi => (
            "nvidia-smi",
            &[
                OsStr::new("--query-gpu=name,driver_version,memory.total,compute_cap,pci.bus_id"),
                OsStr::new("--format=csv,noheader,nounits"),
            ],
        ),
        ProbeId::PowerShell => (
            "powershell.exe",
            &[
                OsStr::new("-NoProfile"),
                OsStr::new("-Command"),
                OsStr::new("$PSVersionTable.PSVersion.ToString()"),
            ],
        ),
        ProbeId::Python => ("python", &[OsStr::new("--version")]),
    };
    let Some(executable) = resolve_executable(name) else {
        return ProbeResult {
            id,
            required,
            status: ProbeStatus::Missing,
            executable: None,
            output_sha256: None,
            summary: None,
        };
    };
    match run_bounded(&executable, arguments, timeout) {
        Ok(output) => {
            let combined = format!("{}{}", output.stdout, output.stderr);
            ProbeResult {
                id,
                required,
                status: if output.timed_out {
                    ProbeStatus::TimedOut
                } else if output.status.success() {
                    ProbeStatus::Passed
                } else {
                    ProbeStatus::Failed
                },
                executable: Some(executable.display().to_string()),
                output_sha256: Some(hex_sha256(combined.as_bytes())),
                summary: combined
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .map(|line| line.trim().to_owned()),
            }
        }
        Err(error) => ProbeResult {
            id,
            required,
            status: ProbeStatus::Failed,
            executable: Some(executable.display().to_string()),
            output_sha256: None,
            summary: Some(error.to_string()),
        },
    }
}

pub fn read_envelope(path: &Path) -> Result<(SupportEnvelope, Vec<u8>), String> {
    let bytes = std::fs::read(path).map_err(|error| {
        format!(
            "failed to read Support Envelope {}: {error}",
            path.display()
        )
    })?;
    let envelope = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid Support Envelope {}: {error}", path.display()))?;
    Ok((envelope, bytes))
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_schema_is_rejected() {
        let envelope = SupportEnvelope {
            schema: 2,
            id: "future".to_owned(),
            platforms: vec![],
            required_probes: vec![],
            optional_probes: vec![],
        };
        assert!(inspect(&envelope, b"{}", Duration::from_millis(1)).is_err());
    }

    #[test]
    fn matching_platform_without_required_probes_is_not_proven() {
        let envelope = SupportEnvelope {
            schema: 1,
            id: "fixture".to_owned(),
            platforms: vec![Platform {
                os: std::env::consts::OS.to_owned(),
                architecture: std::env::consts::ARCH.to_owned(),
            }],
            required_probes: vec![],
            optional_probes: vec![],
        };
        let report = inspect(&envelope, b"{}", Duration::from_millis(1)).unwrap();
        assert_eq!(report.decision, Decision::NotProven);
    }

    #[test]
    fn unmatched_platform_is_unsupported() {
        let envelope = SupportEnvelope {
            schema: 1,
            id: "fixture".to_owned(),
            platforms: vec![Platform {
                os: "not-this-os".to_owned(),
                architecture: "not-this-arch".to_owned(),
            }],
            required_probes: vec![],
            optional_probes: vec![],
        };
        let report = inspect(&envelope, b"{}", Duration::from_millis(1)).unwrap();
        assert_eq!(report.decision, Decision::Unsupported);
    }
}
