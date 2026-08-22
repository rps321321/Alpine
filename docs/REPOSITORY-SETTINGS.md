# Repository settings contract

This document defines Alpine's owner-level GitHub settings. Repository files enforce the parts that can be reviewed in source; `scripts/configure_github_repository.py` applies and verifies settings that exist only in GitHub's administrative state.

## Visibility decision

The maintainer made `rps321321/Alpine` public on August 22, 2026. That explicit decision supersedes the earlier plan to keep the repository private until the M6 gate. Public visibility does **not** mean that Alpine has published a supported v0.1 release, a signed binary, or a production-qualified deployment.

The hardening script asserts that the repository is public and never changes visibility. Any future visibility change remains a deliberate human-owner action.

## Required owner-level state

| Area | Required state | Rationale |
| --- | --- | --- |
| Main-branch entry | Pull request required | Prevent ordinary direct changes to the release branch. |
| Administrator enforcement | Enabled | The solo maintainer follows the same protected path during ordinary work. |
| Required checks | `canonical-verification`, `project-management-validation` | Code and repository-policy contracts must both pass. |
| Branch freshness | Required | A pull request must be tested against the current protected branch. |
| Review conversations | Must be resolved | Prevent merge while known review concerns remain open. |
| Approvals | Zero required for now | Avoid deadlocking a solo maintainer while still requiring the PR path, checks, and resolved conversations. Revisit after adding a trusted maintainer. |
| Force pushes and deletion | Disabled | Preserve public history and the protected branch. |
| Merge method | Squash only | Keep one reviewed issue/PR outcome per main-branch commit. |
| Merged branches | Delete automatically | Reduce stale public branches. |
| Actions token default | Read-only | Workflows receive no write authority unless their source declares it. |
| PR approval by Actions | Disabled | Automation must not approve its own changes. |
| Allowed Actions | GitHub-owned only, full commit SHA required | Reduce third-party and mutable-tag supply-chain risk. |
| Issue writes | Only reviewed reconciliation workflows | Ordinary CI, dependency review, and verification are read-only. |
| Private vulnerability reporting | Enabled | Public vulnerability details must not be disclosed in issues. |
| Dependency graph and Dependabot alerts | Enabled and independently verified | Surface known vulnerable dependencies and make an SBOM available. |
| Dependabot security updates | Enabled | Open remediation PRs for eligible vulnerable dependencies. |
| Dependabot version updates | Cargo and GitHub Actions | Controlled by `.github/dependabot.yml`. |
| Secret scanning | Enabled | Detect supported secret patterns in public history and pushes. |
| Push protection | Enabled | Block supported secrets before they enter the repository. |
| Code scanning | GitHub CodeQL default setup, extended queries, remote-and-local threat model | Use GitHub's existing supported setup without a conflicting advanced workflow. |
| Signing environment | Owner approval required | Signing credentials and effects remain human-controlled. |
| Release environment | Owner approval required | Publication remains an explicit human decision. |

## Repository-controlled enforcement

The hardening change includes:

- `.github/workflows/verify.yml` with the stable `canonical-verification` check;
- `.github/workflows/project-management-validation.yml` with the stable `project-management-validation` check;
- `.github/workflows/dependency-review.yml`;
- `.github/dependabot.yml`;
- `.github/CODEOWNERS`;
- `scripts/validate_repository_policy.py`.

Every workflow must declare explicit top-level permissions. Actions must use immutable 40-character commit SHAs. `issues: write` is rejected outside the reviewed reconciliation workflow allowlist. `pull_request_target` is rejected unless a dedicated security design changes that policy.

## Applying the administrative state

Run the following only from a trusted local checkout after reviewing the script and branch. Use an owner-authenticated token with the repository permissions required by GitHub for administration, Actions policy, environments, and security settings.

```powershell
$env:GH_TOKEN = '<retrieve securely; do not paste into chat, issues, PRs, or logs>'
python scripts/configure_github_repository.py --repo rps321321/Alpine --apply
Remove-Item Env:GH_TOKEN
```

On a Unix-like shell:

```bash
export GH_TOKEN='<retrieve securely; do not paste into chat, issues, PRs, or logs>'
python scripts/configure_github_repository.py --repo rps321321/Alpine --apply
unset GH_TOKEN
```

The operation is idempotent. Re-run it after suspected settings drift. Verify without changing state using:

```bash
python scripts/configure_github_repository.py --repo rps321321/Alpine --check
```

The script does not print the token, change repository visibility, merge pull requests, publish releases, or use signing credentials.

## Code scanning configuration

GitHub CodeQL default setup was already enabled when this hardening work began. A proposed advanced CodeQL workflow successfully produced SARIF but GitHub rejected it because default and advanced setup cannot be active together. Alpine therefore keeps default setup and deliberately does **not** maintain an advanced CodeQL workflow.

The owner-settings reconciler configures default setup with:

- `state: configured`;
- the `extended` query suite;
- the `remote_and_local` threat model;
- a standard GitHub-hosted runner;
- language detection managed by GitHub.

The policy validator rejects `.github/workflows/codeql.yml` while this decision is in force. Moving to advanced setup later requires an explicit architecture/security decision, disabling default setup, and proving that the replacement analysis succeeds before treating it as coverage.

## Protected environments

The `signing` and `release` environments require the human repository owner as a reviewer. Because Alpine has one human maintainer, self-review remains allowed; the consequential action still requires a distinct approval interaction. Future release or signing workflows must reference the appropriate environment and must not read credentials before approval.

## Audit cadence

Verify these settings:

- immediately after this hardening change merges;
- before making a release;
- after adding a maintainer, GitHub App, or privileged workflow;
- after changing repository visibility or plan;
- after a security incident or unexpected direct push;
- at least once per release milestone.
