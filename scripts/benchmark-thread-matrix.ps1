param(
    [ValidateRange(16, 2048)]
    [int]$Tokens = 256
)

$ErrorActionPreference = 'Stop'

$root = '%USERPROFILE%\local-models'
$server = Join-Path $root 'runtime\llama-server.exe'
$model = Join-Path $root 'models\Qwen3.8-27B-ABLITERATED-Q4_K_M.gguf'
$logs = Join-Path $root 'logs'
$stopScript = Join-Path $root 'scripts\stop-session.ps1'
$startScript = Join-Path $root 'scripts\start-session.ps1'
$benchmarkScript = Join-Path $PSScriptRoot 'benchmark-inference.ps1'
$baseUrl = 'http://127.0.0.1:8100'

$variants = @(8, 12, 16, 20, 24) | ForEach-Object {
    [pscustomobject]@{ Label = "text-fit512-t$_"; Threads = $_ }
}

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
        if (-not (Get-NetTCPConnection -LocalPort 8100 -State Listen -ErrorAction SilentlyContinue)) { return }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $deadline)
    throw 'Port 8100 did not become free.'
}

$active = $null
$results = @()
try {
    & $stopScript

    foreach ($variant in $variants) {
        $args = @(
            '-m', $model,
            '--host', '127.0.0.1',
            '--port', '8100',
            '-c', '16384',
            '-np', '1',
            '--no-webui',
            '--jinja',
            '-fa', 'on',
            '--fit', 'on',
            '--fit-ctx', '16384',
            '--fit-target', '512',
            '--threads', [string]$variant.Threads,
            '--threads-batch', [string]$variant.Threads,
            '-ctk', 'q8_0',
            '-ctv', 'q8_0',
            '--spec-type', 'draft-mtp',
            '--spec-draft-n-max', '3',
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
        $measurement | Add-Member -NotePropertyName threads -NotePropertyValue $variant.Threads
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
    & $startScript
}

$results | ConvertTo-Json -Depth 4
