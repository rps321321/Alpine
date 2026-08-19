from __future__ import annotations

import json
import subprocess
import tempfile
import unittest
from pathlib import Path

from tests.test_config import write_fixture


REPO_ROOT = Path(__file__).resolve().parents[1]
MATRICES = (
    "benchmark-server-matrix.ps1",
    "benchmark-batch-matrix.ps1",
    "benchmark-fit-matrix.ps1",
    "benchmark-kv-matrix.ps1",
    "benchmark-thread-matrix.ps1",
    "benchmark-ubatch-focused.ps1",
    "benchmark-tensor-placement.ps1",
    "benchmark-pr27173.ps1",
    "benchmark-inference.ps1",
)

PORT_WAIT_MATRICES = MATRICES[:-1]


class PerformanceMatrixConfigTests(unittest.TestCase):
    def test_process_matrices_use_the_shared_port_wait(self) -> None:
        for name in PORT_WAIT_MATRICES:
            source = (REPO_ROOT / "scripts" / name).read_text(encoding="utf-8-sig")
            self.assertNotIn("function Wait-PortFree", source, name)
            self.assertIn("Wait-BenchmarkPortFree $benchmark", source, name)

    def test_every_matrix_resolves_a_non_default_install_and_profile(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_fixture(root)
            (root / "models").mkdir()
            (root / "models" / "model.gguf").write_bytes(b"model")
            (root / "models" / "mmproj.gguf").write_bytes(b"vision")
            (root / "config" / "chat.jinja").write_text("template", encoding="utf-8")
            for name in MATRICES:
                script = REPO_ROOT / "scripts" / name
                source = script.read_text(encoding="utf-8-sig")
                self.assertIn("[string]$InstallRoot", source, name)
                self.assertIn("[string]$Profile", source, name)
                self.assertIn("[switch]$ResolveOnly", source, name)
                for forbidden in (
                    "%USERPROFILE%",
                    "8100",
                    "127.0.0.1:8100",
                    "LocalPort 8100",
                    "runtime-legacy",
                    "Qwen3.8-27B-ABLITERATED-Q4_K_M.gguf",
                ):
                    self.assertNotIn(forbidden, source, name)
                result = subprocess.run(
                    [
                        "powershell.exe",
                        "-NoProfile",
                        "-File",
                        str(script),
                        "-InstallRoot",
                        str(root),
                        "-Profile",
                        "stable-16k",
                        "-ResolveOnly",
                    ],
                    capture_output=True,
                    text=True,
                    check=False,
                    timeout=30,
                )
                self.assertEqual(result.returncode, 0, f"{name}: {result.stderr}")
                resolved = json.loads(result.stdout)
                self.assertTrue(Path(resolved["install_root"]).samefile(root))
                self.assertEqual(resolved["profile"], "stable-16k")
                self.assertEqual(resolved["port"], 8123)
                self.assertEqual(resolved["context"], 16384)
                self.assertTrue(Path(resolved["server"]).samefile(root / "runtime" / "llama-server.exe"))


if __name__ == "__main__":
    unittest.main()
