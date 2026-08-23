# Desktop design QA

## Source and implementation

- Source: the official OpenAI Codex app launch image captured from <https://openai.com/index/introducing-the-codex-app/> at 1280 x 720.
- Source capture: `.artifacts/design/codex-official-reference.png`.
- Implementation capture: `.artifacts/design/alpine-home.png` at 1280 x 720.
- Combined comparison: `.artifacts/design/codex-alpine-comparison.png`.
- Model workflow capture: `.artifacts/design/alpine-models.png`.
- Analysis workflow capture: `.artifacts/design/alpine-analysis.png`.
- Settings workflow capture: `.artifacts/design/alpine-settings.png`.

## Comparison

The implementation preserves the source hierarchy: a narrow persistent task rail, a quiet central workspace, a bottom-anchored task composer, compact model controls inside the composer, and a contextual right-side surface. Alpine intentionally adapts the source's light macOS treatment to the product's dark Windows-first direction and replaces Codex's repository context with local hardware, model-fit evidence, and runtime status.

Checked at the shared viewport:

- no clipping, overlap, unstable sizing, or accidental overflow;
- clear rail, workspace, composer, and inspector hierarchy;
- realistic hardware and model content rather than placeholder cards;
- visible focus styles and semantic controls;
- functional search, selection, verified download, default-model, settings,
  browser, measured diagnostic, and task-composer paths;
- estimate language remains visibly distinct from measured analysis.
- the Analysis and Settings pages retain the source density and full-height rail
  anatomy at 1280 x 720 with no console warnings or errors.

## Result

Passed. The visible differences are intentional product adaptations, not unresolved source mismatches.
