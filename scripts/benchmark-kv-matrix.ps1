param(
    [ValidateRange(16, 2048)]
    [int]$Tokens = 256,
    [string]$InstallRoot = (Join-Path $env:USERPROFILE 'local-models'),
    [string]$Profile,
    [switch]$ResolveOnly
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'benchmark-common.ps1')
$benchmark = Get-BenchmarkContext $InstallRoot $Profile
if ($ResolveOnly) { Write-BenchmarkResolution $benchmark; return }
$Profile = $benchmark.ProfileName
$profileConfig = $benchmark.Profile
$server = $benchmark.Server
$model = $benchmark.Model
$logs = $benchmark.Logs
$stopScript = $benchmark.StopScript
$startScript = $benchmark.StartScript
$benchmarkScript = Join-Path $PSScriptRoot 'benchmark-inference.ps1'
$baseUrl = $benchmark.BaseUrl

$variants = @(
    [pscustomobject]@{ Label = 'kv-target-q8-draft-f16'; Target = 'q8_0'; Draft = $null },
    [pscustomobject]@{ Label = 'kv-target-q8-draft-q8';  Target = 'q8_0'; Draft = 'q8_0' },
    [pscustomobject]@{ Label = 'kv-target-q5-draft-q8';  Target = 'q5_1'; Draft = 'q8_0' },
    [pscustomobject]@{ Label = 'kv-target-q4-draft-q8';  Target = 'q4_1'; Draft = 'q8_0' },
    [pscustomobject]@{ Label = 'kv-target-q8-draft-q5';  Target = 'q8_0'; Draft = 'q5_1' }
)

function Wait-Health {
    param([Diagnostics.Process]$Process)

    $deadline = (Get-Date).AddMinutes(5)
    do {
        Start-Sleep -Milliseconds 500
        if ($Process.HasExited) { throw "Benchmark server exited with code $($Process.ExitCode)." }
        try {
            if ((Invoke-WebRequest -UseBasicParsing -Uri "$baseUrl/health" -TimeoutSec 2).StatusCode -eq 200) { return }
        }
        catch {
            # Startup is still in progress.
        }
    } while ((Get-Date) -lt $deadline)

    throw 'Benchmark server did not become healthy within five minutes.'
}

function Wait-PortFree {
    $deadline = (Get-Date).AddSeconds(30)
    do {
        if (-not (Get-NetTCPConnection -LocalPort $benchmark.Port -State Listen -ErrorAction SilentlyContinue)) { return }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $deadline)
    throw "Port $($benchmark.Port) did not become free."
}

$active = $null
$results = @()
try {
    & $stopScript

    foreach ($variant in $variants) {
        $args = @(
            '-m', $model,
            '--host', $benchmark.Host,
            '--port', [string]$benchmark.Port,
            '-c', [string]$profileConfig.context,
            '-np', [string]$profileConfig.parallel,
            '--no-webui',
            '--jinja',
            '-fa', 'on',
            '--fit', 'on',
            '--fit-ctx', [string]$profileConfig.context,
            '--fit-target', [string]$profileConfig.fit_target_mib,
            '--threads', [string]$profileConfig.threads,
            '--threads-batch', [string]$profileConfig.threads,
            '-ctk', $variant.Target,
            '-ctv', $variant.Target,
            '--spec-type', 'draft-mtp',
            '--spec-draft-n-max', [string]$profileConfig.mtp_depth,
            '--reasoning', 'off'
        )
        if ($variant.Draft) {
            $args += @(
                '--spec-draft-type-k', $variant.Draft,
                '--spec-draft-type-v', $variant.Draft
            )
        }

        $stdout = Join-Path $logs ("bench-$($variant.Label).out.log")
        $stderr = Join-Path $logs ("bench-$($variant.Label).err.log")
        $startupTimer = [Diagnostics.Stopwatch]::StartNew()
        $active = Start-Process -FilePath $server -ArgumentList $args `
            -RedirectStandardOutput $stdout -RedirectStandardError $stderr `
            -PassThru -WindowStyle Hidden
        Wait-Health -Process $active
        $startupTimer.Stop()

        $gpuMemory = (& nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits | Select-Object -First 1).Trim()
        $freeMemoryMiB = [Math]::Round((Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory / 1024, 0)
        $measurement = (& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $benchmarkScript `
            -BaseUrl $baseUrl -Label $variant.Label -Tokens $Tokens -Runs 1 | Out-String) | ConvertFrom-Json
        $measurement | Add-Member -NotePropertyName target_kv -NotePropertyValue $variant.Target
        $measurement | Add-Member -NotePropertyName draft_kv -NotePropertyValue $(if ($variant.Draft) { $variant.Draft } else { 'f16-default' })
        $measurement | Add-Member -NotePropertyName startup_seconds -NotePropertyValue ([Math]::Round($startupTimer.Elapsed.TotalSeconds, 3))
        $measurement | Add-Member -NotePropertyName gpu_memory_mib -NotePropertyValue ([int]$gpuMemory)
        $measurement | Add-Member -NotePropertyName free_system_memory_mib -NotePropertyValue ([int]$freeMemoryMiB)
        $results += $measurement

        Write-Output ("RESULT " + ($measurement | ConvertTo-Json -Compress))
        [Console]::Out.Flush()

        Stop-Process -Id $active.Id -Force
        $active.WaitForExit(10000) | Out-Null
        $active = $null
        Wait-PortFree
    }
}
finally {
    if ($active -and -not $active.HasExited) {
        Stop-Process -Id $active.Id -Force -ErrorAction SilentlyContinue
        $active.WaitForExit(10000) | Out-Null
        Wait-PortFree
    }
    & $startScript -Profile $Profile
}

$results | ConvertTo-Json -Depth 4
