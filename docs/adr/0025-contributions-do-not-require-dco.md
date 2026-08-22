# ADR 0025: Contributions do not require DCO sign-off

## Status

Accepted — 2026-08-22

## Context

ADR 0023 selected Developer Certificate of Origin 1.1 sign-off for contributions to the public source repository. That made a `Signed-off-by` trailer a mandatory per-commit gate and required the contributor, rather than an automated coding agent, to supply the attestation. The project owner has decided that this additional contribution ceremony is not required for Alpine.

The Project License remains Apache-2.0. Third-Party Material retains its separate license boundaries and obligations, and removing DCO sign-off does not reduce provenance or license-review responsibilities.

## Decision

Alpine does not require a DCO sign-off, `Signed-off-by` commit trailer or contributor license agreement. Contributions use the ordinary pull-request workflow. Contributors remain responsible for having the right to submit their changes, for identifying Third-Party Material, and for satisfying applicable provenance and license obligations.

The DCO document, commit-trailer verifier, CI gate and pull-request checklist item are removed. Agent-generation disclosure remains required for materially agent-generated changes because it supplies review context rather than a legal attestation.

This decision supersedes only the DCO contribution-attestation portion of ADR 0023. It does not change the source-only release boundary, Project License, security-reporting process, evidence privacy rules or third-party licensing boundaries.

## Consequences

Unsigned commits can pass canonical verification and be merged. Alpine no longer collects a per-commit DCO certification. Maintainers must continue to review provenance and third-party obligations, and contributors remain accountable for the material they submit. Reintroducing a DCO, CLA or other contribution attestation requires a new owner decision and ADR.
