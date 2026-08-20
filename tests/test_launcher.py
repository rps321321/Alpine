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
    def test_minimal_cmd_forwards_directly_to_alpine(self) -> None:
        command = (REPO_ROOT / "runtime" / "launcher" / "Open Minimal OpenCode.cmd").read_text(
            encoding="utf-8"
        )
        self.assertIn("alpine.exe\" opencode", command)
        self.assertNotIn("powershell", command.lower())

    def test_adapter_fallback_uses_the_rust_failure_log_lock(self) -> None:
        source = (REPO_ROOT / "runtime" / "launcher" / "OpenLocalQwen.cs").read_text(
            encoding="utf-8"
        )
        self.assertIn('"launcher-failure-log.lock"', source)
        self.assertNotIn("OpenLocalQwenAdapterFailureLog", source)
        rust_record_branch = source[source.index("if (File.Exists(invocationLog))") :]
        rust_record_branch = rust_record_branch[: rust_record_branch.index("else")]
        self.assertNotIn("PublishStableFailure", rust_record_branch)

    def build_fixture_launcher(
        self,
        install: Path,
        *,
        real_entrypoint: bool = False,
        rust_owned: bool = False,
    ) -> Path:
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
        if rust_owned or real_entrypoint:
            alpine = REPO_ROOT / "target" / "debug" / "alpine.exe"
            if not alpine.is_file():
                build = subprocess.run(
                    ["cargo", "build", "--bin", "alpine"],
                    cwd=REPO_ROOT,
                    capture_output=True,
                    text=True,
                    check=False,
                    timeout=180,
                )
                self.assertEqual(build.returncode, 0, build.stderr)
            shutil.copy2(alpine, install / "alpine.exe")
        else:
            self.build_fake_alpine(install)
        return launcher

    def build_fake_alpine(self, install: Path) -> None:
        source = install / "FakeAlpine.cs"
        source.write_text(
            r'''
using System;
using System.IO;
using System.Text;
using System.Threading;

internal static class FakeAlpine
{
    private static string ValueAfter(string[] args, string name)
    {
        for (int i = 0; i + 1 < args.Length; i++) if (args[i] == name) return args[i + 1];
        return null;
    }
    private static bool Has(string[] args, string name)
    {
        foreach (string arg in args) if (arg == name) return true;
        return false;
    }
    private static string Escape(string value)
    {
        return value.Replace("\\", "\\\\").Replace("\"", "\\\"");
    }
    public static int Main(string[] args)
    {
        Thread.Sleep(600);
        string root = ValueAfter(args, "--install-root") ?? AppDomain.CurrentDomain.BaseDirectory;
        string project = ValueAfter(args, "--project") ?? root;
        string profile = ValueAfter(args, "--profile") ?? "stable-16k";
        string launchId = ValueAfter(args, "--launch-id") ?? Guid.NewGuid().ToString("N");
        string logs = Path.Combine(root, "logs");
        Directory.CreateDirectory(logs);
        string observed = "{\"project\":\"" + Escape(project) + "\",\"profile\":\"" + Escape(profile) +
            "\",\"vision\":" + Has(args, "--vision").ToString().ToLowerInvariant() +
            ",\"lean\":" + Has(args, "--lean").ToString().ToLowerInvariant() +
            ",\"full_prompt\":" + Has(args, "--full-prompt").ToString().ToLowerInvariant() +
            ",\"plugins\":" + Has(args, "--plugins").ToString().ToLowerInvariant() +
            ",\"skills\":" + Has(args, "--skills").ToString().ToLowerInvariant() + "}";
        File.WriteAllText(Path.Combine(logs, "launcher-args.json"), observed, new UTF8Encoding(false));
        bool diagnostic = Has(args, "--diagnostic-failure");
        string fixtureMode = Environment.GetEnvironmentVariable("LOCALMODEL_LAUNCHER_FIXTURE_MODE");
        if (fixtureMode == "exit-without-log") return 23;
        bool failure = diagnostic || fixtureMode == "fail";
        if (!failure) return 0;
        string message = diagnostic
            ? "Deterministic installed launcher diagnostic failure"
            : "fixture PowerShell failure for " + profile;
        string content = "profile=" + profile + Environment.NewLine + "error:" + Environment.NewLine + message + Environment.NewLine;
        string errors = Path.Combine(logs, "launcher-errors");
        Directory.CreateDirectory(errors);
        File.WriteAllText(Path.Combine(errors, launchId + ".log"), content, new UTF8Encoding(false));
        using (Mutex mutex = new Mutex(false, @"Local\FakeAlpineFailureLog"))
        {
            bool acquired = false;
            try
            {
                try { acquired = mutex.WaitOne(TimeSpan.FromSeconds(5)); }
                catch (AbandonedMutexException) { acquired = true; }
                if (acquired) File.WriteAllText(Path.Combine(logs, "launcher-last-error.log"), content, new UTF8Encoding(false));
            }
            finally { if (acquired) mutex.ReleaseMutex(); }
        }
        return 1;
    }
}
''',
            encoding="utf-8",
        )
        output = install / "alpine.exe"
        expression = (
            f"Add-Type -TypeDefinition (Get-Content -Raw -LiteralPath '{source}') "
            f"-Language CSharp -ReferencedAssemblies @('System.dll') "
            f"-OutputAssembly '{output}' -OutputType ConsoleApplication"
        )
        result = subprocess.run(
            ["powershell.exe", "-NoProfile", "-Command", expression],
            capture_output=True,
            text=True,
            check=False,
            timeout=60,
        )
        self.assertEqual(result.returncode, 0, result.stderr)

    def build_fake_opencode(self, install: Path) -> Path:
        fixture_bin = install / "fixture-bin"
        fixture_bin.mkdir(parents=True, exist_ok=True)
        source = fixture_bin / "FakeOpenCode.cs"
        source.write_text(
            r'''
using System;

internal static class FakeOpenCode
{
    public static int Main(string[] args)
    {
        foreach (string arg in args)
        {
            if (arg == "--version")
            {
                Console.WriteLine("1.18.18");
                return 0;
            }
        }
        for (int i = 0; i + 1 < args.Length; i++)
        {
            if (args[i] == "debug" && args[i + 1] == "config")
            {
                Console.Write(Environment.GetEnvironmentVariable("OPENCODE_CONFIG_CONTENT") ?? "{}");
                return 0;
            }
        }
        return 64;
    }
}
''',
            encoding="utf-8",
        )
        output = fixture_bin / "opencode.exe"
        expression = (
            f"Add-Type -TypeDefinition (Get-Content -Raw -LiteralPath '{source}') "
            f"-Language CSharp -ReferencedAssemblies @('System.dll') "
            f"-OutputAssembly '{output}' -OutputType ConsoleApplication"
        )
        result = subprocess.run(
            ["powershell.exe", "-NoProfile", "-Command", expression],
            capture_output=True,
            text=True,
            check=False,
            timeout=60,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        return fixture_bin

    def write_rust_check_fixture(self, install: Path) -> None:
        (install / "config").mkdir(parents=True, exist_ok=True)
        (install / "profiles").mkdir(parents=True, exist_ok=True)
        (install / "runtime").mkdir(parents=True, exist_ok=True)
        (install / "models").mkdir(parents=True, exist_ok=True)
        runtime = install / "runtime" / "llama-server.exe"
        runtime.write_bytes(b"fixture runtime")
        (install / "config" / "api-key.txt").write_text("sk-local-fixture", encoding="utf-8")
        (install / "config" / "base-url.txt").write_text(
            "http://127.0.0.1:8100/v1", encoding="utf-8"
        )
        (install / "profiles" / "stable-16k.json").write_text(
            json.dumps(
                {
                    "name": "stable-16k",
                    "status": "production",
                    "runtime": "official",
                    "context": 16384,
                    "output": 4096,
                    "parallel": 1,
                    "threads": 1,
                    "batch_size": 32,
                    "ubatch_size": 16,
                    "kv_cache": "f16",
                    "tensor_cpu_through_block": 0,
                    "mtp_depth": 1,
                    "ngram_mod": False,
                    "ngram_reset_on_begin": False,
                    "external_skills": False,
                    "skill_tool": False,
                    "vision_fit": False,
                    "fit_target_mib": 1,
                }
            ),
            encoding="utf-8",
        )
        (install / "config" / "session.json").write_text(
            json.dumps(
                {
                    "schema": 3,
                    "root": str(install),
                    "host": "127.0.0.1",
                    "port": 8100,
                    "active_profile": "stable-16k",
                    "runtimes": {"official": str(runtime)},
                    "model": str(install / "models" / "model.gguf"),
                    "mmproj": str(install / "models" / "mmproj.gguf"),
                    "chat_template": str(install / "config" / "chat.jinja"),
                    "api_key_file": str(install / "config" / "api-key.txt"),
                    "base_url_file": str(install / "config" / "base-url.txt"),
                    "state_file": str(install / "logs" / "session-state.json"),
                    "cleanup": {"enabled": False},
                }
            ),
            encoding="utf-8",
        )

    def test_executable_uses_the_rust_owned_model_free_check_when_installed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            install = Path(directory) / "install"
            launcher = self.build_fixture_launcher(install, rust_owned=True)
            self.write_rust_check_fixture(install)
            fixture_bin = self.build_fake_opencode(install)
            environment = os.environ.copy()
            environment["LOCALMODEL_LAUNCHER_NO_DIALOG"] = "1"
            environment["PATH"] = str(fixture_bin) + os.pathsep + environment.get("PATH", "")

            result = subprocess.run(
                [str(launcher), "--check", "--profile", "stable-16k"],
                capture_output=True,
                text=True,
                check=False,
                env=environment,
                timeout=30,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            check_log = (install / "logs" / "launcher-check.log").read_text(encoding="utf-8")
            self.assertIn("OpenCode check passed: stable-16k context=16384", check_log)

    def test_executable_uses_rust_redacted_failure_records_when_installed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            install = Path(directory) / "install"
            project = Path(directory) / "project"
            project.mkdir(parents=True)
            launcher = self.build_fixture_launcher(install, rust_owned=True)
            environment = os.environ.copy()
            environment["LOCALMODEL_LAUNCHER_NO_DIALOG"] = "1"

            result = subprocess.run(
                [str(launcher), "--project", str(project), "--diagnostic-failure"],
                capture_output=True,
                text=True,
                check=False,
                env=environment,
                timeout=30,
            )

            self.assertNotEqual(result.returncode, 0)
            observed = (install / "logs" / "launcher-last-error.log").read_text(encoding="utf-8")
            self.assertIn("Deterministic installed launcher diagnostic failure", observed)

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

    def test_executable_preserves_an_actual_early_rust_error(self) -> None:
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
            self.assertIn("Session Config missing", observed)
            self.assertIn("profile=stable-16k", observed)

    def test_adapter_records_a_corrupt_alpine_binary_without_exposing_details(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            install = Path(directory) / "install"
            project = Path(directory) / "project"
            project.mkdir(parents=True)
            launcher = self.build_fixture_launcher(install, real_entrypoint=True)
            (install / "alpine.exe").write_bytes(b"not a Windows executable")
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
            self.assertIn("could not start Alpine", observed)
            self.assertIn("Win32Exception", observed)

    def test_adapter_failure_does_not_persist_an_untrusted_profile_value(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            install = Path(directory) / "install"
            project = Path(directory) / "project"
            project.mkdir(parents=True)
            launcher = self.build_fixture_launcher(install, real_entrypoint=True)
            (install / "alpine.exe").write_bytes(b"not a Windows executable")
            environment = os.environ.copy()
            environment["LOCALMODEL_LAUNCHER_NO_DIALOG"] = "1"
            untrusted_profile = "DATABASE_URL=postgresql://user:secret@host/db"

            result = subprocess.run(
                [str(launcher), "--project", str(project), "--profile", untrusted_profile],
                capture_output=True,
                text=True,
                check=False,
                env=environment,
                timeout=10,
            )

            self.assertNotEqual(result.returncode, 0)
            observed = (install / "logs" / "launcher-last-error.log").read_text(encoding="utf-8")
            self.assertIn("profile=<invalid>", observed)
            self.assertNotIn(untrusted_profile, observed)
            self.assertNotIn("secret", observed)

    def test_nonzero_child_without_its_own_record_publishes_a_stable_failure(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            install = Path(directory) / "install"
            project = Path(directory) / "project"
            project.mkdir(parents=True)
            launcher = self.build_fixture_launcher(install)
            environment = os.environ.copy()
            environment["LOCALMODEL_LAUNCHER_NO_DIALOG"] = "1"
            environment["LOCALMODEL_LAUNCHER_FIXTURE_MODE"] = "exit-without-log"

            result = subprocess.run(
                [str(launcher), "--project", str(project), "--profile", "stable-16k"],
                capture_output=True,
                text=True,
                check=False,
                env=environment,
                timeout=10,
            )

            self.assertEqual(result.returncode, 23)
            failure_log = install / "logs" / "launcher-last-error.log"
            self.assertTrue(failure_log.is_file())
            observed = failure_log.read_text(encoding="utf-8")
            self.assertIn("Alpine exited with code 23", observed)
            self.assertIn("without producing its per-launch failure record", observed)
            invocation_logs = list((install / "logs" / "launcher-errors").glob("*.log"))
            self.assertEqual(len(invocation_logs), 1)
            self.assertEqual(invocation_logs[0].read_text(encoding="utf-8"), observed)

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
