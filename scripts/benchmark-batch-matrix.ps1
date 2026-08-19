param(
    [ValidateRange(32, 2048)]
    [int]$Tokens = 128,
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
$template = $benchmark.ChatTemplate
$logs = $benchmark.Logs
$stopScript = $benchmark.StopScript
$startScript = $benchmark.StartScript
$baseUrl = $benchmark.BaseUrl

$variants = @(
    [pscustomobject]@{ Label = 'b2048-ub512';  Batch = 2048; UBatch = 512 },
    [pscustomobject]@{ Label = 'b2048-ub1024'; Batch = 2048; UBatch = 1024 },
    [pscustomobject]@{ Label = 'b4096-ub512';  Batch = 4096; UBatch = 512 },
    [pscustomobject]@{ Label = 'b4096-ub1024'; Batch = 4096; UBatch = 1024 },
    [pscustomobject]@{ Label = 'b1024-ub256';  Batch = 1024; UBatch = 256 }
)

$prompt = ('A compiler optimization is safe when observable behavior is preserved across all permitted inputs. ' * 270).Trim()

function Wait-Health {
    param([Diagnostics.Process]$Process)
    $deadline = (Get-Date).AddMinutes(5)
    do {
        Start-Sleep -Milliseconds 500
        if ($Process.HasExited) { throw "Benchmark server exited with code $($Process.ExitCode)." }
        try {
            if ((Invoke-WebRequest -UseBasicParsing -Uri "$baseUrl/health" -TimeoutSec 2).StatusCode -eq 200) { return }
        }
        catch { }
    } while ((Get-Date) -lt $deadline)
    throw 'Benchmark server did not become healthy within five minutes.'
}

$active = $null
$results = @()
try {
    & $stopScript
    foreach ($variant in $variants) {
        $args = @(
            '-m', $model, '--host', $benchmark.Host, '--port', [string]$benchmark.Port,
            '-c', [string]$profileConfig.context, '-np', [string]$profileConfig.parallel,
            '--threads', [string]$profileConfig.threads, '--threads-batch', [string]$profileConfig.threads,
            '--no-webui', '--jinja', '--chat-template-file', $template,
            '-fa', 'on', '--fit', 'on', '--fit-ctx', [string]$profileConfig.context, '--fit-target', [string]$profileConfig.fit_target_mib,
            '-ctk', [string]$profileConfig.kv_cache, '-ctv', [string]$profileConfig.kv_cache, '--reasoning', 'off',
            '--spec-type', 'draft-mtp', '--spec-draft-n-max', [string]$profileConfig.mtp_depth,
            '-b', [string]$variant.Batch, '-ub', [string]$variant.UBatch
        )
        $stdout = Join-Path $logs "bench-$($variant.Label).out.log"
        $stderr = Join-Path $logs "bench-$($variant.Label).err.log"
        $active = Start-Process -FilePath $server -ArgumentList $args `
            -RedirectStandardOutput $stdout -RedirectStandardError $stderr `
            -PassThru -WindowStyle Hidden
        Wait-Health -Process $active

        $body = @{
            prompt = $prompt
            n_predict = $Tokens
            temperature = 0.0
            top_k = 1
            seed = 42
            ignore_eos = $true
            cache_prompt = $false
        } | ConvertTo-Json -Compress
        $timer = [Diagnostics.Stopwatch]::StartNew()
        $response = Invoke-RestMethod -Method Post -Uri "$baseUrl/completion" `
            -ContentType 'application/json' -Body $body -TimeoutSec 900
        $timer.Stop()
        $gpuMemory = [int]((& nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits | Select-Object -First 1).Trim())
        $result = [pscustomobject]@{
            label = $variant.Label
            batch = $variant.Batch
            ubatch = $variant.UBatch
            wall_seconds = [Math]::Round($timer.Elapsed.TotalSeconds, 3)
            tokens_evaluated = [int]$response.tokens_evaluated
            tokens_predicted = [int]$response.tokens_predicted
            prompt_per_second = [Math]::Round([double]$response.timings.prompt_per_second, 3)
            predicted_per_second = [Math]::Round([double]$response.timings.predicted_per_second, 3)
            gpu_memory_mib = $gpuMemory
        }
        $results += $result
        Write-Output ('RESULT ' + ($result | ConvertTo-Json -Compress))
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
