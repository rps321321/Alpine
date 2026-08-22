# Reproducible Windows setup

## Fast path

Clone the repository, open PowerShell in it, and run:

```powershell
.\setup.ps1
```

The default install root is `%USERPROFILE%\local-models`. A fresh installation conservatively initializes `stable-16k` as both `daily_default` and `rollback_profile`; it inherits no Qualification:

```powershell
.\setup.ps1 -Profile stable-16k
```

Setup verifies or resumably downloads the pinned model, projector, official Qwen template, official `llama.cpp` release, and CUDA DLL archive. The custom profiles use the exact pinned source commit plus the repository patch. One installation-scoped lock serializes setup. Runtimes and the complete control plane are staged before publication; a recovery journal restores the prior usable installation if publication is interrupted. Invalid downloads are preserved with unique names rather than destroyed.

On a new Windows installation, permit setup to install the pinned prerequisites:

```powershell
.\setup.ps1 -Profile stable-16k -InstallPrerequisites
```

This may install Git, CMake 4.2.3, CUDA 13.2, Visual Studio 2022 Build Tools, Node.js, Rustup/Cargo if needed, and OpenCode 1.18.18. Review this system-wide operation before running it. Setup builds Alpine with the locked Cargo dependency graph and publishes `alpine.exe` in the same recoverable installation transaction as the launcher.

Large artifact downloads are verified by byte size and SHA-256 from `config/artifacts.json`. The installer is safe to rerun and resumes `.part` downloads. A bounded "another setup transaction owns" error means a setup is still active; rerunning after an owner crash repairs any journaled partial publication first.

## Launch

Double-click `Open Local Qwen.exe` in the install root, choose a project folder, and work in the OpenCode terminal. With no Profile argument, it uses the deployment `daily_default`. The executable is a thin folder picker that directly supervises `alpine.exe opencode`; PowerShell is not in the production launch path. Alpine owns the policy, full-lifetime inference lease, transactional Session restoration, Ctrl-C handling, crash-recovery journal, and redacted failure publication. Native OpenCode diagnostics remain visible in the terminal and are not copied into logs.

Do not pipe a lifecycle command through `Tee-Object` while intentionally leaving or restoring a long-lived inference server. On Windows, that descendant can retain the pipeline handle after the Alpine command itself has finished, so the pipe does not report EOF until the server stops. This is a capture-harness limitation, not a server-health failure. Normal interactive launcher use is unaffected; automation should write bounded artifacts directly or return the Session to stopped before waiting for piped EOF.

To verify this failure path without loading the model, run `Open Local Qwen.exe --project <existing-folder> --diagnostic-failure`. It deliberately presents a diagnostic error, writes the stable redacted log, and exits non-zero.

Verify the complete effective policy without loading the model:

```powershell
.\alpine.exe opencode --install-root . --project C:\path\to\project --check
```

The direct installed CLI is:

```powershell
.\alpine.exe opencode --install-root . --project C:\path\to\project
```

The launcher refuses to steal port 8100 from a different executable. `alpine session status` verifies process ownership rather than treating any healthy HTTP listener as the configured model.

## Verify a restored machine

Use Rust to resolve the installed Session/Profile contract, capture the current hardware identity, and inspect the Support Envelope:

```powershell
cargo run --release --bin alpine -- resolve
cargo run --release --bin alpine -- hardware
cargo run --release --bin alpine -- inspect
```

The independent final stage of `alpine evaluate` performs a complete model SHA-256 and current runtime/configuration verification. Historical machine inventories are private, machine-local evidence and are not part of the public source tree. New Rust evidence embeds a canonical live CPU/RAM/GPU/driver snapshot, and qualification captures the host again; it does not trust an inventory file.

The same Rust CLI owns the supported maintenance tools. It can write the canonical hardware identity atomically, rebuild the thin Windows launcher without PowerShell, and package the pinned custom runtime with a per-file hash manifest:

```powershell
cargo run --release --bin alpine -- hardware --output C:\path\to\hardware.json
cargo run --release --bin alpine -- build-launcher --root runtime --output "C:\path\to\Open Local Qwen.exe" --no-shortcut
cargo run --release --bin alpine -- package-runtime --built-runtime C:\path\to\llama-build\bin --output C:\path\to\runtime-custom
```

Omit `--no-shortcut` when building the installed launcher to refresh its desktop shortcuts. `package-runtime` validates the pinned llama.cpp commit, copies the required CUDA runtime DLLs, and writes `build-manifest.json`; its input and output directories must be distinct.

## Profiles

```text
stable-16k   official runtime, MTP3, no n-gram; initialized rollback role
turbo-16k    custom request-local n-gram + MTP3; requires exact Qualification before Promotion
fast-32k     32K request-local n-gram Profile; no production claim
long-64k     research-only long-context Profile; no production claim
```

Omit `--profile` for the deployment `daily_default`, or select a one-session override explicitly. Neither operation edits deployment history:

```powershell
cargo run --release --bin alpine -- session start --profile stable-16k
cargo run --release --bin alpine -- opencode --project C:\path\to\project
```

Profile files contain inference material only. Qualification and append-only deployment roles carry lifecycle facts; see `config/promotion-policy.json` and `docs/DEPLOYMENT.md`.

## Benchmark and inspect evidence

The supported automatic path is:

```powershell
cargo run --release --bin alpine -- evaluate
```

The versioned plan fixes the search space, workloads, request budget, timeouts and target. Alpine measures every declared 16K Profile, selects the best policy-eligible result without editing Profiles, produces distinct final evidence, runs the inherited automated gates, proves Stable rollback, and publishes an atomic report. A production target remains `not-proven` until the human capability review is attached.

