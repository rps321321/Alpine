# ADR 0030: Browser Surface uses isolated native child webviews and explicit host consent

## Status

Accepted — 2026-08-24

## Context

ADR 0029 limited the first Preview to loopback pages until Alpine had an isolated
browser profile and explicit data controls. The product goal now requires the
shared Browser workflow visible in current Codex Desktop: a contextual panel,
tabs, address navigation, downloads, sign-in state separate from the user's
regular browser, and a clear-data control. An iframe cannot provide that browser
surface reliably and must not inherit Alpine's application IPC authority.

Tauri 2.11 exposes child `Webview` construction, profile data directories,
navigation policy callbacks, page/title events, new-window interception,
download interception and browsing-data deletion behind its `unstable` feature.
That API is sufficient for a bounded native Browser Surface, but its unstable
status is a version-pinning and native-smoke-test obligation.

## Decision

Alpine creates one native child WebView per visible Browser tab and positions it
over the React-owned viewport. Browser tabs share one Alpine-owned
`browser-profile` directory under application data. They never use the user's
regular browser profile. React owns the tab strip, address bar, permission prompt,
loading state and viewport geometry; Rust owns URL parsing, host consent, child
WebView lifecycle, profile storage, new-window policy and download destination.

Loopback HTTP and HTTPS pages open without a prompt. Every new external host uses
HTTPS by default and requires an explicit **Allow once** decision scoped to the
originating Browser tab. Alpine rejects active-content schemes, URLs containing
credentials, malformed tab identifiers and invalid geometry before creating or
navigating a view. A page-requested popup becomes a new Alpine-controlled tab and
runs through the same host policy; the native popup is denied. Closing a tab
destroys its child WebView and its ephemeral host decisions.

The Browser profile may retain cookies and site state across application launches
until the user invokes **Clear browsing data** in Settings. That command clears
WebView browsing data and removes the profile directory when no child view is
active. Browser downloads are initiated by a page action, use sanitized unique
filenames in the operating-system Downloads folder, and publish typed start,
completion or failure events to the UI.

The Browser Surface is a human-visible application surface, not an Agent Runtime
tool. Navigating a page does not grant Pi access to its DOM, cookies, credentials,
downloads or network authority. External child views do not match Alpine's `main`
window capability grant and therefore cannot invoke the Desktop Interface. Alpine
does not save passwords itself and does not transmit browser history through local
performance metrics.

Settings schema 3 adds `browser_allowed_hosts` as a validated host-only field so a
future persistent permission UI can be added without storing full browsing URLs.
The current UI grants only per-tab, in-memory consent.

## Alternatives considered

### Keep the loopback iframe

Rejected. It cannot match the requested browser behavior, sign-in/profile
isolation, controlled new windows, data clearing or native page lifecycle.

### Reuse the system browser profile

Rejected. Ambient personal cookies and credentials would cross a product and
authority boundary the user did not grant, and Alpine could not provide truthful
profile clearing semantics.

### Give external pages Alpine IPC access

Rejected. The Browser is untrusted content. Application commands remain available
only to the `main` Alpine webview and consequential Task tools still require their
existing exact approvals.

## Consequences

Alpine now provides a shared native browser with tabs, local and external pages,
separate sign-in state, downloads, per-host consent and clear-data controls. The
React preview keeps a sandboxed iframe adapter for deterministic browser-based UI
tests, while packaged and development Tauri builds use native child WebViews.

The Tauri `unstable` feature is now a deliberate dependency. Upgrades must re-run
Rust policy tests, rendered browser tests, a native child-view smoke test, bundle
verification and the canonical repository verifier. History/download management,
persistent site permissions, annotations, screenshots, OAuth broker handoff and
agent-controlled browsing remain separate slices; none is implied by this ADR.
