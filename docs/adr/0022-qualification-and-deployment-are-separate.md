# ADR 0022: Qualification and deployment are separate

## Status

Accepted — 2026-08-20

Profiles contain inference-material settings, Qualification records what evidence established about an exact identity, and machine-local Deployment Roles select the daily default and rollback Profile. Evaluation never deploys. Explicit Promotion, Rollback, Incident and incident-resolution actions append immutable events under a deployment lock; current roles and eligibility are derived from that history. Turbo may become the daily default only after still-current production Qualification and an explicit operator Promotion, while Stable remains the independently proven rollback Profile.
