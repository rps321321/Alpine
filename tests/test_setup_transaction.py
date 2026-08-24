from __future__ import annotations

import json
import hashlib
import subprocess
import tempfile
import time
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
MODULE = REPO_ROOT / "runtime" / "scripts" / "setup-transaction.ps1"
SETUP_SCRIPT = REPO_ROOT / "scripts" / "setup-local-qwen.ps1"


def invoke(expression: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["powershell.exe", "-NoProfile", "-Command", f". '{MODULE}'; {expression}"],
        capture_output=True,
        text=True,
        check=False,
    )


class SetupTransactionTests(unittest.TestCase):
    def test_native_version_probe_is_powershell_51_safe(self) -> None:
        result = invoke(
            "$version=Get-NativeVersionText (Join-Path $env:SystemRoot 'System32\\cmd.exe') '/d /c ver' 5000;"
            "$version"
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Microsoft Windows", result.stdout)

        setup_source = SETUP_SCRIPT.read_text(encoding="utf-8-sig")
        package_source = (
            REPO_ROOT / "scripts" / "package-custom-runtime.ps1"
        ).read_text(encoding="utf-8-sig")
        self.assertNotIn("--version 2>&1", setup_source)
        self.assertNotIn("--version 2>&1", package_source)

    def test_setup_uses_manifest_bytes_for_download_publication(self) -> None:
        source = SETUP_SCRIPT.read_text(encoding="utf-8-sig")
        self.assertIn(
            "Publish-VerifiedDownload $partial $destination ([long]$Artifact.bytes)",
            source,
        )
        self.assertNotIn("([long]$Artifact.size)", source)
        self.assertLess(
            source.index("Publish-CompletedPartialDownload"),
            source.index("& curl.exe"),
        )

    def test_staged_session_config_contains_only_final_installation_paths(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            install = Path(directory) / "install"
            stage = install / ".stage"
            expression = (
                f"$config=New-SessionConfigDocument -InstallRoot '{install}' -ProfileName stable-16k "
                f"-ProfileRuntime official -OfficialServer '{install / 'runtime-official' / 'llama-server.exe'}' "
                f"-CustomServer '' -ModelPath '{install / 'models' / 'model.gguf'}' "
                f"-MmprojPath '{install / 'models' / 'mmproj.gguf'}' -ChatTemplatePath '{install / 'config' / 'chat.jinja'}' "
                "-Cleanup ([ordered]@{enabled=$false}); $config | ConvertTo-Json -Depth 5 -Compress"
            )
            result = invoke(expression)
            self.assertEqual(result.returncode, 0, result.stderr)
            config = json.loads(result.stdout)
            self.assertEqual(config["schema"], 5)
            self.assertNotIn("active_profile", config)
            self.assertNotIn("llama_server", config)
            self.assertEqual(Path(config["api_key_file"]), install / "config" / "api-key.txt")
            self.assertEqual(Path(config["base_url_file"]), install / "config" / "base-url.txt")
            self.assertNotIn(str(stage), result.stdout)

    def test_setup_drops_retired_fields_from_a_disabled_cleanup_handoff(self) -> None:
        result = invoke(
            "$cleanup=Get-PreservedCleanupConfig ([pscustomobject]@{enabled=$false;port=9191;"
            "exe='cleanup.exe';start_script='start.ps1';health='http://cleanup/health'});"
            "$cleanup | ConvertTo-Json -Compress"
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        cleanup = json.loads(result.stdout)
        self.assertEqual(cleanup, {"enabled": False})

    def test_setup_preserves_typed_cleanup_launch_data_without_a_script(self) -> None:
        result = invoke(
            "$cleanup=Get-PreservedCleanupConfig ([pscustomobject]@{enabled=$true;port=9191;"
            "executable='C:\\fixture\\llama-server.exe';arguments=@('--host','127.0.0.1','--port','9191');"
            "stdout='C:\\fixture\\logs\\cleanup-out.log';stderr='C:\\fixture\\logs\\cleanup-err.log';"
            "health='http://127.0.0.1:9191/v1/models'});$cleanup | ConvertTo-Json -Depth 5 -Compress"
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        cleanup = json.loads(result.stdout)
        self.assertEqual(cleanup["executable"], r"C:\fixture\llama-server.exe")
        self.assertEqual(cleanup["arguments"], ["--host", "127.0.0.1", "--port", "9191"])
        self.assertEqual(cleanup["stdout"], r"C:\fixture\logs\cleanup-out.log")
        self.assertEqual(cleanup["stderr"], r"C:\fixture\logs\cleanup-err.log")
        self.assertNotIn("exe", cleanup)
        self.assertNotIn("start_script", cleanup)

    def test_setup_rejects_enabled_legacy_cleanup_without_guessing_argv(self) -> None:
        result = invoke(
            "$cleanup=Get-PreservedCleanupConfig ([pscustomobject]@{enabled=$true;port=9191;"
            "exe='C:\\fixture\\cleanup.exe';start_script='C:\\fixture\\start.ps1';"
            "health='http://127.0.0.1:9191/health'})"
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("retired exe/start_script contract", result.stderr)

    def test_interrupted_download_remains_resumable_and_is_not_published_early(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            partial = root / "artifact.bin.part"
            destination = root / "artifact.bin"
            complete = b"complete-artifact"
            partial.write_bytes(complete[:5])
            digest = hashlib.sha256(complete).hexdigest()

            incomplete = invoke(
                f"Publish-VerifiedDownload '{partial}' '{destination}' {len(complete)} '{digest}'"
            )
            self.assertNotEqual(incomplete.returncode, 0)
            self.assertTrue(partial.is_file())
            self.assertFalse(destination.exists())

            partial.write_bytes(complete)
            resumed = invoke(
                f"Publish-VerifiedDownload '{partial}' '{destination}' {len(complete)} '{digest}'"
            )
            self.assertEqual(resumed.returncode, 0, resumed.stderr)
            self.assertEqual(destination.read_bytes(), complete)
            self.assertFalse(partial.exists())

    def test_complete_download_with_bad_checksum_is_quarantined_before_retry(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            partial = root / "artifact.bin.part"
            destination = root / "artifact.bin"
            complete = b"complete-artifact"
            partial.write_bytes(b"x" * len(complete))
            digest = hashlib.sha256(complete).hexdigest()

            rejected = invoke(
                f"Publish-VerifiedDownload '{partial}' '{destination}' {len(complete)} '{digest}'"
            )
            self.assertNotEqual(rejected.returncode, 0)
            self.assertFalse(partial.exists())
            quarantined = list(root.glob("artifact.bin.part.invalid-*"))
            self.assertEqual(len(quarantined), 1)
            self.assertEqual(quarantined[0].read_bytes(), b"x" * len(complete))
            self.assertFalse(destination.exists())

            partial.write_bytes(complete)
            resumed = invoke(
                f"Publish-VerifiedDownload '{partial}' '{destination}' {len(complete)} '{digest}'"
            )
            self.assertEqual(resumed.returncode, 0, resumed.stderr)
            self.assertEqual(destination.read_bytes(), complete)

    def test_existing_complete_corrupt_partial_is_quarantined_before_curl_resume(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            partial = root / "artifact.bin.part"
            destination = root / "artifact.bin"
            complete = b"complete-artifact"
            partial.write_bytes(b"x" * len(complete))
            digest = hashlib.sha256(complete).hexdigest()

            result = invoke(
                f"Publish-CompletedPartialDownload '{partial}' '{destination}' "
                f"{len(complete)} '{digest}' | ConvertTo-Json -Compress"
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout.splitlines()[-1]), False)
            self.assertFalse(partial.exists())
            self.assertEqual(len(list(root.glob("artifact.bin.part.invalid-*"))), 1)
            self.assertFalse(destination.exists())

    def test_bundle_publication_replaces_all_items_and_removes_marker(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            install, stage = root / "install", root / "install" / ".stage"
            (install / "scripts").mkdir(parents=True)
            (install / "scripts" / "version.txt").write_text("old", encoding="utf-8")
            (stage / "scripts").mkdir(parents=True)
            (stage / "scripts" / "version.txt").write_text("new", encoding="utf-8")
            (stage / "config").mkdir()
            (stage / "config" / "identity.json").write_text("new-id", encoding="utf-8")
            expression = (
                f"$items=@([pscustomobject]@{{stage='scripts';destination='scripts'}},"
                "[pscustomobject]@{stage='config\\identity.json';destination='config\\identity.json'});"
                f"Publish-SetupBundle -InstallRoot '{install}' -StageRoot '{stage}' -Items $items"
            )
            result = invoke(expression)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual((install / "scripts" / "version.txt").read_text(), "new")
            self.assertEqual((install / "config" / "identity.json").read_text(), "new-id")
            self.assertFalse((install / ".setup-publishing.json").exists())

    def test_mid_publication_failure_rolls_back_prior_installation(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            install, stage = root / "install", root / "install" / ".stage"
            (install / "runtime-official").mkdir(parents=True)
            (install / "runtime-official" / "version.txt").write_text("old-runtime", encoding="utf-8")
            (install / "scripts").mkdir(parents=True)
            (install / "scripts" / "version.txt").write_text("old", encoding="utf-8")
            (install / "blocked").write_text("parent-is-a-file", encoding="utf-8")
            (stage / "runtime-official").mkdir(parents=True)
            (stage / "runtime-official" / "version.txt").write_text("new-runtime", encoding="utf-8")
            (stage / "scripts").mkdir(parents=True)
            (stage / "scripts" / "version.txt").write_text("new", encoding="utf-8")
            (stage / "second.txt").write_text("new-second", encoding="utf-8")
            expression = (
                f"$items=@([pscustomobject]@{{stage='runtime-official';destination='runtime-official'}},"
                "[pscustomobject]@{stage='scripts';destination='scripts'},"
                "[pscustomobject]@{stage='second.txt';destination='blocked\\second.txt'});"
                f"Publish-SetupBundle -InstallRoot '{install}' -StageRoot '{stage}' -Items $items"
            )
            result = invoke(expression)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(
                (install / "runtime-official" / "version.txt").read_text(), "old-runtime"
            )
            self.assertEqual((install / "scripts" / "version.txt").read_text(), "old")
            self.assertEqual((install / "blocked").read_text(), "parent-is-a-file")
            self.assertFalse((install / ".setup-publishing.json").exists())

    def test_stale_publication_marker_recovers_backup_deterministically(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            install = root / "install"
            backup = install / ".backup"
            stage = install / ".stage"
            (install / "scripts").mkdir(parents=True)
            (install / "scripts" / "version.txt").write_text("partial-new", encoding="utf-8")
            (backup / "scripts").mkdir(parents=True)
            (backup / "scripts" / "version.txt").write_text("prior-good", encoding="utf-8")
            stage.mkdir()
            marker = {
                "schema": 1,
                "backup_root": str(backup),
                "stage_root": str(stage),
                "items": [{"stage": "scripts", "destination": "scripts", "had_prior": True}],
            }
            (install / ".setup-publishing.json").write_text(json.dumps(marker), encoding="utf-8")
            result = invoke(f"Repair-InterruptedSetupPublication '{install}' | ConvertTo-Json -Compress")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual((install / "scripts" / "version.txt").read_text(), "prior-good")
            self.assertFalse((install / ".setup-publishing.json").exists())

    def test_competing_setup_is_bounded_and_owner_death_releases_lock(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            install = Path(directory) / "install"
            ready = Path(directory) / "ready"
            owner_script = (
                f". '{MODULE}'; $lock=Enter-SetupLock '{install}' 1000; "
                f"Set-Content -LiteralPath '{ready}' -Value ready; Start-Sleep -Seconds 30"
            )
            owner = subprocess.Popen(
                ["powershell.exe", "-NoProfile", "-Command", owner_script],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            try:
                deadline = time.monotonic() + 10
                while not ready.exists() and time.monotonic() < deadline:
                    time.sleep(0.05)
                self.assertTrue(ready.exists())
                blocked = invoke(f"$lock=Enter-SetupLock '{install}' 100")
                self.assertNotEqual(blocked.returncode, 0)
                self.assertIn("Another setup transaction owns", blocked.stderr)
                owner.kill()
                owner.wait(timeout=10)
                recovered = invoke(
                    f"$lock=Enter-SetupLock '{install}' 1000; Exit-InterprocessLock $lock; 'recovered'"
                )
                self.assertEqual(recovered.returncode, 0, recovered.stderr)
                self.assertIn("recovered", recovered.stdout)
            finally:
                if owner.poll() is None:
                    owner.kill()
                    owner.wait(timeout=10)


if __name__ == "__main__":
    unittest.main()
