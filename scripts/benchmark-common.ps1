$ErrorActionPreference = 'Stop'
$script:BenchmarkRepoRoot = Split-Path $PSScriptRoot -Parent
. (Join-Path $script:BenchmarkRepoRoot 'runtime\scripts\lib.ps1')

function Get-BenchmarkContext {
    param([string]$InstallRoot, [string]$Profile)
    if (-not $InstallRoot) { $InstallRoot = Join-Path $env:USERPROFILE 'local-models' }
    $resolved = Get-ResolvedSession -InstallRoot $InstallRoot -Name $Profile -RequireRuntime
    $session = $resolved.Session
    return [pscustomobject][ordered]@{
        InstallRoot = [IO.Path]::GetFullPath($InstallRoot)
        ProfileName = $resolved.ProfileName
        Profile = $resolved.Profile
        Session = $session
        Server = [string]$resolved.ServerPath
        Model = [string]$resolved.Model
        Mmproj = [string]$resolved.Mmproj
        ChatTemplate = [string]$resolved.ChatTemplate
        Logs = Join-Path $session.root 'logs'
        StopScript = Join-Path $session.root 'scripts\stop-session.ps1'
        StartScript = Join-Path $session.root 'scripts\start-session.ps1'
        BaseUrl = $resolved.BaseUrl
        ApiKeyFile = [string]$resolved.ApiKeyFile
        Host = [string]$session.host
        Port = [int]$session.port
    }
}

function Write-BenchmarkResolution($Context) {
    [pscustomobject][ordered]@{
        install_root = $Context.InstallRoot
        profile = $Context.ProfileName
        runtime = [string]$Context.Profile.runtime
        server = $Context.Server
        model = $Context.Model
        host = $Context.Host
        port = $Context.Port
        context = [int]$Context.Profile.context
        output = [int]$Context.Profile.output
        base_url = $Context.BaseUrl
    } | ConvertTo-Json -Compress
}

function Wait-BenchmarkHealth {
    param([Diagnostics.Process]$Process, $Context, [int]$Minutes = 5)
    $deadline = (Get-Date).AddMinutes($Minutes)
    do {
        Start-Sleep -Milliseconds 500
        if ($Process.HasExited) { throw "Benchmark server exited with code $($Process.ExitCode)." }
        try {
            if ((Invoke-WebRequest -UseBasicParsing -Uri "$($Context.BaseUrl)/health" -TimeoutSec 2).StatusCode -eq 200) { return }
        } catch { }
    } while ((Get-Date) -lt $deadline)
    throw "Benchmark server did not become healthy within $Minutes minutes."
}

function Wait-BenchmarkPortFree {
    param($Context, [int]$TimeoutSeconds = 30)
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        if (-not (Get-NetTCPConnection -LocalPort $Context.Port -State Listen -ErrorAction SilentlyContinue)) { return }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $deadline)
    throw "Port $($Context.Port) did not become free."
}
