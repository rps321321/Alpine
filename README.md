# Project Alpine

Project Alpine is the Rust control plane for discovering, measuring, tuning, qualifying and operating local AI inference. Inference remains the responsibility of `llama.cpp` and its C++/CUDA implementation below the Inference Server boundary. Retained setup adapters are not runtime authorities; selecting, measuring, qualifying and operating a supported Profile are Rust-owned workflows.

## Current Rust interface

Run the bounded end-to-end evaluation declared in `config/evaluation-plan.json`:

```powershell
cargo run --release --bin alpine -- evaluate
```

This one Rust transaction checks the Support Envelope, measures the declared baseline and candidate search space, selects without mutating Profiles, produces an independently measured and fully hashed final run, executes the required stability/context/golden-agent gates, proves rollback, and evaluates the requested lifecycle target. The checked-in plan currently compares only the 16K Stable and Turbo Profiles, consumes at most 72 microbenchmark requests, and targets production. It is expected to report `not-proven` until the separately recorded human capability review exists; it does not fabricate or bypass that gate. The complete evaluation report is published atomically under `results/evaluations/`, while raw measurements and evidence remain in SQLite and per-run artifacts.

The production OpenCode workflow is a single Rust transaction. It verifies the effective 16K policy before loading the model, holds the inference-capacity lease for the complete interactive child lifetime, survives Ctrl-C long enough to restore the prior Session, and keeps an atomic crash-recovery journal:

```powershell
cargo run --release --bin alpine -- opencode --project C:\path\to\project
```

The installed `Open Local Qwen.exe` is only a folder-picker Adapter for `alpine.exe opencode`; it does not own Session, policy, logging, or OpenCode behavior. `Open Minimal OpenCode.cmd` is a transparent fallback to that same Rust command.

Resolve and validate the generated Session Config and selected Profile without Python or PowerShell:

```powershell
cargo run --bin alpine -- resolve
```

Inspect the active listener and preview the exact `llama.cpp` argument/environment contract:

```powershell
cargo run --bin alpine -- session status
cargo run --bin alpine -- session plan --profile fast-32k
```

Rust Session start/stop is implemented behind the same inference-capacity and transition locks used by the migrating installation. It publishes typed state atomically, records PID plus process-start identity, refuses foreign listeners, performs bounded optimized-to-MTP fallback, and restores a configured cleanup process after failure or stop. A one-time stop of a PowerShell-authored state requires `--allow-legacy-identity`; Rust-authored states do not:

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

These commands open the shared evidence database read-only. Missing identity dimensions are reported explicitly; old evidence is not retroactively upgraded or treated as qualification proof.

Run the Rust-owned microbenchmark against an already healthy, exactly matching Inference Session:

```powershell
cargo run --release --bin alpine -- benchmark --profile fast-32k --phase tuning --runs 5 --warmups 1
cargo run --release --bin alpine -- benchmark --profile fast-32k --phase final --runs 5 --warmups 1 --deep-verify-artifacts
```

Alpine holds the cross-process inference-capacity lease and rejects a mismatched or legacy-authored running Session before sending a request. New runs require a verified PID/executable/port/process-start identity plus complete hardware, Alpine binary, model, runtime, workload, material configuration and promotion-policy identities. `--phase tuning` is the default; final qualification runs must explicitly use `--phase final`. The first run fully hashes the model; unchanged later runs reuse a metadata-bound local attestation. Use `--deep-verify-artifacts` for a fresh full digest, including final qualification runs. SQLite is the authority; the per-run JSON/JSONL files are durable inspection copies.

Rank bounded tuning candidates without mutating the installation:

```powershell
cargo run --release --bin alpine -- tune --baseline-run <baseline-tuning-run-id> --candidate-run <candidate-tuning-run-id>
```

Tuning holds hardware, Alpine binary, model, workload suite and policy constant while treating runtime and material configuration as search dimensions. Every candidate must pass row-level quality, deterministic-output, sample-count and variability gates using the metric explicitly assigned to each workload by policy. A candidate is recommended only if its equally weighted geometric-mean performance score clears the versioned improvement threshold without violating any per-workload regression floor; otherwise Alpine explicitly retains the baseline.

Inspect the host against the versioned Support Envelope:

```powershell
cargo run --bin alpine -- inspect
```

An eligible host intentionally reports `not-proven` and exits with code 2 until identity-bound qualification evidence is evaluated. Alpine never turns successful discovery into an unsupported qualification claim.

Evaluate a final run directly from SQLite against a distinct tuning baseline:

```powershell
cargo run --release --bin alpine -- qualify <final-run-id> --tuning-run <tuning-run-id> --target candidate
```

The qualifier recomputes quality, deterministic output hashes, sample count, decode variability and per-workload median regression from non-warmup SQLite rows. It also re-hashes the current policy, workload suite, Alpine binary, runtime and complete model artifact, captures the current hardware/driver identity through bounded Rust probes, and recomputes material configuration identity. Qualification outcomes are `qualified`, `unsupported`, `inconclusive`, `regressed` and `not-proven`. Validated and production targets remain `not-proven` until their inherited external evidence is independently verified.

Run the Rust-owned same-process stability gate against the exact passed final run. It alternates 50 distinct contaminating prompts with 50 identical greedy target requests, records every raw token id, verifies one target token hash and restores the prior Session transaction:

```powershell
cargo run --release --bin alpine -- same-process-stability <final-run-id>
cargo run --release --bin alpine -- clean-restart-stability <final-run-id>
cargo run --release --bin alpine -- near-limit-context <final-run-id> --ratio 0.85
cargo run --release --bin alpine -- golden-agent <final-run-id> --task python-off-by-one
cargo run --release --bin alpine -- rollback-proof <final-run-id>
```

