# Alpine desktop UX research

Research date: 2026-08-23
Scope: desktop information architecture, Codex-inspired interaction patterns, accessibility, motion performance, embedded browsing, and local compute placement.

## Result

Alpine should reproduce Codex's information architecture and task behavior, not attempt an unverified pixel clone. The strongest documented pattern is a stable three-surface workspace: a project/task navigator on the left, a streaming task in the center with a fixed-bottom composer, and independently toggled contextual tools such as Browser, Files, Review, and Terminal. Models should be discovered dynamically, with a compact default control and advanced runtime/placement choices behind disclosure.

The accessibility target should be **WCAG 2.2 AA**. Useful AAA criteria can be adopted selectively, but W3C explicitly advises against requiring every page to satisfy all AAA criteria. Motion should be judged with frame evidence, not the `60fps.design` aesthetic gallery: target smooth frames, animate compositor-friendly properties, and honor reduced-motion preferences.

For hardware placement, Alpine can enumerate adapters and benchmark supported llama.cpp placements, but it should not promise that assigning the UI to an iGPU and inference to a dGPU will work or improve performance on every machine. Windows ultimately applies graphics policy, and current llama.cpp behavior and multi-GPU guidance do not justify silently combining heterogeneous GPUs.

## Evidence boundary

- **Observed/documented** means the behavior or capability is stated in an official OpenAI, W3C, Microsoft, Chrome, or llama.cpp source.
- **Inference/recommendation** means a proposed Alpine design derived from that evidence. Exact Codex dimensions, icons, easing curves, and undocumented menus are not claimed as facts.
- Public product documentation can establish behavior and information architecture, but it is not a complete design specification. Alpine should not use OpenAI names, marks, or proprietary assets as its own visual identity.

## 1. Codex desktop patterns

### Projects and the left rail

