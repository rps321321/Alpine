$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not (Get-Command Get-ResolvedSession -ErrorAction SilentlyContinue)) {
    . (Join-Path $PSScriptRoot 'lib.ps1')
}

function Resolve-InferenceSessionPlan {
    param($Current, [string]$Profile, [bool]$Vision)
    if ([bool](Get-PropertyValue $Current 'Foreign' $false)) { return 'refuse' }
    if (-not [bool](Get-PropertyValue $Current 'Active' $false)) { return 'start' }
    $matches = (
        [bool](Get-PropertyValue $Current 'Healthy' $false) -and
        [string](Get-PropertyValue $Current 'Profile' '') -eq $Profile -and
        [bool](Get-PropertyValue $Current 'Vision' $false) -eq $Vision
    )
    if ($matches) { return 'reuse' }
    return 'replace'
}

function Split-WindowsCommandLine {
    param([string]$CommandLine)
    if (-not ('LocalModel.WindowsCommandLine' -as [type])) {
        Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;

namespace LocalModel {
    public static class WindowsCommandLine {
        [DllImport("shell32.dll", SetLastError = true)]
        private static extern IntPtr CommandLineToArgvW(
            [MarshalAs(UnmanagedType.LPWStr)] string commandLine,
            out int argumentCount);

        [DllImport("kernel32.dll")]
        private static extern IntPtr LocalFree(IntPtr memory);

        public static string[] Split(string commandLine) {
            int count;
            IntPtr arguments = CommandLineToArgvW(commandLine, out count);
            if (arguments == IntPtr.Zero) {
                throw new Win32Exception(Marshal.GetLastWin32Error());
            }
            try {
                string[] result = new string[count];
                for (int index = 0; index < count; index++) {
                    IntPtr argument = Marshal.ReadIntPtr(arguments, index * IntPtr.Size);
                    result[index] = Marshal.PtrToStringUni(argument);
                }
                return result;
            }
            finally {
                LocalFree(arguments);
            }
        }
    }
}
'@
    }
    return [LocalModel.WindowsCommandLine]::Split($CommandLine)
}

function Test-CommandLinePort {
    param([string]$CommandLine, [int]$Port)
    if (-not $CommandLine) { return $false }
    try { $arguments = @(Split-WindowsCommandLine $CommandLine) }
    catch { return $false }
    for ($index = 0; $index -lt $arguments.Count; $index++) {
        $argument = [string]$arguments[$index]
        if ($argument -ieq '--port') {
            if ($index + 1 -lt $arguments.Count -and [string]$arguments[$index + 1] -eq [string]$Port) { return $true }
            continue
        }
        if ($argument -imatch '^--port=(\d+)$' -and $Matches[1] -eq [string]$Port) { return $true }
    }
    return $false
}

function Test-InferenceProcessIdentity {
    param($Session, $State, $Process, [string]$CommandLine)
    if ($null -eq $Session -or $null -eq $State -or $null -eq $Process) { return $false }
    $statePid = Get-PropertyValue $State 'pid' $null
    $server = [string](Get-PropertyValue $State 'server' '')
    if ($null -eq $statePid -or [int]$Process.Id -ne [int]$statePid) { return $false }
    if (-not $Process.Path -or -not $server -or ([IO.Path]::GetFullPath([string]$Process.Path) -ine [IO.Path]::GetFullPath($server))) {
        return $false
    }
    return Test-CommandLinePort $CommandLine ([int]$Session.port)
}

