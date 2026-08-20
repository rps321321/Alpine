use crate::identity::sha256_bytes;
use crate::process::{resolve_executable, run_bounded};
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::time::Duration;
use sysinfo::System;

pub const INLINE_HARDWARE_MANIFEST: &str = "inline:rust-hardware-snapshot-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HardwareSnapshot {
    pub schema: u32,
    pub platform: HardwarePlatform,
    pub cpu: CpuIdentity,
    pub physical_memory_bytes: u64,
    pub nvidia_gpus: Vec<NvidiaGpuIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HardwarePlatform {
    pub os: String,
    pub architecture: String,
    pub os_version: Option<String>,
    pub kernel_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CpuIdentity {
    pub vendor: String,
    pub brand: String,
    pub physical_cores: Option<usize>,
    pub logical_processors: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NvidiaGpuIdentity {
    pub pci_bus_id: String,
    pub name: String,
    pub vram_mib: u64,
    pub compute_capability: String,
    pub driver_version: String,
    pub vbios_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HardwareReport {
    pub snapshot: HardwareSnapshot,
    pub sha256: String,
}

pub fn report(timeout: Duration) -> Result<HardwareReport, String> {
    let snapshot = capture(timeout)?;
    let sha256 = sha256(&snapshot)?;
    Ok(HardwareReport { snapshot, sha256 })
}

pub fn capture(timeout: Duration) -> Result<HardwareSnapshot, String> {
    let system = System::new_all();
    let first_cpu = system
        .cpus()
        .first()
        .ok_or_else(|| "live hardware discovery found no logical processors".to_owned())?;
    let physical_memory_bytes = system.total_memory();
    if physical_memory_bytes == 0 {
        return Err("live hardware discovery reported zero physical memory".to_owned());
    }
    let mut snapshot = HardwareSnapshot {
        schema: 1,
        platform: HardwarePlatform {
            os: std::env::consts::OS.to_owned(),
            architecture: std::env::consts::ARCH.to_owned(),
            os_version: System::os_version(),
            kernel_version: System::kernel_version(),
        },
        cpu: CpuIdentity {
            vendor: first_cpu.vendor_id().trim().to_owned(),
            brand: first_cpu.brand().trim().to_owned(),
            physical_cores: System::physical_core_count(),
            logical_processors: system.cpus().len(),
        },
        physical_memory_bytes,
        nvidia_gpus: capture_nvidia_gpus(timeout)?,
    };
    snapshot
        .nvidia_gpus
        .sort_by(|left, right| left.pci_bus_id.cmp(&right.pci_bus_id));
    validate(&snapshot)?;
    Ok(snapshot)
}

pub fn sha256(snapshot: &HardwareSnapshot) -> Result<String, String> {
    validate(snapshot)?;
    serde_json::to_vec(snapshot)
        .map(|bytes| sha256_bytes(&bytes))
        .map_err(|error| format!("failed to encode live hardware identity: {error}"))
}

fn capture_nvidia_gpus(timeout: Duration) -> Result<Vec<NvidiaGpuIdentity>, String> {
    let Some(executable) = resolve_executable("nvidia-smi") else {
        return Ok(Vec::new());
    };
    let output = run_bounded(
        &executable,
        &[
            OsStr::new(
                "--query-gpu=pci.bus_id,name,memory.total,compute_cap,driver_version,vbios_version",
            ),
            OsStr::new("--format=csv,noheader,nounits"),
        ],
        timeout,
    )
    .map_err(|error| format!("failed to run NVIDIA hardware discovery: {error}"))?;
    if output.timed_out {
        return Err("NVIDIA hardware discovery timed out".to_owned());
    }
    if !output.status.success() {
        return Err(format!(
            "NVIDIA hardware discovery failed: {}",
            output.stderr.trim()
        ));
    }
    parse_nvidia_csv(&output.stdout)
}

fn parse_nvidia_csv(value: &str) -> Result<Vec<NvidiaGpuIdentity>, String> {
    let mut gpus = Vec::new();
    for (index, line) in value
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
    {
        let fields = line.split(',').map(str::trim).collect::<Vec<_>>();
        if fields.len() != 6 || fields.iter().any(|field| field.is_empty()) {
            return Err(format!(
                "NVIDIA hardware discovery row {} is malformed",
                index + 1
            ));
        }
        let vram_mib = fields[2].parse::<u64>().map_err(|_| {
            format!(
                "NVIDIA hardware discovery row {} has invalid VRAM",
                index + 1
            )
        })?;
        gpus.push(NvidiaGpuIdentity {
            pci_bus_id: fields[0].to_ascii_lowercase(),
            name: fields[1].to_owned(),
            vram_mib,
            compute_capability: fields[3].to_owned(),
            driver_version: fields[4].to_owned(),
            vbios_version: fields[5].to_owned(),
        });
    }
    gpus.sort_by(|left, right| left.pci_bus_id.cmp(&right.pci_bus_id));
    Ok(gpus)
}

fn validate(snapshot: &HardwareSnapshot) -> Result<(), String> {
    if snapshot.schema != 1 {
        return Err(format!(
            "unsupported live hardware snapshot schema {}; expected 1",
            snapshot.schema
        ));
    }
    if snapshot.platform.os.trim().is_empty()
        || snapshot.platform.architecture.trim().is_empty()
        || snapshot.cpu.brand.trim().is_empty()
        || snapshot.cpu.logical_processors == 0
        || snapshot.physical_memory_bytes == 0
    {
        return Err("live hardware snapshot is incomplete".to_owned());
    }
    if snapshot.nvidia_gpus.iter().any(|gpu| {
        gpu.pci_bus_id.trim().is_empty()
            || gpu.name.trim().is_empty()
            || gpu.vram_mib == 0
            || gpu.compute_capability.trim().is_empty()
            || gpu.driver_version.trim().is_empty()
            || gpu.vbios_version.trim().is_empty()
    }) {
        return Err("live NVIDIA hardware identity is incomplete".to_owned());
    }
    let unique = snapshot
        .nvidia_gpus
        .iter()
        .map(|gpu| gpu.pci_bus_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    if unique.len() != snapshot.nvidia_gpus.len() {
        return Err("live NVIDIA hardware identity contains duplicate PCI devices".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> HardwareSnapshot {
        HardwareSnapshot {
            schema: 1,
            platform: HardwarePlatform {
                os: "windows".to_owned(),
                architecture: "x86_64".to_owned(),
                os_version: Some("11".to_owned()),
                kernel_version: Some("fixture".to_owned()),
            },
            cpu: CpuIdentity {
                vendor: "GenuineIntel".to_owned(),
                brand: "Fixture CPU".to_owned(),
                physical_cores: Some(8),
                logical_processors: 16,
            },
            physical_memory_bytes: 32 * 1024 * 1024 * 1024,
            nvidia_gpus: vec![NvidiaGpuIdentity {
                pci_bus_id: "00000000:01:00.0".to_owned(),
                name: "Fixture GPU".to_owned(),
                vram_mib: 12_288,
                compute_capability: "12.0".to_owned(),
                driver_version: "1.2.3".to_owned(),
                vbios_version: "fixture".to_owned(),
            }],
        }
    }

    #[test]
    fn nvidia_rows_are_parsed_and_sorted_by_pci_identity() {
        let parsed = parse_nvidia_csv(
            "00000000:02:00.0, GPU B, 8192, 9.0, 1.2.3, b\n00000000:01:00.0, GPU A, 12288, 12.0, 1.2.3, a\n",
        )
        .unwrap();
        assert_eq!(parsed[0].name, "GPU A");
        assert_eq!(parsed[1].name, "GPU B");
    }

    #[test]
    fn malformed_nvidia_rows_fail_closed() {
        assert!(parse_nvidia_csv("GPU, missing, fields\n").is_err());
        assert!(parse_nvidia_csv("pci, GPU, nope, 12.0, driver, vbios\n").is_err());
    }

    #[test]
    fn material_hardware_changes_stale_the_identity() {
        let original = snapshot();
        let mut changed = original.clone();
        changed.nvidia_gpus[0].driver_version = "2.0.0".to_owned();
        assert_ne!(sha256(&original).unwrap(), sha256(&changed).unwrap());
    }
}
