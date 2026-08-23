# Alpine Desktop v0 product specification

## Problem Statement

Local-model users currently have to assemble model discovery, hardware inspection,
GGUF selection, llama.cpp lifecycle, tuning evidence, deployment choices and a
coding harness across separate commands and tools. The result is difficult to
understand, easy to misconfigure and unlike the coherent task-centered experience
of a modern coding-agent desktop application.

## Solution

Alpine Desktop is one local-first Tauri application that lets a user discover or
import a model, understand whether it fits the current host, download the exact
artifact, evaluate conservative settings, select a launch default and work with a
Pi-backed coding agent. Alpine remains the authority for hardware, model,
Inference Session, evidence, settings and recovery. Pi is the first replaceable
Agent Runtime Adapter and never becomes a second durable authority.

The desktop experience follows the proven Codex arrangement: a compact project
and task rail, a calm task stream and composer, and a contextual side panel for
model diagnostics, files, diffs, terminal output and browser artifacts.

## User Stories

1. As a first-time user, I want Alpine to inspect my host automatically, so that I immediately understand my CPU, RAM, GPU, VRAM, driver and runtime readiness.
2. As a first-time user, I want hardware failures to be actionable, so that a missing driver or runtime does not look like an unsupported model.
3. As a local-model user, I want to search Hugging Face for GGUF models, so that I do not need to find download URLs manually.
4. As a local-model user, I want search results to show publisher, downloads, update time, files, sizes, quantizations and gating, so that I can choose the exact artifact deliberately.
5. As a local-model user, I want Alpine to estimate RAM, VRAM and storage fit before download, so that I do not waste bandwidth on an obviously incompatible artifact.
6. As a local-model user, I want estimates labelled separately from measured evidence, so that a heuristic never looks qualified.
7. As a local-model user, I want resumable, cancellable downloads with integrity and provenance, so that large artifacts survive interruptions without becoming trusted partial files.
8. As a local-model user, I want to import an existing GGUF, so that Alpine can manage models I already have.
9. As a local-model user, I want downloaded and imported models in one Model Registry, so that selection is consistent across settings, tuning and tasks.
10. As a local-model user, I want to assess conservative llama.cpp placement first, so that tuning does not begin with unsafe or paging-heavy settings.
11. As a local-model user, I want prompt processing, novel decode and speculative repeated-token throughput shown separately, so that a fast-looking profile is not misleading.
12. As a local-model user, I want correctness, determinism, spill, stability and tool use shown beside speed, so that model suitability is capability-first.
13. As a local-model user, I want the recommended configuration explained, so that I can see which evidence selected it and which limitations remain.
14. As an operator, I want evaluation never to change the daily default, so that experimentation cannot silently promote a Profile.
15. As an operator, I want one explicit default model selection for new tasks, so that the chosen model is used when the Agent Runtime starts.
16. As an operator, I want Stable rollback preserved, so that a failed model or runtime transition restores the prior Inference Session.
17. As a developer, I want to create durable tasks grouped by Selected Project, so that agent work has a stable place to live.
18. As a developer, I want Pi to stream text, tool lifecycle, usage and errors as typed events, so that the UI never parses terminal-formatted output.
19. As a developer, I want to steer a running task and queue follow-up work, so that I can correct direction without discarding progress.
20. As a developer, I want cancellation and worker failure to settle visibly, so that I know whether the task and Inference Session are safe to continue.
21. As a developer, I want files, diffs, tests and terminal output in the side panel, so that review remains in the task context.
22. As a developer, I want a local browser surface in the side panel, so that localhost previews and small web artifacts can be inspected without leaving Alpine.
23. As a developer, I want external or authenticated browsing to require an explicit surface and authority choice, so that an embedded preview is not mistaken for a trusted full browser.
24. As a privacy-conscious user, I want secrets, private prompts and raw repository content excluded from diagnostics by default, so that useful telemetry does not become a provenance leak.
25. As a user, I want a proper Settings page for models, runtime, storage, Agent Runtime, appearance, safety and diagnostics, so that configuration is discoverable and not scattered.
26. As a user, I want settings validation and restart requirements shown before saving, so that a bad path or port does not fail later during a task.
27. As a maintainer, I want cold-start, command latency, stream latency, memory and bundle size measured, so that the desktop app remains responsive as features grow.
28. As a maintainer, I want unit, command, harness and rendered-flow tests, so that a polished screen is not accepted when the underlying workflow is broken.

