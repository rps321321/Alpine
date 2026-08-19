[CmdletBinding()]
param(
    [string]$InstallRoot = (Join-Path $env:USERPROFILE 'local-models'),
    [string]$Profile = 'stable-16k',
    [ValidateSet('Custom', 'Official')]
    [string]$Runtime = 'Custom',
    [string]$ReuseArtifactsFrom,
    [switch]$InstallPrerequisites,
    [switch]$SkipVision,
    [switch]$VerifyOnly,
    [switch]$NoShortcut
)

$ErrorActionPreference = 'Stop'
$entry = Join-Path $PSScriptRoot 'scripts\setup-local-qwen.ps1'
if (-not (Test-Path -LiteralPath $entry -PathType Leaf)) {
    throw "Installer module missing: $entry"
}
$arguments = @{
    InstallRoot = $InstallRoot
    Profile = $Profile
    Runtime = $Runtime
    InstallPrerequisites = $InstallPrerequisites
    SkipVision = $SkipVision
    VerifyOnly = $VerifyOnly
    NoShortcut = $NoShortcut
}
if ($ReuseArtifactsFrom) { $arguments.ReuseArtifactsFrom = $ReuseArtifactsFrom }
& $entry @arguments