function Get-InferenceSessionStatusCore {
    param([string]$InstallRoot)
    $resolved = Get-ResolvedSession -InstallRoot $InstallRoot
    $session = $resolved.Session
    $state = Read-SessionState $session
    $process = Get-ProcessOnPort ([int]$session.port)
    $commandLine = if ($process) { Get-CommandLine ([int]$process.Id) } else { $null }
    $owned = [bool]($process -and (Test-InferenceProcessIdentity $session $state $process $commandLine))
    $stateProfile = if ($state) { [string](Get-PropertyValue $state 'profile' '') } else { '' }
    $profile = if ($stateProfile) { $stateProfile } else { $resolved.ProfileName }
    return [pscustomobject][ordered]@{
        Active = $owned
        Foreign = [bool]($process -and -not $owned)
        Healthy = [bool]($owned -and (Test-HttpOk "$($resolved.BaseUrl)/health" 3))
        Profile = $profile
        Vision = if ($state) { [bool](Get-PropertyValue $state 'vision' $false) } else { $false }
        Runtime = if ($state) { [string](Get-PropertyValue $state 'runtime' $resolved.RuntimeName) } else { $resolved.RuntimeName }
        Pid = if ($process) { [int]$process.Id } else { $null }
        ProcessPath = if ($process) { [string]$process.Path } else { $null }
        ExpectedPath = if ($state -and (Get-PropertyValue $state 'server' '')) { [string]$state.server } else { [string]$resolved.ServerPath }
        Fallback = if ($state) { Get-PropertyValue $state 'fallback' $null } else { $null }
        State = $state
    }
}

function Get-InferenceSessionStatus {
    param([string]$InstallRoot, [int]$LockTimeoutMilliseconds = 15000)
    $session = Get-SessionConfig $InstallRoot
    $lock = Enter-InterprocessLock "$($session.state_file).session.lock" $LockTimeoutMilliseconds
    try { return Get-InferenceSessionStatusCore -InstallRoot $InstallRoot }
    finally { Exit-InterprocessLock $lock }
}

function Get-InferenceSessionSnapshot {
    param([string]$InstallRoot)
    $status = Get-InferenceSessionStatusCore -InstallRoot $InstallRoot
    if ($status.Foreign) {
        throw "Inference Session port is owned by an unrecognized listener; refusing to capture it."
    }
    return [pscustomobject][ordered]@{
        Active = [bool]$status.Active
        Healthy = [bool]$status.Healthy
        Profile = [string]$status.Profile
        Vision = [bool]$status.Vision
        Runtime = [string]$status.Runtime
        Fallback = $status.Fallback
        Arguments = @(Get-PropertyValue $status.State 'arguments' @())
        Environment = Get-PropertyValue $status.State 'environment' ([pscustomobject]@{})
        State = $status.State
    }
}

function Test-RestoredInferenceState {
    param($Status, $Snapshot)
    if (-not $Status.Healthy -or $Status.Profile -ne $Snapshot.Profile -or
        [bool]$Status.Vision -ne [bool]$Snapshot.Vision -or $Status.Runtime -ne $Snapshot.Runtime -or
        [string]$Status.Fallback -ne [string]$Snapshot.Fallback) { return $false }
    $observedArguments = @(Get-PropertyValue $Status.State 'arguments' @()) | ConvertTo-Json -Depth 8 -Compress
    $expectedArguments = @(Get-PropertyValue $Snapshot 'Arguments' @()) | ConvertTo-Json -Depth 8 -Compress
    if ($observedArguments -ne $expectedArguments) { return $false }
    $observedEnvironment = Get-PropertyValue $Status.State 'environment' ([pscustomobject]@{}) | ConvertTo-Json -Depth 8 -Compress
    $expectedEnvironment = Get-PropertyValue $Snapshot 'Environment' ([pscustomobject]@{}) | ConvertTo-Json -Depth 8 -Compress
    return $observedEnvironment -eq $expectedEnvironment
}

