# Project Alpine

The Rust control plane for automatically discovering, measuring, tuning, qualifying and operating local AI inference. `llama.cpp` and its C++/CUDA implementation remain below the Inference Server boundary. Every evaluated deployment is identity-bound evidence, not a permanent product assumption.

Three different things must not be collapsed:

- **Abliterated Model** behavior (weights)
- **Harness Policy Boundary** (what OpenCode auto-allows / asks / denies)
- **Attack-Lab Isolation Boundary** (later real blast-radius containment)

The model is an untrusted worker.

## Current system state

This repository is the versioned control plane. `%USERPROFILE%\local-models` is the default generated machine-local installation containing large artifacts, runtime bundles, secrets and logs. Run `setup.ps1` to reconstruct it; do not treat the generated install as the source of truth.

Rust is the sole control-plane authority for supported discovery, selection, measurement, tuning, qualification, Session and OpenCode workflows. Stable-16K remains the rollback Profile, but Rust owns its operation and proof. Python/PowerShell code is compatibility and research tooling; C# is a folder-picker Adapter. `cargo run --locked --bin alpine-verify` verifies the repository and retained compatibility surface. `alpine evaluate` is the single executable path that generates and independently verifies bounded product claims.

Fresh installations initialize `stable-16k` as both `daily_default` and `rollback_profile` without inheriting Qualification. `turbo-16k`, `fast-32k`, and `long-64k` remain available inference configurations, but a Profile file carries no lifecycle claim. Turbo becomes the daily default only after fresh current evidence, the project owner's substantive Capability Review, production Qualification, and an explicit Promotion event.

OpenCode's bounded first request is now about 5.5K tokens in a live greeting, not 24–29K. Core task, todo, file, search, shell and web tools remain. Skill catalogs are omitted from bounded profiles as a context-budget decision and may be explicitly enabled.

## Language

**Project Alpine**:
The Rust control plane above local Inference Servers. It owns Support Envelopes, Experiments, Qualification and transactional Sessions; it does not implement tensor inference.
_Avoid_: Rust wrapper, llama.cpp replacement, benchmark script collection

**Support Envelope**:
A versioned, capability-based statement of environments Alpine can evaluate. It is not a list of assumptions about the current PC, and eligibility is not qualification.
_Avoid_: supported hardware list (when evidence is incomplete), current machine config

**Verified Deployment**:
An exact environment and material configuration that has completed identity-bound Qualification. A Verified Deployment does not prove that other machines within the Support Envelope are production-ready.
_Avoid_: supported platform, Windows/NVIDIA support, compatible machine

**Evaluation Plan**:
A versioned, bounded search space plus workload, request budget and qualification target. It may describe a deployment-specific experiment, but it is not a permanent hardware policy and cannot weaken Promotion Policy gates.
_Avoid_: unbounded autotuning, global optimum claim, hidden benchmark matrix

**Evidence Identity**:
The hardware, software, model, runtime, workload, configuration and policy identities to which evidence applies. A material identity change makes prior evidence stale for the changed claim.
_Avoid_: latest result, same machine probably, benchmark name alone

**Qualification**:
An evidence decision with one of five outcomes: `qualified`, `unsupported`, `inconclusive`, `regressed` or `not-proven`. Final evidence must be independent from tuning and selection evidence.
_Avoid_: promotion Boolean, benchmark passed (when external gates remain), best config

**Profile**:
The inference-material configuration evaluated by Alpine. Qualification state, daily-default selection and rollback responsibility are not Profile fields.
_Avoid_: deployment status, launcher choice, promotion record

**Deployment Role**:
A local operational assignment of a still-current qualified Profile, currently `daily_default` or `rollback_profile`. Changing a Deployment Role does not mutate the qualified Profile identity.
_Avoid_: Profile status, benchmark selection, inherited qualification

**Profile Override**:
An explicit Profile selection for one Session that does not alter Deployment Roles or append a Deployment Event. Testing, maintenance and temporary Stable use remain Overrides unless the operator separately invokes Rollback.
_Avoid_: Rollback, Promotion, default change

**Promotion**:
The transaction that verifies still-current production Qualification, records that decision immutably, and assigns the candidate Profile to the `daily_default` Deployment Role. A fresh installation has no inherited Promotion.
_Avoid_: editing Profile status, changing a shortcut, benchmark selection

