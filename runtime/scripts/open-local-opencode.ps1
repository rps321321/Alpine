[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [string]$Project = (Get-Location).Path,
    [string]$Profile,
    [switch]$Lean,
    [switch]$FullPrompt,
    [switch]$WithVision,
    [switch]$WithConvex,
    [switch]$WithSkills,
    [switch]$WithProjectConfig,
    [switch]$WithPlugins,
    [switch]$KeepServer,
    [switch]$Check,
    [string]$CaptureEndpoint,
    [string]$RunPrompt,
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$OpenCodeArgs
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'inference-session.ps1')
. (Join-Path $PSScriptRoot 'harness-policy.ps1')

$resolved = Get-ResolvedSession -Name $Profile
$session = $resolved.Session
$profileConfig = $resolved.Profile
$Profile = $resolved.ProfileName
$serverPath = [string]$resolved.ServerPath
$skillsEnabled = [bool](Get-PropertyValue $profileConfig 'external_skills' $false) -or [bool]$WithSkills
if ($Lean -and $FullPrompt) { throw 'Choose either -Lean or -FullPrompt, not both.' }
if (-not $FullPrompt) { $Lean = $true }
$projectPath = (Resolve-Path -LiteralPath $Project).Path
if (-not (Test-Path -LiteralPath $projectPath -PathType Container)) { throw "Project is not a directory: $projectPath" }
$openCode = Get-Command opencode -ErrorAction Stop
$modelId = 'local-models/Qwen3.8-27B-ABLITERATED'
$acquisition = $null
$environmentState = $null
if ($OpenCodeArgs | Where-Object { $_ -match '^--auto(?:=|$)' }) {
    throw '--auto disables the consent boundary and is not accepted by this launcher.'
}

try {
    if ($CaptureEndpoint -and -not $RunPrompt) { throw '-CaptureEndpoint requires -RunPrompt.' }
    if (-not $CaptureEndpoint) {
        Ensure-LocalApiKey $session
        [IO.File]::WriteAllText($session.base_url_file, "http://$($session.host):$($session.port)/v1", [Text.Encoding]::ASCII)
    }
    # Claude Code's ambient global prompt contains hosted-model routing and
    # delegation instructions that are unrelated to this local OpenCode worker.
    # Skills remain available on demand; only the foreign harness prompt is off.
    $policy = New-HarnessPolicy -Session $session -Profile $profileConfig -Lean ([bool]$Lean) `
        -SkillsEnabled $skillsEnabled -WithConvex ([bool]$WithConvex) -CaptureEndpoint $CaptureEndpoint
    $environmentState = Enter-HarnessEnvironment -ConfigJson ($policy | ConvertTo-Json -Depth 14 -Compress) `
        -SkillsEnabled $skillsEnabled -WithProjectConfig ([bool]$WithProjectConfig)

    [string[]]$pureArgs = if ($WithPlugins) { @() } else { @('--pure') }
    if ($Check) {
        $raw = & $openCode.Source @pureArgs debug config
        if (-not $?) { throw 'OpenCode effective-config check failed.' }
        $effective = $raw | ConvertFrom-Json
        Assert-EffectiveHarnessPolicy -Effective $effective -Profile $profileConfig -SkillsEnabled $skillsEnabled
        Write-Host "OpenCode check passed: $Profile context=$($profileConfig.context) lean=$([bool]$Lean) skills=$skillsEnabled plugins=$([bool]$WithPlugins)"
        Write-Host 'Core agent capabilities are inherited; only safety-sensitive effects are gated.'
        return
    }

    if ($CaptureEndpoint) {
        Write-Host "Capturing one OpenCode request at $CaptureEndpoint"
    } else {
        Assert-InferenceCapacityAvailable
        $acquisition = Enter-InferenceSession -Profile $Profile -Vision:$WithVision
    }

    Write-Host "Opening OpenCode: $Profile | context=$($profileConfig.context) | project=$projectPath"
    Write-Host "Capabilities: core agent tools on | external skills=$skillsEnabled | lean prompt=$([bool]$Lean) | plugins=$([bool]$WithPlugins) | Convex=$([bool]$WithConvex)"
    Write-Host 'Boundary: consent tripwires and credential shielding; this is not a hostile-code sandbox.'
    if ($RunPrompt) {
        & $openCode.Source run @pureArgs --model $modelId --agent build --format json --dir $projectPath $RunPrompt @OpenCodeArgs
    } else {
        & $openCode.Source @pureArgs --model $modelId $projectPath @OpenCodeArgs
    }
    $exitCode = if ($?) { 0 } elseif ($null -ne $LASTEXITCODE) { $LASTEXITCODE } else { 1 }
} finally {
    if ($null -ne $acquisition) {
        try { Exit-InferenceSession -Acquisition $acquisition -KeepServer:$KeepServer }
        catch { Write-Warning "Inference Session restoration failed: $($_.Exception.Message)" }
    }
    if ($null -ne $environmentState) { Exit-HarnessEnvironment $environmentState }
}
exit $exitCode
