#!/usr/bin/env python3
"""Apply and verify Alpine's owner-level GitHub repository hardening.

This script is intentionally not run by an ordinary GitHub Actions token. It
changes consequential repository settings and therefore requires an
owner-authenticated token with the permissions GitHub demands for repository
administration, Actions, environments, and security settings.

The token is read from GH_TOKEN or GITHUB_TOKEN and is never printed.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from typing import Any, Iterable

API_ROOT = "https://api.github.com"
API_VERSION = "2026-03-10"
DEFAULT_REPOSITORY = "rps321321/Alpine"
REQUIRED_CHECKS = {"canonical-verification", "project-management-validation"}
PROTECTED_ENVIRONMENTS = ("signing", "release")


@dataclass
class ApiError(RuntimeError):
    method: str
    path: str
    status: int
    message: str

    def __str__(self) -> str:
        return f"{self.method} {self.path} failed with HTTP {self.status}: {self.message}"


class GitHubApi:
    def __init__(self, token: str) -> None:
        self._token = token

    def request(
        self,
        method: str,
        path: str,
        payload: dict[str, Any] | None = None,
        expected: Iterable[int] = (200, 201, 204),
    ) -> tuple[int, Any]:
        data = None
        headers = {
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {self._token}",
            "User-Agent": "alpine-repository-hardening",
            "X-GitHub-Api-Version": API_VERSION,
        }
        if payload is not None:
            data = json.dumps(payload).encode("utf-8")
            headers["Content-Type"] = "application/json"

        request = urllib.request.Request(
            f"{API_ROOT}{path}",
            data=data,
            headers=headers,
            method=method,
        )
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                status = response.status
                body = response.read()
        except urllib.error.HTTPError as error:
            body = error.read()
            try:
                decoded = json.loads(body.decode("utf-8")) if body else {}
                message = decoded.get("message", decoded)
            except (UnicodeDecodeError, json.JSONDecodeError):
                message = body.decode("utf-8", errors="replace") or error.reason
            raise ApiError(method, path, error.code, str(message)) from error
        except urllib.error.URLError as error:
            raise RuntimeError(f"{method} {path} failed: {error.reason}") from error

        if status not in set(expected):
            raise ApiError(method, path, status, body.decode("utf-8", errors="replace"))

        if not body:
            return status, None
        try:
            return status, json.loads(body.decode("utf-8"))
        except json.JSONDecodeError:
            return status, body.decode("utf-8", errors="replace")


def repository_path(repository: str, suffix: str = "") -> str:
    owner, name = repository.split("/", 1)
    return f"/repos/{urllib.parse.quote(owner)}/{urllib.parse.quote(name)}{suffix}"


def apply_repository_settings(api: GitHubApi, repository: str) -> dict[str, Any]:
    path = repository_path(repository)
    _, repo = api.request("GET", path)
    if repo.get("private", True):
        raise RuntimeError(
            "The repository is private. This script records the maintainer's explicit "
            "decision to keep Alpine public and will not change visibility automatically."
        )

    print("Applying merge and security-analysis settings...")
    _, updated = api.request(
        "PATCH",
        path,
        {
            "allow_squash_merge": True,
            "allow_merge_commit": False,
            "allow_rebase_merge": False,
            "allow_auto_merge": False,
            "delete_branch_on_merge": True,
            "squash_merge_commit_title": "PR_TITLE",
            "squash_merge_commit_message": "PR_BODY",
            "security_and_analysis": {
                "secret_scanning": {"status": "enabled"},
                "secret_scanning_push_protection": {"status": "enabled"},
            },
        },
    )
    return updated


def apply_actions_settings(api: GitHubApi, repository: str) -> None:
    base = repository_path(repository, "/actions/permissions")
    print("Applying read-only Actions token defaults...")
    api.request(
        "PUT",
        f"{base}/workflow",
        {
            "default_workflow_permissions": "read",
            "can_approve_pull_request_reviews": False,
        },
    )

    print("Restricting third-party Actions and requiring immutable SHA pins...")
    api.request(
        "PUT",
        base,
        {
            "enabled": True,
            "allowed_actions": "selected",
            "sha_pinning_required": True,
        },
    )
    api.request(
        "PUT",
        f"{base}/selected-actions",
        {
            "github_owned_allowed": True,
            "verified_allowed": False,
            "patterns_allowed": [],
        },
    )


def apply_security_features(api: GitHubApi, repository: str) -> None:
    base = repository_path(repository)
    print("Enabling private vulnerability reporting...")
    api.request("PUT", f"{base}/private-vulnerability-reporting")

    print("Enabling Dependabot alerts...")
    api.request("PUT", f"{base}/vulnerability-alerts")

    print("Enabling Dependabot security updates...")
    api.request("PUT", f"{base}/automated-security-fixes")

    print("Configuring GitHub CodeQL default setup...")
    api.request(
        "PATCH",
        f"{base}/code-scanning/default-setup",
        {
            "state": "configured",
            "query_suite": "extended",
            "threat_model": "remote_and_local",
            "runner_type": "standard",
        },
        expected=(200, 202),
    )


def apply_branch_protection(api: GitHubApi, repository: str) -> None:
    path = repository_path(repository, "/branches/main/protection")
    print("Protecting main with pull requests, required checks, and admin enforcement...")
    api.request(
        "PUT",
        path,
        {
            "required_status_checks": {
                "strict": True,
                "contexts": sorted(REQUIRED_CHECKS),
            },
            "enforce_admins": True,
            # Zero approvals avoids deadlocking a solo maintainer while still requiring
            # the pull-request path, checks, and resolved conversations.
            "required_pull_request_reviews": {
                "dismiss_stale_reviews": False,
                "require_code_owner_reviews": False,
                "required_approving_review_count": 0,
                "require_last_push_approval": False,
            },
            "restrictions": None,
            "required_linear_history": True,
            "allow_force_pushes": False,
            "allow_deletions": False,
            "block_creations": False,
            "required_conversation_resolution": True,
            "lock_branch": False,
            "allow_fork_syncing": True,
        },
    )


def apply_protected_environments(
    api: GitHubApi, repository: str, owner_id: int
) -> None:
    for environment in PROTECTED_ENVIRONMENTS:
        encoded = urllib.parse.quote(environment, safe="")
        path = repository_path(repository, f"/environments/{encoded}")
        print(f"Protecting {environment!r} environment with human approval...")
        api.request(
            "PUT",
            path,
            {
                "wait_timer": 0,
                # Alpine has one human maintainer. Self-review must remain possible,
                # but an explicit approval step is still required before deployment.
                "prevent_self_review": False,
                "reviewers": [{"type": "User", "id": owner_id}],
                "deployment_branch_policy": {
                    "protected_branches": True,
                    "custom_branch_policies": False,
                },
            },
        )


def nested_enabled(value: Any) -> bool:
    return isinstance(value, dict) and value.get("enabled") is True


def verify_repository(api: GitHubApi, repository: str) -> list[str]:
    failures: list[str] = []
    base = repository_path(repository)

    _, repo = api.request("GET", base)
    expected_repo_values = {
        "private": False,
        "allow_squash_merge": True,
        "allow_merge_commit": False,
        "allow_rebase_merge": False,
        "delete_branch_on_merge": True,
    }
    for key, expected in expected_repo_values.items():
        if repo.get(key) != expected:
            failures.append(f"repository {key} is {repo.get(key)!r}; expected {expected!r}")

    analysis = repo.get("security_and_analysis") or {}
    for feature in ("secret_scanning", "secret_scanning_push_protection"):
        if (analysis.get(feature) or {}).get("status") != "enabled":
            failures.append(f"{feature} is not enabled")

    _, workflow_permissions = api.request("GET", f"{base}/actions/permissions/workflow")
    if workflow_permissions.get("default_workflow_permissions") != "read":
        failures.append("Actions default workflow permissions are not read-only")
    if workflow_permissions.get("can_approve_pull_request_reviews") is not False:
        failures.append("Actions may approve pull-request reviews")

    _, actions_permissions = api.request("GET", f"{base}/actions/permissions")
    if actions_permissions.get("allowed_actions") != "selected":
        failures.append("Actions are not restricted to the selected policy")
    if actions_permissions.get("sha_pinning_required") is not True:
        failures.append("full-SHA pinning is not required for Actions")

    _, protection = api.request("GET", f"{base}/branches/main/protection")
    checks = protection.get("required_status_checks") or {}
    contexts = set(checks.get("contexts") or [])
    contexts.update(check.get("context", "") for check in checks.get("checks") or [])
    contexts.discard("")
    missing_checks = REQUIRED_CHECKS - contexts
    if missing_checks:
        failures.append(f"main is missing required checks: {sorted(missing_checks)}")
    if checks.get("strict") is not True:
        failures.append("main does not require branches to be up to date before merge")
    if not nested_enabled(protection.get("enforce_admins")):
        failures.append("branch protection is not enforced for administrators")
    if protection.get("required_pull_request_reviews") is None:
        failures.append("main does not require changes through pull requests")
    if not nested_enabled(protection.get("required_conversation_resolution")):
        failures.append("review-conversation resolution is not required")
    if nested_enabled(protection.get("allow_force_pushes")):
        failures.append("force pushes are allowed on main")
    if nested_enabled(protection.get("allow_deletions")):
        failures.append("main may be deleted")

    _, private_reporting = api.request("GET", f"{base}/private-vulnerability-reporting")
    if not isinstance(private_reporting, dict) or private_reporting.get("enabled") is not True:
        failures.append("private vulnerability reporting is not enabled")

    # These endpoints return 204 only when the feature is enabled.
    api.request("GET", f"{base}/vulnerability-alerts", expected=(204,))
    api.request("GET", f"{base}/automated-security-fixes", expected=(204,))

    # A public repository's dependency graph should be available. Successfully
    # generating the SBOM verifies the graph rather than inferring it from docs.
    api.request("GET", f"{base}/dependency-graph/sbom", expected=(200,))

    _, default_setup = api.request("GET", f"{base}/code-scanning/default-setup")
    if default_setup.get("state") != "configured":
        failures.append("CodeQL default setup is not configured")
    if default_setup.get("query_suite") != "extended":
        failures.append("CodeQL default setup is not using the extended query suite")
    if default_setup.get("threat_model") != "remote_and_local":
        failures.append("CodeQL default setup is not using the remote-and-local threat model")

    for environment in PROTECTED_ENVIRONMENTS:
        encoded = urllib.parse.quote(environment, safe="")
        _, data = api.request("GET", f"{base}/environments/{encoded}")
        rules = data.get("protection_rules") or []
        if not any(rule.get("type") == "required_reviewers" for rule in rules):
            failures.append(f"{environment} environment lacks required human reviewers")

    return failures


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--repo",
        default=os.environ.get("GITHUB_REPOSITORY", DEFAULT_REPOSITORY),
        help="Repository in owner/name form.",
    )
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--apply", action="store_true", help="Apply settings, then verify.")
    mode.add_argument("--check", action="store_true", help="Read and verify settings only.")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.repo.count("/") != 1:
        print("--repo must use owner/name form", file=sys.stderr)
        return 2

    token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
    if not token:
        print(
            "Set GH_TOKEN (preferred) or GITHUB_TOKEN to an owner-authenticated token. "
            "Never paste the token into an issue, pull request, command argument, or log.",
            file=sys.stderr,
        )
        return 2

    api = GitHubApi(token)
    try:
        if args.apply:
            repo = apply_repository_settings(api, args.repo)
            apply_actions_settings(api, args.repo)
            apply_security_features(api, args.repo)
            apply_branch_protection(api, args.repo)
            apply_protected_environments(api, args.repo, int(repo["owner"]["id"]))

        failures = verify_repository(api, args.repo)
    except (ApiError, RuntimeError, KeyError, TypeError, ValueError) as error:
        print(f"Repository hardening failed: {error}", file=sys.stderr)
        return 1

    if failures:
        print("Repository hardening verification failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1

    print(f"Repository hardening verified for {args.repo}.")
    print("GitHub CodeQL default setup is configured and independently verified.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
