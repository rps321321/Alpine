# Project Alpine

Project Alpine is a Rust control plane for measuring, tuning, qualifying and operating local AI inference. Inference remains the responsibility of `llama.cpp` and its C++/CUDA implementation below the Inference Server boundary.

Alpine is being introduced through verified vertical replacements. The existing Stable-16K path remains the rollback implementation until the Rust workflows have passed compatibility and independent qualification. Current Python, PowerShell and C# code is therefore migration evidence, not the destination architecture.

## Current Rust interface

Resolve and validate the generated Session Config and selected Profile without Python or PowerShell:

```powershell
cargo run --bin alpine -- resolve
```

List existing experiment runs and inspect one run's identity-bound evidence directly from the SQLite store:

```powershell
cargo run --bin alpine -- runs --limit 10
cargo run --bin alpine -- evidence 20260819T232908Z-0b18494a
```

These commands open the legacy database read-only during migration. Missing identity dimensions are reported explicitly; old evidence is not retroactively upgraded or treated as qualification proof.

Inspect the host against the versioned Support Envelope:

```powershell
cargo run --bin alpine -- inspect
```

An eligible host intentionally reports `not-proven` and exits with code 2 until identity-bound qualification evidence is evaluated. Alpine never turns successful discovery into an unsupported qualification claim.

Evaluate a Qualification Request:

```powershell
cargo run --bin alpine -- qualify --request tests/fixtures/alpine/qualified.json
```

Qualification outcomes are `qualified`, `unsupported`, `inconclusive`, `regressed` and `not-proven`. Evidence is bound to hardware, software, model, runtime, workload, configuration and policy identities. Final qualification evidence must be independent from tuning and selection evidence.

Run the canonical repository verification:

```powershell
.\scripts\verify.ps1
```

During migration this runs Rust formatting, clippy and tests plus the complete legacy compatibility suite.

## Repository boundaries

- `src/`: Rust control-plane Modules and the `alpine` CLI.
- `config/support-envelope.json`: versioned capability envelope; current-machine observations do not belong here.
- `config/profiles/` and `config/artifacts.json`: versioned legacy production inputs retained during migration.
- `%USERPROFILE%\local-models`: default generated machine-local installation containing large artifacts, runtime bundles, local credentials and logs. The install root remains configurable; it is not source code and must not be committed.
- `results/`: local identity-bound evidence, intentionally ignored by Git unless an explicit redacted publication artifact is prepared.

Architecture and migration rules are recorded in [ADR 0019](docs/adr/0019-rust-control-plane-boundary.md).

## Publication status

The repository is currently private and has no owner-selected open-source license. Visibility and licensing are explicit release gates; until both are resolved, this project must not be represented as publicly open source.
