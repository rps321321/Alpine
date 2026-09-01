# Durable Executions

A **Task** is the long-lived conversation and project context. An **Execution** is one immutable attempt to advance that Task.

## Identity

Every new Execution stores an immutable specification containing:

- the verified Model Registry identifier, repository/revision, filename, and SHA-256;
- Session Config and Profile SHA-256 identities;
- runtime name and runtime-binary SHA-256;
- versioned adapter and policy identities;
- context, output, and temperature settings.

The specification is created once with the Execution and has no update API. A retry creates a new Execution and a new specification instead of rewriting the previous attempt.

## Lifecycle

The persisted state machine is:

```text
queued -> preparing -> running -> completed
                         |  ^
                         v  |
                waiting-for-approval

queued/preparing/running/waiting-for-approval -> cancelling -> cancelled
active states -> failed | interrupted
```

Terminal states cannot be reopened. The store validates each transition and records start, finish, failure, and update timestamps against the `ExecutionId`.

Messages, events, approvals, tool outcomes, metrics, and completion state all carry the same `ExecutionId`. The Task list receives an explicit projection (`ready`, `active`, `done`, or `needs-attention`) plus explicit active/latest Execution IDs; callers do not infer the active attempt from array order.

## Restart and migration

On startup, each unfinished Execution becomes `interrupted` and retains a restart failure detail. Alpine never resumes effects merely because a process restarted.

Schema-3 task history is migrated deterministically to `legacy-execution-<task-id>` and `legacy-spec-<task-id>` when legacy activity can be associated with a Task. Known task-level model/profile fields are retained, while unavailable runtime, adapter, policy, and digest evidence is marked `legacyUnverified` rather than fabricated.

## Host-owned execution authority

The visible React renderer is a projection client. It can submit prompt,
cancellation, steering, follow-up and approval intents, but it cannot construct
the provider adapter, receive the runtime credential, append durable task facts
or transition Execution state. A Rust `TaskSupervisor` reserves local inference
capacity, creates the immutable Execution and persists every authoritative
outcome. Pi runs in an isolated `agent-worker` webview and reports bounded events
over a Tauri channel; worker-originated calls are checked against both webview
identity and the active Task/Execution pair.

The desktop architecture gate rejects reintroduction of renderer-side provider
imports, runtime credentials, row-level task mutation primitives, or direct
Execution transition APIs.