function Enter-InferenceCapacityLease {
    param([string]$InstallRoot, [int]$TimeoutMilliseconds = 100)
    $session = Get-SessionConfig $InstallRoot
    $leasePath = Join-Path $session.root 'logs\inference.lease'
    $ownerPath = "$leasePath.owner.json"
    $callerLease = [string](Get-PropertyValue ([pscustomobject]@{ value = $env:LOCALMODEL_INFERENCE_LEASE_ID }) 'value' '')
    if ($callerLease -and (Test-Path -LiteralPath $ownerPath)) {
        try {
            $owner = Get-Content -Raw -LiteralPath $ownerPath | ConvertFrom-Json
            $ownerPid = [int](Get-PropertyValue $owner 'pid' 0)
            $ownerAlive = $ownerPid -gt 0 -and $null -ne (Get-Process -Id $ownerPid -ErrorAction SilentlyContinue)
            if ($ownerAlive -and [string](Get-PropertyValue $owner 'lease_id' '') -eq $callerLease) {
                return [pscustomobject]@{ Borrowed=$true; LeaseId=$callerLease; Lock=$null; OwnerPath=$ownerPath }
            }
        } catch { }
    }
    try {
        $lock = Enter-InterprocessLock $leasePath $TimeoutMilliseconds
    } catch {
        throw 'Inference capacity is leased by a measured benchmark; wait for it to finish before opening the Harness.'
    }
    $leaseId = [Guid]::NewGuid().ToString('N')
    $prior = Get-Item Env:LOCALMODEL_INFERENCE_LEASE_ID -ErrorAction SilentlyContinue
    $owner = [ordered]@{
        kind = 'interactive-harness'
        lease_id = $leaseId
        pid = $PID
        acquired_at = (Get-Date).ToUniversalTime().ToString('o')
        session_identity = $null
    }
    try {
        Write-AtomicText $ownerPath (($owner | ConvertTo-Json -Depth 4) + [Environment]::NewLine)
        $env:LOCALMODEL_INFERENCE_LEASE_ID = $leaseId
        return [pscustomobject]@{
            Borrowed = $false
            LeaseId = $leaseId
            Lock = $lock
            OwnerPath = $ownerPath
            PriorExists = ($null -ne $prior)
            PriorValue = if ($prior) { [string]$prior.Value } else { $null }
        }
    } catch {
        Exit-InterprocessLock $lock
        throw
    }
}

function Update-InferenceCapacityLease {
    param($Lease, [string]$SessionIdentity)
    if ($null -eq $Lease -or [bool]$Lease.Borrowed) { return }
    $owner = [ordered]@{
        kind = 'interactive-harness'
        lease_id = [string]$Lease.LeaseId
        pid = $PID
        acquired_at = (Get-Date).ToUniversalTime().ToString('o')
        session_identity = $SessionIdentity
    }
    Write-AtomicText ([string]$Lease.OwnerPath) (($owner | ConvertTo-Json -Depth 4) + [Environment]::NewLine)
}

function Exit-InferenceCapacityLease {
    param($Lease)
    if ($null -eq $Lease -or [bool]$Lease.Borrowed) { return }
    try {
        if (Test-Path -LiteralPath $Lease.OwnerPath) {
            try {
                $owner = Get-Content -Raw -LiteralPath $Lease.OwnerPath | ConvertFrom-Json
                if ([string](Get-PropertyValue $owner 'lease_id' '') -eq [string]$Lease.LeaseId) {
                    Remove-Item -LiteralPath $Lease.OwnerPath -Force
                }
            } catch { }
        }
        if ([bool]$Lease.PriorExists) { $env:LOCALMODEL_INFERENCE_LEASE_ID = [string]$Lease.PriorValue }
        else { Remove-Item Env:LOCALMODEL_INFERENCE_LEASE_ID -ErrorAction SilentlyContinue }
    } finally { Exit-InterprocessLock $Lease.Lock }
}

function Assert-InferenceCapacityAvailable {
    param([string]$InstallRoot, [int]$TimeoutMilliseconds = 100)
    $lease = Enter-InferenceCapacityLease -InstallRoot $InstallRoot -TimeoutMilliseconds $TimeoutMilliseconds
    Exit-InferenceCapacityLease $lease
}

