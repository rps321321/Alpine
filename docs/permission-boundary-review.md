# OpenCode Permission Boundary Security Review

## Executive summary

The existing Harness Policy Boundary is useful as an operator-consent layer for ordinary tool calls, but it is not a security boundary against a hostile, hijacked, or deliberately evasive model. OpenCode 1.18.18 runs the shell as the Windows user, inherits the process environment, defaults the build agent to allow unmatched tools, and applies Bash policy to parsed command strings. The correct v1 claim is therefore **accident prevention and explicit consent**, not containment.

The minimal launcher preserves model capability: it does not filter topics, techniques, code, research destinations, or shell access. It removes context-heavy integrations, strips ambient credential-like environment variables, denies direct Read-tool access to personal credential stores, and asks before common irreversible or externally visible effects. A restricted Windows identity, VM, or equivalent Attack-Lab Isolation Boundary is still required before treating a compromised agent as contained.

The localhost inference endpoint now requires a random file-backed bearer key for completion requests. OpenCode reads it locally and the direct Read tool is denied that file. An unauthenticated completion returned HTTP 401 while the authenticated OpenCode loop remained functional. This prevents an unrelated web origin from freely driving the GPU, but it is not protection from another process already running as the same Windows user.

## Critical findings

### PB-01 — Shell rules are bypassable and cannot enforce the documented Selected Project boundary

**Impact:** A malicious or prompt-injected model can act with the Windows user's filesystem, process, credential, and network authority despite the configured command exceptions.

OpenCode's build agent starts from `"*": "allow"` and later applies user exceptions (`packages/opencode/src/agent/agent.ts:119`). Its shell tool asks against parsed command-source strings and performs external-directory discovery only for a limited set of recognized file commands with statically resolvable paths (`packages/opencode/src/tool/shell.ts:378`). Wrappers such as another shell, aliases, language runtimes, dynamic paths, libraries, and alternate executables can escape those checks. The reviewed machine-local configuration enumerated only a small set of Git and SSH strings (`%USERPROFILE%\.config\opencode\opencode.jsonc`).

**Disposition:** Reclassify the Harness Policy Boundary as a capability-preserving consent/tripwire layer. Do not claim hostile-agent containment. Keep a separate Attack-Lab Isolation Boundary as the real solution.

### PB-02 — Ambient secrets are inherited by the shell

**Impact:** A tool call can read or transmit credential-bearing environment variables without touching a protected file.

The OpenCode shell copies `process.env` into every command (`packages/opencode/src/tool/shell.ts:416`). At review time, the parent environment contained a credential-like variable name. No secret value was inspected or recorded.

**Fix implemented:** The minimal launcher removes token, secret, password, API-key, credential, and auth-like environment variables, plus SSH askpass/agent variables and `GOOGLE_APPLICATION_CREDENTIALS`, before starting OpenCode. This reduces ambient authority but does not block disk-backed Windows Credential Manager or other user-level stores.

## High findings

### PB-03 — The Git policy is simultaneously over-restrictive and incomplete

The reviewed configuration permanently denied `git push`, all `git remote` commands, rebases, and selected reset/config operations. This blocked harmless inspection such as `git remote -v` and prevented explicitly authorized pushes, yet omitted other destructive operations such as `git clean`, `git restore`, forced checkout, stash deletion, `filter-repo`, and `update-ref`. All string rules remain bypassable through shell indirection.

**Fix implemented for the minimal profile:** Harmless remote inspection is allowed. Common remote writes, destructive working-tree operations, and history changes use `ask`, not `deny`. The operator can authorize intended work without granting silent execution.

### PB-04 — Personal credential protection is narrow and provides false confidence

The reviewed direct-read policy protected only `~/.ssh`. It did not cover common cloud, container, package-manager, GitHub CLI, or OpenCode credential files. More importantly, direct Read-tool rules do not mediate arbitrary shell-based reads.

**Fix implemented:** The minimal profile denies external-directory tool access to common credential directories, denies direct Read-tool access to common personal credential files, and asks before SSH/SCP/SFTP use. Test Credentials must live outside those personal stores. This remains best-effort until the process runs under a restricted identity.

### PB-05 — Remote effects are not comprehensively mediated

Web research and technique execution are intentionally open under ADR 0009. That is capability-preserving, but the same unrestricted shell can also invoke APIs, deployment CLIs, credential helpers, or custom programs that create external effects. Enumerating command names cannot make this complete.

**Disposition:** Keep research, scanning, reverse engineering, and authorized testing available. Ask on common publication/deployment commands as an accident barrier. Require operator oversight now and OS/network isolation for hostile-target work later.

### PB-06 — Project OpenCode configuration can override the global permission rules

OpenCode merges project-defined agent permissions after the global/user rules (`packages/opencode/src/agent/agent.ts:267`). A repository-controlled `.opencode` agent can therefore weaken an otherwise careful launcher policy even when external plugins are disabled.

