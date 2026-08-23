# Desktop design QA

## Source and implementation

- Source: the official OpenAI Codex app frame from the launch video embedded at <https://openai.com/index/introducing-the-codex-app/>, captured at 1280 x 720 on 2026-08-23.
- Implementation: the final local production preview at 1280 x 720.
- The source frame and final Alpine home screen were emitted together in the same browser comparison input before the final judgment.
- The model-discovery, full-analysis and Settings states were separately captured and inspected at the same viewport.

## Comparison

The implementation preserves the source hierarchy: a narrow persistent task rail, a quiet central workspace, a bottom-anchored task composer, compact model controls inside the composer, and a contextual right-side surface. Alpine intentionally adapts the source's light macOS treatment to the product's dark Windows-first direction and replaces Codex's repository context with local hardware, model-fit evidence, and runtime status.

Checked at the shared viewport:

- no clipping, overlap, unstable sizing, or accidental overflow;
- clear rail, workspace, composer, and inspector hierarchy;
- realistic hardware and model content rather than placeholder cards;
- visible focus styles and semantic controls;
- functional search, capacity and placement assessment, immutable-revision
  download guard, default-model guard, settings, runtime controls, browser,
  full evaluation, measured diagnostic, and task-composer paths;
- estimate language remains visibly distinct from measured analysis.
- Analysis-to-Settings navigation resets the main scroll position, so every view
  opens at its heading rather than inheriting the prior report offset.
- the Analysis and Settings pages retain the source density and full-height rail
  anatomy at 1280 x 720 with no browser console warnings or errors.
- final browser diagnostics contained zero entries.

## Result

Passed. The visible differences are intentional product adaptations, not unresolved source mismatches.
