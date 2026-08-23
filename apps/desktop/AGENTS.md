# Prototype Instructions

Run the local server yourself and open the preview in the browser available to this environment. Do not give the user server-start instructions when you can run it.

Before making substantial visual changes, use the Product Design plugin's `get-context` skill when the visual source is unclear or no longer matches the current goal. When the user gives durable prototype-specific design feedback, preferences, or decisions, record them in `AGENTS.md`.

When implementing from a selected generated mock, treat that image as the source of truth for layout, component anatomy, density, spacing, color, typography, visible content, and hierarchy.

Build app UI in `src/`. Keep `.openai/hosting.json`, `worker/index.js`, `scripts/prepare-sites-build.mjs`, and `tests/sites-worker.test.mjs` intact so the same local prototype can be handed to Sites. Before a Sites handoff, run `npm run build` and `npm run test:sites`; the build must leave `dist/client/index.html`, `dist/server/index.js`, and `dist/.openai/hosting.json`.

## Durable desktop design contract

- Use a restrained dark desktop interface with a subtle light-teal accent. Prefer hierarchy, whitespace, and disclosure over card-heavy or neon dashboard styling.
- Keep the task transcript and composer central. The composer occupies a stable bottom row and must not move when a run starts, streams, stops, errors, or queues a follow-up.
- Keep project selection in the left rail. The composer `+` menu is only for request context such as supported attachments, skills, and tools; unavailable capabilities must explain why they are unavailable.
- Merge model discovery, downloads, imports, installed state, and the new-task default into one Models lifecycle.
- Treat Browser/Preview, Files, Changes, and Terminal as contextual right-panel tools rather than permanent top-level destinations. The left and right rails collapse independently and restore their local UI preference.
- Use plain user-facing language. Keep Profile, Qualification, Deployment Role, Evidence Identity, tensor split, and backend details behind advanced or evidence-oriented surfaces when they are necessary.
- Target WCAG 2.2 AA across supported workflows and adopt useful AAA outcomes selectively. Preserve keyboard operation, visible non-obscured focus, status announcements, at least 24 by 24 CSS-pixel targets, sufficient contrast, and reduced-motion behavior.
- Use short transform/opacity motion only where it clarifies state. Validate frame behavior and long tasks in the real WebView runtime; `60fps.design` is inspiration, not evidence.
- Do not claim that an iGPU drives the interface, that heterogeneous GPUs improve inference, or that a model fits from adapter presence alone. Report detected compute devices and measured placements; let Windows schedule WebView composition.
