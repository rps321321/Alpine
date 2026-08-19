from __future__ import annotations

import json
import subprocess
import tempfile
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
          Test-InferenceProcessIdentity $session $state $process '"C:\\runtime\\llama-server.exe" "--port" "8100"'
          Test-InferenceProcessIdentity $session $state $process '"C:\\runtime\\llama-server.exe" "--port=8100"'
          Test-InferenceProcessIdentity $session $state $process '"C:\\runtime\\llama-server.exe" --port="8100"'
          Test-InferenceProcessIdentity $session $state $process '"C:\\runtime\\llama-server.exe" --port 9999'
          Test-InferenceProcessIdentity $session $state $process '"C:\\runtime\\llama-server.exe" --model "note --port 8100 text"'
          Test-InferenceProcessIdentity $session $state $process '"C:\\runtime\\llama-server.exe" --port 81000'
          Test-InferenceProcessIdentity $session $state ([pscustomobject]@{ Id=99; Path='C:\\runtime\\llama-server.exe' }) '"C:\\runtime\\llama-server.exe" --port 8100'
        ) | ConvertTo-Json -Compress
        """
        result = invoke(expression)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            json.loads(result.stdout),
            [True, True, True, True, False, False, False, False],
        )

    def test_transaction_handles_idle_reuse_replace_restore_and_start_failure(self) -> None:
        expression = r"""
        function Get-SessionConfig { [pscustomobject]@{ state_file='C:\fixture\state.json' } }
        function Enter-InferenceCapacityLease { [pscustomobject]@{ Borrowed=$true } }
        function Exit-InferenceCapacityLease { param($Lease) }
        function Enter-InterprocessLock { [pscustomobject]@{ handle=$true } }
        function Exit-InterprocessLock { param($Lock) }
        function New-FixtureStatus([bool]$Active, [string]$Profile, [bool]$Vision, [string]$Identity, [bool]$Foreign=$false, [string]$Fallback='') {
          $arguments = if ($Fallback) { @('--spec-type','draft-mtp') } else { @('--spec-type','draft-mtp,ngram-mod') }
          $environment = [pscustomobject]@{ LLAMA_NGRAM_MOD_RESET_ON_BEGIN = if ($Fallback) { $null } else { '1' } }
          [pscustomobject]@{
            Active=$Active; Foreign=$Foreign; Healthy=$Active; Profile=$Profile; Vision=$Vision
            Runtime='fixture'; ExpectedPath='fixture.exe'; Fallback=$Fallback
            State=[pscustomobject]@{ transaction_id=$Identity; profile_sha256="hash-$Profile"; arguments=$arguments; environment=$environment }
          }
        }
        function Get-InferenceSessionStatusCore { return $global:fixtureStatus }
        function Get-InferenceSessionSnapshot {
          return [pscustomobject]@{
            Active=$global:fixtureStatus.Active; Healthy=$global:fixtureStatus.Healthy
            Profile=$global:fixtureStatus.Profile; Vision=$global:fixtureStatus.Vision
            Runtime=$global:fixtureStatus.Runtime; Fallback=$global:fixtureStatus.Fallback
            Arguments=$global:fixtureStatus.State.arguments; Environment=$global:fixtureStatus.State.environment
            State=$global:fixtureStatus.State
          }
        }
        function Start-InferenceSessionCore {
          param([string]$InstallRoot, [string]$Profile, [switch]$Vision, [switch]$ForceFallback)
          $global:starts++
          if ($global:failProfile -eq $Profile) { throw "fixture start failed: $Profile" }
          $global:generation++
          $fallback = if ($ForceFallback) { 'mtp-only' } else { '' }
          $global:fixtureStatus = New-FixtureStatus $true $Profile ([bool]$Vision) "tx-$global:generation" $false $fallback
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
        $global:fixtureStatus = New-FixtureStatus $true 'turbo-16k' $true 'prior' $false 'mtp-only'
        $replace = Enter-InferenceSession -Profile 'stable-16k'
        Exit-InferenceSession -Acquisition $replace
        $replaceResult = [pscustomobject]@{ changed=$replace.changed; starts=$global:starts; stops=$global:stops; profile=$global:fixtureStatus.Profile; vision=$global:fixtureStatus.Vision; fallback=$global:fixtureStatus.Fallback }

        $global:starts=0; $global:stops=0; $global:failProfile='stable-16k'
        $global:fixtureStatus = New-FixtureStatus $true 'turbo-16k' $true 'prior' $false 'mtp-only'
        $failure = try { Enter-InferenceSession -Profile 'stable-16k'; 'no-error' } catch { $_.Exception.Message }
        $failureResult = [pscustomobject]@{ message=$failure; starts=$global:starts; stops=$global:stops; profile=$global:fixtureStatus.Profile; vision=$global:fixtureStatus.Vision; healthy=$global:fixtureStatus.Healthy; fallback=$global:fixtureStatus.Fallback }

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
            {
                "changed": True,
                "starts": 2,
                "stops": 2,
                "profile": "turbo-16k",
                "vision": True,
                "fallback": "mtp-only",
            },
        )
        self.assertIn("fixture start failed", observed["failure"]["message"])
        self.assertEqual(observed["failure"]["profile"], "turbo-16k")
        self.assertTrue(observed["failure"]["vision"])
        self.assertTrue(observed["failure"]["healthy"])
        self.assertEqual(observed["failure"]["fallback"], "mtp-only")

    def test_cleanup_restoration_requires_configured_health_check(self) -> None:
        expression = r"""
        function Test-CleanupEnabled { return $true }
        function Get-ProcessOnPort { [pscustomobject]@{ Id=9191; Path='C:\fixture\cleanup.exe' } }
        function Get-CommandLine { return '"C:\fixture\cleanup.exe" --port 9191' }
        function powershell.exe { param([switch]$NoProfile, [string]$ExecutionPolicy, [string]$File) }
        function Wait-HttpOk { return $false }
        $session = [pscustomobject]@{ cleanup=[pscustomobject]@{ enabled=$true; port=9191; exe='C:\fixture\cleanup.exe'; start_script='C:\fixture\start.ps1'; health='http://127.0.0.1:9191/health' } }
        $state = [pscustomobject]@{ cleanup_paused=$true }
        Restore-CleanupProcess $session $state
        """
        result = invoke(expression)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Cleanup health check failed", result.stderr)

    def test_cleanup_restoration_waits_for_asynchronous_start_before_identity_check(self) -> None:
        expression = r"""
        function Test-CleanupEnabled { return $true }
        function Get-ProcessOnPort {
          if ($global:cleanupReady) { [pscustomobject]@{ Id=9191; Path='C:\fixture\cleanup.exe' } }
        }
        function Get-CommandLine { return '"C:\fixture\cleanup.exe" --port 9191' }
        function powershell.exe {
          param([switch]$NoProfile, [string]$ExecutionPolicy, [string]$File)
          $global:startInvoked = $true
        }
        function Wait-HttpOk {
          $global:healthChecks++
          $global:cleanupReady = $true
          return $true
        }
        $global:cleanupReady=$false; $global:startInvoked=$false; $global:healthChecks=0
        $session = [pscustomobject]@{ cleanup=[pscustomobject]@{ enabled=$true; port=9191; exe='C:\fixture\cleanup.exe'; start_script='C:\fixture\start.ps1'; health='http://127.0.0.1:9191/health' } }
        $state = [pscustomobject]@{ cleanup_paused=$true }
        Restore-CleanupProcess $session $state
        [pscustomobject]@{started=$global:startInvoked;healthChecks=$global:healthChecks;ready=$global:cleanupReady} | ConvertTo-Json -Compress
        """
        result = invoke(expression)
        self.assertEqual(result.returncode, 0, result.stderr)
        observed = json.loads(result.stdout.splitlines()[-1])
        self.assertTrue(observed["started"])
        self.assertEqual(observed["healthChecks"], 1)
        self.assertTrue(observed["ready"])

    def test_optimized_start_retries_once_with_pinned_mtp_only_fallback(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            server = root / "server.exe"
            model = root / "model.gguf"
            template = root / "chat.jinja"
            profile_path = root / "profiles" / "fixture.json"
            profile_path.parent.mkdir()
            for path in (server, model, template, profile_path):
                path.write_text("fixture", encoding="utf-8")
            expression = rf"""
            $session = [pscustomobject]@{{
              root='{root}'; model='{model}'; host='127.0.0.1'; port=8100; chat_template='{template}'
              api_key_file='{root / 'api-key.txt'}'; base_url_file='{root / 'base-url.txt'}'
              state_file='{root / 'state.json'}'; cleanup=[pscustomobject]@{{enabled=$false}}
            }}
            $profile = [pscustomobject]@{{
              context=16384; parallel=1; threads=8; batch_size=1024; ubatch_size=256
              kv_cache='q8_0'; tensor_cpu_through_block=24; mtp_depth=3
              ngram_mod=$true; ngram_reset_on_begin=$true; fit_target_mib=11900
            }}
            $resolved = [pscustomobject]@{{
              Session=$session; Profile=$profile; ProfileName='fixture'; RuntimeName='custom'
              ServerPath='{server}'; Model='{model}'; ChatTemplate='{template}'
              Mmproj=''; BaseUrl='http://127.0.0.1:8100'
            }}
            function Get-ResolvedSession {{ return $resolved }}
            function Get-InferenceSessionStatusCore {{
              [pscustomobject]@{{ Active=$false; Foreign=$false; Healthy=$false; Profile='fixture'; Vision=$false }}
            }}
            function Test-CleanupEnabled {{ return $false }}
            function Ensure-LocalApiKey {{ param($Session) }}
            function Write-AtomicText {{ param($Path,$Content,$Encoding) }}
            function Save-SessionState {{ param($State,$Session); $global:lastState = $State | ConvertTo-Json -Depth 8 | ConvertFrom-Json }}
            function Start-InferenceProcess {{
              param($ServerPath,$Arguments,$OutLog,$ErrLog,$ResetNgram)
              $global:starts++
              [pscustomobject]@{{ Id=(100 + $global:starts); StartTime=(Get-Date); HasExited=$false }}
            }}
            function Test-StartedProcessHealthy {{ $global:healthChecks++; return $global:healthChecks -eq 2 }}
            function Stop-Process {{ param($Id,[switch]$Force,$ErrorAction) }}
            function Wait-PortFree {{ return $true }}
            $global:starts=0; $global:healthChecks=0; $global:lastState=$null
            Start-InferenceSessionCore -Profile fixture 3>$null | Out-Null
            $specIndex = [Array]::IndexOf([object[]]$global:lastState.arguments, '--spec-type')
            [pscustomobject]@{{
              starts=$global:starts
              fallback=$global:lastState.fallback
              specType=$global:lastState.arguments[($specIndex + 1)]
              reset=$global:lastState.environment.LLAMA_NGRAM_MOD_RESET_ON_BEGIN
            }} | ConvertTo-Json -Compress
            """
            result = invoke(expression)
            self.assertEqual(result.returncode, 0, result.stderr)
            observed = json.loads(result.stdout.splitlines()[-1])
            self.assertEqual(observed["starts"], 2)
            self.assertEqual(observed["fallback"], "mtp-only")
            self.assertEqual(observed["specType"], "draft-mtp")
            self.assertIsNone(observed["reset"])

    def test_failure_after_cleanup_pause_restores_and_health_checks_cleanup(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            server = root / "server.exe"
            model = root / "model.gguf"
            template = root / "chat.jinja"
            for path in (server, model, template):
                path.write_text("fixture", encoding="utf-8")
            expression = rf"""
            $session = [pscustomobject]@{{
              root='{root}'; model='{model}'; host='127.0.0.1'; port=8123; chat_template='{template}'
              api_key_file='{root / 'api-key.txt'}'; base_url_file='{root / 'base-url.txt'}'
              state_file='{root / 'state.json'}'
              cleanup=[pscustomobject]@{{enabled=$true; port=9191; exe='C:\fixture\cleanup.exe'; start_script='C:\fixture\start.ps1'; health='http://127.0.0.1:9191/health'}}
            }}
            $resolved = [pscustomobject]@{{
              Session=$session; Profile=[pscustomobject]@{{ngram_reset_on_begin=$false}}
              ProfileName='fixture'; RuntimeName='custom'; ServerPath='{server}'
              Model='{model}'; ChatTemplate='{template}'; Mmproj=''; BaseUrl='http://127.0.0.1:8123'
            }}
            function Get-ResolvedSession {{ return $resolved }}
            function Get-InferenceSessionStatusCore {{ [pscustomobject]@{{Active=$false;Foreign=$false}} }}
            function Test-CleanupEnabled {{ return $true }}
            function Get-ProcessOnPort {{
              param($Port)
              if ($Port -eq 9191 -and $global:cleanupRunning) {{ [pscustomobject]@{{Id=9191;Path='C:\fixture\cleanup.exe'}} }}
            }}
            function Get-CommandLine {{ return '"C:\fixture\cleanup.exe" --port 9191' }}
            function Stop-Process {{ param($Id,[switch]$Force,$ErrorAction); if ($Id -eq 9191) {{ $global:cleanupRunning=$false }} }}
            function Wait-PortFree {{ return $true }}
            function powershell.exe {{ param([switch]$NoProfile,[string]$ExecutionPolicy,[string]$File); $global:cleanupRunning=$true }}
            function Wait-HttpOk {{ $global:healthChecks++; return $true }}
            function Ensure-LocalApiKey {{ param($Session) }}
            function Write-AtomicText {{ param($Path,$Content,$Encoding) }}
            function Get-FileSha256 {{ throw 'fixture hash failure' }}
            function Save-SessionState {{ param($State,$Session); $global:lastState=$State }}
            $global:cleanupRunning=$true; $global:healthChecks=0; $global:lastState=$null
            $message = try {{ Start-InferenceSessionCore -Profile fixture; 'no-error' }} catch {{ $_.Exception.Message }}
            [pscustomobject]@{{message=$message;cleanupRunning=$global:cleanupRunning;healthChecks=$global:healthChecks;paused=$global:lastState.cleanup_paused}} | ConvertTo-Json -Compress
            """
            result = invoke(expression)
            self.assertEqual(result.returncode, 0, result.stderr)
            observed = json.loads(result.stdout.splitlines()[-1])
            self.assertIn("fixture hash failure", observed["message"])
            self.assertTrue(observed["cleanupRunning"])
            self.assertGreaterEqual(observed["healthChecks"], 1)
            self.assertTrue(observed["paused"])


if __name__ == "__main__":
    unittest.main()
