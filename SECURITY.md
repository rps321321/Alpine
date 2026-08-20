# Security policy

## Reporting a vulnerability

After the repository is public and GitHub private vulnerability reporting has been enabled, report vulnerabilities through the repository's **Security → Advisories → New draft security advisory** flow. Do not include exploit details, credentials, private evidence, or affected-user data in a public issue.

If private reporting is not available, open a content-free issue asking the maintainers to establish a private channel. Do not publish the vulnerability details there.

Maintainers should acknowledge a private report, establish a private coordination channel, assess affected versions, prepare and verify remediation, and coordinate disclosure. No response-time or support-level guarantee is made for v0.1.

## Supported versions

Before v0.1 is actually published, no public version is security-supported. Once published, only the latest tagged source release and current `main` are expected to receive security fixes unless a release notice states otherwise.

## Security boundary

Alpine gates protected effects and refuses known unsafe configuration, but it is not an operating-system sandbox or data-loss-prevention boundary. OpenCode tools and child processes run with the current Windows user's authority. Shell aliases, interpreters, PowerShell/.NET APIs, custom executables, and alternative command spellings can bypass command-string permission patterns. Use a disposable VM or a separately restricted operating-system account for hostile repositories or adversarial-model testing.

Never submit model API keys, credentials, private review artifacts, raw evidence, machine inventories, or generated installation state with a report unless a maintainer has established an appropriate private transfer path and explicitly requested the minimum necessary material.
