[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Split-Path $PSScriptRoot -Parent
$trackedOrProposed = @(
    & git -C $root ls-files --cached --others --exclude-standard |
        Where-Object { $_ -and (Test-Path -LiteralPath (Join-Path $root $_) -PathType Leaf) } |
        Sort-Object -Unique
)
if ($LASTEXITCODE -ne 0) { throw 'Could not enumerate the proposed public tree.' }

$required = @(
    'LICENSE', 'NOTICE', 'DCO', 'THIRD_PARTY.md', 'CONTRIBUTING.md',
    'SECURITY.md', 'SUPPORT.md', 'GOVERNANCE.md',
    'third_party/llama.cpp-LICENSE',
    '.github/workflows/verify.yml', '.github/pull_request_template.md'
)
foreach ($relative in $required) {
    if (-not (Test-Path -LiteralPath (Join-Path $root $relative) -PathType Leaf)) {
        throw "Public source contract is missing: $relative"
    }
}

$forbiddenPrefixes = @(
    '.artifacts/', '.codex/', 'build/', 'dist/', 'inventory/', 'logs/',
    'models/', 'results/', 'runtime-official/', 'runtime-custom/', 'target/'
)
$forbiddenExtensions = @(
    '.7z', '.bin', '.db', '.dll', '.env', '.exe', '.gguf', '.gz', '.jsonl',
    '.key', '.log', '.p12', '.pem', '.pfx', '.safetensors', '.sqlite',
    '.sqlite3', '.tar', '.zip'
)
foreach ($relative in $trackedOrProposed) {
    $slash = $relative.Replace('\', '/')
    if ($forbiddenPrefixes | Where-Object { $slash.StartsWith($_, [StringComparison]::OrdinalIgnoreCase) }) {
        throw "Generated/private path is present in the proposed public tree: $relative"
    }
    if ($forbiddenExtensions -contains ([IO.Path]::GetExtension($relative).ToLowerInvariant())) {
        throw "Binary/private artifact extension is present in the proposed public tree: $relative"
    }
    $path = Join-Path $root $relative
    if ((Get-Item -LiteralPath $path).Length -gt 2MB) {
        throw "Unexpected source file larger than 2 MiB requires explicit release review: $relative"
    }
    $bytes = [IO.File]::ReadAllBytes($path)
    if ($bytes -contains 0) {
        throw "Unexpected binary content is present in the proposed public tree: $relative"
    }
    $text = [Text.Encoding]::UTF8.GetString($bytes)
    if ($text -match '(?i)C:\\Users\\(?!<you>\\|private-user\\|fixture\\|Public\\|Default\\)[^\\\r\n]+\\') {
        throw "Personal Windows home path is present in the proposed public tree: $relative"
    }
    if ($text -match '-----BEGIN (RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----') {
        throw "Private-key material is present in the proposed public tree: $relative"
    }
    if ($text -match '\bAKIA[0-9A-Z]{16}\b' -or
        $text -match '\bgh[pousr]_[A-Za-z0-9_]{20,}\b') {
        throw "Provider credential shape is present in the proposed public tree: $relative"
    }
}

Write-Host "Public-tree audit passed: $($trackedOrProposed.Count) source files checked."
