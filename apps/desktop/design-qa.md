# Alpine Desktop design QA — 2026-08-24

## Visual sources

The comparison used these local-only, non-product reference captures:

- `apps/desktop/design-research/2026-08-24/assets/user-coral-palette.png`
- `apps/desktop/design-research/2026-08-24/option-1.png`
- `apps/desktop/design-research/2026-08-24/option-2.png`
- `apps/desktop/design-research/2026-08-24/option-3.png`
- Apple Human Interface Guidelines root and the sections recorded in
  `docs/research/alpine-desktop-ux-research.md`

The final combined comparison input is retained locally at
`apps/desktop/.artifacts/goal-audit-2026-08-24-v2/reference-vs-implementation-final.png`.
The source captures remain excluded from the proposed public tree; only the
runtime mountain raster is shipped, under the digest and size contract in
`config/public-binary-assets.json`.

## Viewports and states

- Wide: 1511 by 1272 CSS pixels at device pixel ratio 1 in the selected in-app
  browser. Checked Home, Model Library, Browser, Settings, an active Task row,
  and a failed Task alert.
- Narrow: the built app was loaded in 700 by 900 and 480 by 850 CSS-pixel
  frames in the same browser. Both overlay panes began closed; the heading,
  machine summary, model control and bottom composer remained readable without
  horizontal clipping.
- Density: Windows Segoe UI Variable at normal display density. Long model names
  truncate inside bounded selectors instead of resizing the shell.
- Motion: the implementation includes a reduced-motion override; the inspected
  machine did not request reduced motion. Pointer/keyboard divider behavior and
  stable composer geometry are covered by interaction tests.

## Comparison result

- Option 1: the implementation retains the persistent left project rail, thin
  resizable split, Task canvas and separately resizable Context Panel. Settings
  is integrated into the same shell instead of appearing as a detached dark
  surface.
- Option 2: the Model Library uses one selected-default control, a verified local
  list, GGUF import, immediate Hugging Face search and a dedicated result area.
  The Browser uses a tab row, Back/Forward/Reload controls and an address field
  in one clear hierarchy.
- Option 3: the Task rail contains immediate search, a nearby filter, Today and
  Earlier grouping, status text and Alpine mountain artwork at the lower edge.
- Palette: warm cream/blush panes, coral-to-rose primary chrome and deep plum
  text visibly match the operator palette. Normal text and status tokens pass
  the automated WCAG AA contrast test.

## Findings history

| Priority | Finding | Resolution |
| --- | --- | --- |
| P1 | Quiet and status tokens did not all meet 4.5:1 on the two primary surfaces. | Darkened the semantic tokens and kept the contrast test as a release check. |
| P1 | The Preview Model Library labeled a row Default while its selector showed Not selected. | Bound the preview default to the same exact Registry identity tuple as the row. |
| P1 | A persisted wide layout could open both overlays over a 700-pixel Task canvas. | Moved open-state ownership into the layout module and collapse both overlays on compact entry. |
| P1 | A failed Pi launch could leave only the rail status visible after the user message persisted. | Kept the runtime error in an alert beside the composer and added a regression test. |
| P2 | The mountain and window edge were weaker than the selected references. | Increased the raster's visual weight and added a restrained coral inset frame. |
| P2 | Browser chrome exposed Back and Reload but omitted Forward. | Added the existing trusted Forward command to the visible toolbar and covered it through the Desktop Interface mock. |
| P2 | Quick and full analysis could be launched at the same time, with progress confined to button text. | Enforced a single analysis operation and added a persistent, accessible indeterminate progress region. |
| P2 | Local-model connection failures gave no next action. | Added plain-language recovery, Retry, Open Settings, and the Control/Command-comma settings shortcut. |
| P2 | Diagnostic result text visually collided at the value boundary. | Added a wrapping result layout with explicit row and column gaps. |

The detailed component-by-component HIG applicability check is tracked in
`docs/research/alpine-desktop-hig-audit-2026-08-24.md`.

No unresolved P0, P1 or P2 visual defects were found in the inspected states.

**final result: passed**
