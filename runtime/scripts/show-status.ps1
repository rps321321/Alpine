$ErrorActionPreference = 'Continue'
. (Join-Path $PSScriptRoot 'inference-session.ps1')

$status = Get-InferenceSessionStatus
$session = Get-SessionConfig
$profile = Get-ProfileConfig $session $status.Profile
[pscustomobject]@{
    profile = $status.Profile
    context = $profile.context
    runtime = $status.Runtime
    healthy = $status.Healthy
    owned = $status.Active
    foreign = $status.Foreign
    pid = $status.Pid
    process_path = $status.ProcessPath
    expected_path = $status.ExpectedPath
    fallback = $status.Fallback
    gpu_memory_mib = if ($status.Active) { [int]((& nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits | Select-Object -First 1).Trim()) } else { $null }
}
