param(
    [ValidateRange(64, 1024)]
    [int]$DecodeTokens = 256,
    [ValidateRange(1, 5)]
    [int]$Runs = 1,
    [ValidateSet('Novel', 'Repeat')]
    [string]$Workload = 'Novel',

    [ValidateRange(8192, 262144)]
    [int]$Context = 16384,

    [ValidateRange(0, 63)]
    [int]$TensorCpuThroughBlock = 43,

    [string[]]$Labels
)

$ErrorActionPreference = 'Stop'
$root = '%USERPROFILE%\local-models'
$officialServer = Join-Path $root 'runtime\llama-server.exe'
$prServer = Join-Path $root 'src\llama.cpp-pr27173\build-cuda132\bin\Release\llama-server.exe'
$customServer = Join-Path $root 'src\llama.cpp-b10453-ngram-reset\build-cuda132\bin\Release\llama-server.exe'
$model = Join-Path $root 'models\Qwen3.8-27B-ABLITERATED-Q4_K_M.gguf'
$template = Join-Path $root 'config\qwen3.8-official-chat-template.jinja'
$logs = Join-Path $root 'logs'
$stopScript = Join-Path $root 'scripts\stop-session.ps1'
$startScript = Join-Path $root 'scripts\start-session.ps1'
$baseUrl = 'http://127.0.0.1:8100'
$blockPattern = ((0..$TensorCpuThroughBlock) -join '|')
$tensorOverride = "blk\.($blockPattern)\.ffn_.*=CPU"

$prefillPrompt = ('A compiler optimization is safe when observable behavior is preserved across all permitted inputs. ' * 270).Trim()
$novelDecodePrompt = "Complete the following technical explanation with coherent prose and examples:`nA compiler optimization is safe when"
$repeatBlock = @'
function normalizePath(input) {
  const slashes = input.replaceAll('\\', '/');
  const parts = slashes.split('/');
  const output = [];
  for (const part of parts) {
    if (!part || part === '.') continue;
    if (part === '..') output.pop();
    else output.push(part);
  }
  return output.join('/');
}

function isInsideWorkspace(workspace, candidate) {
  const root = normalizePath(workspace).toLowerCase();
  const path = normalizePath(candidate).toLowerCase();
  return path === root || path.startsWith(root + '/');
}
'@
$repeatDecodePrompt = "Reproduce the source below exactly, without a fence or explanation.`nSOURCE:`n$repeatBlock`nEND SOURCE`nCOPY:`n"
$decodePrompt = if ($Workload -eq 'Repeat') { $repeatDecodePrompt } else { $novelDecodePrompt }