function New-InferenceArguments {
    param($Session, $Profile, [string]$ServerPath, [bool]$Vision, [switch]$Fallback)
    $arguments = @(
        '-m', $Session.model, '--host', $Session.host, '--port', "$($Session.port)",
        '-c', "$($Profile.context)", '-np', "$($Profile.parallel)",
        '--threads', "$($Profile.threads)", '--threads-batch', "$($Profile.threads)",
        '-b', "$($Profile.batch_size)", '-ub', "$($Profile.ubatch_size)",
        '--no-webui', '--jinja', '--chat-template-file', $Session.chat_template,
        '--api-key-file', $Session.api_key_file, '-fa', 'on',
        '-ctk', $Profile.kv_cache, '-ctv', $Profile.kv_cache,
        '--reasoning', 'off'
    )
    if ($Vision) {
        $arguments += @('--mmproj', $Session.mmproj, '--fit', 'on', '--fit-ctx', "$($Profile.context)", '--fit-target', "$($Profile.fit_target_mib)")
    } else {
        $arguments += @('-ngl', 'all', '--fit', 'off', '-ot', (Get-TensorOverride ([int]$Profile.tensor_cpu_through_block)), '--load-mode', 'none')
    }
    $specTypes = @('draft-mtp')
    if (-not $Fallback -and [bool]$Profile.ngram_mod) { $specTypes += 'ngram-mod' }
    $arguments += @('--spec-type', ($specTypes -join ','), '--spec-draft-n-max', "$($Profile.mtp_depth)")
    if (-not $Fallback -and [bool]$Profile.ngram_mod) {
        $arguments += @('--spec-ngram-mod-n-match', '24', '--spec-ngram-mod-n-min', '16', '--spec-ngram-mod-n-max', '64')
    }
    return ,$arguments
}

function Start-InferenceProcess {
    param([string]$ServerPath, [string[]]$Arguments, [string]$OutLog, [string]$ErrLog, [bool]$ResetNgram)
    $old = Get-Item Env:LLAMA_NGRAM_MOD_RESET_ON_BEGIN -ErrorAction SilentlyContinue
    try {
        if ($ResetNgram) { $env:LLAMA_NGRAM_MOD_RESET_ON_BEGIN = '1' }
        else { Remove-Item Env:LLAMA_NGRAM_MOD_RESET_ON_BEGIN -ErrorAction SilentlyContinue }
        $quote = [string][char]34
        $nativeArguments = @($Arguments | ForEach-Object { $quote + ([string]$_) + $quote })
        return Start-Process -FilePath $ServerPath -ArgumentList $nativeArguments `
            -RedirectStandardOutput $OutLog -RedirectStandardError $ErrLog `
            -PassThru -WindowStyle Hidden
    } finally {
        if ($old) { $env:LLAMA_NGRAM_MOD_RESET_ON_BEGIN = $old.Value }
        else { Remove-Item Env:LLAMA_NGRAM_MOD_RESET_ON_BEGIN -ErrorAction SilentlyContinue }
    }
}

function Test-StartedProcessHealthy {
    param($Resolved, $State, $Process, [int]$TimeoutSec = 600)
    if (-not (Wait-HttpOk "$($Resolved.BaseUrl)/health" $TimeoutSec)) { return $false }
    $listenerProcess = Get-ProcessOnPort ([int]$Resolved.Session.port)
    if (-not $listenerProcess -or [int]$listenerProcess.Id -ne [int]$Process.Id) { return $false }
    $commandLine = Get-CommandLine ([int]$listenerProcess.Id)
    return Test-InferenceProcessIdentity $Resolved.Session $State $listenerProcess $commandLine
}

function Restore-CleanupProcess {
    param($Session, $State)
    $paused = [bool](Get-PropertyValue $State 'cleanup_paused' $false)
    if (-not $paused -or -not (Test-CleanupEnabled $Session)) { return }
    $port = [int](Get-PropertyValue $Session.cleanup 'port' 0)
    if ($port -lt 1) { throw 'Cleanup restoration requested without a valid configured port.' }
    $expected = [string](Get-PropertyValue $Session.cleanup 'exe' '')
    $health = [string](Get-PropertyValue $Session.cleanup 'health' '')
    if (-not $health) { throw 'Cleanup restoration requires a configured health endpoint.' }
    $process = Get-ProcessOnPort $port
    $started = $false
    if (-not $process) {
        $start = [string](Get-PropertyValue $Session.cleanup 'start_script' '')
        if (-not $start) { throw 'Cleanup restoration requested without a start script.' }
        & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $start
        $started = $true
        if (-not (Wait-HttpOk $health 120)) { throw "Cleanup health check failed: $health" }
        $process = Get-ProcessOnPort $port
    }
    $commandLine = if ($process) { Get-CommandLine ([int]$process.Id) } else { $null }
    if (-not $process -or -not $process.Path -or ([IO.Path]::GetFullPath([string]$process.Path) -ine [IO.Path]::GetFullPath($expected)) -or -not (Test-CommandLinePort $commandLine $port)) {
        throw "Cleanup restoration did not produce the configured process on port $port."
    }
    if (-not $started -and -not (Wait-HttpOk $health 120)) { throw "Cleanup health check failed: $health" }
}

