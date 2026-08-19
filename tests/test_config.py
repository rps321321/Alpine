from __future__ import annotations

import json
import subprocess
import tempfile
import unittest
from pathlib import Path

from localmodel.config import ConfigError, resolve_session, select_active_profile


REPO_ROOT = Path(__file__).resolve().parents[1]


def write_fixture(root: Path, *, schema: int = 3, profile: str = "stable-16k", runtime: str = "official") -> None:
    (root / "config").mkdir(parents=True)
    (root / "profiles").mkdir()
    (root / "runtime").mkdir()
    server = root / "runtime" / "llama-server.exe"
    server.write_bytes(b"fixture")
    profile_data = {
        "name": profile,
        "status": "production",
        "runtime": runtime,
        "context": 16384,
        "output": 4096,
        "parallel": 1,
        "threads": 16,
        "batch_size": 2048,
        "ubatch_size": 768,
        "kv_cache": "q8_0",
        "tensor_cpu_through_block": 43,
        "mtp_depth": 3,
        "ngram_mod": False,
        "ngram_reset_on_begin": False,
        "external_skills": False,
        "skill_tool": False,
        "vision_fit": True,
        "fit_target_mib": 512,
    }
    (root / "profiles" / f"{profile}.json").write_text(json.dumps(profile_data), encoding="utf-8")
    session = {
        "schema": schema,
        "root": str(root),
        "host": "127.0.0.1",
        "port": 8123,
        "active_profile": profile,
        "runtimes": {"official": str(server), "custom": None},
        "llama_server": str(server),
        "model": str(root / "models" / "model.gguf"),
        "mmproj": str(root / "models" / "mmproj.gguf"),
        "chat_template": str(root / "config" / "chat.jinja"),
        "api_key_file": str(root / "config" / "api-key.txt"),
        "base_url_file": str(root / "config" / "base-url.txt"),
        "state_file": str(root / "logs" / "session-state.json"),
        "cleanup": {"enabled": False},
    }
    (root / "config" / "session.json").write_text(json.dumps(session), encoding="utf-8")


class SessionConfigTests(unittest.TestCase):
    def invoke_powershell(self, root: Path, profile: str | None = None) -> subprocess.CompletedProcess[str]:
        selected = f" -Name '{profile}'" if profile else ""
        command = (
            f". '{REPO_ROOT / 'runtime' / 'scripts' / 'lib.ps1'}'; "
            f"Get-ResolvedSession -InstallRoot '{root}'{selected} -RequireRuntime | Out-Null"
        )
        return subprocess.run(
            ["powershell.exe", "-NoProfile", "-Command", command],
            capture_output=True,
            text=True,
            check=False,
        )

    def test_python_and_powershell_resolve_the_same_domain_values(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_fixture(root)
            resolved = resolve_session(root, require_runtime=True)

            command = (
                f". '{REPO_ROOT / 'runtime' / 'scripts' / 'lib.ps1'}'; "
                f"Get-ResolvedSession -InstallRoot '{root}' | ConvertTo-Json -Depth 8 -Compress"
            )
            result = subprocess.run(
                ["powershell.exe", "-NoProfile", "-Command", command],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            powershell = json.loads(result.stdout)
            self.assertEqual(powershell["ProfileName"], resolved.profile_name)
            self.assertEqual(Path(powershell["ServerPath"]), resolved.server)
            self.assertEqual(powershell["BaseUrl"], resolved.base_url)
            self.assertEqual(powershell["Profile"]["context"], resolved.profile["context"])

    def test_rejects_unsupported_schema(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_fixture(root, schema=2)
            with self.assertRaisesRegex(ConfigError, "unsupported Session Config schema"):
                resolve_session(root)

    def test_rejects_missing_selected_profile(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_fixture(root)
            with self.assertRaisesRegex(ConfigError, "Profile missing"):
                resolve_session(root, "does-not-exist")

    def test_rejects_unavailable_selected_runtime(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_fixture(root, runtime="custom")
            with self.assertRaisesRegex(ConfigError, "Runtime 'custom' is unavailable"):
                resolve_session(root, require_runtime=True)

    def test_both_adapters_reject_invalid_fixtures_actionably(self) -> None:
        cases = (
            ("schema", lambda root: write_fixture(root, schema=2), None, "Unsupported Session Config schema"),
            ("profile", write_fixture, "does-not-exist", "Profile missing"),
            ("runtime", lambda root: write_fixture(root, runtime="custom"), None, "Runtime 'custom'"),
        )
        for name, arrange, selected, message in cases:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                arrange(root)
                result = self.invoke_powershell(root, selected)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(message, result.stderr)

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_fixture(root)
            session_path = root / "config" / "session.json"
            session = json.loads(session_path.read_text(encoding="utf-8"))
            session["port"] = 0
            session_path.write_text(json.dumps(session), encoding="utf-8")
            with self.assertRaisesRegex(ConfigError, "port"):
                resolve_session(root)
            self.assertIn("port must be between", self.invoke_powershell(root).stderr)

    def test_incomplete_setup_publication_blocks_both_adapters(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_fixture(root)
            (root / ".setup-publishing.json").write_text("{}", encoding="utf-8")
            with self.assertRaisesRegex(ConfigError, "Setup publication is incomplete"):
                resolve_session(root)
            result = self.invoke_powershell(root)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Setup publication is incomplete", result.stderr)

    def test_active_profile_selection_is_validated_atomic_and_recoverable(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_fixture(root)
            candidate = json.loads((root / "profiles" / "stable-16k.json").read_text(encoding="utf-8"))
            candidate["name"] = "candidate-16k"
            candidate["status"] = "candidate"
            (root / "profiles" / "candidate-16k.json").write_text(json.dumps(candidate), encoding="utf-8")

            backup = select_active_profile(root, "candidate-16k")

            self.assertEqual(resolve_session(root).profile_name, "candidate-16k")
            self.assertEqual(json.loads(backup.read_text(encoding="utf-8"))["active_profile"], "stable-16k")
            self.assertFalse(list((root / "config").glob("session.json.*.tmp")))


if __name__ == "__main__":
    unittest.main()
