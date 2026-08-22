from __future__ import annotations

import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]


class SetupFrontDoorTests(unittest.TestCase):
    def test_source_bootstrap_is_native_and_propagates_arguments_and_failure(self) -> None:
        source = (REPO_ROOT / "setup.cmd").read_text(encoding="utf-8").lower()

        self.assertNotIn("powershell", source)
        self.assertIn("where.exe cargo", source)
        self.assertIn("where.exe winget.exe", source)
        self.assertIn("rustlang.rustup", source)
        self.assertIn("run --locked --release --bin alpine -- setup", source)
        self.assertIn('--repository-root "%~dp0." %*', source)
        self.assertIn("set \"alpine_exit=%errorlevel%\"", source)
        self.assertIn("exit /b %alpine_exit%", source)


if __name__ == "__main__":
    unittest.main()
