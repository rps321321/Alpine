# Architecture verification

Alpine uses two required source-verification checks. They deliberately prove different things.

## `canonical-verification`

This is the existing control-plane gate. It runs on Windows with the pinned root Rust toolchain and executes `cargo run --locked --bin alpine-verify`.

It verifies the proposed public tree, Rust formatting and Clippy, all root Rust targets, and the retained Python compatibility suite. It does not start the large model or claim live Qualification.

## `desktop-verification`

This is the desktop source and architecture gate. It uses the desktop package's declared Node and Rust toolchains and runs:

- the repository package inventory and architecture-policy tests;
- renderer unit and accessibility tests;
- TypeScript type checking;
- the production Vite/Sites build and Sites handoff tests;
- desktop Rust formatting, strict Clippy, and all native tests.

`config/architecture-policy.json` is the machine-checked inventory for shipped package manifests and current cross-layer boundaries. Adding a `Cargo.toml`, `package.json`, or `pyproject.toml` outside ignored generated directories requires an explicit package entry and verification owner.

The policy also freezes two known architectural debts instead of allowing them to spread:

- Pi implementation packages are confined to the adapter and its focused test. Existing direct Pi-event use in `task-execution` is a temporary exception owned by issue #49.
- Renderer-facing row mutation primitives are confined to the existing desktop interface and renderer orchestrator. Their removal is owned by issue #47.

A temporary exception must include an issue and reason. Verification fails when a new caller appears, when an exception becomes stale, or when a package/workflow command disappears. This makes the allowlist shrink as the architecture migration progresses.

Generated Rust/TypeScript contract checks will be added to the same policy when issue #50 introduces generated contracts. An empty generated-contract list does not imply that handwritten contracts are proven equivalent.

## What source CI does not prove

Passing both jobs proves that the checked-in source builds and satisfies the declared automated boundaries. It does **not** prove:

- native minimum-window behavior, child-WebView geometry, file pickers, keyboard focus, screen-reader announcements, or rendered contrast;
- live hardware discovery, model download, inference startup, exact Session restoration, or agent capability;
- Profile Qualification, Deployment promotion, or signed release packaging.

Packaged native UI evidence is tracked in issue #43. Live integrated-desktop and local-model evidence remains separately authorized work tracked from issue #25 and the applicable Qualification issues. These distinctions prevent a source-level green check from being presented as packaged-app or live-capability proof.

## Local commands

From the repository root:

```console
node --test scripts/verify-architecture.test.mjs
node scripts/verify-architecture.mjs
cargo run --locked --bin alpine-verify
```

From `apps/desktop`:

```console
npm ci
npm test
npm run typecheck
npm run build
npm run test:sites
```

From `apps/desktop/src-tauri` with its pinned toolchain:

```console
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
```
