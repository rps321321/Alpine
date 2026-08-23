# ADR 0027: Tauri hosts Alpine Desktop and Pi is its first experimental Agent Runtime

## Status

Accepted — 2026-08-23

## Context

The feature-ideas intake asks for one coherent local-first desktop coding agent:
hardware discovery, Hugging Face model selection and download, model-fit advice,
llama.cpp lifecycle, bounded tuning, task work, settings, diagnostics and an
in-app browser. The target experience is the compact task-and-artifact workflow
of Codex Desktop, while Alpine must retain the evidence and recovery guarantees
that distinguish it from a generic chat wrapper.

ADR 0019 makes Rust the sole control-plane authority. ADR 0026 rejected a
production Agent Engine boundary because no candidate demonstrated bounded event
delivery plus restart recovery from Alpine-owned state. The current product
direction nevertheless explicitly selects Pi as the first harness integration.
That requires a narrower adoption decision that does not misstate the bake-off
evidence.

## Decision

Alpine Desktop uses Tauri 2 with a React and TypeScript webview. The existing
Rust crate is linked into the Tauri host and remains authoritative for hardware,
Profiles, the Model Registry, Inference Sessions, experiments, evidence,
Qualification, Deployment Roles and recovery. The webview calls one typed
Desktop Interface and consumes typed events; it does not shell out to Alpine or
infer state from formatted terminal text.

Pi SDK/core 0.84.2 is adopted as the first **experimental Agent Runtime Adapter**,
not as a second control plane and not as a production-qualified Harness. Alpine
supplies the model descriptor, endpoint, policy, tools, credentials and durable
task state. Pi supplies the in-memory agent loop, tool lifecycle, streaming,
steering and follow-up behavior. Pi-owned project, session, configuration and
credential stores are not application authority. The adapter is version and
integrity pinned to the source already reviewed by the agent-engine bake-off.

This decision partially supersedes ADR 0026's prohibition on absorbing a
candidate dependency, only for the explicitly experimental desktop adapter. ADR
0026's evidence findings and production adoption gates remain in force. The UI
must call the path experimental until Alpine-owned typed state import/export,
bounded event delivery, cancellation settlement and worker restart recovery are
demonstrated under the existing bake-off contract.

Hugging Face is the first Model Catalog Adapter. Pre-download Model Assessments
are estimates derived from live hardware and exact artifact metadata; they are
visually and structurally distinct from Experiment and Qualification evidence.
A selected default model is a machine-local new-task preference. It is not a
Profile, Promotion or Deployment Role, and changing it never mutates the daily
default Inference Session.

Before Pi receives a launch configuration, the host resolves the configured
Alpine Profile, proves that its exact model filename matches the selected desktop
default, and asks Alpine to start or reuse that Profile's llama.cpp Session. A
mismatch fails closed instead of sending Pi a model identifier that differs from
the model actually served by the endpoint.

Hugging Face downloads publish only after the expected byte count and, when the
Hub exposes an LFS object identifier, SHA-256 digest match. Interrupted transfers
remain as resumable partial files and an active transfer can be cancelled between
bounded chunks. The application never treats a partial artifact as installed.

The desktop Browser Surface starts with localhost and generated artifact review
inside the contextual side panel. External or authenticated browsing must use a
separately declared browser authority and must not inherit credentials merely
because a URL was displayed.

The tested seams are the Desktop Interface, the Agent Runtime Adapter and the
rendered primary workflow. The application records cold start, command duration,
time to first event, stream duration, dropped events and renderer memory without
recording raw prompts, secrets or private file contents.

The first Analysis surface is a bounded exact-output runtime diagnostic. It is
explicitly labelled measured diagnostic rather than Experiment or Qualification;
full multi-Profile evaluation continues to use Alpine's existing evidence engine.

## Alternatives considered

### Fork Pi into the main application

Rejected for the first integration. It would create a large maintenance and
provenance obligation before the public SDK seam has been exhausted.

### Use Pi process/RPC as the permanent boundary

Rejected. It brings a larger dependency closure and candidate-owned session
behavior while the bake-off found no acceptable exact policy/recovery path.

### Keep OpenCode as the desktop runtime

Rejected for this product slice because the user explicitly selected Pi and the
existing OpenCode process owns too much user-visible configuration and session
behavior for the intended seamless application.

### Build an Electron or browser-only application

Rejected. Electron adds a permanent Node/Chromium host the product does not need,
while a browser-only application cannot own local lifecycle, files and hardware
with the required authority. Tauri keeps the trusted host in Rust and the UI
replaceable.

## Consequences

The first runnable desktop milestone can reuse Alpine's live hardware capture and
typed control-plane modules while keeping the UI independently testable in an
ordinary browser. It adds a focused JavaScript dependency surface for React,
Tauri and the pinned Pi packages, plus a second Rust manifest for the native
shell.

The Tauri shell pins Rust 1.88 independently while the established control-plane
crate remains on Rust 1.85. Current Tauri transitive dependencies require 1.88;
isolating the toolchain avoids turning the desktop scaffold into an unrelated
control-plane compiler migration.

Desktop icon binaries are materialized from text-encoded sources by the shell's
build script. This keeps the public repository source-only while still producing
the PNG and ICO assets required by Tauri packaging.

The initial Pi path remains an experimental product capability even when its UI
works. Durable task recovery, full multi-Profile analysis, packaging evidence and
live capability qualification stay open gates. No model is downloaded during
tests, no Profile is promoted by the desktop app, and Stable rollback is
preserved.
