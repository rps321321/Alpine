# Project Alpine

Project Alpine is a Rust control plane for measuring, tuning, qualifying and operating local AI inference. Inference remains the responsibility of `llama.cpp` and its C++/CUDA implementation below the Inference Server boundary.

Alpine is being introduced through verified vertical replacements. The existing Stable-16K path remains the rollback implementation until the Rust workflows have passed compatibility and independent qualification. Current Python, PowerShell and C# code is therefore migration evidence, not the destination architecture.

## Current Rust interface

Resolve and validate the generated Session Config and selected Profile without Python or PowerShell:

```powershell
cargo run --bin alpine -- resolve
```

Inspect the active listener and preview the exact `llama.cpp` argument/environment contract:

```powershell
cargo run --bin alpine -- session status
cargo run --bin alpine -- session plan --profile fast-32k
```

Rust Session start/stop is implemented behind the same inference-capacity and transition locks used by the migrating installation. It publishes typed state atomically, records PID plus process-start identity, refuses foreign listeners, performs bounded optimized-to-MTP fallback, and restores a configured cleanup process after failure or stop. A one-time stop of a PowerShell-authored state requires `--allow-legacy-identity`; Rust-authored states do not. These commands are not yet routed from the production launcher because live compatibility and independent qualification are still migration gates:

```powershell
cargo run --release --bin alpine -- session start --profile fast-32k
cargo run --release --bin alpine -- session stop
```

Harnesses use the transactional interface instead of direct start/stop. `acquire` atomically reuses or replaces the Session and writes the exact prior-state contract; `release` rejects stale identities and restores that contract or the prior idle state. Failed replacement rolls back immediately, and failed release restoration recovers the acquired Session when it can still be replayed exactly:

```powershell
cargo run --release --bin alpine -- session acquire --profile fast-32k --output acquisition.json
cargo run --release --bin alpine -- session release --acquisition acquisition.json
```

A matching PowerShell-authored Session is deliberately migrated rather than silently accepted as Rust-authored evidence. Its one-time replacement requires `--allow-legacy-identity`.

List existing experiment runs and inspect one run's identity-bound evidence directly from the SQLite store:

```powershell
cargo run --bin alpine -- runs --limit 10
cargo run --bin alpine -- evidence 20260819T232908Z-0b18494a
```

These commands open the legacy database read-only during migration. Missing identity dimensions are reported explicitly; old evidence is not retroactively upgraded or treated as qualification proof.

Run the Rust-owned microbenchmark against an already healthy, exactly matching Inference Session:

```powershell
cargo run --release --bin alpine -- benchmark --profile fast-32k --phase tuning --runs 5 --warmups 1
cargo run --release --bin alpine -- benchmark --profile fast-32k --phase final --runs 5 --warmups 1 --deep-verify-artifacts
```

Alpine holds the cross-process inference-capacity lease and rejects a mismatched or legacy-authored running Session before sending a request. New runs require a verified PID/executable/port/process-start identity plus complete hardware, Alpine binary, model, runtime, workload, material configuration and promotion-policy identities. `--phase tuning` is the default; final qualification runs must explicitly use `--phase final`. The first run fully hashes the model; unchanged later runs reuse a metadata-bound local attestation. Use `--deep-verify-artifacts` for a fresh full digest, including final qualification runs. SQLite is the authority; the per-run JSON/JSONL files are durable inspection copies.

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
