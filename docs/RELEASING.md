# Source-only v0.1 release

The first public release establishes Alpine's source project and reproducibility. It does not ship a generated installation or runnable binary bundle.

## Ordered gates

1. Finish the new bounded evaluation and human Capability Review.
2. Establish a fresh production Qualification and explicitly promote Turbo locally, keeping Stable as rollback.
3. Complete the release/privacy/redistribution and all-history audit.
4. Perform the separately approved history rewrite and verify the public mirror.
5. Enable repository protection and private vulnerability reporting.
6. Make the repository public through a separately approved visibility change.
7. Tag and publish v0.1 through a separately approved release action.

Local production qualification and public release are deliberately separate. Passing one never performs the other.

## Included in v0.1

- Alpine source, tests, documentation, ADRs, configuration contracts, and patches;
- `Cargo.lock`, `rust-toolchain.toml`, pinned GitHub Actions, and artifact hashes needed for reproducibility;
- Apache-2.0 `LICENSE`, `NOTICE`, DCO, contribution/security/governance documents, and third-party boundaries;
- optional generated Public Evidence produced by the allowlisted schema and explicitly reviewed for publication.

## Excluded from v0.1

- model weights, projectors, chat-template downloads, paused or complete artifact downloads;
- generated `%USERPROFILE%\local-models` installations, Alpine executables/installers, runtime bundles, CUDA libraries, and build output;
- API keys, credentials, logs, session/deployment machine state, SQLite databases, raw evidence, private review material, transcripts, machine inventories, and personal paths;
- `refs/codex/turn-diffs/**` or any other private tool/session refs.

Binary packaging, signing, installers, update UX, a complete binary-closure license/SBOM audit, and broader verified hardware support are future deliverables. They must not be implied by the source-only tag.

## Release verification

Before tagging, work from a public-safe mirror of the rewritten history and verify:

```console
cargo run --locked --bin alpine-verify
git status --short
git ls-files
git log --all --format='%H%x09%an%x09%ae%x09%aI%x09%cI'
```

Run the separately retained private publication/history audit plan, confirm only intended refs will be pushed, and manually inspect the release archive. Creating the tag, changing visibility, and publishing the GitHub release are separate external writes requiring explicit approval.
