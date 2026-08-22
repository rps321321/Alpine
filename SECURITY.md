# Security policy

## Reporting a vulnerability

The repository is public. Use GitHub's private vulnerability-reporting flow from **Security → Advisories → Report a vulnerability** whenever that control is available. Do not include exploit details, credentials, private evidence, affected-user data, or reproduction artifacts in a public issue, pull request, discussion, commit, or workflow log.

If private reporting is unavailable, open a content-free issue that says only that a private security channel is required. Do not identify the affected component or publish technical details there. The maintainer must establish a private channel before requesting the minimum necessary evidence.

Maintainers should acknowledge a private report, preserve confidentiality, assess affected versions and identities, prepare and independently verify remediation, coordinate disclosure, and record any release or support consequences. No response-time or support-level guarantee is made before a supported release exists.

## Supported versions

Before v0.1 is actually published, no public version is security-supported. Public repository visibility is not a release or support claim. Once a release is published, only versions explicitly listed in release or support documentation receive security fixes; absent such a statement, expect only current `main` and the latest tagged source release to be considered.

## Security boundary

Alpine gates selected protected effects and refuses known unsafe configuration, but it is not currently an operating-system sandbox or data-loss-prevention boundary. OpenCode tools and child processes run with the current Windows user's authority. Shell aliases, interpreters, PowerShell/.NET APIs, custom executables, and alternative command spellings can bypass command-string permission patterns.

Use a disposable VM or a separately restricted operating-system account for hostile repositories, untrusted plugins, adversarial-model testing, or sensitive credentials. Treat models, repositories, tool output, downloaded metadata, plugins, MCP servers, and external processes as untrusted input.

Never submit model API keys, credentials, private review artifacts, raw evidence, machine inventories, generated installation state, private repository content, or personal filesystem paths unless the maintainer has established an appropriate private transfer path and explicitly requested the minimum necessary material.

## Repository security controls

The intended owner-level controls are defined in `docs/REPOSITORY-SETTINGS.md`. Private vulnerability reporting, secret scanning, push protection, Dependabot security features, protected `main`, CodeQL, dependency review, and protected signing/release environments must be verified rather than inferred from the presence of documentation.
