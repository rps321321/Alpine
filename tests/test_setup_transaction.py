from __future__ import annotations

import json
import subprocess
import tempfile
import time
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
MODULE = REPO_ROOT / "runtime" / "scripts" / "setup-transaction.ps1"


def invoke(expression: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["powershell.exe", "-NoProfile", "-Command", f". '{MODULE}'; {expression}"],
        capture_output=True,
        text=True,
        check=False,
    )


class SetupTransactionTests(unittest.TestCase):
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
