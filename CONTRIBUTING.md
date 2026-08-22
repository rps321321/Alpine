# Contributing to Project Alpine

Project Alpine accepts contributions to its own source and documentation under Apache License 2.0. It does not require a Developer Certificate of Origin sign-off or a contributor license agreement.

## Contribution terms

No special commit-message trailer is required. Contributors remain responsible for having the right to submit their changes and for the correctness, provenance, licensing, security, privacy, and verification of everything they submit, regardless of which tools assisted them. Third-Party Material keeps its original license boundary and obligations.

## Development workflow

1. Keep changes focused and preserve the architecture and evidence contracts in `CONTEXT.md` and `docs/adr/`.
2. Add or update tests for changed behavior.
3. Run `cargo run --locked --bin alpine-verify` on any supported development platform.
4. Open a pull request against `main`, complete the template, and resolve review conversations.
5. Do not commit model weights, runtime bundles, generated installations, credentials, machine-local state, raw/private evidence, or personal filesystem paths.

The canonical `verify` workflow must pass before an ordinary merge. Maintainers may use an explicit administrative bypass only for exceptional repository recovery and must document the reason afterward.

## Agent-generated changes

Ordinary autocomplete, research, explanations, and suggestions that the contributor manually reimplements do not require a tool-use disclosure. If an AI or software agent generated a substantial part of the submitted diff or performed a multi-step change directly in the repository, the pull request must say so and describe the human verification performed. This is review context, not a transfer of responsibility and not a substitute for provenance or license review.

## Security reports

Do not disclose a suspected vulnerability in a public issue. Follow [SECURITY.md](SECURITY.md).
