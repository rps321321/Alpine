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
    [pscustomobject]@{ Label = 'text-fit1024'; FitTarget = 1024 },
    [pscustomobject]@{ Label = 'text-fit768';  FitTarget = 768 },
    [pscustomobject]@{ Label = 'text-fit512';  FitTarget = 512 },
    [pscustomobject]@{ Label = 'text-fit384';  FitTarget = 384 },
    [pscustomobject]@{ Label = 'text-fit256';  FitTarget = 256 }
)

function Wait-Health {
    param([Diagnostics.Process]$Process)

    $deadline = (Get-Date).AddMinutes(5)
    do {
        Start-Sleep -Milliseconds 500
        if ($Process.HasExited) {
            throw "Benchmark server exited with code $($Process.ExitCode)."
        }
        try {
            if ((Invoke-WebRequest -UseBasicParsing -Uri "$baseUrl/health" -TimeoutSec 2).StatusCode -eq 200) {
                return
            }
        }
        catch {
            # Startup is still in progress.
        }
    } while ((Get-Date) -lt $deadline)

    throw 'Benchmark server did not become healthy within five minutes.'
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
            '--fit-target', [string]$variant.FitTarget,
            '--threads', [string]$profileConfig.threads,
            '--threads-batch', [string]$profileConfig.threads,
            '-ctk', [string]$profileConfig.kv_cache,
            '-ctv', [string]$profileConfig.kv_cache,
            '--spec-type', 'draft-mtp',
            '--spec-draft-n-max', [string]$profileConfig.mtp_depth,
            '--reasoning', 'off'
        )

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
        $measurement | Add-Member -NotePropertyName fit_target_mib -NotePropertyValue $variant.FitTarget
        $measurement | Add-Member -NotePropertyName startup_seconds -NotePropertyValue ([Math]::Round($startupTimer.Elapsed.TotalSeconds, 3))
        $measurement | Add-Member -NotePropertyName gpu_memory_mib -NotePropertyValue ([int]$gpuMemory)
        $measurement | Add-Member -NotePropertyName free_system_memory_mib -NotePropertyValue ([int]$freeMemoryMiB)
        $results += $measurement

        Write-Output ("RESULT " + ($measurement | ConvertTo-Json -Compress))
        [Console]::Out.Flush()

        Stop-Process -Id $active.Id -Force
        $active.WaitForExit(10000) | Out-Null
        $active = $null
        Wait-BenchmarkPortFree $benchmark
    }
}
finally {
    if ($active -and -not $active.HasExited) {
        Stop-Process -Id $active.Id -Force -ErrorAction SilentlyContinue
        $active.WaitForExit(10000) | Out-Null
        Wait-BenchmarkPortFree $benchmark
    }
    & $startScript -Profile $Profile
}

$results | ConvertTo-Json -Depth 4
