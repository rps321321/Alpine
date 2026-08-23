# Bounded headless agent-engine bake-off

## Result

**No-go for adopting an agent-engine boundary now.** Keep OpenCode as the v1 Harness. Carry Pi SDK/core and Cline's stateless `@cline/agents` surface into a later recovery-focused follow-up only after the missing upstream hooks close; retain Pi RPC only as a disposable process comparison.

This result is deliberately narrower than a product or packaging decision. The repository now has an executable Alpine-owned runner and candidate-native adapters, and all four candidate seams completed the required scenario accounting under one live Evidence Identity. The report is complete because every scenario is either demonstrated or an explicit typed failure; it does not claim that every capability passed. Global installs, defaults, Profiles, model configuration and Deployment Roles were not changed for the run.

## Shared contract

`config/agent-engine-bakeoff.json` supplies one contract for all candidates:

- exact model artifact, Profile, runtime, chat template, prompt/tool policy, task fixtures, request budget and recovery scenarios must be identical;
- hardware, software, model, runtime, workload, configuration and policy hashes must all be present and repeated by every candidate observation;
- at most 24 requests per candidate, a 120-second request timeout, 16,384 maximum input tokens, 2,048 maximum output tokens, two retries, one worker restart and a 128-event bounded queue;
- streaming, tools, steering, follow-up, cancellation, retry, compaction, backpressure, worker restart, continuation recovery and normalized errors must be demonstrated or recorded as explicit typed failures;
- acquired material Session state must be verified after every scenario, and the exact prior material Session must be restored after each candidate, while the transaction identity may change;
- candidate state must be disposed after assessment.

The command does not accept an evidence file. Rust computes the identity, assigns scenario and sequence fields, measures subprocess wall time, verifies package-lock integrity and owns Session acquisition/restoration. Software identity binds the running Alpine and Node executables, the repository worker and every file or internal symlink in the isolated package/transitive closure. Before each scenario Rust supplies only the candidate's remaining request, retry, restart and event capacity; SDK adapters count native provider turns and stop on native-event overflow. The repository-owned Node adapter maps native observations to the Rust event schema. Its raw child output stays in the disposable run and is never a report field. The report contains typed events, typed error codes, per-scenario request/time/restoration facts and hashes; it has no fields for raw stdout/stderr, prompts, credentials, tool arguments, generated content, model artifacts or machine-local paths.

## Reviewed candidates

