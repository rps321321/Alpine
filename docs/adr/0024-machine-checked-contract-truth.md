# ADR 0024: Machine-checked contracts must match the claim

## Status

Accepted — 2026-08-22

## Context

The schema-2 microbenchmark labeled an exact-copy workload with a nonempty predicate, accepted structured JSON with incorrect values, and blended a repeated-visible-output specialization into the default selection score. Profile and Session Config decoding ignored unknown fields, while material parameter relationships were mostly unchecked. The production evaluation plan also executed only one simple golden repair, and a detailed Focus Timer contract lived under configuration even though no Alpine runner executed or scored it.

Those contracts could produce precise-looking evidence for a weaker claim than their names and prompts implied.

## Decision

Microbenchmark schema 3 uses typed semantic predicates. Exact-copy evidence compares the complete UTF-8 payload to a digest-bound expected file under an explicit terminal-newline policy. Structured JSON must have exactly the configured keys and values and a bounded nonempty reason. Prefill and novel-response probes are labeled as mechanical response checks rather than coding-quality claims.

Promotion Policy schema 4 separates a general selection score from repeated-visible-output specialization. Only the configured general workload set can select the default. Repeated specialization is reported independently, while every required workload still retains quality, determinism, variability, and regression gates. Tuning reports use schema 2. Evidence produced under the old workload or policy schemas is stale and is rejected; it is not silently migrated.

Profile and Session Config objects fail on unknown fields. `status` is specifically rejected because lifecycle and deployment roles belong to append-only deployment history. `config/profile-capabilities.json` is the single runtime/KV/request-local-n-gram capability table consumed by Rust, Python and PowerShell; setup publishes it with the retained adapters and binds it into control-plane identity. Reset-on-begin requires n-gram mode, and n-gram mode requires the patched custom runtime. Output cannot exceed context, micro-batch cannot exceed batch, and the thread count has a conservative table-defined sanity ceiling. Rust setup validates the complete Profile before creating the installation root. The retained adapters also reject non-loopback endpoints and apply the same versioned cleanup boundary before reading or using local credentials.

Golden task schema 2 identifies checked-in fixtures as public and declares their machine-checked capabilities. Golden evidence schema 3 binds the task suite, OpenCode executable and test executable identities. The default `public-v1` fixture adds multi-file TDD repair, required tool-error recovery, and early-constraint retention through context growth. Recovery requires exactly one failed `read` of the declared relative sentinel followed by a successful tool effect; conflicting call-ID reuse and duplicate matching effects fail. Promotion Policy names `public-v1` and its three required capabilities, so a simpler task cannot satisfy Qualification through the generic evidence kind. Public reports retain only structural counts, booleans and digests; prompts, tool input, raw output and final prose are not evidence. The Focus Timer contract is retained only as an explicitly unimplemented experiment outside qualification configuration. The narrow Pi/OpenCode exact-read comparison remains isolated and does not introduce a generalized engine protocol.

The public-tree verifier continues to reject arbitrary binary content. A shipped
desktop raster is admitted only through `config/public-binary-assets.json`,
which restricts entries to the desktop asset directory and requires an exact
lowercase SHA-256, a size ceiling no larger than the source-file limit, and a
nonempty provenance statement. Missing, changed, oversized, duplicated or
unlisted binary assets fail verification. Design-reference captures are local
research inputs, not proposed public-tree content.

Native executable version probes accept an explicit argument contract and have
a bounded wait before terminating the exact probe process. This keeps the
release verifier from hanging on a valid executable whose version syntax is not
`--version`, while llama.cpp probes retain that default.

Test-only subprocess checks retain their exact exit, output and boundary
assertions but allow up to 30 seconds for cold Windows CI process startup. This
does not change any production request, benchmark, worker or inference timeout;
it prevents a busy clean runner from turning executable launch latency into a
false contract failure.

## Consequences

Existing benchmark, policy, tuning-report, and golden-agent evidence must be regenerated before it can support a current claim. The stricter contracts may expose configuration mistakes that older adapters ignored; errors are reported before runtime or setup mutation. Checked-in task fixtures are reproducible public tests and must never be described as hidden evaluation evidence. Live GPU, model, and agent runs remain separate evidence and are not implied by source-level verification.
