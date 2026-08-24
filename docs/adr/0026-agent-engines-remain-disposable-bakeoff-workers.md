# ADR 0026: Agent engines remain disposable bake-off workers

## Status

Accepted — 2026-08-23

## Context

ADR 0005 selected OpenCode as the v1 Harness, ADR 0019 made Rust the sole control-plane authority above inference, and ADR 0024 retained the Pi/OpenCode exact-read probe as an isolated comparison rather than a generalized engine protocol. Issue #24 asked whether the current OpenCode process path, Pi's SDK/core, Pi's process/RPC mode and a maintained embeddable alternative could support a future integrated agent without creating a second project, Session, configuration, lifecycle or evidence authority.

The source review pinned OpenCode 1.18.21, Pi 0.84.2 and Cline `@cline/agents` 0.0.78. Pi SDK/core and Cline's stateless agents package expose the narrowest embeddable surfaces. Pi RPC exposes the broadest ready-made headless lifecycle. All candidates still run tools with the worker process authority rather than a security sandbox. Their event, queue, storage and recovery contracts differ. The bake-off completed identical scenario accounting under current complete Evidence Identity, but no candidate demonstrated all required capabilities.

## Decision

Alpine adopts a bounded agent-engine bake-off evidence contract, not an Agent Engine Protocol or a production engine boundary.

The versioned plan pins the four reviewed candidate seams, one request/retry/restart/event-queue budget, the complete seven-dimension Evidence Identity contract, identical material inputs, required recovery scenarios, privacy exclusions, license and dependency facts, security boundaries and the recommendation. Candidate observations enter Alpine only as strict typed events and errors. Terminal-formatted text, raw prompts, credentials, machine paths, worker transcripts and generated files are not application state or public report fields.

The runner does not import caller-authored observations. It verifies an isolated package lock against the reviewed versions and registry integrity pins, hashes the running Alpine and Node executables plus the repository worker and complete installed package/transitive file closure, computes the seven Evidence Identity dimensions once, acquires the pinned Inference Session and launches each named adapter for each scenario. The effective read-only single-file policy and temperature/input/output limits are one serialized identity-bound object used by every adapter. A seam that cannot enforce that exact policy is an explicit failure and is not launched with broader authority. Alpine supplies only remaining candidate request/retry/restart/event capacity before each scenario; SDK adapters count native provider turns and stop on native-event overflow. The repository-owned adapter maps native observations; it is a bounded prototype implementation detail, not a generic protocol. Alpine assigns event sequence and scenario identity, measures wall time, verifies material Session state after every scenario and restores the exact prior Session after every candidate. A new Session transaction identity is allowed; profile, runtime, arguments, environment, vision and fallback state must match exactly. Malformed, partial, unsuccessful or reopened lifecycles fail closed, while a missing capability remains an explicit typed failure with no partial events.

OpenCode remains the v1 Harness. No candidate code or dependency is absorbed, vendored, automatically installed or promoted by this decision. Candidate packages live in an operator-prepared isolated root; candidate data/config/cache roots are created and deleted by Alpine. The checked-in plan cannot inject commands, and the runner invokes only the repository-owned adapter plus the four exact package names. Alpine remains the only durable owner of projects, Sessions, Evidence Identity, configuration, lifecycle and recovery.

The current recommendation is no-go for engine-boundary adoption. Pi SDK/core and Cline `@cline/agents` remain conditional follow-up seams, while Pi RPC remains a disposable isolation comparison. A future adoption decision requires identical live evidence demonstrating the required capabilities, exact restart recovery from Alpine-owned state and a bounded event-flow contract from at least two engines. That decision must update or supersede this ADR and any affected Harness ADR before production implementation.

## Consequences

The normal machine-local Session Config used a retired schema, and Alpine rejected it before mutation. A complete run was then performed through a disposable schema-5 install view that referenced the same stable Profile, model, runtime and chat template while leaving global installs, defaults, Profiles, model configuration and Deployment Roles unchanged. All 44 scenario-level Session checks and all four exact prior-Session restorations passed, and all candidate roots were disposed. Pi SDK/core demonstrated streaming, exact-read tools, cancellation and normalized errors; Cline demonstrated streaming, exact-read tools and cancellation. OpenCode process and Pi RPC were not launched with broader or non-identical prompt/tool authority and instead recorded explicit typed policy failures. Both live embeddable candidates exceeded the native-event bound in the backpressure scenario; no candidate demonstrated restart/continuation recovery. The no-go decision therefore remains.

The smallest reviewed upstream gaps are recorded per candidate in `config/agent-engine-bakeoff.json`. The common missing proof is an Alpine-owned typed state import/export path combined with bounded event delivery. Until that proof exists, candidate session stores are disposable implementation details, not recovery authority.
