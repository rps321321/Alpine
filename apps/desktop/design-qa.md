# Desktop design QA

## Source and implementation

- Source: official Codex product frames inspected on [OpenAI's Codex app page](https://openai.com/index/introducing-the-codex-app/) and launch video on 2026-08-23. The frames establish observable task-composer, project, skills and contextual-workspace anatomy; they are not treated as a private pixel specification.
- Implementation: Alpine's rebuilt production preview at `http://127.0.0.1:4174/`, inspected in the in-app browser on 2026-08-24.
- The official composer/task frame and Alpine home screen were emitted together in one comparison input before this judgment. Model Library, model assessment, Settings, contextual Preview, collapsed rails and composer before/after-send states were also captured.
- Local QA artifacts are retained under the ignored `apps/desktop/.artifacts/design/` directory and are excluded from the proposed public source tree.

## Comparison and workflow checks

The implementation preserves the useful source hierarchy: project and task history in a narrow left rail, a quiet central task surface, one bottom-anchored rounded composer with compact runtime/model controls, and an independently toggled contextual panel. Alpine intentionally uses its own dark Windows-first identity and light-teal accent instead of OpenAI assets or marks.

Verified in the live production preview:

- Models and Downloads are one Model Library with installed/default, import, search, loading, compatibility, download and error states.
- Browser is no longer a top-level route. Local Preview, Files, Changes and Terminal remain beside the Task in the Context Panel.
- Project switching is separate from the composer. The `+` menu contains only request-context capabilities and explains why text-only Pi cannot yet accept images, PDFs or registry-backed skills.
- The left rail and Context Panel collapse independently and remain collapsed after reload; `Ctrl+B` and `Ctrl+Shift+B` mirror those actions.
- The composer occupies the same bottom row before and after a task starts. Streaming content scrolls independently, and the action changes to stop/steer without changing composer height.
- Settings use General, Runtime, Browser, Safety and Privacy disclosure. A model selected for inspection does not leak into unrelated Settings context.
- Hardware details show OS, architecture, CPU topology, memory, detected CUDA compute devices and driver state without promising an unsupported iGPU/dGPU split.
- All inspected controls have programmatic labels and visible focus styles. Primary text, muted text, teal accents and accent buttons exceed WCAG 2.2 AA text contrast; meaningful control borders are at least 3:1 against the background. Primary controls meet or exceed the 24 by 24 CSS-pixel target, and reduced-motion mode suppresses nonessential motion.
- Browser diagnostics contained no warnings or errors; the retained debug entries were Vite connection messages from the local preview. Local evidence reported a 16 ms renderer bootstrap, zero long tasks, a 200 KB transferred client surface, and approximately 10 MB renderer heap in this preview run. Pi request timing in the browser preview reflects the deliberately unavailable local test endpoint and is not a model-performance result.

## Intentional boundaries

- The first Pi model descriptor is text-only. Image/PDF actions remain visibly unavailable rather than being silently ignored.
- Preview is localhost-only and iframe-backed in the browser-preview build. An unrestricted embedded browser still requires an isolated WebView2 profile, clear-data controls and system-browser OAuth.
- WCAG 2.2 AA is the supported conformance target. Useful AAA outcomes are adopted selectively; no whole-application AAA claim is made.
- `60fps.design` informed restraint and microinteraction taste only. The release evidence remains WebView frame/long-task data and reduced-motion behavior.

## Result

passed
