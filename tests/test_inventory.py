from __future__ import annotations

import json
import subprocess
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]


class HardwareInventoryTests(unittest.TestCase):
    def test_native_hardware_inventory_can_be_written_atomically(self) -> None:
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
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "nested" / "hardware.json"
            result = subprocess.run(
                [
                    str(alpine),
                    "hardware",
                    "--output",
                    str(output),
                    "--compact",
                ],
                capture_output=True,
                text=True,
                check=False,
                timeout=30,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            report = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(report["snapshot"]["schema"], 1)
            self.assertGreater(report["snapshot"]["cpu"]["logical_processors"], 0)
            self.assertGreater(report["snapshot"]["physical_memory_bytes"], 0)
            self.assertTrue(report["sha256"])


if __name__ == "__main__":
    unittest.main()