**Deployment Event**:
An append-only record of a Promotion, Rollback or other Deployment Role transition. Current deployment state is derived from this history; earlier events and their Qualification references are never rewritten.
_Avoid_: mutable deployment status row, edited Promotion, launcher preference

**Rollback**:
An explicit deployment transaction that restores the `daily_default` role to the Rollback Profile and appends the operator, reason and reversed Promotion. It does not erase the prior Promotion or its historical Qualification.
_Avoid_: temporary Stable launch, editing a Promotion, automatic re-promotion

**Incident**:
Append-only evidence that later operation contradicted or materially challenged a deployment decision. An Incident can suspend eligibility and expose a deficient Qualification policy without rewriting what earlier evidence established.
_Avoid_: benchmark failure, deleted Qualification, silent demotion

**Capability Review**:
A substantive human evaluation of whether the complete production workflow is useful and trustworthy across required capability categories and reviewer-chosen realistic scenarios. Its versioned evidence contract records expectations, observations, outcomes, limitations and accepted residual risks while leaving the final judgment to the named human reviewer.
_Avoid_: capability checkbox, benchmark pass, operator rubber stamp

**Supporting Review Artifact**:
An optional private, content-addressed supplement to a Capability Review. It contains only redacted material appropriate for retention; raw credentials and secrets are neither stored nor hashed as proof.
_Avoid_: mandatory transcript, embedded private repository, secret digest

**Production Profile**:
The daily-default Profile that has satisfied the complete production Qualification, including its Capability Review. Qualification must precede promotion.
_Avoid_: fastest Profile, selected candidate, benchmark winner

**Rollback Profile**:
A retained, independently proven Profile that can restore known-good operation if the Production Profile fails. Promoting a candidate does not erase or weaken this fallback.
_Avoid_: old Profile, backup config, untested fallback

**Public Source Release**:
The deliberately licensed and privacy-reviewed source form of Project Alpine. v0.1 establishes reproducible source and excludes binaries, installers, generated installations, model weights, runtime bundles, secrets, machine-local state and raw private evidence.
_Avoid_: making the working directory public, binary/model bundle, public evidence dump

**Public Evidence**:
A generated, allowlisted representation of private evidence containing only structural facts deliberately approved for reproducibility and public claims. Private review content and reviewer identity remain excluded; any public limitations or risk narrative is separately authored and explicitly approved rather than copied from private evidence.
_Avoid_: sanitized raw report, public inventory, raw evidence export

**Project License**:
Apache-2.0 as applied to Project Alpine's own source and documentation. It does not relicense Third-Party Material or erase its notices and attribution requirements.
_Avoid_: repository-wide relicensing, model license, dependency license

**Third-Party Material**:
Code, patches, models, templates, runtimes or other artifacts whose rights originate outside Project Alpine. Each item retains its own license boundary, notices, attribution and redistribution conditions.
_Avoid_: Alpine source, bundled by implication, covered by the Project License

**Contribution Terms**:
Project contributions are submitted through the ordinary pull-request workflow and distributed under the Project License. No per-commit DCO sign-off or contributor license agreement is required. Contributors remain responsible for submission rights, provenance and third-party obligations.
_Avoid_: Signed-off-by requirement, copyright assignment, CLA, assuming repository license erases third-party terms

**Agent-Generation Disclosure**:
A concise pull-request statement required when a generative agent produced a material part of the submitted diff or autonomously planned and executed multi-step work embodied in it. It identifies that work and the human verification performed; ordinary autocomplete, writing help, explanations, research and manually reimplemented suggestions are outside this term.
_Avoid_: blanket AI-use disclosure, quality guarantee, transfer of responsibility to a tool

**Frontier Model**:
The hosted default when raw capability dominates. Today that is GPT-5.6 Sol.
_Avoid_: best model, aligned coding model (when you mean Sol), local model (when you mean Sol)

**Local Model**:
The locally served Abliterated Model. It exists because local control and low refusal matter, not because “local” is a goal. One Local Model covers ordinary coding and authorized security work.
_Avoid_: coding model, security model (as separate products), treating “local” as the product

**Abliterated Model**:
A local-model checkpoint whose built-in refusal behavior has been reduced (ablation / de-refusal). It is a behavior profile of the weights, not a permission grant. This is the primary Local Model.
_Avoid_: uncensored model (when used as if it implies host access), jailbroken model

