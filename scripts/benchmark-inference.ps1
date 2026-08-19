param(
    [string]$BaseUrl = 'http://127.0.0.1:8100',
    [ValidateRange(1, 8192)]
    [int]$Tokens = 256,
    [ValidateRange(1, 20)]
    [int]$Runs = 1,
    [string]$Label = 'current',
    [string]$ApiKeyFile
)

$ErrorActionPreference = 'Stop'

$prompt = @'
Complete the following technical explanation with coherent prose and examples:
A compiler optimization is safe when
'@

$headers = @{}
if ($ApiKeyFile) {
    $apiKey = [IO.File]::ReadAllText($ApiKeyFile).Trim()
    $headers.Authorization = "Bearer $apiKey"
}

$results = for ($run = 1; $run -le $Runs; $run++) {
    $body = @{
        prompt       = $prompt.Trim()
        n_predict    = $Tokens
        temperature  = 0.0
        top_k        = 1
        seed         = 42
        ignore_eos   = $true
        cache_prompt = $false
    } | ConvertTo-Json -Compress

    $timer = [Diagnostics.Stopwatch]::StartNew()
    $response = Invoke-RestMethod -Method Post -Uri "$BaseUrl/completion" `
        -Headers $headers -ContentType 'application/json' -Body $body -TimeoutSec 900
    $timer.Stop()

    if ([int]$response.tokens_predicted -ne $Tokens) {
        throw "Expected $Tokens predicted tokens, got $($response.tokens_predicted)."
    }

    $content = [string]$response.content
    $bytes = [Text.Encoding]::UTF8.GetBytes($content)
    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        $hashBytes = $sha256.ComputeHash($bytes)
    }
    finally {
        $sha256.Dispose()
    }
    $hash = ([BitConverter]::ToString($hashBytes) -replace '-', '').ToLowerInvariant()

    [pscustomobject]@{
        label                = $Label
        run                  = $run
        wall_seconds         = [Math]::Round($timer.Elapsed.TotalSeconds, 3)
        tokens_predicted     = [int]$response.tokens_predicted
        tokens_evaluated     = [int]$response.tokens_evaluated
        predicted_per_second = [Math]::Round([double]$response.timings.predicted_per_second, 4)
        prompt_per_second    = [Math]::Round([double]$response.timings.prompt_per_second, 4)
        stop_type            = [string]$response.stop_type
        output_sha256        = $hash
    }
}

$results | ConvertTo-Json -Depth 4
