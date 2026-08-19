$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'lib.ps1')

$session = Get-SessionConfig
$state = Read-SessionState $session
$process = Get-ProcessOnPort $session.port
if ($process) {
    $expectedServer = if ($state -and $state.PSObject.Properties['server']) { [string]$state.server } else { [string]$session.llama_server }
    $ours = ($process.Path -and ($process.Path -ieq $expectedServer)) -or ($state -and $state.pid -and $process.Id -eq $state.pid)
    if (-not $ours) { throw "Port $($session.port) belongs to PID $($process.Id); refusing to kill it." }
    Stop-Process -Id $process.Id -Force
    if (-not (Wait-PortFree $session.port 30)) { throw "Port $($session.port) did not become free." }
    Write-Host "Stopped local Qwen PID $($process.Id)"
} else { Write-Host "No local Qwen listener on :$($session.port)" }

if ($state -and $state.cleanup_paused -and (Test-CleanupEnabled $session)) {
    $port = [int](Get-PropertyValue $session.cleanup 'port' 0)
    if ($port -gt 0 -and -not (Get-Listener $port)) {
        $start = [string](Get-PropertyValue $session.cleanup 'start_script' '')
        if ($start) { & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $start }
    }
}
if ($state) {
    $state | Add-Member -NotePropertyName stopped_at -NotePropertyValue (Get-Date).ToUniversalTime().ToString('o') -Force
    Save-SessionState $state $session
}
