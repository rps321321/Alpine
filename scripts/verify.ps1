[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
try {
    & (Join-Path $PSScriptRoot 'audit-public-tree.ps1')
    if ($LASTEXITCODE -ne 0) { throw 'public-tree audit failed' }

    & cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { throw 'cargo fmt failed' }

    & cargo clippy --all-targets --all-features -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'cargo clippy failed' }

    & cargo test --all-targets --all-features
    if ($LASTEXITCODE -ne 0) { throw 'cargo test failed' }

    & python.exe -m unittest discover -s tests
    if ($LASTEXITCODE -ne 0) { throw 'legacy compatibility tests failed' }
} finally {
    Pop-Location
}
