# Alpine Desktop security review — 2026-08-24

## Scope and trust boundaries

This review covers the Tauri Desktop Interface, embedded Pi adapter, Selected
Project tools, Model Registry/default selection, native Browser surface and the
proposed Graph Context integration. It is a source and test review, not a claim
of OS sandboxing or adversarial isolation.

The renderer is untrusted for filesystem and process authority. Rust owns
durable state, exact model identity, project-root resolution, approval state and
native browser policy. Pi receives a local endpoint credential in memory and can
request only the five Alpine-owned project tools exposed by the adapter.

## Threat review

| Threat | Current control | Residual risk |
| --- | --- | --- |
| Tool misuse or prompt-driven code execution | List/read/search stay under the canonical Selected Project. Exact edits and PowerShell commands create durable approval proposals; the approved proposal is claimed once before execution. Approval decision and Task Event are committed in one SQLite transaction. | An approved command executes as the current Windows user. Approval is a consent boundary, not an OS sandbox. |
| Path traversal and link escape | Rust canonicalizes the project root, rejects traversal, checks link targets and bounds reads, searches and output. | A command the operator explicitly approves can use the user's ambient Windows authority outside the project; the UI states this plainly. |
| Approval replay or proposal substitution | Approval rows bind Task, tool call, operation and exact typed proposal. Claim is one-shot and settlement is durable. | Renderer polling remains an implementation detail of the experimental Pi adapter; notification-based delivery is deferred. |
| Model substitution | A default requires the exact Registry ID, filename, SHA-256 and immutable Hugging Face revision, or the full digest-derived import identity. | Existing legacy settings may need the operator to reselect a registered artifact before they satisfy the stronger identity tuple. |
| Credential exposure | The Tauri launch command reads the local key only at launch; the live Pi smoke accepts it from an environment variable and prints only adapter, model and visible character count. The disposable key copy was removed after the run. | Pi and the local provider run in the same user session and can observe their own process memory. |
| Untrusted web content | URL policy rejects active-content and credential-bearing URLs, external hosts require consent, popups are intercepted, and native child WebViews use Alpine-owned state. Browser tabs and geometry remain renderer presentation state. | The Browser is a human surface, not an agent web tool. Content can still socially engineer the operator; it grants no Pi tool authority. |
| Graph/context subprocess abuse | Graphify is reported unavailable. ADR 0031 requires a pinned argv-only, read-only, bounded, cancellable adapter and forbids installers, hooks and ambient config mutation. | No Graphify subprocess is shipped; context graph benefits remain unavailable until the adapter is implemented and reviewed. |
| Misleading capability or isolation claims | Settings exposes the tested Pi capabilities and explicit gaps. Safety states that shell execution uses the current Windows user and is not sandboxed. | Upstream Pi capabilities can change; the manifest must be updated only with adapter tests, not documentation inference. |

## Verification evidence

- `apps/desktop/src-tauri/tests/workspace.rs` covers project containment, exact
  approved edit and approved shell execution.
- `apps/desktop/src-tauri/tests/desktop_store.rs` covers canonical project roots,
  exact one-shot approval claims, atomic decision events, task recovery and exact
  model provenance.
- `apps/desktop/src-tauri/src/browser.rs` unit tests cover external-host consent,
  loopback navigation and rejected active content/credentials.
- `apps/desktop/src/task-execution.test.ts` covers streaming persistence,
  cancellation during launch and history restoration/failure settlement.
- `apps/desktop/src/harness/capabilities.test.ts` prevents the UI from presenting
  unverified Pi parity.
- `apps/desktop/src/App.test.tsx` covers the Safety disclosure, capability gaps,
  approval-facing interaction states, compact layout and accessibility.
- The live smoke started a disposable schema-5 Session view, streamed exact
  `ALPINE_OK` through Pi 0.84.2 and the installed local model, stopped the exact
  verified process, proved port 8100 free and rechecked the normal config/key/URL
  hashes.

## Decision

The implemented Desktop slice is acceptable for local development and operator-
approved project work. It must not be marketed as an isolation boundary. Attack-
lab execution, unattended consequential tools, renderer-owned credentials and
ambient Graphify installation remain out of scope until separate architecture
and verification contracts exist.
