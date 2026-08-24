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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlacementCandidate {
    pub id: &'static str,
    pub label: &'static str,
    pub gpu_residency_percent: u8,
    pub estimated_gpu_bytes: u64,
    pub estimated_system_bytes: u64,
    pub gpu_headroom_bytes: u64,
    pub system_headroom_bytes: u64,
    pub viable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlacementPlan {
    pub recommended_id: Option<&'static str>,
    pub candidates: Vec<PlacementCandidate>,
    pub profile_hint: &'static str,
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

pub fn plan_placement(hardware: &HardwareCapacity, artifact_bytes: u64) -> PlacementPlan {
    let runtime_bytes = assess_model(hardware, artifact_bytes).estimated_runtime_bytes;
    let gpu_budget = hardware.dedicated_vram_bytes.saturating_mul(85) / 100;
    let system_budget = hardware.total_memory_bytes.saturating_mul(75) / 100;
    let candidates = [
        ("full-gpu", "Full GPU residency", 100_u8),
        ("balanced-hybrid", "Balanced GPU + CPU", 75_u8),
        ("conservative-hybrid", "Conservative GPU + CPU", 50_u8),
        ("cpu-only", "CPU-only fallback", 0_u8),
    ]
    .into_iter()
    .map(|(id, label, gpu_residency_percent)| {
        let estimated_gpu_bytes =
            runtime_bytes.saturating_mul(u64::from(gpu_residency_percent)) / 100;
        let control_overhead = if gpu_residency_percent == 100 {
            512 * MEBIBYTE
        } else {
            1024 * MEBIBYTE
        };
        let estimated_system_bytes = runtime_bytes
            .saturating_sub(estimated_gpu_bytes)
            .saturating_add(control_overhead);
        PlacementCandidate {
            id,
            label,
            gpu_residency_percent,
            estimated_gpu_bytes,
            estimated_system_bytes,
            gpu_headroom_bytes: gpu_budget.saturating_sub(estimated_gpu_bytes),
            system_headroom_bytes: system_budget.saturating_sub(estimated_system_bytes),
            viable: estimated_gpu_bytes <= gpu_budget && estimated_system_bytes <= system_budget,
        }
    })
    .collect::<Vec<_>>();
    let recommended_id = candidates
        .iter()
        .find(|candidate| candidate.viable)
        .map(|candidate| candidate.id);
    PlacementPlan {
        recommended_id,
        candidates,
        profile_hint: "Start with the stable Profile; context and layer placement remain measured tuning inputs.",
        evidence_label: "Capacity estimate — validate with a bounded Alpine evaluation",
    }
}

#[cfg(test)]
mod tests {
    use super::{HardwareCapacity, plan_placement};

    #[test]
    fn placement_prefers_full_gpu_only_with_operating_headroom() {
        let hardware = HardwareCapacity {
            total_memory_bytes: 64 * 1024_u64.pow(3),
            dedicated_vram_bytes: 24 * 1024_u64.pow(3),
        };

        let plan = plan_placement(&hardware, 6 * 1024_u64.pow(3));

        assert_eq!(plan.recommended_id, Some("full-gpu"));
        assert!(plan.candidates[0].viable);
        assert_eq!(plan.candidates[0].gpu_residency_percent, 100);
    }

    #[test]
    fn placement_falls_back_to_cpu_when_there_is_no_dedicated_vram() {
        let hardware = HardwareCapacity {
            total_memory_bytes: 32 * 1024_u64.pow(3),
            dedicated_vram_bytes: 0,
        };

        let plan = plan_placement(&hardware, 8 * 1024_u64.pow(3));

        assert_eq!(plan.recommended_id, Some("cpu-only"));
        assert!(plan.candidates[3].viable);
        assert!(!plan.candidates[0].viable);
    }

    #[test]
    fn placement_refuses_to_recommend_a_model_beyond_safe_system_capacity() {
        let hardware = HardwareCapacity {
            total_memory_bytes: 8 * 1024_u64.pow(3),
            dedicated_vram_bytes: 0,
        };

        let plan = plan_placement(&hardware, 12 * 1024_u64.pow(3));

        assert_eq!(plan.recommended_id, None);
        assert!(plan.candidates.iter().all(|candidate| !candidate.viable));
    }
}