$variants = @(
    [pscustomobject]@{ label = 'official-none'; server = $officialServer; depth = 0; ngram = $false; chain = $false; sub = 0; pool = 0 },
    [pscustomobject]@{ label = 'official-ngram'; server = $officialServer; depth = 0; ngram = $true; chain = $false; sub = 0; pool = 0 },
    [pscustomobject]@{ label = 'official-mtp1'; server = $officialServer; depth = 1; ngram = $false; chain = $false; sub = 0; pool = 0 },
    [pscustomobject]@{ label = 'official-mtp1-ngram'; server = $officialServer; depth = 1; ngram = $true; chain = $false; sub = 0; pool = 0 },
    [pscustomobject]@{ label = 'official-mtp3'; server = $officialServer; depth = 3; ngram = $false; chain = $false; sub = 0; pool = 0 },
    [pscustomobject]@{ label = 'official-mtp3-ngram'; server = $officialServer; depth = 3; ngram = $true; chain = $false; sub = 0; pool = 0 },
    [pscustomobject]@{ label = 'custom-mtp3-control'; server = $customServer; depth = 3; ngram = $false; chain = $false; reset_ngram = $false; sub = 0; pool = 0 },
    [pscustomobject]@{ label = 'custom-mtp3-ngram-shared'; server = $customServer; depth = 3; ngram = $true; chain = $false; reset_ngram = $false; sub = 0; pool = 0 },
    [pscustomobject]@{ label = 'custom-mtp3-ngram-reset'; server = $customServer; depth = 3; ngram = $true; chain = $false; reset_ngram = $true; sub = 0; pool = 0 },
    [pscustomobject]@{ label = 'pr-none'; server = $prServer; depth = 0; ngram = $false; chain = $false; sub = 0; pool = 0 },
    [pscustomobject]@{ label = 'pr-mtp3-control'; server = $prServer; depth = 3; ngram = $false; chain = $false; sub = 0; pool = 0 },
    [pscustomobject]@{ label = 'pr-chain-d3-sub32768'; server = $prServer; depth = 3; ngram = $false; chain = $true; sub = 32768; pool = 8 },
    [pscustomobject]@{ label = 'pr-chain-d3-sub98304'; server = $prServer; depth = 3; ngram = $false; chain = $true; sub = 98304; pool = 8 },
    [pscustomobject]@{ label = 'pr-chain-d5-sub98304'; server = $prServer; depth = 5; ngram = $false; chain = $true; sub = 98304; pool = 8 },
    [pscustomobject]@{ label = 'pr-chain-d8-sub98304'; server = $prServer; depth = 8; ngram = $false; chain = $true; sub = 98304; pool = 8 }
)
if ($Labels) {
    $variants = @($variants | Where-Object { $_.label -in $Labels })
    if (-not $variants) { throw "No benchmark variants matched: $($Labels -join ', ')" }
}

