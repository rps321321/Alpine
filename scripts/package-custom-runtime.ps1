[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$BuiltRuntime,
    [Parameter(Mandatory = $true)]
    [string]$Output,
    [string]$CudaBin = 'C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.2\bin\x64'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repoRoot = Split-Path $PSScriptRoot -Parent
$artifacts = Get-Content -Raw -LiteralPath (Join-Path $repoRoot 'config\artifacts.json') | ConvertFrom-Json
New-Item -ItemType Directory -Force -Path $Output | Out-Null
Get-ChildItem -LiteralPath $BuiltRuntime -File | Copy-Item -Destination $Output -Force
foreach ($name in @('cublas64_13.dll', 'cublasLt64_13.dll', 'cudart64_13.dll')) {
    $source = Join-Path $CudaBin $name
    if (-not (Test-Path -LiteralPath $source)) { throw "CUDA runtime dependency missing: $source" }
    Copy-Item -LiteralPath $source -Destination $Output -Force
}
$server = Join-Path $Output 'llama-server.exe'
$version = (& $server --version 2>&1 | Out-String).Trim()
if ($version -notmatch '3cb7ffb') { throw "Unexpected llama-server build:`n$version" }
$files = [ordered]@{}
Get-ChildItem -LiteralPath $Output -File | Where-Object Name -ne 'build-manifest.json' | Sort-Object Name | ForEach-Object {
    $files[$_.Name] = [ordered]@{
        bytes = $_.Length
        sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}
$manifest = [ordered]@{
    schema = 1
    llama_cpp_commit = $artifacts.llama_cpp.commit
    source_patch_sha256 = (Get-FileHash -LiteralPath (Join-Path $repoRoot $artifacts.llama_cpp.patch) -Algorithm SHA256).Hash.ToLowerInvariant()
    cuda_toolkit = $artifacts.llama_cpp.custom_build.cuda
    cuda_architecture = $artifacts.llama_cpp.custom_build.architecture
    cmake_options = $artifacts.llama_cpp.custom_build.options
    server_version = $version
    files = $files
}
$json = $manifest | ConvertTo-Json -Depth 8
[IO.File]::WriteAllText((Join-Path $Output 'build-manifest.json'), $json + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
Write-Host "Packaged verified custom runtime: $Output"