function Start-InferenceSessionCore {
    [CmdletBinding()]
    param([string]$InstallRoot, [string]$Profile, [switch]$Vision, [switch]$ForceFallback)
    $resolved = Get-ResolvedSession -InstallRoot $InstallRoot -Name $Profile -RequireRuntime
    $session = $resolved.Session
    $profileConfig = $resolved.Profile
    $Profile = $resolved.ProfileName
    $current = Get-InferenceSessionStatusCore -InstallRoot $InstallRoot
    $plan = Resolve-InferenceSessionPlan $current $Profile ([bool]$Vision)
    if ($plan -eq 'reuse') { return [pscustomobject]@{ Started = $false; Status = $current } }
    if ($plan -eq 'refuse') { throw "Port $($session.port) is occupied by a foreign listener; refusing to steal it." }
    if ($plan -eq 'replace') { throw "A different or unhealthy owned Inference Session is active; stop or restore it transactionally before starting $Profile." }
    foreach ($path in @($resolved.ServerPath, $resolved.Model, $resolved.ChatTemplate)) {
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Required file missing: $path" }
    }
    if ($Vision -and -not (Test-Path -LiteralPath $resolved.Mmproj -PathType Leaf)) {
        throw "Vision projector missing: $($resolved.Mmproj)"
    }
    Ensure-LocalApiKey $session
    Write-AtomicText $session.base_url_file "$($resolved.BaseUrl)/v1" ([Text.Encoding]::ASCII)
    $logs = Join-Path $session.root 'logs'
    New-Item -ItemType Directory -Force -Path $logs | Out-Null
    $outLog = Join-Path $logs 'session-out.log'
    $errLog = Join-Path $logs 'session-err.log'
    $state = [ordered]@{
        schema = 1
        transaction_id = [Guid]::NewGuid().ToString('N')
        phase = 'starting'
        started_at = (Get-Date).ToUniversalTime().ToString('o')
        pid = $null
        process_started_at = $null
        profile = $Profile
        runtime = $resolved.RuntimeName
        server = [string]$resolved.ServerPath
        server_sha256 = $null
        runtime_build_sha256 = $null
        vision = [bool]$Vision
        cleanup_paused = $false
        cleanup_pid = $null
        fallback = if ($ForceFallback) { 'mtp-only' } else { $null }
    }
    $process = $null
    try {
        if (Test-CleanupEnabled $session) {
            $cleanupPort = [int](Get-PropertyValue $session.cleanup 'port' 0)
            $cleanupProcess = if ($cleanupPort -gt 0) { Get-ProcessOnPort $cleanupPort } else { $null }
            $cleanupExe = [string](Get-PropertyValue $session.cleanup 'exe' '')
            $cleanupCommand = if ($cleanupProcess) { Get-CommandLine ([int]$cleanupProcess.Id) } else { $null }
            if ($cleanupProcess -and $cleanupProcess.Path -and ($cleanupProcess.Path -ieq $cleanupExe) -and (Test-CommandLinePort $cleanupCommand $cleanupPort)) {
                $state.cleanup_pid = [int]$cleanupProcess.Id
                Stop-Process -Id $state.cleanup_pid -Force
                $state.cleanup_paused = $true
                if (-not (Wait-PortFree $cleanupPort 30)) { throw "Cleanup port $cleanupPort did not become free." }
            }
        }
        $state.server_sha256 = Get-FileSha256 ([string]$resolved.ServerPath)
        $runtimeBuildManifest = Join-Path (Split-Path ([string]$resolved.ServerPath) -Parent) 'build-manifest.json'
        if (Test-Path -LiteralPath $runtimeBuildManifest -PathType Leaf) {
            $state.runtime_build_sha256 = Get-FileSha256 $runtimeBuildManifest
        }
        $arguments = New-InferenceArguments $session $profileConfig ([string]$resolved.ServerPath) ([bool]$Vision) -Fallback:$ForceFallback
        $state.arguments = @($arguments)
        $state.profile_sha256 = Get-FileSha256 (Join-Path $session.root "profiles\$Profile.json")
        $state.environment = [ordered]@{
            LLAMA_NGRAM_MOD_RESET_ON_BEGIN = if (-not $ForceFallback -and [bool]$profileConfig.ngram_reset_on_begin) { '1' } else { $null }
        }
        Save-SessionState $state $session
        Write-Host "Starting $Profile on $($resolved.BaseUrl)/health"
        $resetNgram = -not $ForceFallback -and [bool]$profileConfig.ngram_reset_on_begin
        $process = Start-InferenceProcess ([string]$resolved.ServerPath) $arguments $outLog $errLog $resetNgram
        $state.pid = [int]$process.Id
        try { $state.process_started_at = $process.StartTime.ToUniversalTime().ToString('o') } catch { $state.process_started_at = $null }
        Save-SessionState $state $session
        if (-not (Test-StartedProcessHealthy $resolved $state $process 600)) {
            if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue }
            Wait-PortFree ([int]$session.port) 30 | Out-Null
            if ($ForceFallback) { throw 'Pinned MTP-only start failed.' }
            Write-Warning 'Optimized speculative mode failed health; retrying the pinned MTP-only fallback.'
            $arguments = New-InferenceArguments $session $profileConfig ([string]$resolved.ServerPath) ([bool]$Vision) -Fallback
            $process = Start-InferenceProcess ([string]$resolved.ServerPath) $arguments $outLog $errLog $false
            $state.pid = [int]$process.Id
            $state.fallback = 'mtp-only'
            $state.arguments = @($arguments)
            $state.environment.LLAMA_NGRAM_MOD_RESET_ON_BEGIN = $null
            try { $state.process_started_at = $process.StartTime.ToUniversalTime().ToString('o') } catch { $state.process_started_at = $null }
            Save-SessionState $state $session
            if (-not (Test-StartedProcessHealthy $resolved $state $process 600)) { throw 'Both optimized and MTP-only starts failed.' }
        }
        $state.phase = 'healthy'
        $state.healthy_at = (Get-Date).ToUniversalTime().ToString('o')
        Save-SessionState $state $session
        Write-Host "Healthy: $Profile pid=$($process.Id) context=$($profileConfig.context)"
        return [pscustomobject]@{ Started = $true; Status = (Get-InferenceSessionStatusCore -InstallRoot $InstallRoot) }
    } catch {
        $startupFailure = $_
        if ($process -and -not $process.HasExited) { Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue }
        Wait-PortFree ([int]$session.port) 30 | Out-Null
        $cleanupFailure = $null
        try { Restore-CleanupProcess $session $state } catch { $cleanupFailure = $_ }
        $state.phase = 'failed'
        $state.failed = $startupFailure.Exception.Message
        if ($cleanupFailure) { $state.cleanup_restore_failed = $cleanupFailure.Exception.Message }
        $state.failed_at = (Get-Date).ToUniversalTime().ToString('o')
        Save-SessionState $state $session
        if ($cleanupFailure) {
            throw "Inference start failed: $($startupFailure.Exception.Message) Cleanup restoration also failed: $($cleanupFailure.Exception.Message)"
        }
        throw $startupFailure
    }
}

