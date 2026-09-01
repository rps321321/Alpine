# ADR 0028: Alpine owns durable desktop state and project-scoped workspace authority

## Status

Accepted — 2026-08-23

## Context

ADR 0027 establishes Pi as a replaceable Agent Runtime Adapter and leaves durable
task recovery, tool lifecycle and project-scoped coding work as open gates. A
useful desktop coding application must survive restarts, restore its task rail,
show exact tool history and let the model inspect or change a Selected Project.
Neither Pi's in-memory transcript nor formatted terminal output can become that
authority.

The desktop also needs a clear trust boundary. Read-only project inspection may
run inside the selected root, while writes and shell execution can create
consequential effects. Filesystem reachability is not operator consent.

## Decision

Amended 2026-09-01: durable task facts are written only by the Tauri host's
`TaskSupervisor` and workspace services. The visible renderer has no arbitrary
message/event/status append commands. The isolated Agent Worker can propose
exact effects and report bounded adapter events, but host-side webview identity
checks, Execution identity checks and SQLite transitions remain authoritative.
Approval decisions are persisted by the host and delivered directly to the
specific waiting worker continuation; the database is not polled as a message
queue.

Alpine Desktop stores Desktop Project Records, Tasks, Task Messages and typed Task
Events in an app-local SQLite database owned by the Tauri host. The database uses
explicit schema migrations, foreign keys and ordered per-task sequences. Pi may
be reconstructed from Alpine-owned Messages, but its private state is never the
recovery source of truth.

The same database owns the desktop Model Registry. A registered artifact records
its source, exact Hugging Face repository and immutable revision when applicable,
observed bytes, SHA-256 digest, canonical local path, origin URL and verification
time. Imported GGUF files are copied into managed storage without overwriting a
different artifact. Download completion and import both converge on this record;
a filename in the models directory alone is not equivalent to verified
provenance. A Hugging Face search result cannot become the desktop default until
that exact repository, immutable revision and filename are present in the
registry, preventing a remote label from masquerading as a runnable local model.

Desktop Settings schema 4 binds every newly selected default to the exact Model
Registry identifier and SHA-256 digest as well as repository, immutable revision
and filename. Imported artifacts use `local/import/<full-sha256>` as their
machine-local Task identity. The trusted host validates the complete tuple
against the Registry; the renderer cannot promote an unregistered scan result,
and two artifacts that share a filename remain distinct.

Every project root is canonicalized before persistence. All workspace commands
resolve their requested paths against that canonical root and reject traversal
or symlink escape. Read, list and search operations may execute without a Tool
Approval. Exact-text edits and shell commands require a pending Tool Approval
whose decision is persisted before execution. Each approval is bound to one
Task, operation kind and structured proposal; it cannot authorize later work by
category alone.

A Tool Approval decision and its `approval.decided` Task Event are committed in
one SQLite transaction and returned as one typed decision. The renderer no
longer performs a second event append after deciding, so the approval row and
durable Task history cannot diverge between those writes.

The Desktop Interface exposes durable project/task operations and normalized
tool events. The Pi Adapter maps Pi lifecycle events into that interface and
uses small Alpine-owned Agent Tools. It does not receive a general filesystem or
process handle. Cancellation settles the Task visibly and leaves the local
Inference Session governed by Alpine's existing transactional lifecycle.

## Alternatives considered

### Persist Pi sessions directly

Rejected. It would make a disposable candidate's schema and recovery behavior a
permanent product authority and would not satisfy ADR 0026's recovery evidence
gap.

### Store the desktop state in JSON files

Rejected. Ordered concurrent event appends, schema migration, referential
integrity and atomic approval settlement are deeper than a collection of mutable
documents.

### Give Pi the built-in Node filesystem and shell tools

Rejected. The Tauri webview should not acquire ambient Node authority, and a
generic execution environment would bypass project-root checks and Alpine-owned
approval evidence.

## Consequences

The native host gains a small SQLite module and project-scoped workspace module.
The webview remains independently previewable through a typed Desktop Interface.
Task content remains machine-local and is excluded from diagnostics by default.

Crash recovery can reconstruct task history and identify interrupted runs, but
replaying an interrupted consequential tool call is prohibited. A pending or
running Tool Approval must settle as interrupted and be proposed again after a
restart. This favors visible recovery over silent duplicate effects.

This decision does not broaden the Harness Policy Boundary into an Attack-Lab
Isolation Boundary. Approved shell commands still run as the Windows user inside
the Selected Project and must be presented honestly as consequential host work.
The model registry similarly proves local identity and provenance, not checkpoint
quality, trustworthiness or runtime Qualification.
