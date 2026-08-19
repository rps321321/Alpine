from __future__ import annotations

import json
import subprocess
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
MODULE = REPO_ROOT / "runtime" / "scripts" / "inference-session.ps1"


def invoke(expression: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["powershell.exe", "-NoProfile", "-Command", f". '{MODULE}'; {expression}"],
        capture_output=True,
        text=True,
        check=False,
    )


class InferenceSessionTests(unittest.TestCase):
    def test_planner_distinguishes_idle_reuse_replace_and_foreign_listener(self) -> None:
        expression = """
        @(
          Resolve-InferenceSessionPlan -Current ([pscustomobject]@{ Active=$false; Foreign=$false }) -Profile stable-16k -Vision:$false
          Resolve-InferenceSessionPlan -Current ([pscustomobject]@{ Active=$true; Foreign=$false; Healthy=$true; Profile='stable-16k'; Vision=$false }) -Profile stable-16k -Vision:$false
          Resolve-InferenceSessionPlan -Current ([pscustomobject]@{ Active=$true; Foreign=$false; Healthy=$true; Profile='turbo-16k'; Vision=$false }) -Profile stable-16k -Vision:$false
          Resolve-InferenceSessionPlan -Current ([pscustomobject]@{ Active=$false; Foreign=$true }) -Profile stable-16k -Vision:$false
        ) | ConvertTo-Json -Compress
        """
        result = invoke(expression)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout), ["start", "reuse", "replace", "refuse"])

    def test_process_identity_requires_listener_pid_path_and_port_command_line(self) -> None:
        expression = """
        $session = [pscustomobject]@{ port=8100 }
        $state = [pscustomobject]@{ pid=42; server='C:\\runtime\\llama-server.exe' }
        $process = [pscustomobject]@{ Id=42; Path='C:\\runtime\\llama-server.exe' }
        @(
          Test-InferenceProcessIdentity $session $state $process '"C:\\runtime\\llama-server.exe" --port 8100'
          Test-InferenceProcessIdentity $session $state $process '"C:\\runtime\\llama-server.exe" --port 9999'
          Test-InferenceProcessIdentity $session $state ([pscustomobject]@{ Id=99; Path='C:\\runtime\\llama-server.exe' }) '"C:\\runtime\\llama-server.exe" --port 8100'
        ) | ConvertTo-Json -Compress
        """
        result = invoke(expression)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout), [True, False, False])

    def test_transaction_handles_idle_reuse_replace_restore_and_start_failure(self) -> None:
        expression = r"""
        function Get-SessionConfig { [pscustomobject]@{ state_file='C:\fixture\state.json' } }
        function Enter-InterprocessLock { [pscustomobject]@{ handle=$true } }
        function Exit-InterprocessLock { param($Lock) }
        function New-FixtureStatus([bool]$Active, [string]$Profile, [bool]$Vision, [string]$Identity, [bool]$Foreign=$false) {
          [pscustomobject]@{
            Active=$Active; Foreign=$Foreign; Healthy=$Active; Profile=$Profile; Vision=$Vision
            Runtime='fixture'; ExpectedPath='fixture.exe'; Fallback=$null
            State=[pscustomobject]@{ transaction_id=$Identity; profile_sha256="hash-$Profile"; arguments=@(); environment=[pscustomobject]@{} }
          }
        }
        function Get-InferenceSessionStatus { return $global:fixtureStatus }
        function Get-InferenceSessionSnapshot {
          return [pscustomobject]@{
            Active=$global:fixtureStatus.Active; Healthy=$global:fixtureStatus.Healthy
            Profile=$global:fixtureStatus.Profile; Vision=$global:fixtureStatus.Vision
            Runtime=$global:fixtureStatus.Runtime; State=$global:fixtureStatus.State
          }
        }
        function Start-InferenceSessionCore {
          param([string]$InstallRoot, [string]$Profile, [switch]$Vision)
          $global:starts++
          if ($global:failProfile -eq $Profile) { throw "fixture start failed: $Profile" }
          $global:generation++
          $global:fixtureStatus = New-FixtureStatus $true $Profile ([bool]$Vision) "tx-$global:generation"
        }
        function Stop-InferenceSessionCore {
          param([string]$InstallRoot)
          $global:stops++
          $global:fixtureStatus = New-FixtureStatus $false $global:fixtureStatus.Profile $global:fixtureStatus.Vision ''
        }

        $global:starts=0; $global:stops=0; $global:generation=0; $global:failProfile=''
        $global:fixtureStatus = New-FixtureStatus $false 'stable-16k' $false ''
        $idle = Enter-InferenceSession -Profile 'stable-16k'
        Exit-InferenceSession -Acquisition $idle
        $idleResult = [pscustomobject]@{ changed=$idle.changed; starts=$global:starts; stops=$global:stops; active=$global:fixtureStatus.Active }

        $global:starts=0; $global:stops=0
        $global:fixtureStatus = New-FixtureStatus $true 'stable-16k' $false 'existing'
        $reuse = Enter-InferenceSession -Profile 'stable-16k'
        Exit-InferenceSession -Acquisition $reuse
        $reuseResult = [pscustomobject]@{ changed=$reuse.changed; starts=$global:starts; stops=$global:stops; identity=$global:fixtureStatus.State.transaction_id }

        $global:starts=0; $global:stops=0
        $global:fixtureStatus = New-FixtureStatus $true 'turbo-16k' $true 'prior'
        $replace = Enter-InferenceSession -Profile 'stable-16k'
        Exit-InferenceSession -Acquisition $replace
        $replaceResult = [pscustomobject]@{ changed=$replace.changed; starts=$global:starts; stops=$global:stops; profile=$global:fixtureStatus.Profile; vision=$global:fixtureStatus.Vision }

        $global:starts=0; $global:stops=0; $global:failProfile='stable-16k'
        $global:fixtureStatus = New-FixtureStatus $true 'turbo-16k' $true 'prior'
        $failure = try { Enter-InferenceSession -Profile 'stable-16k'; 'no-error' } catch { $_.Exception.Message }
        $failureResult = [pscustomobject]@{ message=$failure; starts=$global:starts; stops=$global:stops; profile=$global:fixtureStatus.Profile; vision=$global:fixtureStatus.Vision; healthy=$global:fixtureStatus.Healthy }

        [pscustomobject]@{ idle=$idleResult; reuse=$reuseResult; replace=$replaceResult; failure=$failureResult } | ConvertTo-Json -Depth 6 -Compress
        """
        result = invoke(expression)
        self.assertEqual(result.returncode, 0, result.stderr)
        observed = json.loads(result.stdout)
        self.assertEqual(observed["idle"], {"changed": True, "starts": 1, "stops": 1, "active": False})
        self.assertEqual(
            observed["reuse"],
            {"changed": False, "starts": 0, "stops": 0, "identity": "existing"},
        )
        self.assertEqual(
            observed["replace"],
            {"changed": True, "starts": 2, "stops": 2, "profile": "turbo-16k", "vision": True},
        )
        self.assertIn("fixture start failed", observed["failure"]["message"])
        self.assertEqual(observed["failure"]["profile"], "turbo-16k")
        self.assertTrue(observed["failure"]["vision"])
        self.assertTrue(observed["failure"]["healthy"])

    def test_cleanup_restoration_requires_configured_health_check(self) -> None:
        expression = r"""
        function Test-CleanupEnabled { return $true }
        function Get-ProcessOnPort { return $null }
        function powershell.exe { param([switch]$NoProfile, [string]$ExecutionPolicy, [string]$File) }
        function Wait-HttpOk { return $false }
        $session = [pscustomobject]@{ cleanup=[pscustomobject]@{ enabled=$true; port=9191; exe='C:\fixture\cleanup.exe'; start_script='C:\fixture\start.ps1'; health='http://127.0.0.1:9191/health' } }
        $state = [pscustomobject]@{ cleanup_paused=$true }
        Restore-CleanupProcess $session $state
        """
        result = invoke(expression)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Cleanup health check failed", result.stderr)


if __name__ == "__main__":
    unittest.main()