| Candidate seam | Reviewed source | License and dependency surface | Maintenance and packaging | Security boundary | Source-level fit and smallest missing hook |
| --- | --- | --- | --- | --- | --- |
| OpenCode process | [`anomalyco/opencode` v1.18.21, `826d9ad`](https://github.com/anomalyco/opencode/tree/826d9ad46a22bef0294998e08daa3c4904fea28f) and `opencode-ai@1.18.21` | MIT; source package declares 99 direct runtime dependencies; npm launcher selects from 12 platform packages | Active after the reviewed release; high bundled-Bun/server/provider/storage update and packaging cost | No sandbox; permission prompts are UX gates; server authentication is operator-owned | Raw JSON streaming and tools exist, but `run` creates OpenCode-owned Session state and has no explicit bounded event sink. Missing: ephemeral no-session execution with typed transcript import/export and flow control. |
| Pi SDK/core | [`earendil-works/pi` v0.84.2, `914cf14`](https://github.com/earendil-works/pi/tree/914cf1472e715297caa30db4b9535d534a9eb718), `@earendil-works/pi-agent-core@0.84.2` and directly imported `@earendil-works/pi-ai@0.84.2` | MIT; five direct core dependencies plus the reviewed provider package; 1,908,824-byte unpacked core package; Node.js 22.19+ | Active after the release; focused core but fast pre-1.0 cadence | No sandbox; tools/extensions have process authority; credentials can be injected in memory | Best Pi seam: typed streaming/tool events, awaited listeners and in-memory state. Missing: a versioned typed state import/export contract proven across worker restart without a Pi store. |
| Pi process/RPC | Same Pi commit; `@earendil-works/pi-coding-agent@0.84.2` | MIT; 21 direct dependencies, one optional dependency, 140 shrinkwrapped package entries, 13,709,606-byte unpacked package | Active; complete CLI/RPC/provider/tool closure has medium-high update cost | No sandbox; temporary agent root and least-privilege local key are required | Strict LF JSONL, correlated commands/events, steer/follow-up, abort, retry and compaction are documented. Missing: import/export of Alpine-owned conversation state without making a Pi session file durable authority. |
| Cline agents | [`cline/cline` `be8b984`, `@cline/agents@0.0.78`](https://github.com/cline/cline/tree/be8b984d10d1ad0e9a3917e051ac697f592587d2/sdk/packages/agents) | Apache-2.0; three direct dependencies; 364,877-byte unpacked package; Node.js 22+ | Current-day source activity; experimental 0.0.x package inside a fast monorepo | Caller tools and credentials retain embedding-process authority; only the latest minor is security-supported | Stateless loop, typed events, abort/continue, state restore, overflow compaction and awaitable event hooks are promising. Missing: a typed steering/follow-up queue with explicit retry and compaction lifecycle control. |

Registry integrity strings, commit dates, runtime requirements, packaging/update findings and security risks are machine-checked in the plan. No candidate package or source was copied into Alpine.

## Live capability report

The live report used plan `f1b3669a8c4ac3d56ebf9e9df35fc065ef20fa3d79fe21de245e5cac0bdda47e`. Its hardware, software, model, runtime, workload, configuration and policy identities were respectively `602c5973569c649da37ff9a1dc7e1aa056af81bbb829ffa22c7ff33de0ec3c6c`, `6b01fbca497c287671dd1403a21c83ab33dcdb7020453e67b0878302f8a7a656`, `5d53637a59cfcd3a4d8354e254ffd44943e5a693da2405a3e228c62962355509`, `b9774cfcd3bb431039464d78313bfe08a9d8813b90b9be31654aff49d43ca7e6`, `5ff0a7173869fe187732ee3139e4b47bf965d32adbd4fd66c80204dfd64b4363`, `9e3399e12e51ed439088d78887badb0e52ed398b428e19b78528e6ea31e9cb6e` and `849873922f83904d3050eddada1a82b27eafa8c9b5f253c17542515deec527b3`.

| Candidate | Requests | Demonstrated | Explicit failures |
| --- | ---: | --- | --- |
| OpenCode process | 0 | None | All 11: the CLI cannot replace its complete built-in system prompt with the exact shared benchmark prompt without retaining additional OpenCode policy. |
| Pi SDK/core | 9 | Streaming, exact-read tool, cancellation, normalized error | Steering and follow-up output did not satisfy the exact marker; retry, compaction, restart and continuation lack the required controlled/versioned hooks; backpressure exceeded the 128-native-event bound. |
| Pi process/RPC | 0 | None | All 11: the built-in `read` tool cannot enforce the exact single-file path policy, so Alpine refused to launch it under a non-identical policy. |
| Cline agents | 5 | Streaming, exact-read tool, cancellation | No steering/follow-up queue, explicit retry/compaction lifecycle or versioned restart/continuation; backpressure exceeded the 128-native-event bound; empty continuation was accepted rather than normalized as an error. |

The report recorded `evidence_complete: true`, `all_scenarios_demonstrated: false`, `all_prior_sessions_restored: true`, 44 of 44 per-scenario restoration checks, deletion of all four candidate-state roots, no retries and no worker restarts. The runner upgrades a capability to `demonstrated` only when the exact adapter invocation emits a complete native lifecycle and the Session restoration check passes. Source support alone is never a runtime demonstration.

## Execution status

The isolated candidate root was prepared with the five exact reviewed packages and used first for adapter API/error-path smoke checks. The normal configured install still uses the retired schema-4 cleanup contract, so Alpine correctly rejected it before Session mutation. The complete run therefore used a disposable schema-5 install view with cleanup disabled that referenced the same stable Profile, model, runtime, chat template and existing local credential read-only for the bounded transaction. It did not edit or select any global install, credential, Profile, model configuration, default or Deployment Role.

The corrected live command exited with Alpine's no-go status after emitting the complete report. It started and stopped only the temporary install view's exact Session process, returned that view to its prior idle material state after every candidate, and left no listener on the inference port. The report intentionally remains no-go because the restart/continuation proof is absent and both embeddable seams exceeded the event bound in the backpressure scenario.

## Recommendation gates

A later ADR may select an engine boundary only after:

1. at least two candidates run the same identity-bound fixtures and budget;
2. cancellation and backpressure are observed under load rather than inferred from APIs;
3. a killed worker restarts from Alpine-owned state with exact continuation and no candidate store retained;
4. the exact prior material Inference Session is restored after success, failure and cancellation;
5. the candidate's locked redistributed closure receives license, notice, vulnerability and packaging review;
6. the smallest missing upstream hook is accepted upstream or implemented without a fork or second durable authority.

Until then the evidence-backed answer to #24 is a bounded no-go, not approval for a fork, desktop, installer, Profile change or production claim.
