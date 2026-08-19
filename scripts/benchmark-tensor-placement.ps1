param(
    [ValidateRange(64, 1024)]
    [int]$DecodeTokens = 256,
    [string[]]$Labels
)

$ErrorActionPreference = 'Stop'
$root = '%USERPROFILE%\local-models'
$server = Join-Path $root 'runtime\llama-server.exe'
$model = Join-Path $root 'models\Qwen3.8-27B-ABLITERATED-Q4_K_M.gguf'
$template = Join-Path $root 'config\qwen3.8-official-chat-template.jinja'
$logs = Join-Path $root 'logs'
$stopScript = Join-Path $root 'scripts\stop-session.ps1'
$startScript = Join-Path $root 'scripts\start-session.ps1'
$baseUrl = 'http://127.0.0.1:8100'

$prefillPrompt = ('A compiler optimization is safe when observable behavior is preserved across all permitted inputs. ' * 270).Trim()
$decodePrompt = "Complete the following technical explanation with coherent prose and examples:`nA compiler optimization is safe when"

# llama.cpp's automatic fit offloads complete early layers. These two alternatives
# keep attention/SSM tensors on the GPU and place only the large FFN matrices on CPU.
$variants = @(
    [pscustomobject]@{ label = 'auto-fit'; override = $null; loadMode = $null },
    [pscustomobject]@{ label = 'ffn-cpu-0-41-load-none'; override = 'blk\.([0-9]|[1-3][0-9]|4[0-1])\.ffn_.*=CPU'; loadMode = 'none' },
    [pscustomobject]@{ label = 'ffn-cpu-0-42-load-none'; override = 'blk\.([0-9]|[1-3][0-9]|4[0-2])\.ffn_.*=CPU'; loadMode = 'none' },
    [pscustomobject]@{ label = 'ffn-cpu-0-43'; override = 'blk\.([0-9]|[1-3][0-9]|4[0-3])\.ffn_.*=CPU'; loadMode = $null },
    [pscustomobject]@{ label = 'ffn-cpu-0-43-load-none'; override = 'blk\.([0-9]|[1-3][0-9]|4[0-3])\.ffn_.*=CPU'; loadMode = 'none' },
    [pscustomobject]@{ label = 'ffn-cpu-0-47'; override = 'blk\.([0-9]|[1-3][0-9]|4[0-7])\.ffn_.*=CPU'; loadMode = $null }
)
if ($Labels) {
    $variants = @($variants | Where-Object { $_.label -in $Labels })
    if (-not $variants) { throw "No benchmark variants matched: $($Labels -join ', ')" }
}

function Wait-Health([Diagnostics.Process]$Process) {
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

function Wait-PortFree {
    $deadline = (Get-Date).AddSeconds(30)
    do {
        if (-not (Get-NetTCPConnection -LocalPort 8100 -State Listen -ErrorAction SilentlyContinue)) { return }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $deadline)
    throw 'Port 8100 did not become free.'
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
    foreach ($variant in $variants) {
        $args = @(
            '-m', $model, '--host', '127.0.0.1', '--port', '8100', '-c', '16384', '-np', '1',
            '--threads', '16', '--threads-batch', '16', '-b', '2048', '-ub', '768',
            '--no-webui', '--jinja', '--chat-template-file', $template,
            '-fa', 'on', '-ctk', 'q8_0', '-ctv', 'q8_0', '--reasoning', 'off',
            '--spec-type', 'draft-mtp', '--spec-draft-n-max', '3', '-lv', '4'
        )
        if ($variant.override) {
            $args += @('-ngl', 'all', '--fit', 'off', '-ot', $variant.override)
            if ($variant.loadMode) { $args += @('--load-mode', $variant.loadMode) }
        }
        else {
            $args += @('--fit', 'on', '--fit-ctx', '16384', '--fit-target', '512')
        }

        $stdout = Join-Path $logs "bench-placement-$($variant.label).out.log"
        $stderr = Join-Path $logs "bench-placement-$($variant.label).err.log"
        $active = Start-Process -FilePath $server -ArgumentList $args `
            -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru -WindowStyle Hidden
        Wait-Health $active

        $prefill = Invoke-Completion -Prompt $prefillPrompt -TokenCount 1
        $decode = Invoke-Completion -Prompt $decodePrompt -TokenCount $DecodeTokens
        $gpuMemory = [int]((& nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits | Select-Object -First 1).Trim())
        $process = Get-Process -Id $active.Id
        $result = [pscustomobject]@{
            label = $variant.label
            override = $variant.override
            prefill_tokens = [int]$prefill.tokens_evaluated
            prefill_tps = [Math]::Round([double]$prefill.timings.prompt_per_second, 3)
            decode_tps = [Math]::Round([double]$decode.timings.predicted_per_second, 4)
            combined_seconds = [Math]::Round(
                ([double]$prefill.tokens_evaluated / [double]$prefill.timings.prompt_per_second) +
                ([double]$decode.tokens_predicted / [double]$decode.timings.predicted_per_second), 3)
            output_sha256 = Get-ContentHash ([string]$decode.content)
            gpu_memory_mib = $gpuMemory
            working_set_mib = [Math]::Round($process.WorkingSet64 / 1MB)
            private_memory_mib = [Math]::Round($process.PrivateMemorySize64 / 1MB)
            server_log = $stderr
        }
        $results += $result
        Write-Output ('RESULT ' + ($result | ConvertTo-Json -Compress))
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
