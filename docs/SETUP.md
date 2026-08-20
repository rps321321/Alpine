# Reproducible Windows setup

## Fast path

Clone the repository, open PowerShell in it, and run:

```powershell
.\setup.ps1
```

The default install root is `C:\Users\<you>\local-models`, and the default selected profile is `stable-16k`:

```powershell
.\setup.ps1 -Profile stable-16k
```

Setup verifies or resumably downloads the pinned model, projector, official Qwen template, official `llama.cpp` release, and CUDA DLL archive. The custom profiles use the exact pinned source commit plus the repository patch. One installation-scoped lock serializes setup. Runtimes and the complete control plane are staged before publication; a recovery journal restores the prior usable installation if publication is interrupted. Invalid downloads are preserved with unique names rather than destroyed.

On a new Windows installation, permit setup to install the pinned prerequisites:

```powershell
.\setup.ps1 -Profile stable-16k -InstallPrerequisites
```

This may install Git, CMake 4.2.3, CUDA 13.2, Visual Studio 2022 Build Tools, Node.js if needed, and OpenCode 1.18.18. Review this system-wide operation before running it.

Large artifact downloads are verified by byte size and SHA-256 from `config/artifacts.json`. The installer is safe to rerun and resumes `.part` downloads. A bounded "another setup transaction owns" error means a setup is still active; rerunning after an owner crash repairs any journaled partial publication first.

## Launch

Double-click `Open Local Qwen.exe` in the install root, choose a project folder, and work in the OpenCode terminal. The executable selects the folder and supervises the visible PowerShell process; the reviewed policy and runtime behavior remain in the installed scripts. A failed session stays visible until acknowledged, and startup PowerShell errors are redacted into a per-launch record before being atomically published at `logs\launcher-last-error.log`. Native OpenCode diagnostics remain visible in that terminal; the stable record includes their non-zero exit code without recording the interactive transcript.

To verify this failure path without loading the model, run `Open Local Qwen.exe --project <existing-folder> --diagnostic-failure`. It deliberately presents a diagnostic error, writes the stable redacted log, and exits non-zero.

CLI alternatives from the repository:

```powershell
.\localmodel.ps1 profiles
.\localmodel.ps1 status
.\localmodel.ps1 start --profile stable-16k
.\localmodel.ps1 opencode --profile stable-16k --project C:\path\to\project
.\localmodel.ps1 stop
```

The launcher refuses to steal port 8100 from a different executable. `status` verifies process ownership rather than treating any healthy HTTP listener as the configured model.

## Verify a restored machine

Fast size/version check:

```powershell
.\localmodel.ps1 doctor
```

Full model/projector/template hash verification:

```powershell
.\localmodel.ps1 doctor --deep
```

Collect a new machine manifest after a reinstall or hardware change:

```powershell
.\localmodel.ps1 inventory
```

The current manifest is `inventory/hardware-5070-2026-08-19.json`. Do not compare performance runs across hardware manifests as though they were the same environment.

## Profiles

```text
stable-16k   production    official runtime, MTP3, no n-gram
turbo-16k    candidate     custom request-local n-gram + MTP3
fast-32k     candidate     32K request-local n-gram profile
long-64k     experimental  research only; failed first near-limit quality gate
```

Apply validates the installed Profile/runtime, changes the selected default atomically under a Session Config lock, and preserves a uniquely named backup:

```powershell
.\localmodel.ps1 apply stable-16k
```

Profile status is a lifecycle claim, not a menu label. See `config/promotion-policy.json`.

## Benchmark and inspect evidence

```powershell
.\localmodel.ps1 benchmark --profile stable-16k --runs 5 --warmups 1
.\localmodel.ps1 context-stress --profile fast-32k --ratio 0.85 --runs 2
.\localmodel.ps1 agent-benchmark --profile stable-16k --task python-off-by-one
.\localmodel.ps1 runs --limit 20
.\localmodel.ps1 compare stable-16k turbo-16k fast-32k
.\localmodel.ps1 qualify <run-id> --target candidate
.\localmodel.ps1 report stable-16k turbo-16k fast-32k
```

SQLite lives at `results/results.sqlite3`; each run keeps raw JSONL, outputs, logs, configurations and compressed long prompts under `results/runs/<run-id>`. Generated evidence is local by default. Summary reports are regeneratable.

The Rust path is authoritative for new microbenchmark qualification evidence. Run a tuning baseline and a separate, freshly hashed final pass with identical material configuration, then qualify the final SQLite rows:

