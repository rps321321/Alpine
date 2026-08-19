$ErrorActionPreference = 'Continue'
. (Join-Path $PSScriptRoot 'lib.ps1')
$session = Get-SessionConfig
$state = Read-SessionState $session
$process = Get-ProcessOnPort $session.port
$stateProfile = if ($state) { [string](Get-PropertyValue $state 'profile' '') } else { '' }
$profile = if ($stateProfile) { Get-ProfileConfig $session $stateProfile } else { Get-ProfileConfig $session $null }
$expectedServer = Get-RuntimePath $session $profile
$owned = [bool]($process -and $process.Path -and ($process.Path -ieq $expectedServer))
[pscustomobject]@{
    profile = $profile.name
    context = $profile.context
    runtime = Get-PropertyValue $profile 'runtime' 'legacy'
    healthy = ($owned -and (Test-HttpOk "http://$($session.host):$($session.port)/health" 3))
    owned = $owned
    pid = if ($process) { $process.Id } else { $null }
    process_path = if ($process) { $process.Path } else { $null }
    expected_path = $expectedServer
    fallback = if ($state) { Get-PropertyValue $state 'fallback' $null } else { $null }
    gpu_memory_mib = if ($owned) { [int]((& nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits | Select-Object -First 1).Trim()) } else { $null }
}
