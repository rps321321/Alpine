from __future__ import annotations

import json
import subprocess
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
MODULE = REPO_ROOT / "runtime" / "scripts" / "harness-policy.ps1"


class HarnessPolicyTests(unittest.TestCase):
    def test_policy_preserves_capability_and_gates_only_sensitive_effects(self) -> None:
        expression = f"""
        . '{MODULE}'
        $session = [pscustomobject]@{{ api_key_file='C:\\fixture\\api-key.txt'; base_url_file='C:\\fixture\\base-url.txt' }}
        $profile = [pscustomobject]@{{ name='stable-16k'; context=16384; output=4096; external_skills=$false }}
        New-HarnessPolicy -Session $session -Profile $profile -Lean $true -SkillsEnabled $false | ConvertTo-Json -Depth 14 -Compress
        """
        result = subprocess.run(
            ["powershell.exe", "-NoProfile", "-Command", expression],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        policy = json.loads(result.stdout)
        self.assertNotEqual(policy["permission"].get("task"), "deny")
        self.assertNotEqual(policy["permission"].get("todowrite"), "deny")
        self.assertEqual(policy["permission"]["bash"]["git remote *"], "allow")
        self.assertEqual(policy["permission"]["bash"]["git push *"], "ask")
        self.assertEqual(policy["permission"]["skill"], "deny")
        self.assertEqual(policy["permission"]["read"]["C:/fixture/api-key.txt"], "deny")
        self.assertIn("prompt", policy["agent"]["build"])

    def test_explicit_full_skills_convex_capture_and_project_config_modes_are_preserved(self) -> None:
        expression = f"""
        . '{MODULE}'
        $session = [pscustomobject]@{{ api_key_file='C:\\fixture\\api-key.txt'; base_url_file='C:\\fixture\\base-url.txt' }}
        $profile = [pscustomobject]@{{ name='stable-16k'; context=16384; output=4096 }}
        $policy = New-HarnessPolicy -Session $session -Profile $profile -Lean $false -SkillsEnabled $true -WithConvex $true -CaptureEndpoint 'http://127.0.0.1:9191/'
        $state = Enter-HarnessEnvironment -ConfigJson '{{}}' -SkillsEnabled $true -WithProjectConfig $true
        $environment = [pscustomobject]@{{ skills=$env:OPENCODE_DISABLE_EXTERNAL_SKILLS; project=$env:OPENCODE_DISABLE_PROJECT_CONFIG }}
        Exit-HarnessEnvironment $state
        [pscustomobject]@{{ policy=$policy; environment=$environment }} | ConvertTo-Json -Depth 14 -Compress
        """
        result = subprocess.run(
            ["powershell.exe", "-NoProfile", "-Command", expression],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        observed = json.loads(result.stdout)
        policy = observed["policy"]
        self.assertNotIn("build", policy["agent"])
        self.assertNotIn("skill", policy["permission"])
        self.assertTrue(policy["mcp"]["convex"]["enabled"])
        self.assertEqual(policy["provider"]["local-models"]["options"]["baseURL"], "http://127.0.0.1:9191")
        self.assertEqual(observed["environment"], {"skills": "false", "project": "false"})

    def test_environment_is_shielded_then_restored_exactly(self) -> None:
        expression = f"""
        . '{MODULE}'
        $env:LOCALMODEL_TEST_TOKEN = 'before'
        Remove-Item Env:LOCALMODEL_TEST_ABSENT -ErrorAction SilentlyContinue
        $state = Enter-HarnessEnvironment -ConfigJson '{{"fixture":true}}' -SkillsEnabled $false -WithProjectConfig $false
        $during = [pscustomobject]@{{ token=(Test-Path Env:LOCALMODEL_TEST_TOKEN); config=$env:OPENCODE_CONFIG_CONTENT }}
        Exit-HarnessEnvironment $state
        [pscustomobject]@{{ during=$during; restored=$env:LOCALMODEL_TEST_TOKEN; absent=(Test-Path Env:LOCALMODEL_TEST_ABSENT) }} | ConvertTo-Json -Compress
        """
        result = subprocess.run(
            ["powershell.exe", "-NoProfile", "-Command", expression],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        observed = json.loads(result.stdout)
        self.assertFalse(observed["during"]["token"])
        self.assertEqual(observed["during"]["config"], '{"fixture":true}')
        self.assertEqual(observed["restored"], "before")
        self.assertFalse(observed["absent"])


if __name__ == "__main__":
    unittest.main()
