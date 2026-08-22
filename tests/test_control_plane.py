from __future__ import annotations

import shutil
import tempfile
import unittest
from pathlib import Path

from localmodel.controlplane import verify_control_plane, write_control_plane_identity


def make_control_plane(root: Path) -> None:
    files = {
        "runtime/scripts/setup-transaction.ps1": "setup-v1",
        "runtime/scripts/build-launcher.ps1": "builder-v1",
        "runtime/launcher/OpenLocalQwen.cs": "launcher-v1",
        "config/profiles/stable-16k.json": '{"name":"stable-16k"}',
        "config/artifacts.json": '{"schema":1}',
    }
    for relative, content in files.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")


def copy_control_plane(repo: Path, install: Path) -> None:
    mappings = (
        (repo / "runtime" / "scripts", install / "scripts"),
        (repo / "runtime" / "launcher", install / "launcher"),
        (repo / "config" / "profiles", install / "profiles"),
    )
    for source, destination in mappings:
        shutil.copytree(source, destination)
    (install / "config").mkdir()
    shutil.copy2(repo / "config" / "artifacts.json", install / "config" / "artifacts.json")


class ControlPlaneIdentityTests(unittest.TestCase):
    def test_exact_install_passes_without_mutation(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo, install = root / "repo", root / "install"
            make_control_plane(repo)
            copy_control_plane(repo, install)
            identity = write_control_plane_identity(repo, install, source_commit="abc123")
            before = (install / "config" / "control-plane.json").read_bytes()

            result = verify_control_plane(repo, install)

            self.assertTrue(result["exact_match"])
            self.assertEqual(result["source_commit"], "abc123")
            self.assertEqual(before, (install / "config" / "control-plane.json").read_bytes())
            recorded = {entry["path"] for entry in identity["files"]}
            self.assertNotIn("config/api-key.txt", recorded)
            self.assertNotIn("logs/session-state.json", recorded)

    def test_reports_missing_modified_and_expected_source_drift(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo, install = root / "repo", root / "install"
            make_control_plane(repo)
            copy_control_plane(repo, install)
            write_control_plane_identity(repo, install)

            (install / "scripts" / "setup-transaction.ps1").unlink()
            (install / "scripts" / "build-launcher.ps1").write_text("locally-modified", encoding="utf-8")
            updated = '{"name":"stable-16k","status":"production"}'
            (repo / "config" / "profiles" / "stable-16k.json").write_text(updated, encoding="utf-8")
            (install / "profiles" / "stable-16k.json").write_text(updated, encoding="utf-8")

            result = verify_control_plane(repo, install)

            self.assertEqual(result["missing"], ["scripts/setup-transaction.ps1"])
            self.assertEqual(result["modified"], ["scripts/build-launcher.ps1"])
            self.assertEqual(result["stale"], ["profiles/stable-16k.json"])
            self.assertFalse(result["exact_match"])

    def test_generated_launcher_is_verified_without_being_source_stale(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo, install = root / "repo", root / "install"
            make_control_plane(repo)
            copy_control_plane(repo, install)
            identity = write_control_plane_identity(repo, install)
            launcher = install / "Open Local Qwen.exe"
            launcher.write_bytes(b"fixture launcher")
            from localmodel.config import sha256

            identity["files"].append(
                {"path": "Open Local Qwen.exe", "sha256": sha256(launcher), "generated": True}
            )
            import json

            (install / "config" / "control-plane.json").write_text(
                json.dumps(identity), encoding="utf-8"
            )
            self.assertTrue(verify_control_plane(repo, install)["exact_match"])
            launcher.write_bytes(b"modified")
            self.assertEqual(
                verify_control_plane(repo, install)["modified"], ["Open Local Qwen.exe"]
            )


if __name__ == "__main__":
    unittest.main()