```powershell
cargo run --release --bin alpine -- benchmark --profile fast-32k --phase tuning --runs 5 --warmups 1
cargo run --release --bin alpine -- benchmark --profile fast-32k --phase final --runs 5 --warmups 1 --deep-verify-artifacts
cargo run --release --bin alpine -- tune --baseline-run <baseline-tuning-run-id> --candidate-run <candidate-tuning-run-id>
cargo run --release --bin alpine -- qualify <final-run-id> --tuning-run <tuning-run-id> --target candidate
```

The tuner is read-only: it recommends a measured configuration or retains the baseline and never edits the installed Profile. The legacy `localmodel.ps1 qualify` command remains migration evidence; it is not sufficient for a new Alpine qualification claim.

Measured runs hold an inference-capacity lease tied to their exact Inference Session identity. A second benchmark or unrelated interactive launcher receives a visible busy refusal instead of silently queueing and contaminating timings. The agent benchmark's own launcher carries the governing lease token.

If a harness is interrupted after producing evidence:

```powershell
.\localmodel.ps1 reconcile <run-id>
```

This classifies preserved evidence; it does not delete or invent samples.

Validated/production automated evidence is generated by Rust-owned harnesses against the exact passed final run. The same-process gate records raw token ids for 50 contaminant/target pairs and restores the prior Session:

```powershell
cargo run --release --bin alpine -- same-process-stability <final-run-id>
cargo run --release --bin alpine -- clean-restart-stability <final-run-id>
cargo run --release --bin alpine -- near-limit-context <final-run-id> --ratio 0.85
cargo run --release --bin alpine -- golden-agent <final-run-id> --task python-off-by-one
```

The restart harness performs ten real stop/start cycles, proves distinct process identities, compares raw 128-token greedy outputs, and restores the prior material Session configuration before publishing evidence.

The context harness sizes its deterministic ledger with the live tokenizer, reaches within two percent of the requested ratio, requires two exact three-needle retrievals, and records the raw responses and token digests.

The golden-agent harness runs OpenCode directly from Rust in a copied fixture. It verifies the effective 16K policy, strips ambient credential variables from the child, keeps core coding tools available, runs the versioned tests, rejects protected or unexpected changes, and binds evidence to the exact OpenCode executable and task-suite hashes.

Manual attachment is reserved for the human capability review:

```powershell
cargo run --release --bin alpine -- record-evidence <final-run-id> --kind operator-reviewed-capability-report --evidence C:\path\to\review-details.json --reviewed-by "operator name"
```

The current Alpine executable must match the final run's software identity. Qualification recomputes counts, sequence and token digests from the stability artifact instead of trusting a summary boolean. Exact retries recover safely after interruption, while changed content, stale binaries, digest mismatches, duplicate kinds, and invalid kind-specific claims fail closed. Capability evidence requires an explicit human reviewer; an automated benchmark cannot silently supply that gate.

## OpenCode context and permissions

Bounded 16K/32K profiles disable foreign Claude prompt injection and ambient skill catalogs but keep the nine core coding tools. A captured request is 26,003 bytes: 817 bytes of system prompt and 20,915 bytes of tool schemas. A live fresh `hey` uses 5,529 input tokens. Fast-32K generated an 11-token answer in about 1.55 seconds; the final Stable-16K smoke generated 64 tokens in about 9.76 seconds of model time. Neither compacted.

Skills can be enabled explicitly with the installed PowerShell launcher's `-WithSkills`; `long-64k` enables them by profile. This is a context/resource choice, not a content restriction.

The permission policy asks before common destructive, externally visible, credential or privilege effects and shields direct credential-file reads. It does not censor what the abliterated model may discuss or implement. It is also not hostile-code containment: tools execute as the current Windows user. Use a disposable VM or separate restricted Windows identity for adversarial repositories or untrusted generated code.

The launcher rejects OpenCode `--auto`, disables external plugins unless explicitly requested, strips credential-like environment variables, binds inference to localhost, and uses a random file-backed bearer key.

## Recovery and rollback

`stable-16k` plus the official runtime is the rollback path. The custom runtime is isolated in `runtime-custom`; the patched source/build is reproducible from the recorded commit and patch. The incomplete NVFP4 download is not referenced by any profile and may remain paused/recoverable outside the production path.

Do not delete older profiles, raw results, backups or runtime bundles merely because a candidate is faster. Promote only after the policy gates and retain the previous production profile.