**Fix implemented:** Project OpenCode configuration is off by default in the minimal launcher. Repository `AGENTS.md` instructions and ordinary source files still load. `-WithProjectConfig` is an explicit trust decision for a known project.

## Medium findings

### PB-07 — Global skills, plugins, inherited prompts, and MCP tools consume scarce context and expand trust

The measured contaminated request contained large foreign/global instruction and skill catalogs before meaningful work. Disabling only external-agent skills still left native OpenCode skill entries linked inside its config directory.

**Fix implemented:** The launcher uses OpenCode `--pure`, disables the foreign Claude prompt, denies the skill tool/catalog in bounded 16K/32K profiles, and disables Convex by default. It replaces the generic coding manual with a short neutral prompt and disables the second model request used only to title a session. Core task, todo, read, write, edit, search, shell and web capabilities remain available. `-WithSkills`, `-WithPlugins`, `-WithProjectConfig` and `-WithConvex` are explicit trust/context choices.

The captured request fell from 101,814 bytes to 26,003 bytes: an 817-byte system message plus 20,915 bytes for nine core tool schemas. A live fresh Fast-32K `hey` used 5,529 input tokens, 11 output tokens, completed text in about 1.55 seconds, and did not compact. The final Stable-16K smoke used the same input, generated 64 output tokens in about 9.76 seconds, and also did not compact. The remaining request is primarily real coding-tool schemas, not hidden conversation history.

The GGUF added a 1,389-character instruction block asserting that every response is an execution. That behavior—not a safety boundary—caused greetings to trigger unnecessary Git or shell work. The server now uses Qwen's official template, which is otherwise identical to the embedded template and retains the same tool/vision structure. A real file-read loop was verified after the override.

The lean prompt also requires bounded tool calls and shorter retries after parse failures. This addresses the observed 4,096-token malformed `write` call without filtering subject matter, destinations, or tool capability.

### PB-08 — Permanent denial is the wrong default for operator-authorizable effects

A hard deny is justified for implicit direct access to personal credentials. For ordinary consequential actions, permanent denial defeats legitimate operator intent without creating containment.

**Fix implemented:** The minimal policy prefers `ask` for destructive or externally visible actions. Its short replacement agent prompt contains effect-safety and verification guidance, but no topic bans, technique bans, destination filters, or refusal policy.

### PB-09 — “Approve always” is broader than the displayed one-off command

OpenCode derives an approval family from command arity rather than retaining only the exact command (`packages/opencode/src/permission/arity.ts:1`). For example, approving a consequential Git subcommand “always” can authorize later commands in the same family for the remainder of the session.

**Operator rule:** Use approve-once for destructive, credential-using, publication, deployment, and remote-write prompts. “Always” is appropriate only for a command family the operator intentionally trusts for the current session.

## Agent threat analysis

| Threat | What the current launcher does | What it does not do |
|---|---|---|
| Goal hijack / prompt injection | Uses a short explicit task prompt, disables ambient foreign prompts/project config by default, and keeps consequential effects behind consent. | It cannot prove that repository text or tool output did not redirect the model's reasoning. |
| Tool misuse | Preserves useful tools, records golden-task events, asks before common destructive/external effects, and rejects `--auto`. | String policies cannot mediate every wrapper, runtime, alias or custom executable. |
| Identity / privilege abuse | Removes credential-like environment variables and shields common credential paths from direct reads. | The process still has the current Windows user's token, files, network, credential manager and installed applications. |
| Unexpected code execution | Golden tasks run only in copied fixtures with protected-file hashes, timeouts and deterministic cleanup metadata. | Tests and model shell commands still execute on the host; a copied directory is not an OS sandbox. |
| Memory poisoning | Bounded profiles remove shared skill catalogs; the custom n-gram table is reset per request; runs preserve hashes. | OpenCode conversation/cache state and arbitrary project content can still influence later turns inside a session. |
| Inter-agent communication | External plugins and ambient agent skills are off by default; one local model endpoint is used. | Explicit task/subagent tools and any shell/network client the model invokes can still communicate. |
| Cascading failures | One server slot, bounded retries, process ownership checks, timeouts, raw failure records and Stable rollback limit ordinary cascades. | Host-level process/network/resource exhaustion is not contained by OpenCode permissions. |
| Rogue or compromised agent | The model is treated as untrusted and production changes retain operator consent. | A rogue agent is not contained until it runs under a restricted identity/VM with network and filesystem controls. |

These boundaries intentionally constrain **effects**, not subject matter or model reasoning. Adding topic bans or artificially removing normal coding ability would defeat the purpose of the abliterated worker without creating a real security boundary.

## Residual risk and acceptance boundary

The launcher materially reduces accidental damage and ambient-secret exposure. It does **not** defend against deliberate evasion, a compromised model, a malicious repository instruction, user-level credential stores, arbitrary network clients, or software exploiting the host. Those require a separate Windows account with constrained ACLs and credentials, a VM/sandbox, and—where target containment matters—network controls. Calling the current layer a sandbox would be inaccurate.