function Stop-InferenceSessionCore {
    [CmdletBinding()]
    param([string]$InstallRoot)
    $session = Get-SessionConfig $InstallRoot
        $state = Read-SessionState $session
        $process = Get-ProcessOnPort ([int]$session.port)
        if ($process) {
            $commandLine = Get-CommandLine ([int]$process.Id)
            if (-not (Test-InferenceProcessIdentity $session $state $process $commandLine)) {
                throw "Port $($session.port) belongs to PID $($process.Id); refusing to kill an unowned listener."
            }
            Stop-Process -Id $process.Id -Force
            if (-not (Wait-PortFree ([int]$session.port) 30)) { throw "Port $($session.port) did not become free." }
            Write-Host "Stopped Inference Session PID $($process.Id)"
        } else { Write-Host "No Inference Session listener on :$($session.port)" }
        if ($state) {
            Restore-CleanupProcess $session $state
            $state.phase = 'stopped'
            $state.stopped_at = (Get-Date).ToUniversalTime().ToString('o')
            Save-SessionState $state $session
        }
        return [pscustomobject]@{ Stopped = [bool]$process; State = $state }
}

function Start-InferenceSession {
    [CmdletBinding()]
    param([string]$InstallRoot, [string]$Profile, [switch]$Vision, [int]$LockTimeoutMilliseconds = 15000)
    $capacity = Enter-InferenceCapacityLease -InstallRoot $InstallRoot -TimeoutMilliseconds $LockTimeoutMilliseconds
    try {
        $session = Get-SessionConfig $InstallRoot
        $lock = Enter-InterprocessLock "$($session.state_file).session.lock" $LockTimeoutMilliseconds
        try { return Start-InferenceSessionCore -InstallRoot $InstallRoot -Profile $Profile -Vision:$Vision }
        finally { Exit-InterprocessLock $lock }
    } finally { Exit-InferenceCapacityLease $capacity }
}

