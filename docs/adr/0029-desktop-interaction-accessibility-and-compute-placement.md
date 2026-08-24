# ADR 0029: Desktop interaction state, accessibility and compute placement remain bounded

## Status

Accepted — 2026-08-24

## Context

ADRs 0027 and 0028 establish the Tauri/Pi integration, the Rust authority boundary,
durable Tasks and the project-scoped tool model. The first complete desktop
workflow exposed every capability as a separate destination and left technical
runtime language in primary UI. That increased cognitive load, moved the composer
between empty and running states, and implied browser and attachment capability
beyond what the first adapter can safely provide.

The product direction asks for Codex-like information architecture, independently
collapsible panels, one model lifecycle, contextual browsing, accessible motion,
and deliberate CPU/GPU/iGPU handling. Public product documentation establishes
those workflow patterns but is not a pixel specification or permission to reuse
OpenAI assets. WCAG 2.2 is the conformance baseline; W3C advises against requiring
every page to satisfy every AAA criterion.

## Decision

Alpine Desktop uses three primary surfaces: a project and Task rail, a central
Task transcript with a fixed-bottom composer, and a Context Panel containing
System, Files, Changes, Terminal and Browser. The project rail and Context Panel
toggle independently through labeled controls and keyboard shortcuts. Their open
state is a Desktop UI Preference stored in webview-local storage, not a durable
Task fact or authority grant.

The left and right splits are directly resizable with pointer or keyboard. Each
divider exposes separator semantics, a current value and bounded minimum and
maximum widths; widths persist as disposable local preferences. Browser mode
enforces a larger Context Panel minimum so native child-WebView geometry remains
usable. Repeated Performance or Browser commands toggle the active panel closed,
and Enter submits the composer while Shift+Enter inserts a line break.
At compact window widths, overlay panes begin closed so the Task canvas remains
usable; the operator can reveal either pane explicitly from the top bar.

Desktop interaction follows the platform-applicable Apple HIG contract recorded
in the design QA evidence. The embedded Browser exposes Back, Forward and Reload
beside its address field. Only one measured analysis may run at a time; its
unknown-duration work uses an accessible indeterminate progress state with a
plain-language description, while downloads retain exact determinate progress
and safe cancellation. Failed local-model runs keep the trusted raw failure in
Task history but present an actionable renderer message with Retry and Settings
recovery. The standard settings command is Command-comma on macOS and
Control-comma on Windows. These commands supplement visible controls rather than
replace them.

The selected visual system is light-first: warm cream and blush materials,
coral-to-rose primary chrome and deep plum text derived from the operator's
reference palette. The project rail, Context Panel, Browser chrome, Model
Library search/list structure, Today/Earlier Task grouping and generated Alpine
mountain asset form one composite direction. Settings remains inside the same
workspace shell with a distinct harmonious surface instead of appearing as a
separate dark application. Search and filter controls follow the Apple HIG
interaction guidance captured in the local design-research record and the
repository UX research note. The runtime mountain raster is the only shipped
binary design asset in this slice; its path, size ceiling, provenance and
SHA-256 are pinned in `config/public-binary-assets.json`, while non-product
reference captures remain local-only research material.

Models and downloads are one Model Library lifecycle. It shows installed and
verified artifacts first, provides the new-Task default selector, imports GGUF
artifacts and searches Hugging Face. Search and assessment requests carry
generation guards so a slower stale response cannot overwrite the latest choice.

The composer always occupies its own layout row. Streaming updates are coalesced
to animation frames, event persistence is ordered, one run lock prevents duplicate
launch, and cancellation settles the visible Task without moving the composer.
Run and persistence failures remain visible in an alert beside the composer even
after the durable user message has appeared in the transcript.
Its `+` menu contains request context only; project switching never appears there.
The current local model descriptor is text-only, so image and PDF actions remain
disabled with a visible reason rather than silently dropping unsupported input.
Skills and plugins require a future Alpine-owned capability registry before they
can be attached.

The supported accessibility target is WCAG 2.2 AA, with useful AAA outcomes such
as larger primary targets adopted where density permits. Controls use programmatic
names, visible focus, non-color status labels, at least 24 by 24 CSS-pixel targets,
keyboard access and reduced-motion fallbacks. Motion is limited to short
compositor-friendly feedback. Screenshots and `60fps.design` references are design
inputs; browser/WebView traces and long-task counts are performance evidence.

First launch reports OS, architecture, CPU topology, memory and every CUDA compute
device returned by Alpine's existing trusted hardware capture. The Compute Device
Summary is not a placement promise. Windows remains responsible for WebView and
interface graphics scheduling. llama.cpp model work prefers a detected discrete
compute device only after capacity checks and measurement. Alpine does not silently
combine heterogeneous GPUs or claim that assigning UI work to an iGPU improves
performance. iGPU and multi-GPU placement remain explicit experimental benchmark
candidates until the runtime and evidence contracts support them.

The initial localhost-only Preview boundary is superseded by ADR 0030. External
pages now open in native child WebViews with an Alpine-owned profile, per-host
consent, popup interception and explicit clear-data controls. The Browser remains
a human surface and does not expand Pi or Desktop Interface authority.

## Alternatives considered

### Mirror Codex pixels and private behavior

Rejected. Public sources establish workflow and information architecture, not a
complete proprietary visual specification. Alpine keeps its own identity and
matches only observable, useful interaction patterns.

### Promise automatic iGPU/dGPU division

Rejected. Windows graphics policy and heterogeneous llama.cpp placement do not
provide a portable guarantee. Presenting such a switch before measurement would
turn adapter presence into a false performance claim.

### Enable attachments and ignore unsupported content

Rejected. Pi documents that images sent to a text-only model can be ignored.
Alpine must expose the missing capability rather than imply that the model saw an
attachment.

## Consequences

The main workflow is calmer and more consistent, and the interaction contract can
be tested without granting new authority. Webview-local panel preferences are
recoverable but intentionally disposable. Hardware reporting is richer while
remaining conservative about placement. Full multimodal attachments, a verified
skills/plugins registry, measured iGPU hosts
and heterogeneous multi-GPU experiments remain separate evidence-backed slices.
