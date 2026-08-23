use serde::Serialize;

const MEBIBYTE: u64 = 1024 * 1024;
const ALIGNMENT_BYTES: u64 = 256 * MEBIBYTE;
const LOADER_OVERHEAD_PERCENT: u64 = 10;
const OPERATING_HEADROOM_BYTES: u64 = 512 * MEBIBYTE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HardwareCapacity {
    pub total_memory_bytes: u64,
    pub dedicated_vram_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FitStatus {
    FitsGpuWithHeadroom,
    FitsGpuTight,
    FitsWithCpuOffload,
    UnlikelyToFit,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelAssessment {
    pub status: FitStatus,
    pub artifact_bytes: u64,
    pub estimated_runtime_bytes: u64,
    pub headroom_bytes: u64,
    pub is_measured: bool,
    pub evidence_label: &'static str,
}

pub fn assess_model(hardware: &HardwareCapacity, artifact_bytes: u64) -> ModelAssessment {
    let loader_overhead = artifact_bytes.saturating_mul(LOADER_OVERHEAD_PERCENT) / 100;
    let unaligned = artifact_bytes
        .saturating_add(loader_overhead)
        .saturating_add(OPERATING_HEADROOM_BYTES);
    let estimated_runtime_bytes =
        unaligned.saturating_add(ALIGNMENT_BYTES - 1) / ALIGNMENT_BYTES * ALIGNMENT_BYTES;

    let gpu_headroom = hardware
        .dedicated_vram_bytes
        .saturating_sub(estimated_runtime_bytes);
    let status =
        if estimated_runtime_bytes <= hardware.dedicated_vram_bytes.saturating_mul(85) / 100 {
            FitStatus::FitsGpuWithHeadroom
        } else if estimated_runtime_bytes <= hardware.dedicated_vram_bytes {
            FitStatus::FitsGpuTight
        } else if estimated_runtime_bytes <= hardware.total_memory_bytes.saturating_mul(85) / 100 {
            FitStatus::FitsWithCpuOffload
        } else {
            FitStatus::UnlikelyToFit
        };

    ModelAssessment {
        status,
        artifact_bytes,
        estimated_runtime_bytes,
        headroom_bytes: gpu_headroom,
        is_measured: false,
        evidence_label: "Estimate — run analysis to measure",
    }
}