function Stop-InferenceSession {
    [CmdletBinding()]
    param([string]$InstallRoot, [int]$LockTimeoutMilliseconds = 15000)
    $capacity = Enter-InferenceCapacityLease -InstallRoot $InstallRoot -TimeoutMilliseconds $LockTimeoutMilliseconds
    try {
        $session = Get-SessionConfig $InstallRoot
        $lock = Enter-InterprocessLock "$($session.state_file).session.lock" $LockTimeoutMilliseconds
        try { return Stop-InferenceSessionCore -InstallRoot $InstallRoot }
        finally { Exit-InterprocessLock $lock }
    } finally { Exit-InferenceCapacityLease $capacity }
}

function Enter-InferenceSession {
    [CmdletBinding()]
    param([string]$InstallRoot, [string]$Profile, [switch]$Vision, [int]$LockTimeoutMilliseconds = 15000)
    $capacity = Enter-InferenceCapacityLease -InstallRoot $InstallRoot -TimeoutMilliseconds $LockTimeoutMilliseconds
    try {
        $session = Get-SessionConfig $InstallRoot
        $lock = Enter-InterprocessLock "$($session.state_file).session.lock" $LockTimeoutMilliseconds
        $prior = $null
        $changed = $false
        try {
            $prior = Get-InferenceSessionSnapshot -InstallRoot $InstallRoot
            $current = Get-InferenceSessionStatusCore -InstallRoot $InstallRoot
            $selected = if ($Profile) { $Profile } else { (Get-ResolvedSession -InstallRoot $InstallRoot).ProfileName }
            $plan = Resolve-InferenceSessionPlan $current $selected ([bool]$Vision)
            if ($plan -eq 'refuse') { throw 'Inference Session is owned by a foreign listener.' }
            $changed = $plan -in @('start', 'replace')
            if ($plan -eq 'replace') { Stop-InferenceSessionCore -InstallRoot $InstallRoot | Out-Null }
            if ($changed) { Start-InferenceSessionCore -InstallRoot $InstallRoot -Profile $selected -Vision:$Vision | Out-Null }
            $after = Get-InferenceSessionStatusCore -InstallRoot $InstallRoot
            if (-not $after.Active -or -not $after.Healthy -or $after.Profile -ne $selected -or [bool]$after.Vision -ne [bool]$Vision) {
                throw 'Requested Inference Session did not pass post-transition health verification.'
            }
            return [pscustomobject][ordered]@{
                changed = $changed
                profile = [string]$after.Profile
                runtime = [string]$after.Runtime
                server = [string]$after.ExpectedPath
                server_sha256 = [string](Get-PropertyValue $after.State 'server_sha256' '')
                runtime_build_sha256 = Get-PropertyValue $after.State 'runtime_build_sha256' $null
                session_identity = [string](Get-PropertyValue $after.State 'transaction_id' '')
                profile_sha256 = [string](Get-PropertyValue $after.State 'profile_sha256' '')
                arguments = @(Get-PropertyValue $after.State 'arguments' @())
                environment = Get-PropertyValue $after.State 'environment' ([pscustomobject]@{})
                fallback = $after.Fallback
                prior = [pscustomobject][ordered]@{
                    active = [bool]$prior.Active
                    healthy = [bool]$prior.Healthy
                    profile = [string]$prior.Profile
                    vision = [bool]$prior.Vision
                    runtime = [string]$prior.Runtime
                    fallback = $prior.Fallback
                    arguments = @($prior.Arguments)
                    environment = $prior.Environment
                    session_identity = [string](Get-PropertyValue $prior.State 'transaction_id' '')
                }
            }
        } catch {
            $original = $_
            if ($changed -and $null -ne $prior) {
                try {
                    $failed = Get-InferenceSessionStatusCore -InstallRoot $InstallRoot
                    if ($failed.Active) { Stop-InferenceSessionCore -InstallRoot $InstallRoot | Out-Null }
                    if ($prior.Active) {
                        $forceFallback = [string]$prior.Fallback -eq 'mtp-only'
                        Start-InferenceSessionCore -InstallRoot $InstallRoot -Profile $prior.Profile -Vision:$prior.Vision -ForceFallback:$forceFallback | Out-Null
                        $restored = Get-InferenceSessionStatusCore -InstallRoot $InstallRoot
                        if (-not (Test-RestoredInferenceState $restored $prior)) {
                            throw 'Rollback health verification failed.'
                        }
                    }
                } catch {
                    throw "Inference Session transition failed: $($original.Exception.Message) Rollback failed: $($_.Exception.Message)"
                }
            }
            throw $original
        } finally {
            Exit-InterprocessLock $lock
        }
    } finally {
        Exit-InferenceCapacityLease $capacity
    }
}

