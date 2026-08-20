# ADR 0019: Alpine owns the control plane above inference

## Status

Accepted — 2026-08-20

## Context

The existing Python, PowerShell and C# control plane established a working local-model path, but its authority is split across process lifecycle, configuration, measurement, qualification and launch adapters. Fresh pre-migration verification exposed three failures at those seams: a concurrent launcher-state dialog hang, inventory dependence on PowerShell module autoloading and an inherited-pipe deadlock after a successful profile transition.

This decision reopens the implementation recommendation in `docs/AUDIT-2026-08-19.md`. It does not reopen ADR 0018's production rollback rule or move inference implementation out of `llama.cpp`.

## Decision

Project Alpine is a Rust control plane. `llama.cpp` and its C++/CUDA implementation remain below the Inference Server boundary.

Alpine is organized as five deep Modules behind one small CLI Interface:

- **Support Envelope** discovers a host through a fixed catalog of bounded probes and evaluates it against a versioned, capability-based envelope. Configuration may select probe identities but may not inject commands.
- **Experiment** owns measured workloads, tuning search, telemetry and durable evidence. Tuning evidence is never final qualification evidence.
- **Qualification** binds claims to hardware, software, model, runtime, workload, configuration and policy identities. Its outcomes are `qualified`, `unsupported`, `inconclusive`, `regressed` and `not-proven`.
- **Session** owns transactional start, stop, restore and recovery with exact process identity and bounded operations.
- **OpenCode Harness** owns the capability-preserving policy, credential-shielded child environment, interactive capacity lease, crash journal, terminal inheritance and exact Session restoration. The C# launcher is only a Windows folder-picker Adapter.

Filesystem, process, clock, HTTP and telemetry Adapters are internal Seams. They exist for platform integration and tests; they are not public workflow APIs. The Rust crate initially stays one package so internal seams do not become shallow cross-crate Interfaces.

The migration is replacement by vertical workflow, not a mechanical translation:

1. Preserve Stable-16K and the existing rollback implementation.
2. Capture comparable identity-bound baselines before replacing a workflow.
3. Add Rust compatibility readers and shadow comparisons without dual writers.
4. Move authority for one complete workflow to Rust only after compatibility and independent qualification pass.
5. Remove the legacy implementation for that workflow instead of retaining permanent delegation layers.
6. Retire the rollback implementation only after every supported production workflow is Rust-authoritative and independently verified.

The canonical repository verification command is:

```powershell
.\scripts\verify.ps1
```

During migration it runs both Rust verification and the complete legacy compatibility suite. Required tests may not be weakened solely to make migration pass.

## Alternatives considered

### Translate each Python and PowerShell module into a Rust crate

Rejected. It preserves the current seams, turns them into public package boundaries and produces many shallow Modules with high change amplification.

### Keep Python as the orchestrator and add Rust helpers

Rejected as the destination. It can reduce isolated risk temporarily, but Python would remain the authority and production would still require the legacy runtime.

### One Rust facade with domain Modules and internal platform Adapters

Accepted. The Interface stays small while discovery, evidence, qualification and transactional operation remain deep enough to change independently behind it.

## Consequences

The current machine is evidence, not policy. A Support Envelope may initially cover only Windows x86-64 with NVIDIA discovery, but new platforms are added by versioned capability rules and independently qualified evidence rather than new global assumptions.

The repository is currently private and unlicensed. Publication visibility and an open-source license require an explicit owner decision before Alpine can satisfy the public-open-source release condition.

The vertical replacement completed on 2026-08-20: Rust now owns supported Session, OpenCode, measurement, tuning, qualification, automated external-gate and rollback workflows. The retained Python and PowerShell implementations are compatibility/research tools, not a second production authority. See ADR 0020 for the bounded automatic evaluation workflow and live hardware identity.
