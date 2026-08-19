# Versioned control plane; generated machine-local installation

## Status

Amended — 2026-08-19

## Decision

This Git repository owns artifact pins, profiles, setup/build scripts, the launcher source, benchmark definitions, database schema, reports and ADRs. `%USERPROFILE%\local-models` is a generated machine-local installation containing large model artifacts, runtime bundles, logs, a random localhost API key and rendered Session Config.

The initial Inference Session port is 8100, stored in rendered Session Config. Launch fails clearly if that port is occupied by a different executable; it does not kill the occupant. D: is a USB HDD and is not a weight store.

## Rationale

The former docs-only repository could describe a working machine but could not reconstruct it after an SSD/Windows replacement. `setup.ps1` now verifies/downloads pinned artifacts, reproduces the custom runtime patch/build, renders profiles and builds the launcher. Large weights, secrets and raw logs remain outside Git.