function Exit-InferenceSession {
    [CmdletBinding()]
    param([string]$InstallRoot, [Parameter(Mandatory = $true)]$Acquisition, [switch]$KeepServer, [int]$LockTimeoutMilliseconds = 15000)
    if ($KeepServer -or -not [bool](Get-PropertyValue $Acquisition 'changed' $false)) { return }
    $capacity = Enter-InferenceCapacityLease -InstallRoot $InstallRoot -TimeoutMilliseconds $LockTimeoutMilliseconds
    try {
        $session = Get-SessionConfig $InstallRoot
        $lock = Enter-InterprocessLock "$($session.state_file).session.lock" $LockTimeoutMilliseconds
        try {
            $current = Get-InferenceSessionStatusCore -InstallRoot $InstallRoot
            if ($current.Foreign) { throw 'Cannot restore the prior Session because the port now has a foreign listener.' }
            if ($current.Active) {
                $currentIdentity = [string](Get-PropertyValue $current.State 'transaction_id' '')
                $acquiredIdentity = [string](Get-PropertyValue $Acquisition 'session_identity' '')
                if (-not $acquiredIdentity -or $currentIdentity -ne $acquiredIdentity) {
                    throw 'Cannot restore the prior Session because the active launch identity changed.'
                }
                Stop-InferenceSessionCore -InstallRoot $InstallRoot | Out-Null
            }
            $prior = Get-PropertyValue $Acquisition 'prior' $null
            if ($prior -and [bool](Get-PropertyValue $prior 'active' $false)) {
                $forceFallback = [string](Get-PropertyValue $prior 'fallback' '') -eq 'mtp-only'
                Start-InferenceSessionCore -InstallRoot $InstallRoot -Profile ([string]$prior.profile) -Vision:([bool]$prior.vision) -ForceFallback:$forceFallback | Out-Null
                $restored = Get-InferenceSessionStatusCore -InstallRoot $InstallRoot
                if (-not (Test-RestoredInferenceState $restored $prior)) {
                    throw 'The pre-Harness Inference Session did not pass restoration health verification.'
                }
            } else {
                $restored = Get-InferenceSessionStatusCore -InstallRoot $InstallRoot
                if ($restored.Active -or $restored.Foreign) { throw 'The pre-Harness idle state was not restored.' }
            }
        } finally {
            Exit-InterprocessLock $lock
        }
    } finally {
        Exit-InferenceCapacityLease $capacity
    }
}
