$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not (Get-Command Enter-InterprocessLock -ErrorAction SilentlyContinue)) {
    . (Join-Path $PSScriptRoot 'lib.ps1')
}

function Get-SetupPublicationMarker([string]$InstallRoot) {
    Join-Path ([IO.Path]::GetFullPath($InstallRoot)) '.setup-publishing.json'
}

function Enter-SetupLock([string]$InstallRoot, [int]$TimeoutMilliseconds = 30000) {
    $root = [IO.Path]::GetFullPath($InstallRoot)
    New-Item -ItemType Directory -Force -Path $root | Out-Null
    try { return Enter-InterprocessLock (Join-Path $root '.setup.lock') $TimeoutMilliseconds }
    catch { throw "Another setup transaction owns $root. $($_.Exception.Message)" }
}

function Repair-InterruptedSetupPublication([string]$InstallRoot) {
    $root = [IO.Path]::GetFullPath($InstallRoot)
    $marker = Get-SetupPublicationMarker $root
    if (-not (Test-Path -LiteralPath $marker -PathType Leaf)) { return $false }
    try { $transaction = Get-Content -Raw -LiteralPath $marker | ConvertFrom-Json }
    catch { throw "Setup publication marker is malformed: $marker. Preserve the installation and repair it manually." }
    $backupRoot = [string](Get-PropertyValue $transaction 'backup_root' '')
    $stageRoot = [string](Get-PropertyValue $transaction 'stage_root' '')
    $items = @(Get-PropertyValue $transaction 'items' @())
    for ($index = $items.Count - 1; $index -ge 0; $index--) {
        $item = $items[$index]
        $relative = [string]$item.destination
        $destination = Join-Path $root $relative
        $backup = Join-Path $backupRoot $relative
        if (Test-Path -LiteralPath $backup) {
            if (Test-Path -LiteralPath $destination) { Remove-Item -LiteralPath $destination -Recurse -Force }
            New-Item -ItemType Directory -Force -Path (Split-Path $destination -Parent) | Out-Null
            Move-Item -LiteralPath $backup -Destination $destination
        } elseif (-not [bool]$item.had_prior -and (Test-Path -LiteralPath $destination)) {
            Remove-Item -LiteralPath $destination -Recurse -Force
        }
    }
    foreach ($path in @($backupRoot, $stageRoot)) {
        if ($path -and (Test-Path -LiteralPath $path)) { Remove-Item -LiteralPath $path -Recurse -Force }
    }
    Remove-Item -LiteralPath $marker -Force
    Write-Warning 'Recovered the prior control plane from an interrupted setup publication.'
    return $true
}

function Publish-SetupBundle {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$InstallRoot,
        [Parameter(Mandatory = $true)][string]$StageRoot,
        [Parameter(Mandatory = $true)][object[]]$Items
    )
    $root = [IO.Path]::GetFullPath($InstallRoot)
    $stage = [IO.Path]::GetFullPath($StageRoot)
    if (-not $stage.StartsWith($root.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw "Setup stage must be inside the installation root: $stage"
    }
    $marker = Get-SetupPublicationMarker $root
    if (Test-Path -LiteralPath $marker) { throw "An incomplete setup publication exists: $marker. Recover it before publishing." }
    $backupRoot = Join-Path $root ".setup-backup-$([Guid]::NewGuid().ToString('N'))"
    $journal = @()
    foreach ($item in $Items) {
        $sourceRelative = [string](Get-PropertyValue $item 'stage' '')
        $destinationRelative = [string](Get-PropertyValue $item 'destination' '')
        if (-not $sourceRelative -or -not $destinationRelative) { throw 'Setup publication items require stage and destination values.' }
        $source = Join-Path $stage $sourceRelative
        if (-not (Test-Path -LiteralPath $source)) { throw "Setup stage item is missing: $source" }
        $destination = Join-Path $root $destinationRelative
        $journal += [pscustomobject][ordered]@{
            stage = $sourceRelative
            destination = $destinationRelative
            had_prior = [bool](Test-Path -LiteralPath $destination)
        }
    }
    $transaction = [ordered]@{
        schema = 1
        transaction_id = [Guid]::NewGuid().ToString('N')
        started_at = (Get-Date).ToUniversalTime().ToString('o')
        stage_root = $stage
        backup_root = $backupRoot
        items = $journal
    }
    Write-AtomicText $marker (($transaction | ConvertTo-Json -Depth 6) + [Environment]::NewLine)
    try {
        foreach ($item in $journal) {
            $source = Join-Path $stage $item.stage
            $destination = Join-Path $root $item.destination
            $backup = Join-Path $backupRoot $item.destination
            New-Item -ItemType Directory -Force -Path (Split-Path $destination -Parent) | Out-Null
            if ($item.had_prior) {
                New-Item -ItemType Directory -Force -Path (Split-Path $backup -Parent) | Out-Null
                Move-Item -LiteralPath $destination -Destination $backup
            }
            Move-Item -LiteralPath $source -Destination $destination
        }
        # Removing the marker is the commit point. Until then all backups remain
        # intact so a crash can restore the complete prior publication.
        Remove-Item -LiteralPath $marker -Force
        if (Test-Path -LiteralPath $backupRoot) { Remove-Item -LiteralPath $backupRoot -Recurse -Force }
        if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
    } catch {
        $failure = $_
        try { Repair-InterruptedSetupPublication $root | Out-Null }
        catch { throw "Setup publication failed: $($failure.Exception.Message) Automatic rollback also failed: $($_.Exception.Message)" }
        throw $failure
    }
}
