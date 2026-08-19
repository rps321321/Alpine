param(
    [ValidateRange(64, 1024)]
    [int]$DecodeTokens = 256,
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
$ubatches = 512, 640, 768, 896, 1024
$prefillPrompt = ('A compiler optimization is safe when observable behavior is preserved across all permitted inputs. ' * 270).Trim()
$decodePrompt = "Complete the following technical explanation with coherent prose and examples:`nA compiler optimization is safe when"

function Wait-Health([Diagnostics.Process]$Process) {
    $deadline = (Get-Date).AddMinutes(5)
    do {
        Start-Sleep -Milliseconds 500
        if ($Process.HasExited) { throw "Benchmark server exited with code $($Process.ExitCode)." }
        try { if ((Invoke-WebRequest -UseBasicParsing -Uri "$baseUrl/health" -TimeoutSec 2).StatusCode -eq 200) { return } } catch { }
    } while ((Get-Date) -lt $deadline)
    throw 'Benchmark server did not become healthy within five minutes.'
}

function Invoke-Completion([string]$Prompt, [int]$TokenCount) {
    $body = @{
        prompt = $Prompt
        n_predict = $TokenCount
        temperature = 0.0
        top_k = 1
        seed = 42
        ignore_eos = $true
        cache_prompt = $false
    } | ConvertTo-Json -Compress
    Invoke-RestMethod -Method Post -Uri "$baseUrl/completion" -ContentType 'application/json' -Body $body -TimeoutSec 900
}

function Get-ContentHash([string]$Content) {
    $bytes = [Text.Encoding]::UTF8.GetBytes($Content)
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $digest = $sha.ComputeHash($bytes) } finally { $sha.Dispose() }
    (([BitConverter]::ToString($digest)) -replace '-', '').ToLowerInvariant()
}

$active = $null
$results = @()
try {
    & $stopScript
    foreach ($ubatch in $ubatches) {
        $args = @(
            '-m', $model, '--host', $benchmark.Host, '--port', [string]$benchmark.Port,
            '-c', [string]$profileConfig.context, '-np', [string]$profileConfig.parallel,
            '--threads', [string]$profileConfig.threads, '--threads-batch', [string]$profileConfig.threads,
            '-b', [string]$profileConfig.batch_size, '-ub', [string]$ubatch,
            '--no-webui', '--jinja', '--chat-template-file', $template,
            '-fa', 'on', '--fit', 'on', '--fit-ctx', [string]$profileConfig.context, '--fit-target', [string]$profileConfig.fit_target_mib,
            '-ctk', [string]$profileConfig.kv_cache, '-ctv', [string]$profileConfig.kv_cache, '--reasoning', 'off',
            '--spec-type', 'draft-mtp', '--spec-draft-n-max', [string]$profileConfig.mtp_depth
        )
        $stdout = Join-Path $logs "bench-focused-ub$ubatch.out.log"
        $stderr = Join-Path $logs "bench-focused-ub$ubatch.err.log"
        $active = Start-Process -FilePath $server -ArgumentList $args `
            -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru -WindowStyle Hidden
        Wait-Health $active

        $prefill = Invoke-Completion -Prompt $prefillPrompt -TokenCount 1
        $decode = Invoke-Completion -Prompt $decodePrompt -TokenCount $DecodeTokens
        $gpuMemory = [int]((& nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits | Select-Object -First 1).Trim())
        $result = [pscustomobject]@{
            ubatch = $ubatch
            prefill_tokens = [int]$prefill.tokens_evaluated
            prefill_tps = [Math]::Round([double]$prefill.timings.prompt_per_second, 3)
            prefill_seconds = [Math]::Round(([double]$prefill.tokens_evaluated / [double]$prefill.timings.prompt_per_second), 3)
            decode_tps = [Math]::Round([double]$decode.timings.predicted_per_second, 4)
            decode_seconds = [Math]::Round(([double]$decode.tokens_predicted / [double]$decode.timings.predicted_per_second), 3)
            combined_seconds = [Math]::Round(
                ([double]$prefill.tokens_evaluated / [double]$prefill.timings.prompt_per_second) +
                ([double]$decode.tokens_predicted / [double]$decode.timings.predicted_per_second), 3)
            output_sha256 = Get-ContentHash ([string]$decode.content)
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
