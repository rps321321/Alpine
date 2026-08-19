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
$mmproj = $benchmark.Mmproj
$logs = $benchmark.Logs
$stopScript = $benchmark.StopScript
$startScript = $benchmark.StartScript
$benchmarkScript = Join-Path $PSScriptRoot 'benchmark-inference.ps1'
$baseUrl = $benchmark.BaseUrl

$variants = @(
    [pscustomobject]@{ Label = 'np1-mtp3-vision';   Mtp = $true;  NMax = 3;  PMin = $null; Vision = $true },
    [pscustomobject]@{ Label = 'np1-mtp2-vision';   Mtp = $true;  NMax = 2;  PMin = $null; Vision = $true },
    [pscustomobject]@{ Label = 'np1-mtp1-vision';   Mtp = $true;  NMax = 1;  PMin = $null; Vision = $true },
    [pscustomobject]@{ Label = 'np1-no-mtp-vision'; Mtp = $false; NMax = 0;  PMin = $null; Vision = $true },
    [pscustomobject]@{ Label = 'np1-mtp4-vision';   Mtp = $true;  NMax = 4;  PMin = $null; Vision = $true },
    [pscustomobject]@{ Label = 'np1-mtp8-p80-vision';  Mtp = $true; NMax = 8;  PMin = 0.8; Vision = $true },
    [pscustomobject]@{ Label = 'np1-mtp16-p80-vision'; Mtp = $true; NMax = 16; PMin = 0.8; Vision = $true },
    [pscustomobject]@{ Label = 'np1-mtp3-text';     Mtp = $true;  NMax = 3;  PMin = $null; Vision = $false }
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

function Wait-PortFree {
    $deadline = (Get-Date).AddSeconds(30)
    do {
        $listener = Get-NetTCPConnection -LocalPort $benchmark.Port -State Listen -ErrorAction SilentlyContinue
        if (-not $listener) { return }
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
            '-ctk', [string]$profileConfig.kv_cache,
            '-ctv', [string]$profileConfig.kv_cache,
            '--reasoning', 'off'
        )
        if ($variant.Vision) {
            $args += @('--mmproj', $mmproj)
        }
        if ($variant.Mtp) {
            $args += @('--spec-type', 'draft-mtp', '--spec-draft-n-max', [string]$variant.NMax)
            if ($null -ne $variant.PMin) {
                $args += @('--spec-draft-p-min', [string]$variant.PMin)
            }
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