## Implementation Decisions

- The product is a Tauri 2 desktop application with a React and TypeScript webview. Alpine's Rust library remains the authoritative application host.
- The desktop shell presents one small Desktop Interface to the webview for bootstrap, model discovery, assessment, download, settings, Inference Session and task operations. Commands return typed results and long operations emit typed events.
- Existing Support Envelope, Hardware, Profile, Inference Session, Experiment, Qualification and Deployment modules are reused rather than reimplemented in TypeScript.
- Pi SDK/core 0.84.2 is the first experimental Agent Runtime Adapter. It is source/version/integrity pinned, receives Alpine-owned model and policy input, and exposes normalized events. It owns no Model Registry, Deployment Role, project, recovery truth or credentials.
- Agent Runtime input required for restart recovery is serialized as Alpine-owned Messages and typed events. Interrupted work and pending approvals settle visibly and are never replayed automatically. Pi remains experimental until the complete desktop workflow passes live capability Qualification.
- Hugging Face is the only remote Model Catalog Adapter in v0. Search is restricted to model repositories with GGUF artifacts. Exact repository revision, filename, byte size and origin URL are retained with every download.
- A Model Assessment is a pre-download estimate. Experiment and Qualification remain the only measured evidence paths, and only explicit Promotion changes a Deployment Role.
- Downloads use a temporary partial file, support cancellation and publish the final artifact only after expected byte count and configured digest checks pass.
- The Browser Surface initially optimizes for localhost previews and generated web artifacts in the side panel. Arbitrary authenticated browsing and a shared browser profile are separate authority decisions.
- Settings are machine-local and schema-versioned. The desktop UI never writes versioned Profiles or Deployment Events by editing JSON directly.
- UI performance instrumentation records bootstrap, Pi launch, first-stream-event and stream durations, current asset transfer, long-task count and webview heap when available. Approved command duration is retained with its typed result. Metrics exclude prompts, credentials and repository content and can be disabled locally.
- The Codex reference informs hierarchy, density and interaction placement; Alpine uses its own name, icons, content and model-native states rather than reproducing OpenAI branding.

## Testing Decisions

- Tests verify behavior through public interfaces and do not reach into implementation details.
- The primary command seam is the Desktop Interface: one bootstrap snapshot plus commands/events for model discovery, assessment, download, settings and task launch.
- The Agent Runtime seam is tested with a deterministic stream adapter through the same Pi-facing interface used by the application.
- The rendered-flow seam covers first launch, a successful search and assessment, default selection, task creation, settings navigation and browser-panel opening.
- Hardware and model-fit expected values use worked fixtures rather than recomputing the implementation's formulas in assertions.
- Network tests use recorded schema-minimal responses or a local fixture server; the normal test suite does not download model weights or depend on Hugging Face availability.
- Existing Rust tests and the canonical `alpine-verify` command remain mandatory after control-plane changes. Desktop checks add TypeScript, unit, UI build and Tauri Rust tests.

## Out of Scope

- Claiming that the first Pi integration is production-qualified.
- Silently promoting a downloaded model or recommended Profile.
- Remote model catalogs other than Hugging Face in v0.
- Shipping model weights, llama.cpp binaries or third-party credentials in the source repository.
- Reproducing Codex trademarks, proprietary implementation details or authenticated browser state.
- Attack-Lab containment, multi-user synchronization and cloud relay in the first desktop milestone.

## Further Notes

The implemented application now covers the complete local vertical workflow:
native first-launch hardware capture; immutable-revision Hugging Face discovery;
verified resumable download and existing-GGUF import into one Model Registry;
capacity-labelled fit and placement estimates; default model/Profile selection;
explicit verified llama.cpp start/stop plus task-time reuse; durable
Project/Task/Message/Event/Approval recovery; Pi streaming, steering, follow-up,
cancellation and project-scoped read/search/edit/shell tools; contextual files,
diffs, terminal output and localhost browser preview; and bounded Candidate,
Validated or Production evaluation through Alpine's existing evidence engine.

The remaining release distinction is evidence, not a hidden placeholder: the Pi
adapter is still labelled experimental, authenticated external browsing is not
granted ambient authority, memory-spill fields are shown as not captured when a
measurement plan does not provide them, and signed installer/update distribution
requires a later release key and operator decision. The desktop never promotes a
Profile or changes a Deployment Role during analysis.