**Observed/documented.** Codex Projects unify ChatGPT projects and local folder-backed projects. A project can contain distinct chats for distinct outcomes; local projects can attach folders and designate a primary folder. Projects and chats can be pinned, searched, renamed, and archived. Creating a new or standalone chat is distinct from choosing a project. [OpenAI: Projects](https://learn.chatgpt.com/docs/projects)

**Recommendation for Alpine.** Use the left rail for:

1. New task.
2. Project picker and project-scoped task history.
3. Search, pin, rename, and archive actions.
4. A bottom-pinned Settings entry.

Keep project creation and switching out of the composer attachment menu. When collapsed, retain a narrow rail with labeled tooltips and the same keyboard-accessible destinations.

### Independently collapsible surfaces

**Observed/documented.** Codex exposes separate Windows commands for the sidebar (`Ctrl+B`), bottom panel (`Ctrl+J`), file tree (`Ctrl+Shift+E`), review panel (`Ctrl+Alt+B`), browser panel (`Ctrl+Shift+B`), browser tab (`Ctrl+T`), and project picker (`Ctrl+Alt+Shift+O`). This confirms independent surfaces, but does not establish their exact size, icons, or animation. [OpenAI: Commands](https://learn.chatgpt.com/docs/reference/commands)

Microsoft's `NavigationView` similarly defines expanded, compact, and minimal left-navigation modes, including automatic adaptation at width thresholds. Those WinUI thresholds are useful precedent, not values Tauri must copy. [Microsoft: NavigationView](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/navigationview)

**Recommendation for Alpine.** Give the left navigator, bottom diagnostics, and right contextual panel independent persisted open/closed state. The right panel should switch among Browser, Files, Review, and Terminal without creating permanent top-level application routes.

### Composer and attachments

**Observed/documented.** Codex supports attaching, pasting, and dragging images into a chat; on desktop, holding Shift while dragging is documented. Skills are invoked with `$` or slash commands, while plugins have a dedicated Plugins surface. Official documentation does not define every item in the visual `+` menu. [OpenAI: Image inputs](https://learn.chatgpt.com/docs/image-inputs), [OpenAI: Slash commands](https://learn.chatgpt.com/docs/reference/slash-commands), [OpenAI: Plugins](https://learn.chatgpt.com/docs/plugins)

**Recommendation for Alpine.** Make `+` a request-context menu for files, images, folders, and installed capabilities. Attachments should appear inside the composer, grow upward to a capped height, and never displace the message viewport unpredictably. Keep the composer in a stable bottom row while messages scroll independently.

### Model controls

**Observed/documented.** Codex places model and reasoning controls beneath the composer. Current guidance presents a compact Power control from Faster to Smarter, with Advanced controls for explicit model, effort, and speed. Codex App Server clients are expected to call `model/list` before rendering a selector; the response supplies picker visibility, display names, default and supported reasoning effort, modalities, default/hidden flags, and personality support. [OpenAI: Models](https://learn.chatgpt.com/docs/models), [OpenAI: App Server](https://learn.chatgpt.com/docs/app-server)

**Recommendation for Alpine.** Populate the selector from Alpine's local Model Registry and runtime capability probe rather than a hard-coded list. Show installed and compatible models first. Disable unavailable or hardware-incompatible choices with a plain-language reason. Keep model choice separate from the harness/runtime choice: Pi can be the single initial harness, visibly marked experimental, while llama.cpp is the initial local inference runtime. Put GPU layers, split mode, context size, and device placement under Advanced.

### Browser integration

**Observed/documented.** Codex provides a browser inside a chat as a shared view of a website or local app. It can open from a toolbar, URL, manual navigation, or keyboard command, and supports navigation, an address bar, browse/comment modes, annotations, screenshots, and verification. Codex uses browser data separate from the user's normal browser and provides settings for history, downloads, and permissions. [OpenAI: Browser](https://learn.chatgpt.com/docs/browser)

WebView2 supports app-specific user data folders and multiple profiles with separated cookies, preferences, and cache. Microsoft cautions that many independent data folders increase memory, CPU, and disk cost; its WebView2 guidance also recommends using the system browser or broker for OAuth rather than embedded authentication. [Microsoft: WebView2 user data folders](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/user-data-folder), [Microsoft: Multiple profiles](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/multi-profile-support), [Microsoft: WebView2 for Windows apps](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/webview2)

**Recommendation for Alpine.** Implement Browser as an actual webview panel or tab, not an iframe. Give it one dedicated Alpine profile, separate from the user's everyday browser, with clear controls to clear history, downloads, site data, and permissions. Open authentication flows in the system browser. Treat annotations and screenshots as explicit task attachments with origin metadata.

### Streaming task UX

**Observed/documented.** Codex App Server models execution as thread → turn → item. It streams `item/started`, ordered message/reasoning/plan/tool deltas, and `turn/diff/updated`; `item/completed` supplies the authoritative final item. Turns finish as completed, interrupted, or failed. `turn/steer` can add user input to an in-flight turn. [OpenAI: App Server](https://learn.chatgpt.com/docs/app-server)

**Recommendation for Alpine.** Reconcile the stream by stable `threadId`, `turnId`, and `itemId`:

- Start one item shell on `item/started` and patch deltas into it; never append a new bubble per delta.
- Replace the provisional item with the authoritative completed item.
- Reject stale events using run/generation IDs and sequence ordering.
- Apply a terminal state once and preserve partial output for interrupted or failed runs.
- Keep composer geometry constant while its action changes among Send, Steer, Queue, and Stop.
- Announce important state changes through an accessible status region without moving focus.

## 2. Recommended Alpine information architecture

| Surface | Primary content | Persistence |
| --- | --- | --- |
| Left rail | New task, project picker, task history, search/pin/archive, Settings | Width and collapsed state |
| Center | Streaming task transcript, tool calls, diffs, fixed composer | Scroll position per task |
| Right context | Browser, Files, Review, Terminal | Active tool, width, open state |
| Bottom panel | Logs, performance evidence, downloads, diagnostics | Height and open state |
| Models destination | Search Hugging Face, compatibility, install/download progress, installed models, profiles | Filters and selected profile |

Merge model discovery and downloads into one **Models** lifecycle rather than separate destinations. Use user-facing labels such as “Fits in GPU,” “Uses GPU and system memory,” and “Run a quick check”; place tensor split, offload layers, backend, and raw benchmark evidence behind Advanced.

Settings should hold infrequent preferences, not primary workflow commands. Microsoft recommends sensible defaults, related groups, immediate application of changes, explanations for disabled settings, and shallow advanced disclosure. [Microsoft: App settings guidance](https://learn.microsoft.com/en-us/windows/apps/design/app-settings/guidelines-for-app-settings)

Suggested top-level settings: General, Models & Runtime, Harness, Browser, and Privacy & Diagnostics.

## 3. Accessibility target

**Observed/documented.** WCAG 2.2 is the current W3C Recommendation and extends WCAG 2.1 and 2.0. Level AA conformance requires satisfying all Level A and AA criteria. W3C advises against requiring Level AAA for entire sites because some content cannot satisfy every AAA criterion. [W3C: WCAG 2.2](https://www.w3.org/TR/WCAG22/)

**Alpine target.** Require WCAG 2.2 AA for every supported workflow, then selectively adopt valuable AAA outcomes without claiming global AAA. At minimum:

- Text contrast of 4.5:1 for normal text and 3:1 for large text; 3:1 for meaningful UI components and states.
- Visible keyboard focus that is not obscured by sticky panels or the composer.
- A 24 × 24 CSS-pixel minimum target size or the WCAG spacing exception; aim for the AAA 44 × 44 size on primary controls where density allows.
- Full keyboard access to rails, panels, menus, model controls, attachments, and streaming actions.
- Reflow without loss at 320 CSS pixels, while recognizing that Alpine is desktop-first.
- Programmatically determinable status messages for downloads, model loads, tool activity, errors, and completed runs, without stealing focus.
- Labels that do not rely on color or icon shape alone; useful names for icon-only controls.
- Reduced-motion behavior driven by the operating-system preference. W3C documents `prefers-reduced-motion` as a technique for suppressing nonessential motion. [W3C: Technique C39](https://www.w3.org/WAI/WCAG21/Techniques/css/C39.html)

## 4. Motion and performance

**Observed/documented.** [`60fps.design`](https://60fps.design/) is a real gallery of “delightful details” and motion inspiration. Its [Why page](https://60fps.design/why) explains the editorial goal; it is not a platform specification, accessibility standard, or proof that a particular duration is optimal.

Chrome's primary guidance explains that a 60 Hz display allows roughly 16.7 ms per frame and recommends restricting animation to `transform` and `opacity` where possible. It also warns that excessive promoted layers consume memory. DevTools provides live FPS, dropped-frame, GPU-raster, memory, and trace evidence. [web.dev: Animations and performance](https://web.dev/articles/animations-overview), [web.dev: Animation performance guide](https://web.dev/articles/animations-guide), [Chrome DevTools: Rendering performance](https://developer.chrome.com/docs/devtools/rendering/performance), [Chrome DevTools: Performance](https://developer.chrome.com/docs/devtools/performance)

**Recommendation for Alpine.** Use `60fps.design` only as visual inspiration. For implementation:

- Prefer short, functional transitions; a 120–180 ms range is an Alpine design hypothesis to validate, not a sourced Codex value.
- Animate transform/opacity, not grid widths, layout positions, or composer height frame by frame.
- Disable or simplify nonessential motion under reduced-motion preferences.
- Avoid ambient animation, bounce, and loading effects that continue after useful feedback has been delivered.
- Capture traces on representative integrated and discrete GPU systems. Gate releases on measured dropped frames and long tasks in the actual Tauri/WebView runtime, not screenshots or subjective review alone.

## 5. CPU, dGPU, and iGPU task assignment

**Observed/documented.** DXCore can enumerate compute adapters and request lists ordered for a workload according to operating-system policy. Windows exposes per-app preferences such as Let Windows decide, Power saving, and High performance; applying a changed preference can require restarting the app. These mechanisms are policy inputs, not guarantees that a particular GPU executes every subtask. [Microsoft: DXCore adapter enumeration](https://learn.microsoft.com/en-us/windows/win32/dxcore/dxcore-enum-adapters), [Microsoft: Workload-aware adapter lists](https://learn.microsoft.com/en-us/windows/win32/dxcore/dxcore_interface/nf-dxcore_interface-idxcoreadapterfactory1-createadapterlistbyworkload), [Microsoft: Per-app graphics preference](https://support.microsoft.com/en-au/windows/optimizations-for-windowed-games-in-windows-11-3f006843-2c7e-4ed0-9a5e-f9389e535952)

llama.cpp exposes device selection, GPU-layer offload, split mode, tensor split, main GPU, and fit controls. Its official multi-GPU guidance describes layer split as the default and tensor split as experimental, with results dependent on hardware and interconnect. Current source behavior prefers discrete GPUs and adds integrated GPUs only when no discrete GPU was found, so a default heterogeneous dGPU+iGPU split should not be assumed. [llama.cpp: Server options](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md), [llama.cpp: Multi-GPU](https://github.com/ggml-org/llama.cpp/blob/master/docs/multi-gpu.md), [llama.cpp: Device selection source](https://github.com/ggml-org/llama.cpp/blob/master/src/llama.cpp)

**Recommendation for Alpine.** Use a conservative, evidence-producing scheduler:

- **CPU:** system discovery, model catalog, network orchestration, download/hash/file I/O, task state, and harness control. Run expensive hashing and extraction outside the renderer with bounded concurrency.
- **dGPU:** preferred llama.cpp compute device after backend, memory, and model-fit checks.
- **UI/browser:** leave composition and webview scheduling to Windows/WebView2. Do not present an unsupported “force UI to iGPU” guarantee.
- **iGPU:** enumerate and show as an optional benchmark candidate. Do not silently combine it with a dGPU or claim that using every adapter is faster.
- **Multi-GPU:** start with the supported layer split and expose tensor split as experimental. Save a placement only after a repeatable host-specific benchmark shows a benefit.
- **Isolation:** run llama.cpp in a child process so crashes, memory pressure, cancellation, and restart do not destabilize the Tauri shell. Process separation may also let users apply a distinct Windows graphics preference, but Windows still decides execution.
- **Resource governor:** allow at most one model load/evaluation per selected device initially; bound downloads and hash workers; reserve memory for the shell; attach run IDs and cancellation tokens to every long operation.

Alpine should report what it measured—backend, exact model file, context, placement, memory use, load time, tokens/second, thermals when available, and trial variance—rather than translate adapter presence into a performance promise.

## 6. Implementation acceptance criteria

- Left rail, bottom panel, and right contextual panel toggle independently by mouse and keyboard, with state restored on relaunch.
- Project selection is separate from attachments; `+` adds task context only.
- Composer position does not jump when submitting, streaming, steering, attaching files, or showing an error.
- Streaming tests cover ordered deltas, duplicate/stale events, authoritative completion, interruption, failure, and steer/stop races.
- Model selector is generated from discovered registry/capability data and explains every disabled option.
- Models flow covers search, compatibility, download progress, integrity verification, cancellation, install, benchmark, select, and delete/error states.
- Browser uses an isolated Alpine profile, exposes clear-data controls, and sends OAuth to the system browser.
- Core workflows pass keyboard-only and screen-reader checks at WCAG 2.2 AA; streaming statuses do not steal focus.
- Reduced-motion mode eliminates nonessential transitions.
- Performance evidence includes browser/Tauri traces on representative iGPU-only and dGPU systems; no release claim relies on `60fps.design` or visual inspection alone.
- Placement recommendations are labeled measured, inferred, unsupported, or experimental, and never silently combine heterogeneous GPUs.