function Wait-Health([Diagnostics.Process]$Process) {
    $deadline = (Get-Date).AddMinutes(6)
    do {
        Start-Sleep -Milliseconds 500
        if ($Process.HasExited) { throw "Benchmark server exited with code $($Process.ExitCode)." }
        try {
            if ((Invoke-WebRequest -UseBasicParsing -Uri "$baseUrl/health" -TimeoutSec 2).StatusCode -eq 200) { return }
        }
        catch { }
    } while ((Get-Date) -lt $deadline)
    throw 'Benchmark server did not become healthy within six minutes.'
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

function Get-OptionalNumber($Object, [string[]]$Names) {
    foreach ($name in $Names) {
        $property = $Object.PSObject.Properties[$name]
        if ($null -ne $property -and $null -ne $property.Value) { return [double]$property.Value }
    }
    return $null
}

function Set-BenchmarkEnvironment($Variant) {
    Remove-Item Env:LLAMA_SPEC_CHAIN -ErrorAction SilentlyContinue
    Remove-Item Env:LLAMA_SPEC_CHAIN_SUB -ErrorAction SilentlyContinue
    Remove-Item Env:LLAMA_SCHED_POOL -ErrorAction SilentlyContinue
    Remove-Item Env:LLAMA_NGRAM_MOD_RESET_ON_BEGIN -ErrorAction SilentlyContinue
    if ($Variant.reset_ngram) {
        $env:LLAMA_NGRAM_MOD_RESET_ON_BEGIN = '1'
    }
    if ($Variant.chain) {
        $env:LLAMA_SPEC_CHAIN = '1'
        $env:LLAMA_SPEC_CHAIN_SUB = "$($Variant.sub)"
        $env:LLAMA_SCHED_POOL = "$($Variant.pool)"
    }
}

$active = $null
$results = @()
try {
    & $stopScript
    foreach ($variant in $variants) {
        if (-not (Test-Path -LiteralPath $variant.server)) {
            throw "Benchmark runtime missing for $($variant.label): $($variant.server)"
        }

        $args = @(
            '-m', $model, '--host', '127.0.0.1', '--port', '8100', '-c', "$Context", '-np', '1',
            '--threads', '16', '--threads-batch', '16', '-b', '2048', '-ub', '768',
            '--no-webui', '--jinja', '--chat-template-file', $template,
            '-fa', 'on', '-ctk', 'q8_0', '-ctv', 'q8_0', '--reasoning', 'off',
            '-ngl', 'all', '--fit', 'off', '-ot', $tensorOverride, '--load-mode', 'none', '-lv', '4'
        )
        if ($variant.depth -gt 0 -or $variant.ngram) {
            $specTypes = @()
            if ($variant.depth -gt 0) { $specTypes += 'draft-mtp' }
            if ($variant.ngram) { $specTypes += 'ngram-mod' }
            $args += @('--spec-type', ($specTypes -join ','))
            if ($variant.depth -gt 0) { $args += @('--spec-draft-n-max', "$($variant.depth)") }
            if ($variant.ngram) {
                $args += @('--spec-ngram-mod-n-match', '24', '--spec-ngram-mod-n-min', '16', '--spec-ngram-mod-n-max', '64')
            }
        }

        $stdout = Join-Path $logs "bench-pr27173-$($variant.label).out.log"
        $stderr = Join-Path $logs "bench-pr27173-$($variant.label).err.log"
        Set-BenchmarkEnvironment $variant
        $active = Start-Process -FilePath $variant.server -ArgumentList $args `
            -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru -WindowStyle Hidden
        Set-BenchmarkEnvironment ([pscustomobject]@{ chain = $false })
        Wait-Health $active

        $prefill = Invoke-Completion -Prompt $prefillPrompt -TokenCount 1
        for ($run = 1; $run -le $Runs; $run++) {
            $decode = Invoke-Completion -Prompt $decodePrompt -TokenCount $DecodeTokens
            $gpuMemory = [int]((& nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits | Select-Object -First 1).Trim())
            $process = Get-Process -Id $active.Id
            $drafted = Get-OptionalNumber $decode @('tokens_drafted', 'draft_n')
            $accepted = Get-OptionalNumber $decode @('tokens_drafted_accepted', 'draft_n_accepted')
            $acceptance = if ($drafted -gt 0) { [Math]::Round(100 * $accepted / $drafted, 2) } else { $null }
            $result = [pscustomobject]@{
                label = $variant.label
                workload = $Workload
                context = $Context
                tensor_cpu_through_block = $TensorCpuThroughBlock
                run = $run
                runtime = $variant.server
                mtp_depth = $variant.depth
                ngram_mod = [bool]$variant.ngram
                ngram_reset_on_begin = [bool]$variant.reset_ngram
                chained = [bool]$variant.chain
                chain_sub_vocab = $variant.sub
                prefill_tokens = [int]$prefill.tokens_evaluated
                prefill_tps = [Math]::Round([double]$prefill.timings.prompt_per_second, 3)
                decode_tps = [Math]::Round([double]$decode.timings.predicted_per_second, 4)
                combined_seconds = [Math]::Round(
                    ([double]$prefill.tokens_evaluated / [double]$prefill.timings.prompt_per_second) +
                    ([double]$decode.tokens_predicted / [double]$decode.timings.predicted_per_second), 3)
                drafted_tokens = $drafted
                accepted_tokens = $accepted
                acceptance_percent = $acceptance
                output_sha256 = Get-ContentHash ([string]$decode.content)
                output_preview = ([string]$decode.content).Substring(0, [Math]::Min(160, ([string]$decode.content).Length))
                gpu_memory_mib = $gpuMemory
                working_set_mib = [Math]::Round($process.WorkingSet64 / 1MB)
                private_memory_mib = [Math]::Round($process.PrivateMemorySize64 / 1MB)
                server_log = $stderr
            }
            $results += $result
            Write-Output ('RESULT ' + ($result | ConvertTo-Json -Compress))
            [Console]::Out.Flush()
        }

        Stop-Process -Id $active.Id -Force
        $active.WaitForExit(10000) | Out-Null
        $active = $null
        Wait-PortFree
    }
}
finally {
    Set-BenchmarkEnvironment ([pscustomobject]@{ chain = $false })
    if ($active -and -not $active.HasExited) {
        Stop-Process -Id $active.Id -Force -ErrorAction SilentlyContinue
        $active.WaitForExit(10000) | Out-Null
        Wait-PortFree
    }
    & $startScript
}

$results | ConvertTo-Json -Depth 4
