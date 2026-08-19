[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Project,
    [string]$Profile = 'stable-16k',
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9a-fA-F]{32}$')]
    [string]$LaunchId,
    [switch]$WithVision,
    [switch]$Lean,
    [switch]$FullPrompt,
    [switch]$WithPlugins,
    [switch]$WithSkills,
    [switch]$DiagnosticFailure,
    [string]$DiagnosticFailureMessage = 'Deterministic installed launcher diagnostic failure.'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Protect-LauncherFailureText {
    param([AllowEmptyString()][string]$Text)
    $redacted = [string]$Text
    $sensitiveName = '(?i)(?:KEY|TOKEN|SECRET|PASSWORD|PASS|CREDENTIAL|COOKIE|AUTH|DATABASE_URL|REDIS_URL|DSN|CONNECTION|CONNSTR|PRIVATE)'
    foreach ($item in (Get-ChildItem Env: | Sort-Object { ([string]$_.Value).Length } -Descending)) {
        if ($item.Name -notmatch $sensitiveName -or -not $item.Value) { continue }
        $redacted = $redacted.Replace([string]$item.Value, '<REDACTED>')
    }
    $redacted = [Regex]::Replace(
        $redacted,
        '(?i)\b([a-z][a-z0-9+.-]*://)([^/\s:@]+):([^@\s/]+)@',
        '$1<REDACTED>:<REDACTED>@'
    )
    $redacted = [Regex]::Replace(
        $redacted,
        '(?i)(authorization\s*:\s*bearer\s+)[^\s"'']+',
        '$1<REDACTED>'
    )
    $redacted = [Regex]::Replace($redacted, '(?i)\bsk-[A-Za-z0-9_-]{6,}\b', '<REDACTED>')
    $redacted = [Regex]::Replace(
        $redacted,
        '(?i)\b(api[_-]?key|access[_-]?key|client[_-]?secret|private[_-]?key|token|password|secret|credential|database[_-]?url|redis[_-]?url|dsn|connection[_-]?string)\b(\s*[:=]\s*)(?:"[^"]*"|''[^'']*''|[^\s;]+)',
        '$1$2<REDACTED>'
    )
    $redacted = [Regex]::Replace(
        $redacted,
        '(?i)\b(user\s*id|uid|password|pwd)\b(\s*=\s*)(?:"[^"]*"|''[^'']*''|[^;\s]+)',
        '$1$2<REDACTED>'
    )
    return $redacted
}

function Write-AtomicLauncherText {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Content
    )
    $parent = Split-Path $Path -Parent
    if ($parent) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }
    $temporary = "$Path.$([Guid]::NewGuid().ToString('N')).tmp"
    $backup = "$Path.$([Guid]::NewGuid().ToString('N')).bak"
    try {
        [IO.File]::WriteAllText($temporary, $Content, [Text.UTF8Encoding]::new($false))
        if (Test-Path -LiteralPath $Path) { [IO.File]::Replace($temporary, $Path, $backup) }
        else { [IO.File]::Move($temporary, $Path) }
    } finally {
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
        if (Test-Path -LiteralPath $backup) { Remove-Item -LiteralPath $backup -Force }
    }
}

function Write-LauncherFailureLog {
    param(
        [Parameter(Mandatory = $true)][string]$InvocationPath,
        [Parameter(Mandatory = $true)][string]$StablePath,
        [Parameter(Mandatory = $true)][string]$Message
    )
    $content = @(
        "timestamp=$((Get-Date).ToUniversalTime().ToString('o'))"
        "launch_id=$LaunchId"
        "profile=$(Protect-LauncherFailureText $Profile)"
        "project=$(Protect-LauncherFailureText $Project)"
        'error:'
        (Protect-LauncherFailureText $Message)
    ) -join [Environment]::NewLine
    $content += [Environment]::NewLine
    Write-AtomicLauncherText -Path $InvocationPath -Content $content

    $mutex = [Threading.Mutex]::new($false, 'Local\OpenLocalQwenLauncherFailureLog')
    $acquired = $false
    try {
        try { $acquired = $mutex.WaitOne([TimeSpan]::FromSeconds(5)) }
        catch [Threading.AbandonedMutexException] { $acquired = $true }
        if (-not $acquired) { throw 'Timed out while publishing the stable launcher failure log.' }
        Write-AtomicLauncherText -Path $StablePath -Content $content
    } finally {
        if ($acquired) { $mutex.ReleaseMutex() }
        $mutex.Dispose()
    }
}

$installRoot = Split-Path $PSScriptRoot -Parent
$launcher = Join-Path $PSScriptRoot 'open-local-opencode.ps1'
$invocationLog = Join-Path $installRoot "logs\launcher-errors\$LaunchId.log"
$failureLog = Join-Path $installRoot 'logs\launcher-last-error.log'

try {
    if ($DiagnosticFailure) {
        throw $DiagnosticFailureMessage
    }
    $launcherParameters = @{
        Project = $Project
        Profile = $Profile
        Supervised = $true
    }
    if ($WithVision) { $launcherParameters.WithVision = $true }
    if ($Lean) { $launcherParameters.Lean = $true }
    if ($FullPrompt) { $launcherParameters.FullPrompt = $true }
    if ($WithPlugins) { $launcherParameters.WithPlugins = $true }
    if ($WithSkills) { $launcherParameters.WithSkills = $true }
    & $launcher @launcherParameters
} catch {
    $failureText = Protect-LauncherFailureText ($_ | Out-String)
    try {
        Write-LauncherFailureLog -InvocationPath $invocationLog -StablePath $failureLog -Message $failureText
    } catch {
        $failureText += "`nThe supervisor could not publish its failure log: $failureLog"
    }
    [Console]::Error.WriteLine($failureText.TrimEnd())
    [Console]::Error.WriteLine("Launcher failure log: $failureLog")
    if ($env:LOCALMODEL_LAUNCHER_NO_DIALOG -ne '1') {
        [void](Read-Host 'Press Enter to close this failed launcher')
    }
    exit 1
}
exit 0
