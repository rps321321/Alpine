from __future__ import annotations

import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]


class HardwareInventoryTests(unittest.TestCase):
    def test_inventory_uses_the_repository_hash_implementation(self) -> None:
        source = (REPO_ROOT / "scripts" / "collect-hardware.ps1").read_text(encoding="utf-8")

        self.assertIn("Get-FileSha256 -Path $serverPath", source)
        self.assertNotIn("Get-FileHash", source)


if __name__ == "__main__":
    unittest.main()
