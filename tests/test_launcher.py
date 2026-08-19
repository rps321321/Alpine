from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
import time
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
SUPERVISOR = REPO_ROOT / "runtime" / "scripts" / "launcher-supervisor.ps1"


class LauncherTests(unittest.TestCase):
    def build_fixture_launcher(self, install: Path, *, real_entrypoint: bool = False) -> Path:
        shutil.copytree(REPO_ROOT / "runtime" / "scripts", install / "scripts")
        shutil.copytree(REPO_ROOT / "runtime" / "launcher", install / "launcher")
        fake_script = r"""
param(
    [string]$Project,
    [string]$Profile,
    [switch]$WithVision,
    [switch]$Lean,
    [switch]$FullPrompt,
    [switch]$WithPlugins,
    [switch]$WithSkills,
    [string]$LaunchId,
    [switch]$Supervised
)
Start-Sleep -Milliseconds 600
if ($env:LOCALMODEL_LAUNCHER_FIXTURE_MODE -eq 'fail') {
    throw "fixture PowerShell failure for $Profile; token=fixture-inline-secret"
}
$logs = Join-Path (Split-Path $PSScriptRoot -Parent) 'logs'
New-Item -ItemType Directory -Force -Path $logs | Out-Null
[ordered]@{
    project=$Project
    profile=$Profile
    vision=[bool]$WithVision
    lean=[bool]$Lean
    full_prompt=[bool]$FullPrompt
    plugins=[bool]$WithPlugins
    skills=[bool]$WithSkills
} | ConvertTo-Json -Compress | Set-Content -LiteralPath (Join-Path $logs 'launcher-args.json') -Encoding UTF8
exit 0
"""
        if not real_entrypoint:
            (install / "scripts" / "open-local-opencode.ps1").write_text(
                fake_script,
                encoding="utf-8",
            )
        launcher = install / "Open Local Qwen.exe"
        result = subprocess.run(
            [
                "powershell.exe",
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                str(install / "scripts" / "build-launcher.ps1"),
                "-Output",
                str(launcher),
                "-NoShortcut",
            ],
            capture_output=True,
            text=True,
            check=False,
            timeout=60,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        return launcher

    def test_failure_log_preserves_the_error_and_redacts_secret_values(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            install = Path(directory) / "install"
            project = Path(directory) / "project"
            shutil.copytree(REPO_ROOT / "runtime" / "scripts", install / "scripts")
            project.mkdir()
            launch_id = "0123456789abcdef0123456789abcdef"
            invocation_log = install / "logs" / "launcher-errors" / f"{launch_id}.log"
            failure_log = install / "logs" / "launcher-last-error.log"
            failure_log.parent.mkdir(parents=True)
            failure_log.write_text("stale failure\n", encoding="utf-8")
            environment = os.environ.copy()
            environment["LOCALMODEL_TEST_TOKEN"] = "environment-secret-value"
            environment["LOCALMODEL_SHORT_SECRET"] = "zz"
            environment["LOCALMODEL_LAUNCHER_NO_DIALOG"] = "1"
            failure = "PowerShell startup failed; token=inline-secret Authorization: Bearer bearer-secret short=zz database=postgresql://alice:db-password@localhost/app"
            result = subprocess.run(
                [
                    "powershell.exe",
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                    str(install / "scripts" / SUPERVISOR.name),
                    "-Project",
                    str(project),
                    "-Profile",
                    "stable-16k",
                    "-LaunchId",
                    launch_id,
                    "-DiagnosticFailure",
                    "-DiagnosticFailureMessage",
                    failure,
                ],
                capture_output=True,
                text=True,
                check=False,
                env=environment,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertNotIn("could not publish", result.stderr)
            observed = failure_log.read_text(encoding="utf-8")
            self.assertIn("PowerShell startup failed", observed)
            self.assertIn("<REDACTED>", observed)
            self.assertNotIn("environment-secret-value", observed)
            self.assertNotIn("inline-secret", observed)
            self.assertNotIn("bearer-secret", observed)
            self.assertNotIn("short=zz", observed)
            self.assertNotIn("alice", observed)
            self.assertNotIn("db-password", observed)
            self.assertTrue(invocation_log.is_file())
            self.assertEqual(
                invocation_log.read_text(encoding="utf-8"),
                observed,
            )
            self.assertIn(f"project={project}", observed)

    def test_executable_waits_for_handoff_and_preserves_shortcut_options(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            install = Path(directory) / "install"
            project = Path(directory) / "project"
            project.mkdir(parents=True)
            launcher = self.build_fixture_launcher(install)
            environment = os.environ.copy()
            environment["LOCALMODEL_LAUNCHER_NO_DIALOG"] = "1"

            started = time.monotonic()
            result = subprocess.run(
                [
                    str(launcher),
                    "--project",
                    str(project),
                    "--profile",
                    "turbo-16k",
                    "--vision",
                    "--full-prompt",
                    "--plugins",
                    "--skills",
                ],
                capture_output=True,
                text=True,
                check=False,
                env=environment,
                timeout=10,
            )
            elapsed = time.monotonic() - started
            arguments_path = install / "logs" / "launcher-args.json"
            deadline = time.monotonic() + 5
            while not arguments_path.exists() and time.monotonic() < deadline:
                time.sleep(0.05)

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertGreaterEqual(elapsed, 0.5)
            self.assertTrue(arguments_path.is_file())
            observed = json.loads(arguments_path.read_text(encoding="utf-8-sig"))
            self.assertTrue(Path(observed["project"]).samefile(project))
            self.assertEqual(observed["profile"], "turbo-16k")
            self.assertEqual(
                {name: observed[name] for name in ("vision", "lean", "full_prompt", "plugins", "skills")},
                {"vision": True, "lean": False, "full_prompt": True, "plugins": True, "skills": True},
            )

            lean_result = subprocess.run(
                [str(launcher), "--project", str(project), "--profile", "stable-16k", "--lean"],
                capture_output=True,
                text=True,
                check=False,
                env=environment,
                timeout=10,
            )
            self.assertEqual(lean_result.returncode, 0, lean_result.stderr)
            lean_observed = json.loads(arguments_path.read_text(encoding="utf-8-sig"))
            self.assertTrue(lean_observed["lean"])
            self.assertFalse(lean_observed["full_prompt"])

    def test_executable_reports_a_nonzero_child_at_the_stable_log_path(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            install = Path(directory) / "install"
            project = Path(directory) / "project"
            project.mkdir(parents=True)
            launcher = self.build_fixture_launcher(install)
            environment = os.environ.copy()
            environment["LOCALMODEL_LAUNCHER_NO_DIALOG"] = "1"
            environment["LOCALMODEL_LAUNCHER_FIXTURE_MODE"] = "fail"

            result = subprocess.run(
                [str(launcher), "--project", str(project), "--profile", "stable-16k"],
                capture_output=True,
                text=True,
                check=False,
                env=environment,
                timeout=10,
            )

            self.assertNotEqual(result.returncode, 0)
            failure_log = install / "logs" / "launcher-last-error.log"
            self.assertTrue(failure_log.is_file())
            observed = failure_log.read_text(encoding="utf-8")
            self.assertIn("fixture PowerShell failure", observed)
            self.assertIn("profile=stable-16k", observed)
            self.assertNotIn("fixture-inline-secret", observed)
            invocation_logs = list((install / "logs" / "launcher-errors").glob("*.log"))
            self.assertEqual(len(invocation_logs), 1)
            self.assertEqual(invocation_logs[0].read_text(encoding="utf-8"), observed)

    def test_concurrent_failures_keep_distinct_per_launch_records(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            install = Path(directory) / "install"
            project = Path(directory) / "project"
            project.mkdir(parents=True)
            launcher = self.build_fixture_launcher(install)
            environment = os.environ.copy()
            environment["LOCALMODEL_LAUNCHER_NO_DIALOG"] = "1"
            environment["LOCALMODEL_LAUNCHER_FIXTURE_MODE"] = "fail"

            processes = [
                subprocess.Popen(
                    [str(launcher), "--project", str(project), "--profile", profile],
                    env=environment,
                )
                for profile in ("stable-16k", "turbo-16k")
            ]
            try:
                return_codes = [process.wait(timeout=10) for process in processes]
            finally:
                for process in processes:
                    if process.poll() is None:
                        process.kill()
                    process.wait(timeout=5)

            self.assertTrue(all(code != 0 for code in return_codes))
            invocation_logs = list((install / "logs" / "launcher-errors").glob("*.log"))
            self.assertEqual(len(invocation_logs), 2)
            observed = [path.read_text(encoding="utf-8") for path in invocation_logs]
            self.assertTrue(any("failure for stable-16k" in text for text in observed), observed)
            self.assertTrue(any("failure for turbo-16k" in text for text in observed), observed)
            stable = (install / "logs" / "launcher-last-error.log").read_text(encoding="utf-8")
            self.assertIn(stable, observed)

    def test_unwritable_last_project_state_does_not_block_launch(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            install = Path(directory) / "install"
            project = Path(directory) / "project"
            project.mkdir(parents=True)
            launcher = self.build_fixture_launcher(install)
            environment = os.environ.copy()
            environment["LOCALMODEL_LAUNCHER_NO_DIALOG"] = "1"
            (install / "config" / "launcher-last-project.txt").mkdir(parents=True)

            process = subprocess.Popen(
                [str(launcher), "--project", str(project), "--profile", "stable-16k"],
                env=environment,
            )
            try:
                return_code = process.wait(timeout=5)
            finally:
                if process.poll() is None:
                    process.kill()
                process.wait(timeout=5)

            self.assertEqual(return_code, 0)
            arguments_path = install / "logs" / "launcher-args.json"
            self.assertTrue(arguments_path.is_file())

    def test_early_error_honors_no_dialog_mode(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            install = Path(directory) / "install"
            missing_project = Path(directory) / "missing-project"
            launcher = self.build_fixture_launcher(install)
            environment = os.environ.copy()
            environment["LOCALMODEL_LAUNCHER_NO_DIALOG"] = "1"

            process = subprocess.Popen(
                [str(launcher), "--project", str(missing_project), "--profile", "stable-16k"],
                env=environment,
            )
            try:
                return_code = process.wait(timeout=5)
            finally:
                if process.poll() is None:
                    process.kill()
                process.wait(timeout=5)

            self.assertNotEqual(return_code, 0)

    def test_executable_preserves_an_actual_early_powershell_error(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            install = Path(directory) / "install"
            project = Path(directory) / "project"
            project.mkdir(parents=True)
            launcher = self.build_fixture_launcher(install, real_entrypoint=True)
            environment = os.environ.copy()
            environment["LOCALMODEL_LAUNCHER_NO_DIALOG"] = "1"

            result = subprocess.run(
                [str(launcher), "--project", str(project), "--profile", "stable-16k"],
                capture_output=True,
                text=True,
                check=False,
                env=environment,
                timeout=10,
            )

            self.assertNotEqual(result.returncode, 0)
            observed = (install / "logs" / "launcher-last-error.log").read_text(encoding="utf-8")
            self.assertIn("Session config missing", observed)
            self.assertIn("profile=stable-16k", observed)

    def test_supervisor_captures_a_corrupt_entrypoint_parser_error(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            install = Path(directory) / "install"
            project = Path(directory) / "project"
            project.mkdir(parents=True)
            launcher = self.build_fixture_launcher(install, real_entrypoint=True)
            (install / "scripts" / "open-local-opencode.ps1").write_text(
                "param([string]$Project)\nif ($true) {\n",
                encoding="utf-8",
            )
            environment = os.environ.copy()
            environment["LOCALMODEL_LAUNCHER_NO_DIALOG"] = "1"

            result = subprocess.run(
                [str(launcher), "--project", str(project), "--profile", "stable-16k"],
                capture_output=True,
                text=True,
                check=False,
                env=environment,
                timeout=10,
            )

            self.assertNotEqual(result.returncode, 0)
            observed = (install / "logs" / "launcher-last-error.log").read_text(encoding="utf-8")
            self.assertIn("Missing closing", observed)

    def test_executable_installed_diagnostic_failure_uses_the_supervised_path(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            install = Path(directory) / "install"
            project = Path(directory) / "project"
            project.mkdir(parents=True)
            launcher = self.build_fixture_launcher(install)
            environment = os.environ.copy()
            environment["LOCALMODEL_LAUNCHER_NO_DIALOG"] = "1"

            result = subprocess.run(
                [str(launcher), "--project", str(project), "--diagnostic-failure"],
                capture_output=True,
                text=True,
                check=False,
                env=environment,
                timeout=10,
            )

            self.assertNotEqual(result.returncode, 0)
            failure_log = install / "logs" / "launcher-last-error.log"
            self.assertTrue(failure_log.is_file())
            observed = failure_log.read_text(encoding="utf-8")
            self.assertIn("Deterministic installed launcher diagnostic failure", observed)
            self.assertNotIn("Session config missing", observed)


if __name__ == "__main__":
    unittest.main()
