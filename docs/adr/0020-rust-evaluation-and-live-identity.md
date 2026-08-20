# ADR 0020: Rust owns bounded evaluation and live identity

## Status

Accepted — 2026-08-20

## Context

Rust already owned individual measurement, tuning, qualification, external-gate and rollback operations, but an operator still had to compose them manually. Experiment hardware identity was also the digest of a previously collected repository inventory file. That could neither establish the best justified result within a declared search budget nor make a hardware/driver change stale without first updating the file.

## Decision

`alpine evaluate` is the authoritative end-to-end evaluation transaction. It reads one strict, versioned `config/evaluation-plan.json` snapshot and binds its SHA-256 to an atomic report. The plan declares the baseline, candidate Profiles, exact workloads, identical tuning/final sample conditions, qualification target, timeouts and hard maximum profile/request budgets. Unknown fields, duplicates, empty sets, arithmetic overflow and budget excess fail before any Session mutation.

Alpine transactionally measures each declared Profile, selects only row-policy-eligible tuning evidence, and produces a distinct fully hashed final run for the selected Profile. It then independently requalifies SQLite rows and, according to the target, runs same-process stability, clean restarts, near-limit context, the golden-agent task and Stable rollback. Each harness restores the exact prior material Session. The current production plan intentionally ends `not-proven` when the separate human capability review is absent.

New evidence captures a canonical live Rust hardware snapshot: platform/kernel, CPU identity/topology, physical memory and sorted NVIDIA PCI/GPU/VRAM/compute-capability/driver/VBIOS identity. Qualification validates the stored snapshot and captures the host again. It also fully hashes the current model. A material hardware, driver or model change therefore stales the claim without trusting a manually refreshed inventory file.

PowerShell is an installation/bootstrap Adapter and retained compatibility-test host. Python is retained compatibility/research tooling. C# is a folder-picker Adapter. None is a source of authority for the supported evaluation or runtime workflow.

## Consequences

The checked-in plan is deliberately deployment-specific and currently searches Stable-16K versus Turbo-16K only; this is an explicit experiment, not permanent product policy or a global optimum claim. Other hosts and Profiles require a new versioned plan and their own evidence. SQLite measured rows and immutable external artifacts remain qualification authority; the evaluation report is an orchestration index, not a replacement summary Boolean.

The repository remains private and has no owner-selected open-source license. Alpine must report that release condition as unresolved rather than inventing a license or changing repository visibility.
