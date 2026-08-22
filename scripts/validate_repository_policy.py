#!/usr/bin/env python3
"""Validate Alpine's repository-controlled GitHub security policy.

This check is intentionally dependency-free so it can run in a clean clone and in
GitHub Actions. It validates files that GitHub repository settings depend on, but
it does not claim to replace an authenticated audit of owner-level settings.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW_DIR = ROOT / ".github" / "workflows"
ADVANCED_CODEQL_WORKFLOW = WORKFLOW_DIR / "codeql.yml"

REQUIRED_FILES = (
    ROOT / ".github" / "CODEOWNERS",
    ROOT / ".github" / "dependabot.yml",
    WORKFLOW_DIR / "verify.yml",
    WORKFLOW_DIR / "project-management-validation.yml",
    WORKFLOW_DIR / "dependency-review.yml",
    ROOT / "SECURITY.md",
    ROOT / "docs" / "REPOSITORY-SETTINGS.md",
    ROOT / "scripts" / "configure_github_repository.py",
)

# Only these narrowly scoped workflows may mutate issue metadata. Any future
# reconciliation workflow must be reviewed and added here deliberately.
ISSUES_WRITE_ALLOWLIST = {
    "reconcile-project-management.yml",
    "sync-issue-readiness.yml",
}

FULL_SHA = re.compile(r"^[0-9a-f]{40}$")
USES_LINE = re.compile(r"^\s*-?\s*uses:\s*([^\s#]+)", re.MULTILINE)
ISSUES_WRITE = re.compile(r"^\s*issues:\s*write\s*$", re.MULTILINE)
EXPLICIT_PERMISSIONS = re.compile(r"^permissions:\s*(?:$|\{)", re.MULTILINE)


def add_error(errors: list[str], path: Path, message: str) -> None:
    errors.append(f"{path.relative_to(ROOT)}: {message}")


def validate_action_pins(path: Path, text: str, errors: list[str]) -> None:
    for match in USES_LINE.finditer(text):
        value = match.group(1).strip("'\"")
        if value.startswith("./") or value.startswith("docker://"):
            continue
        if "@" not in value:
            add_error(errors, path, f"action reference is not pinned: {value}")
            continue
        _, ref = value.rsplit("@", 1)
        if not FULL_SHA.fullmatch(ref):
            add_error(
                errors,
                path,
                f"action reference must use a full 40-character commit SHA: {value}",
            )


def validate_workflow(path: Path, errors: list[str]) -> None:
    text = path.read_text(encoding="utf-8")

    if not EXPLICIT_PERMISSIONS.search(text):
        add_error(errors, path, "workflow must declare explicit top-level permissions")

    if "pull_request_target:" in text:
        add_error(
            errors,
            path,
            "pull_request_target is forbidden without a dedicated security design issue",
        )

    if ISSUES_WRITE.search(text) and path.name not in ISSUES_WRITE_ALLOWLIST:
        add_error(
            errors,
            path,
            "issues: write is allowed only in reviewed reconciliation workflows",
        )

    if re.search(r"(?:release|sign)", path.stem, re.IGNORECASE) and "environment:" not in text:
        add_error(
            errors,
            path,
            "release/signing workflows must use a protected GitHub environment",
        )

    validate_action_pins(path, text, errors)


def require_text(path: Path, fragments: tuple[str, ...], errors: list[str]) -> None:
    if not path.exists():
        return
    text = path.read_text(encoding="utf-8")
    for fragment in fragments:
        if fragment not in text:
            add_error(errors, path, f"missing required policy fragment: {fragment!r}")


def main() -> int:
    errors: list[str] = []

    for path in REQUIRED_FILES:
        if not path.exists():
            add_error(errors, path, "required repository-hardening file is missing")

    # Alpine intentionally uses GitHub's existing CodeQL default setup. GitHub
    # rejects SARIF from an advanced CodeQL workflow while default setup is
    # enabled, so keeping both would create a permanently failing security job.
    if ADVANCED_CODEQL_WORKFLOW.exists():
        add_error(
            errors,
            ADVANCED_CODEQL_WORKFLOW,
            "advanced CodeQL workflow conflicts with Alpine's CodeQL default-setup policy",
        )

    if WORKFLOW_DIR.exists():
        workflows = sorted(
            path
            for path in WORKFLOW_DIR.iterdir()
            if path.is_file() and path.suffix in {".yml", ".yaml"}
        )
        if not workflows:
            add_error(errors, WORKFLOW_DIR, "no GitHub Actions workflows found")
        for workflow in workflows:
            validate_workflow(workflow, errors)

    require_text(
        WORKFLOW_DIR / "verify.yml",
        (
            "name: canonical-verification",
            "name: canonical-verification\n    runs-on:",
            "persist-credentials: false",
            "cargo run --locked --bin alpine-verify",
        ),
        errors,
    )
    require_text(
        WORKFLOW_DIR / "project-management-validation.yml",
        (
            "name: project-management-validation",
            "name: project-management-validation\n    runs-on:",
            "python scripts/validate_repository_policy.py",
        ),
        errors,
    )
    require_text(
        WORKFLOW_DIR / "dependency-review.yml",
        ("actions/dependency-review-action@", "fail-on-severity:"),
        errors,
    )
    require_text(
        ROOT / ".github" / "dependabot.yml",
        (
            "version: 2",
            'package-ecosystem: "cargo"',
            'package-ecosystem: "github-actions"',
        ),
        errors,
    )
    require_text(
        ROOT / "docs" / "REPOSITORY-SETTINGS.md",
        ("GitHub CodeQL default setup", "advanced CodeQL workflow"),
        errors,
    )
    require_text(
        ROOT / "scripts" / "configure_github_repository.py",
        ("/code-scanning/default-setup", '"query_suite": "extended"'),
        errors,
    )
    require_text(
        ROOT / "SECURITY.md",
        ("private vulnerability", "Do not"),
        errors,
    )

    if errors:
        print("Repository policy validation failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1

    print("Repository policy validation passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