**Control Checkpoint**:
The upstream Instruct twin of the Abliterated Model, used only as an A/B control. Not a v1 install requirement and not a daily resident.
_Avoid_: Instruct Model (when you mean the daily driver), vanilla, stock, aligned model

**Harness Policy Boundary**:
The v1 OpenCode permission rules: capability-preserving consent tripwires for ordinary tool calls. They auto-allow routine work, ask before common consequential effects, and deny implicit direct reads of personal credentials. They do not comprehensively mediate an evasive shell command and are not a kernel, VM, container, or restricted-account boundary. OpenCode still runs as the Windows user.
_Avoid_: hard host authority boundary, sandbox (when you mean these rules), Attack-Lab Isolation Boundary

**Attack-Lab Isolation Boundary**:
Later real containment of a potentially compromised or hijacked agent: VM, sandbox, namespace, restricted identity. Out of v1.
_Avoid_: OpenCode permissions, Harness Policy Boundary

**Selected Project**:
The one repository the Harness Policy Boundary binds file and Git rules to. Reachability on disk is not consent.
_Avoid_: workspace, folder, whatever the model can see

**Selected Target**:
A later Attack Lab concept: an explicit host, IP, range, or lab authorized for active testing. v1 does not implement a destination allowlist. Operator judgment is the only destination control.
_Avoid_: domain allowlist (for ordinary research), scanning ban, pretending OpenCode enforces target scope

**Test Credential**:
An explicitly selected key or identity for an authorized target. The Harness must not silently use the operator’s personal `~/.ssh` or password stores.
_Avoid_: default SSH agent, opportunistic host identity

**Session Config**:
The generated machine-local selection and path map. Versioned artifact identity and profile knobs live in this repository; setup renders Session Config from them. Scripts, tests, and the Harness read the rendered file rather than independently encoding paths or port values.
_Avoid_: hard-coded 8100, scattered ports, mutable marketing names as provenance

**Harness**:
OpenCode, pointed at a configurable local OpenAI-compatible endpoint. The only path from model output to host actions.
_Avoid_: wrapper, frontend, chatbot, Cline, protocol bridge

**Agent Engine Candidate**:
A source-pinned, untrusted and disposable worker seam evaluated under Alpine-owned Evidence Identity, budgets, typed events/errors and exact Inference Session restoration. Candidate project, session, configuration and transcript stores never become Alpine authority.
_Avoid_: second Harness, embedded authority, permanent generic protocol, candidate session as recovery truth

**Agent Engine Bake-off**:
The bounded headless comparison of reviewed Agent Engine Candidates. Alpine launches exact package-locked adapters from disposable roots, owns Evidence Identity and Session restoration, and turns candidate-native SDK/RPC/JSON observations into typed events and errors. Every required scenario is either demonstrated under identical material inputs or retained as an explicit typed failure. It is architecture evidence, not Qualification, Promotion or approval to absorb a dependency.
_Avoid_: benchmark winner, production engine selection, fork approval, model qualification

**Inference Server**:
The local process that loads weights and serves an OpenAI-compatible API on localhost.
_Avoid_: the model (when you mean the process), runtime (when you mean this specific server)

**Inference Session**:
An on-demand run of the large-model Inference Server. Not a login service. Start/stop is transactional: record whether the Whispering cleanup server was running, pause only that process, restore it only if this Session paused it, and roll back if the large model fails to start.
_Avoid_: always-on local LLM, startup service (for the large model), kill every llama-server

**Capability**:
A named class of host actions the Harness may perform (for example project-scoped write vs controlled shell). Distinct from model refusal.
_Avoid_: tool access, permissions (when you mean a Capability class)

**Repo Loop**:
v1: Abliterated Model → Inference Server → localhost /v1 → one Harness → project-scoped read/write, build, test, controlled shell, Git, open research and testing Internet, iterative tool use.
_Avoid_: Attack Lab (when you mean this loop)

**Attack Lab**:
An isolated environment for authorized red-team targets, separate from personal data. Out of v1. This is where the Attack-Lab Isolation Boundary is built.
_Avoid_: security sandbox (when you mean the Harness Policy Boundary), Phase 10