SQLite lives at `results/results.sqlite3`; each run keeps raw JSONL, outputs, logs, configurations and compressed long prompts under `results/runs/<run-id>`. Generated evidence is local by default. Summary reports are regeneratable.

For a manual Rust investigation, run a tuning baseline and a separate, freshly hashed final pass with identical material configuration, then qualify the final SQLite rows:

```powershell
cargo run --release --bin alpine -- benchmark --profile fast-32k --phase tuning --runs 5 --warmups 1
cargo run --release --bin alpine -- benchmark --profile fast-32k --phase final --runs 5 --warmups 1 --deep-verify-artifacts
cargo run --release --bin alpine -- tune --baseline-run <baseline-tuning-run-id> --candidate-run <candidate-tuning-run-id>
cargo run --release --bin alpine -- qualify <final-run-id> --tuning-run <tuning-run-id> --target candidate
```

The tuner is read-only: it recommends a measured configuration or retains the baseline and never edits the installed Profile.

Measured runs hold an inference-capacity lease tied to their exact Inference Session identity. A second benchmark or unrelated interactive launcher receives a visible busy refusal instead of silently queueing and contaminating timings. The agent benchmark's own launcher carries the governing lease token.

Validated/production automated evidence is generated by Rust-owned harnesses against the exact passed final run. The same-process gate records raw token ids for 50 contaminant/target pairs and restores the prior Session:

```powershell
cargo run --release --bin alpine -- same-process-stability <final-run-id>
cargo run --release --bin alpine -- clean-restart-stability <final-run-id>
cargo run --release --bin alpine -- near-limit-context <final-run-id> --ratio 0.85
cargo run --release --bin alpine -- golden-agent <final-run-id> --task python-off-by-one
cargo run --release --bin alpine -- rollback-proof <final-run-id>
```

The restart harness performs ten real stop/start cycles, proves distinct process identities, compares raw 128-token greedy outputs, and restores the prior material Session configuration before publishing evidence.

The context harness sizes its deterministic ledger with the live tokenizer, reaches within two percent of the requested ratio, requires two exact three-needle retrievals, and records the raw responses and token digests.

The golden-agent harness runs OpenCode directly from Rust in a copied fixture. It verifies the effective 16K policy, strips ambient credential variables from the child, keeps core coding tools available, runs the versioned tests, rejects protected or unexpected changes, and binds evidence to the exact OpenCode executable and task-suite hashes.

The rollback proof launches the current `stable-16k` rollback role, binds its Profile, Session Config and runtime hashes, performs a real inference smoke, and restores the prior material Session. A merely present JSON Profile is not accepted as rollback evidence.

Manual attachment is reserved for the human capability review:

```powershell
cargo run --release --bin alpine -- record-evidence <final-run-id> --kind operator-reviewed-capability-report --evidence C:\path\to\review-details.json --reviewed-by "operator name"
```

The current Alpine executable must match the final run's software identity. Qualification recomputes counts, sequence and token digests from the stability artifact instead of trusting a summary boolean. Exact retries recover safely after interruption, while changed content, stale binaries, digest mismatches, duplicate kinds, and invalid kind-specific claims fail closed. Capability evidence requires an explicit human reviewer; an automated benchmark cannot silently supply that gate.

## OpenCode context and permissions

Bounded 16K/32K profiles disable foreign Claude prompt injection and ambient skill catalogs but explicitly keep core read, edit, write/patch, search, shell, web, task, and todo capabilities. The local model is fixed at 16,384 context and 4,096 output for `stable-16k`. Skills are a reversible context-budget choice, not a safety rule.

The local provider explicitly enables OpenCode's Exa-backed `websearch`; an `allow` permission alone does not make that tool visible for a non-OpenCode provider in OpenCode 1.18.18. Search queries therefore leave the machine for the external search service, just as requested URLs leave the machine for `webfetch`. Model-visible tool output is capped at 500 lines or 12,288 bytes. OpenCode retains the complete result in its managed truncation directory so the agent can inspect focused portions with Grep or bounded Read calls instead of spending most of a 16K window on one result.

Skills can be enabled explicitly with `alpine.exe opencode --skills`; Profiles may also enable them. This is a context/resource choice, not a content restriction.

The permission policy contains no subject, reasoning, research, reverse-engineering, or technique filter. Routine local coding and read-only web research are allowed. Exact raw credential files are denied to direct tools; outside-project access and representative destructive, external-write, credential, and privilege commands ask for consent. The effective merged configuration is checked before Session acquisition, so project config or plugins cannot silently weaken those invariants.

These shell rules are honest accident tripwires, not effect mediation: Python, PowerShell APIs, aliases, custom executables, or another spelling can bypass command-pattern matching, and tool output/session persistence is not a DLP boundary. Tools still execute as the current Windows user. Use a disposable VM or a separate restricted Windows identity for adversarial repositories or a deliberately hostile agent.

The launcher rejects OpenCode `--auto`, disables external plugins and project config unless explicitly requested, strips credential-like environment variables and credential-pointer variables, binds inference to localhost, and uses a random file-backed bearer key.

## Recovery and rollback

`stable-16k` plus the official runtime is initialized as the rollback path. The custom runtime is isolated in `runtime-custom`; the patched source/build is reproducible from the recorded commit and patch. Incomplete downloads are not referenced by any Profile and remain outside the production path.

Do not delete older Profiles, raw results, backups, or runtime bundles merely because another Profile is faster. Evaluation never promotes. Use the explicit Promotion contract only after current automated evidence and the human Capability Review pass, retain Stable as rollback, and record contradictory operation as an Incident or Rollback.
