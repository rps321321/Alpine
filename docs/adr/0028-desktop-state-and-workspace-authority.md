# ADR 0028: Alpine owns durable desktop state and project-scoped workspace authority

## Status

Accepted — 2026-08-23  
Amended — 2026-09-01

## Context

ADR 0027 establishes Pi as a replaceable Agent Runtime Adapter and leaves durable
task recovery, tool lifecycle and project-scoped coding work as Alpine-owned
responsibilities. A useful desktop coding application must survive restarts,
restore its task rail, show exact tool history and let the model inspect or
change a Selected Project. Neither Pi's in-memory transcript nor formatted
terminal output can become that authority.

The original desktop schema persisted Task Messages, generic Task Events and Tool
Approvals in separate tables. Each table allocated its own per-task sequence, so
a transcript row, lifecycle row, approval transition and tool result could not be
replayed as one causally ordered history. The host-owned supervisor introduced in
ADR 0027 removed renderer write authority, but split persistence still left a
recovery ambiguity at crash boundaries.

The desktop also needs a clear trust boundary. Read-only project inspection may
run inside the selected root, while writes and shell execution can create
consequential effects. Filesystem reachability is not operator consent.

## Decision

Alpine's authoritative durable Task history is a **versioned append-only Task
Journal** owned by the Tauri host. Every Task Journal record has one task-wide
monotonic sequence, an optional durable Execution ID, an event-contract version
and a tagged `TaskJournalEvent`. The event type is defined in Rust and a checked
generator emits the TypeScript discriminated union consumed by the renderer.
Unknown fields and unsupported event versions are rejected at the persistence
boundary.

The journal contains product facts rather than provider callbacks. Its typed
families include user prompt/direction acceptance, Execution queue/start/wait/
resume/cancel/finish facts, completed assistant messages, exact tool proposals,
approval decisions, tool claims/results/settlements and explicit restart
interruption. Provider-internal trace events remain adapter diagnostics; they do
not become durable product history merely because Pi emitted them.

Prompt acceptance and immutable Execution creation happen in one SQLite
transaction **before provider launch**. A crash after that commit therefore
recovers an accepted prompt and exact queued Execution, which startup then marks
interrupted; it cannot silently lose the user intent or invent that the provider
ran. Approval decision and waiting-Execution resumption are likewise committed
together before the exact worker continuation is awakened.

Tool effects use a typed proposal (`edit` or `shell`) bound to one Task,
Execution, tool call and approval. Claiming an approved effect appends
`tool-started` before the host effect executes. A successful or failed effect is
then recorded as a typed result plus settlement in one transaction. If Alpine
restarts after the claim but before settlement, recovery records the approval and
Execution as interrupted; it never fabricates a tool result or silently replays
the effect.

Task Messages, Execution rows and Tool Approval rows remain SQLite **projections**
for efficient UI/query access. They are rebuildable from the Task Journal and are
not independent sources of truth. The former `task_events` table is removed in
schema 5; there is no production dual-write path. Workspace Changes and Terminal
views consume typed tool-result journal records instead of decoding an arbitrary
`kind` plus untyped JSON payload.

Legacy schema-3/4 history is migrated into explicit `legacy-imported` journal
records. The migration preserves the source record identifier, source-local
sequence when available and recorded timestamp, but marks causal ordering as
`unverified`; Alpine does not manufacture an ordering guarantee the old schema
never possessed. Existing legacy projections remain usable, and projection
rebuild can deterministically reconstruct them from those provenance records.

Durable task facts are written only by the Tauri host's `TaskSupervisor`, journal
store and workspace services. The visible renderer has no arbitrary
message/event/status append commands. The isolated Agent Worker can propose exact
effects and report bounded, monotonically sequenced adapter messages, but
host-side webview identity checks, Execution identity checks and journal
transactions remain authoritative. Duplicate worker delivery is idempotent;
sequence gaps are rejected and fail the exact active Execution rather than being
silently reordered.

The same database owns the desktop Model Registry. A registered artifact records
its source, exact Hugging Face repository and immutable revision when applicable,
observed bytes, SHA-256 digest, canonical local path, origin URL and verification
time. Imported GGUF files are copied into managed storage without overwriting a
different artifact. Download completion and import both converge on this record;
a filename in the models directory alone is not equivalent to verified
provenance.

Every project root is canonicalized before persistence. All workspace commands
resolve their requested paths against that canonical root and reject traversal
or symlink escape. Read, list and search operations may execute without a Tool
Approval. Exact-text edits and shell commands require the exact typed Tool
Approval described above. Approved shell commands still execute as the current
Windows user; this boundary is policy and accountability, not an Attack-Lab
sandbox.

## Alternatives considered

### Keep Task Messages and Task Events as independent authoritative streams

Rejected. Independent sequence allocators cannot express one causal replay order
and make crash recovery depend on correlating rows after the fact.

### Dual-write the old and new event stores

Rejected for the production cutover. The host supervisor is already the single
writer, so schema 5 can migrate old history once and then make the journal the
only authoritative event store. Maintaining two authorities would create a new
equivalence problem without a migration benefit.

### Persist Pi sessions directly

Rejected. It would make a disposable adapter's schema and recovery behavior a
permanent product authority and would violate ADR 0026's provider-neutrality
boundary.

### Store desktop state in JSON files

Rejected. Ordered concurrent appends, schema migration, referential integrity,
atomic approval settlement and projection rebuilding are transactional database
concerns.

### Give Pi ambient filesystem and shell authority

Rejected. The Tauri webview must not acquire ambient Node authority, and a
generic execution environment would bypass project-root checks and Alpine-owned
approval evidence.

## Consequences

Crash recovery and UI projection now derive from one durable ordered history.
The journal is deliberately stricter than provider event payloads: changing the
contract requires an explicit versioned Rust change plus regenerated TypeScript.
This adds schema discipline but removes a much larger class of partial-history
and renderer/host disagreement bugs.

A failed authoritative journal write is an Execution failure, not a best-effort
logging failure. Once provider work is active, the supervisor stops/fails the
exact Execution rather than continuing with unrecorded facts. Consequential tool
calls are never automatically replayed after an interrupted settlement.

The model registry still proves local identity and provenance, not checkpoint
quality, trustworthiness or runtime Qualification. The Task Journal similarly
proves Alpine's durable product history; it does not turn provider-private trace
state into product truth.