The restart gate performs ten actual stop/start cycles under the same exclusive lease, requires ten distinct PID/start/transaction identities, compares the raw 128-token greedy output from every process, and restores the materially exact prior profile, arguments, environment and fallback mode.

The context gate uses the server tokenizer to construct a reproducible immutable-ledger prompt within two percent of the configured target ratio, performs the three-needle retrieval twice on one verified process, verifies both raw responses, and restores the prior Session.

The golden-agent gate copies the versioned fixture into an isolated temporary worktree, launches OpenCode directly from Rust with the reviewed minimal policy and a secret-scrubbed child environment, verifies the effective context/safety policy, runs the executable tests, rejects protected-path edits or unexpected files, binds the OpenCode executable hash, and restores the prior Session. It does not invoke the PowerShell launcher.

The rollback proof transactionally switches to the configured `stable-16k` rollback Profile, verifies its current deployment role and Profile/Session/runtime identities, performs a real 16-token inference smoke, and restores the prior material Session before publishing evidence.

Only the explicitly human production review uses manual attachment. The evidence file is the strict schema documented in `docs/CAPABILITY-REVIEW.md`, not a pass boolean:

```powershell
cargo run --release --bin alpine -- record-evidence <final-run-id> --kind operator-reviewed-capability-report --evidence C:\path\to\review-details.json --reviewed-by "operator name"
```

Automated evidence cannot be attached from caller-supplied summaries. The harness constructs the versioned envelope, copies the final run's seven-dimensional identity, binds it to the current Alpine executable, publishes it immutably, hashes it, and attaches that digest to SQLite. Qualification recomputes the 50/50 counts, ordering and token hashes from the raw records. Repeating an identical interrupted attachment is idempotent; conflicting or tampered artifacts fail closed. A human reviewer remains mandatory for the operator capability gate.

Qualification does not mutate deployment. Omit `--profile` to use the append-only deployment `daily_default`; an explicit Profile is a one-session override. Promotion, rollback, and incident recording are separate commands:

```powershell
cargo run --release --bin alpine -- deployment-status
cargo run --release --bin alpine -- promote --profile turbo-16k --expected-daily-default stable-16k --final-run-id <final> --tuning-run <baseline> --operator <operator> --reason <reason>
cargo run --release --bin alpine -- rollback --expected-daily-default turbo-16k --promotion-event-id <event> --operator <operator> --reason <reason>
cargo run --release --bin alpine -- incident --profile turbo-16k --promotion-event-id <event> --operator <operator> --reason <reason>
```

Promotion re-runs the complete production Qualification and refuses missing/stale human evidence or unresolved suspensions. Evaluation never promotes. See `docs/DEPLOYMENT.md`.

Generate public structural evidence only through the allowlisted projection:

```powershell
cargo run --release --bin alpine -- public-evidence --final-run-id <final> --tuning-run <baseline> --output public-evidence.json
```

The projection has no fields for private tasks, observations, prompts, transcripts, risk narratives, repository content, or private reviewer identity.

The old caller-assembled request evaluator remains available only as a migration compatibility command:

```powershell
cargo run --bin alpine -- qualify-request --request tests/fixtures/alpine/qualified.json
```

Run the canonical repository verification:

```console
cargo run --locked --bin alpine-verify
```

This runs Rust formatting, clippy and tests plus the retained legacy compatibility suite. It is the repository/CI verifier; `alpine evaluate` is the executable product-claim verifier.

Rust-native maintenance commands replace the former standalone hardware, launcher-build, and runtime-packaging scripts:

```powershell
cargo run --release --bin alpine -- hardware --output hardware.json
cargo run --release --bin alpine -- build-launcher --root runtime --no-shortcut
cargo run --release --bin alpine -- package-runtime --built-runtime C:\path\to\build\bin --output C:\path\to\runtime-custom
```

## Repository boundaries

- `src/`: Rust control-plane Modules and the `alpine` CLI.
- `config/support-envelope.json`: versioned capability envelope; current-machine observations do not belong here.
- `config/evaluation-plan.json`: versioned search space, resource budget and requested qualification target.
- `config/profiles/` and `config/artifacts.json`: versioned inference-only Profile and artifact contracts consumed by Rust; deployment roles are not stored in Profile bytes.
- `%USERPROFILE%\local-models`: default generated machine-local installation containing large artifacts, runtime bundles, local credentials and logs. The install root remains configurable; it is not source code and must not be committed.
- `results/`: local identity-bound evidence, intentionally ignored by Git unless an explicit redacted publication artifact is prepared.

Architecture and replacement rules are recorded in [ADR 0019](docs/adr/0019-rust-control-plane-boundary.md); the completed automated evaluation boundary is recorded in [ADR 0020](docs/adr/0020-rust-evaluation-and-live-identity.md).

## Licensing, support, and publication status

Project Alpine's own source is licensed under Apache-2.0 and contributions use DCO 1.1. Third-party artifacts retain their own license boundaries; see `LICENSE`, `THIRD_PARTY.md`, and `CONTRIBUTING.md`.

The Support Envelope means an environment is eligible to evaluate, not universally production-supported. Production qualification applies only to an exact recorded deployment; see `SUPPORT.md`.

This checkout is being prepared for a source-only v0.1, but repository visibility and release publication remain explicit owner actions and have not been performed by these files. See `docs/RELEASING.md`.
