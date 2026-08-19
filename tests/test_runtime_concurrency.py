from __future__ import annotations

import json
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

from localmodel.locking import FileLease, LeaseBusyError


REPO_ROOT = Path(__file__).resolve().parents[1]
LIB = REPO_ROOT / "runtime" / "scripts" / "lib.ps1"
SESSION_MODULE = REPO_ROOT / "runtime" / "scripts" / "inference-session.ps1"


class RuntimeConcurrencyTests(unittest.TestCase):
    def test_simultaneous_start_and_stop_wrappers_are_linearized(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            state = root / "session-state.json"
            trace = root / "trace.txt"
            barrier = root / "go"
            (root / "logs").mkdir()
            processes: list[subprocess.Popen[str]] = []
            for index in range(6):
                action = "Start" if index % 2 == 0 else "Stop"
                core = f"{action}-InferenceSessionCore"
                invocation = (
                    "Start-InferenceSession -Profile fixture -LockTimeoutMilliseconds 5000"
                    if action == "Start"
                    else "Stop-InferenceSession -LockTimeoutMilliseconds 5000"
                )
                script = (
                    f". '{SESSION_MODULE}'; "
                    f"function Get-SessionConfig {{ [pscustomobject]@{{root='{root}'; state_file='{state}'}} }}; "
                    f"function {core} {{ param([string]$InstallRoot,[string]$Profile,[switch]$Vision); "
                    f"Add-Content -LiteralPath '{trace}' -Value \"$($PID):{action}:begin\"; "
                    "Start-Sleep -Milliseconds 100; "
                    f"Add-Content -LiteralPath '{trace}' -Value \"$($PID):{action}:end\" }}; "
                    f"while(-not(Test-Path -LiteralPath '{barrier}')){{Start-Sleep -Milliseconds 5}}; {invocation} | Out-Null"
                )
                processes.append(
                    subprocess.Popen(
                        ["powershell.exe", "-NoProfile", "-Command", script],
                        stdout=subprocess.PIPE,
                        stderr=subprocess.PIPE,
                        text=True,
                    )
                )
            time.sleep(0.1)
            barrier.touch()
            results = [process.communicate(timeout=30) + (process.returncode,) for process in processes]
            self.assertTrue(all(code == 0 for _, _, code in results), results)
            lines = trace.read_text(encoding="utf-8-sig").splitlines()
            self.assertEqual(len(lines), 12)
            for index in range(0, len(lines), 2):
                begin = lines[index].split(":")
                end = lines[index + 1].split(":")
                self.assertEqual(begin[:2], end[:2])
                self.assertEqual((begin[2], end[2]), ("begin", "end"))

    def test_session_state_publication_is_atomic_for_concurrent_writers(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            state = root / "session-state.json"
            barrier = root / "go"
            script = (
                f". '{LIB}'; "
                f"while (-not (Test-Path -LiteralPath '{barrier}')) {{ Start-Sleep -Milliseconds 5 }}; "
                f"$session = [pscustomobject]@{{ state_file = '{state}' }}; "
                "$writer = $PID; "
                "1..60 | ForEach-Object { Save-SessionState ([ordered]@{ writer=$writer; iteration=$_; payload=('x' * 2048) }) $session }"
            )
            processes = [
                subprocess.Popen(
                    ["powershell.exe", "-NoProfile", "-Command", script],
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    text=True,
                )
                for _ in range(8)
            ]
            reader_script = (
                f". '{LIB}'; "
                f"while (-not (Test-Path -LiteralPath '{barrier}')) {{ Start-Sleep -Milliseconds 5 }}; "
                f"$session = [pscustomobject]@{{ state_file = '{state}' }}; "
                "1..240 | ForEach-Object { "
                "$value = Read-SessionState $session; "
                "if ($value -and ($value.iteration -lt 1 -or $value.iteration -gt 60 -or $value.payload.Length -ne 2048)) "
                "{ throw 'reader observed invalid state' } }"
            )
            readers = [
                subprocess.Popen(
                    ["powershell.exe", "-NoProfile", "-Command", reader_script],
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    text=True,
                )
                for _ in range(4)
            ]
            time.sleep(0.1)
            barrier.touch()
            results = []
            for process in processes:
                stdout, stderr = process.communicate(timeout=60)
                results.append((process.returncode, stdout, stderr))
            reader_results = [reader.communicate(timeout=60) + (reader.returncode,) for reader in readers]
            self.assertTrue(all(code == 0 for code, _, _ in results), results)
            self.assertTrue(all(code == 0 for _, _, code in reader_results), reader_results)
            parsed = json.loads(state.read_text(encoding="utf-8-sig"))
            self.assertIn(parsed["iteration"], range(1, 61))
            self.assertFalse(list(root.glob("session-state.json.*.tmp")))

    def test_concurrent_first_use_converges_on_one_complete_api_key(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            key = root / "api-key.txt"
            barrier = root / "go"
            script = (
                f". '{LIB}'; "
                f"while (-not (Test-Path -LiteralPath '{barrier}')) {{ Start-Sleep -Milliseconds 5 }}; "
                f"Ensure-LocalApiKey ([pscustomobject]@{{ api_key_file = '{key}' }})"
            )
            processes = [
                subprocess.Popen(
                    ["powershell.exe", "-NoProfile", "-Command", script],
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    text=True,
                )
                for _ in range(8)
            ]
            time.sleep(0.1)
            barrier.touch()
            results = []
            for process in processes:
                stdout, stderr = process.communicate(timeout=30)
                results.append((process.returncode, stdout, stderr))
            self.assertTrue(all(code == 0 for code, _, _ in results), results)
            value = key.read_text(encoding="utf-8").strip()
            self.assertRegex(value, r"^sk-local-[0-9a-f]{64}$")

    def test_interactive_capacity_lease_blocks_benchmark_and_recovers_after_owner_death(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            logs = root / "logs"
            logs.mkdir()
            ready = root / "ready"
            script = (
                f". '{SESSION_MODULE}'; "
                f"function Get-SessionConfig {{ [pscustomobject]@{{root='{root}'}} }}; "
                "$lease = Enter-InferenceCapacityLease -TimeoutMilliseconds 2000; "
                f"[IO.File]::WriteAllText('{ready}', 'ready'); "
                "try { Start-Sleep -Seconds 30 } finally { Exit-InferenceCapacityLease $lease }"
            )
            owner = subprocess.Popen(
                ["powershell.exe", "-NoProfile", "-Command", script],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            try:
                deadline = time.monotonic() + 10
                while not ready.exists() and time.monotonic() < deadline:
                    time.sleep(0.02)
                self.assertTrue(ready.exists(), f"lease owner exited with {owner.poll()}")
                benchmark = FileLease(logs / "inference.lease", {"kind": "benchmark"})
                with self.assertRaises(LeaseBusyError):
                    benchmark.acquire()
            finally:
                owner.kill()
                owner.communicate(timeout=10)

            recovered = FileLease(logs / "inference.lease", {"kind": "benchmark"})
            recovered.acquire()
            recovered.release()

    def test_session_transition_recovers_after_lock_owner_death(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "logs").mkdir()
            state = root / "session-state.json"
            ready = root / "ready"
            owner_script = (
                f". '{SESSION_MODULE}'; "
                f"$lock = Enter-InterprocessLock '{state}.session.lock' 2000; "
                f"[IO.File]::WriteAllText('{ready}', 'ready'); "
                "try { Start-Sleep -Seconds 30 } finally { Exit-InterprocessLock $lock }"
            )
            owner = subprocess.Popen(
                ["powershell.exe", "-NoProfile", "-Command", owner_script],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            try:
                deadline = time.monotonic() + 10
                while not ready.exists() and time.monotonic() < deadline:
                    time.sleep(0.02)
                self.assertTrue(ready.exists(), f"session-lock owner exited with {owner.poll()}")
            finally:
                owner.kill()
                owner.communicate(timeout=10)

            recovery_script = (
                f". '{SESSION_MODULE}'; "
                f"function Get-SessionConfig {{ [pscustomobject]@{{root='{root}'; state_file='{state}'}} }}; "
                "function Start-InferenceSessionCore { param($InstallRoot,$Profile,$Vision); 'started' }; "
                "Start-InferenceSession -Profile fixture -LockTimeoutMilliseconds 2000"
            )
            recovered = subprocess.run(
                ["powershell.exe", "-NoProfile", "-Command", recovery_script],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(recovered.returncode, 0, recovered.stderr)
            self.assertIn("started", recovered.stdout)

    def test_status_waits_on_the_same_session_transition_lock(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            state = root / "session-state.json"
            ready = root / "ready"
            owner_script = (
                f". '{SESSION_MODULE}'; "
                f"$lock = Enter-InterprocessLock '{state}.session.lock' 2000; "
                f"[IO.File]::WriteAllText('{ready}', 'ready'); "
                "try { Start-Sleep -Seconds 30 } finally { Exit-InterprocessLock $lock }"
            )
            owner = subprocess.Popen(
                ["powershell.exe", "-NoProfile", "-Command", owner_script],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            try:
                deadline = time.monotonic() + 10
                while not ready.exists() and time.monotonic() < deadline:
                    time.sleep(0.02)
                self.assertTrue(ready.exists(), f"session-lock owner exited with {owner.poll()}")
                probe = (
                    f". '{SESSION_MODULE}'; "
                    f"function Get-SessionConfig {{ [pscustomobject]@{{state_file='{state}'}} }}; "
                    "Get-InferenceSessionStatus -LockTimeoutMilliseconds 100"
                )
                blocked = subprocess.run(
                    ["powershell.exe", "-NoProfile", "-Command", probe],
                    capture_output=True,
                    text=True,
                    check=False,
                )
                self.assertNotEqual(blocked.returncode, 0)
                self.assertIn("Timed out waiting for interprocess lock", blocked.stderr)
            finally:
                owner.kill()
                owner.communicate(timeout=10)


if __name__ == "__main__":
    unittest.main()
