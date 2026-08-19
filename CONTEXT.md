# Local Models

The control plane for one locally served Abliterated Model on hardware-5070. It complements a hosted Frontier Model when local control or low refusal materially matters.

Three different things must not be collapsed:

- **Abliterated Model** behavior (weights)
- **Harness Policy Boundary** (what OpenCode auto-allows / asks / denies)
- **Attack-Lab Isolation Boundary** (later real blast-radius containment)

The model is an untrusted worker.

## Current system state

This repository is the versioned control plane. `%USERPROFILE%\local-models` is the generated machine-local installation containing large artifacts, runtime bundles, secrets and logs. Run `setup.ps1` to reconstruct it; do not treat the generated install as the source of truth.

The production profile is `stable-16k`. `turbo-16k` and `fast-32k` are measured candidates using the request-local n-gram patch. `long-64k` is experimental and failed its first 55.7K-token retrieval gate. See `docs/AUDIT-2026-08-19.md` for evidence and run IDs.

OpenCode's bounded first request is now about 5.5K tokens in a live greeting, not 24–29K. Core task, todo, file, search, shell and web tools remain. Skill catalogs are omitted from bounded profiles as a context-budget decision and may be explicitly enabled.

## Language

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
