$ErrorActionPreference = 'Stop'

function Get-FileSha256([string]$Path) {
    $stream = [IO.File]::OpenRead($Path)
    $algorithm = [Security.Cryptography.SHA256]::Create()
    try {
        ([BitConverter]::ToString($algorithm.ComputeHash($stream)) -replace '-', '').ToLowerInvariant()
    } finally {
        $algorithm.Dispose()
        $stream.Dispose()
    }
}

function Get-SessionConfig {
    $root = Split-Path $PSScriptRoot -Parent
    $path = Join-Path $root 'config\session.json'
    if (-not (Test-Path -LiteralPath $path)) { throw "Session config missing: $path" }
    Get-Content -Raw -LiteralPath $path | ConvertFrom-Json
}

function Get-ProfileConfig {
    param($Session, [string]$Name)
    $selected = if ($Name) { $Name } else { [string]$Session.active_profile }
    $path = Join-Path $Session.root "profiles\$selected.json"
    if (-not (Test-Path -LiteralPath $path)) { throw "Profile missing: $path" }
    Get-Content -Raw -LiteralPath $path | ConvertFrom-Json
}

function Get-RuntimePath {
    param($Session, $Profile)
    $runtimeName = [string](Get-PropertyValue $Profile 'runtime' '')
    if ($runtimeName -and $Session.PSObject.Properties['runtimes']) {
        $property = $Session.runtimes.PSObject.Properties[$runtimeName]
        if (-not $property -or -not $property.Value) { throw "Runtime '$runtimeName' is not installed." }
        return [string]$property.Value
    }
    return [string]$Session.llama_server
}

function Get-PropertyValue {
    param($Object, [string]$Name, $Default = $null)
    if ($null -eq $Object) { return $Default }
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property -or $null -eq $property.Value) { return $Default }
    return $property.Value
}

function Get-TensorOverride([int]$ThroughBlock) {
    $pattern = ((0..$ThroughBlock) -join '|')
    return "blk\.($pattern)\.ffn_.*=CPU"
}

function Get-Listener([int]$Port) {
    Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue | Select-Object -First 1
}

function Get-ProcessOnPort([int]$Port) {
    $listener = Get-Listener $Port
    if (-not $listener) { return $null }
    Get-Process -Id $listener.OwningProcess -ErrorAction SilentlyContinue
}

function Get-CommandLine([int]$Id) {
    (Get-CimInstance Win32_Process -Filter "ProcessId=$Id" -ErrorAction SilentlyContinue).CommandLine
}

function Test-HttpOk([string]$Url, [int]$TimeoutSec = 3) {
    try {
        $code = & curl.exe -sS -o NUL -w '%{http_code}' --connect-timeout $TimeoutSec --max-time $TimeoutSec $Url 2>$null
        return ($LASTEXITCODE -eq 0 -and [int]$code -ge 200 -and [int]$code -lt 300)
    } catch { return $false }
}

function Wait-HttpOk([string]$Url, [int]$TimeoutSec = 600) {
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    do {
        if (Test-HttpOk $Url 3) { return $true }
        Start-Sleep -Seconds 2
    } while ((Get-Date) -lt $deadline)
    return $false
}

function Wait-PortFree([int]$Port, [int]$TimeoutSec = 30) {
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    do {
        if (-not (Get-Listener $Port)) { return $true }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $deadline)
    return $false
}

function Read-SessionState($Session) {
    if (-not (Test-Path -LiteralPath $Session.state_file)) { return $null }
    Get-Content -Raw -LiteralPath $Session.state_file | ConvertFrom-Json
}

function Save-SessionState($State, $Session) {
    $parent = Split-Path $Session.state_file -Parent
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    $json = $State | ConvertTo-Json -Depth 8
    [IO.File]::WriteAllText($Session.state_file, $json + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
}

function Ensure-LocalApiKey($Session) {
    if (Test-Path -LiteralPath $Session.api_key_file) { return }
    $bytes = New-Object byte[] 32
    $rng = [Security.Cryptography.RandomNumberGenerator]::Create()
    try { $rng.GetBytes($bytes) } finally { $rng.Dispose() }
    $key = 'sk-local-' + (([BitConverter]::ToString($bytes)) -replace '-', '').ToLowerInvariant()
    [IO.File]::WriteAllText($Session.api_key_file, $key, [Text.UTF8Encoding]::new($false))
}

function Test-CleanupEnabled($Session) {
    [bool](Get-PropertyValue $Session.cleanup 'enabled' $false)
}
