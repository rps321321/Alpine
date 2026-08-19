[CmdletBinding()]
param(
    [string]$Profile,
    [switch]$Vision
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'lib.ps1')

$session = Get-SessionConfig
$profileConfig = Get-ProfileConfig $session $Profile
$Profile = [string]$profileConfig.name
$profilePath = Join-Path $session.root "profiles\$Profile.json"
$serverPath = Get-RuntimePath $session $profileConfig
$health = "http://$($session.host):$($session.port)/health"
if (Get-Listener $session.port) { throw "Port $($session.port) is occupied; refusing to steal it." }
foreach ($path in @($serverPath, $session.model, $session.chat_template)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Required file missing: $path" }
}
if ($Vision -and -not (Test-Path -LiteralPath $session.mmproj -PathType Leaf)) {
    throw "Vision projector missing: $($session.mmproj)"
}

Ensure-LocalApiKey $session
[IO.File]::WriteAllText($session.base_url_file, "http://$($session.host):$($session.port)/v1", [Text.Encoding]::ASCII)
$logs = Join-Path $session.root 'logs'
New-Item -ItemType Directory -Force -Path $logs | Out-Null
$outLog = Join-Path $logs 'session-out.log'
$errLog = Join-Path $logs 'session-err.log'

$cleanupPaused = $false
$cleanupPid = $null
if (Test-CleanupEnabled $session) {
    $cleanupPort = [int](Get-PropertyValue $session.cleanup 'port' 0)
    if ($cleanupPort -gt 0) {
        $cleanupProcess = Get-ProcessOnPort $cleanupPort
        $cleanupExe = [string](Get-PropertyValue $session.cleanup 'exe' '')
        if ($cleanupProcess -and $cleanupProcess.Path -and ($cleanupProcess.Path -ieq $cleanupExe)) {
            $cleanupPid = $cleanupProcess.Id
            Stop-Process -Id $cleanupPid -Force
            if (-not (Wait-PortFree $cleanupPort 30)) { throw "Cleanup port $cleanupPort did not become free." }
            $cleanupPaused = $true
        }
    }
}

$state = [ordered]@{
    started_at = (Get-Date).ToUniversalTime().ToString('o')
    pid = $null
    profile = $Profile
    runtime = [string]$profileConfig.runtime
    server = $serverPath
    vision = [bool]$Vision
    cleanup_paused = $cleanupPaused
    cleanup_pid = $cleanupPid
    fallback = $null
}
Save-SessionState $state $session

$args = @(
    '-m', $session.model, '--host', $session.host, '--port', "$($session.port)",
    '-c', "$($profileConfig.context)", '-np', "$($profileConfig.parallel)",
    '--threads', "$($profileConfig.threads)", '--threads-batch', "$($profileConfig.threads)",
    '-b', "$($profileConfig.batch_size)", '-ub', "$($profileConfig.ubatch_size)",
    '--no-webui', '--jinja', '--chat-template-file', $session.chat_template,
    '--api-key-file', $session.api_key_file, '-fa', 'on',
    '-ctk', $profileConfig.kv_cache, '-ctv', $profileConfig.kv_cache,
    '--reasoning', 'off'
)
if ($Vision) {
    $args += @('--mmproj', $session.mmproj, '--fit', 'on', '--fit-ctx', "$($profileConfig.context)", '--fit-target', "$($profileConfig.fit_target_mib)")
} else {
    $args += @('-ngl', 'all', '--fit', 'off', '-ot', (Get-TensorOverride ([int]$profileConfig.tensor_cpu_through_block)), '--load-mode', 'none')
}
$baseArgs = @($args)
$specTypes = @('draft-mtp')
if ($profileConfig.ngram_mod) { $specTypes += 'ngram-mod' }
$args += @('--spec-type', ($specTypes -join ','), '--spec-draft-n-max', "$($profileConfig.mtp_depth)")
if ($profileConfig.ngram_mod) {
    $args += @('--spec-ngram-mod-n-match', '24', '--spec-ngram-mod-n-min', '16', '--spec-ngram-mod-n-max', '64')
}
$state.arguments = @($args)
$state.profile_sha256 = Get-FileSha256 $profilePath
$state.environment = [ordered]@{
    LLAMA_NGRAM_MOD_RESET_ON_BEGIN = if ([bool]$profileConfig.ngram_reset_on_begin) { '1' } else { $null }
}
Save-SessionState $state $session

function Start-Llama([string[]]$Arguments, [bool]$ResetNgram) {
    $old = Get-Item Env:LLAMA_NGRAM_MOD_RESET_ON_BEGIN -ErrorAction SilentlyContinue
    try {
        if ($ResetNgram) { $env:LLAMA_NGRAM_MOD_RESET_ON_BEGIN = '1' }
        else { Remove-Item Env:LLAMA_NGRAM_MOD_RESET_ON_BEGIN -ErrorAction SilentlyContinue }
        # Start-Process joins arrays into one Windows command line. Quote every
        # element so install roots and model paths containing spaces survive.
        $quote = [string][char]34
        $nativeArguments = @($Arguments | ForEach-Object { $quote + ([string]$_) + $quote })
        Start-Process -FilePath $serverPath -ArgumentList $nativeArguments `
            -RedirectStandardOutput $outLog -RedirectStandardError $errLog -PassThru -WindowStyle Hidden
    } finally {
        if ($old) { $env:LLAMA_NGRAM_MOD_RESET_ON_BEGIN = $old.Value }
        else { Remove-Item Env:LLAMA_NGRAM_MOD_RESET_ON_BEGIN -ErrorAction SilentlyContinue }
    }
}

try {
    Write-Host "Starting $Profile on $health"
    $process = Start-Llama $args ([bool]$profileConfig.ngram_reset_on_begin)
    $state.pid = $process.Id
    Save-SessionState $state $session
    if (-not (Wait-HttpOk $health 600)) {
        if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue }
        Wait-PortFree $session.port 30 | Out-Null
        Write-Warning 'Optimized speculative mode failed health; retrying the pinned MTP-only fallback.'
        $fallbackArgs = @($baseArgs + @('--spec-type', 'draft-mtp', '--spec-draft-n-max', "$($profileConfig.mtp_depth)"))
        $process = Start-Llama $fallbackArgs $false
        $state.pid = $process.Id
        $state.fallback = 'mtp-only'
        $state.arguments = @($fallbackArgs)
        $state.environment.LLAMA_NGRAM_MOD_RESET_ON_BEGIN = $null
        Save-SessionState $state $session
        if (-not (Wait-HttpOk $health 600)) { throw 'Both optimized and MTP-only starts failed.' }
    }
    Write-Host "Healthy: $Profile pid=$($process.Id) context=$($profileConfig.context)"
} catch {
    if ($cleanupPaused) {
        $cleanupStart = [string](Get-PropertyValue $session.cleanup 'start_script' '')
        if ($cleanupStart) { & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $cleanupStart }
    }
    $state.failed = $_.Exception.Message
    Save-SessionState $state $session
    throw
}
