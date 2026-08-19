[CmdletBinding()]
param(
    [string]$Profile,
    [switch]$Vision
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'inference-session.ps1')

Start-InferenceSession -Profile $Profile -Vision:$Vision | Out-Null
