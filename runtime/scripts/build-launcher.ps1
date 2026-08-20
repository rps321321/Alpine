[CmdletBinding()]
param(
    [string]$Output,
    [switch]$NoShortcut,
    [switch]$ShortcutOnly
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Split-Path $PSScriptRoot -Parent
if (-not $Output) { $Output = Join-Path $root 'Open Local Qwen.exe' }
if ($PSVersionTable.PSEdition -eq 'Core' -and -not $ShortcutOnly) {
    $windowsPowerShell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $arguments = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $PSCommandPath, '-Output', $Output)
    if ($NoShortcut) { $arguments += '-NoShortcut' }
    & $windowsPowerShell @arguments
    if ($LASTEXITCODE -ne 0) { throw 'Windows launcher compilation failed.' }
    return
}
if (-not $ShortcutOnly) {
    $source = Join-Path $root 'launcher\OpenLocalQwen.cs'
    $temporary = Join-Path ([IO.Path]::GetTempPath()) ("OpenLocalQwen-$PID.exe")
    try {
        Add-Type -TypeDefinition (Get-Content -Raw -LiteralPath $source) -Language CSharp `
            -ReferencedAssemblies @('System.dll', 'System.Windows.Forms.dll', 'System.Drawing.dll') `
            -OutputAssembly $temporary -OutputType WindowsApplication
        Move-Item -LiteralPath $temporary -Destination $Output -Force
        Copy-Item -LiteralPath (Join-Path $root 'launcher\Open Minimal OpenCode.cmd') `
            -Destination (Join-Path (Split-Path $Output -Parent) 'Open Minimal OpenCode.cmd') -Force
    } finally {
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
    }
}
if (-not (Test-Path -LiteralPath $Output -PathType Leaf)) { throw "Launcher is missing: $Output" }
if (-not $NoShortcut) {
    $desktop = [Environment]::GetFolderPath([Environment+SpecialFolder]::DesktopDirectory)
    $shell = New-Object -ComObject WScript.Shell
    try {
        $items = @(
            @('Open Local Qwen.lnk', '--profile stable-16k', 'Production local coding profile'),
            @('Open Local Qwen 32K.lnk', '--profile fast-32k', 'Candidate general agent profile'),
            @('Open Local Qwen 16K Stable.lnk', '--profile stable-16k', 'Production rollback profile'),
            @('Open Local Qwen 16K Turbo.lnk', '--profile turbo-16k', 'Repetitive-code candidate profile'),
            @('Open Local Qwen 64K Long.lnk', '--profile long-64k', 'Experimental long-context profile'),
            @('Open Local Qwen Vision.lnk', '--profile fast-32k --vision', 'Vision profile')
        )
        foreach ($item in $items) {
            $shortcut = $shell.CreateShortcut((Join-Path $desktop $item[0]))
            $shortcut.TargetPath = $Output
            $shortcut.Arguments = $item[1]
            $shortcut.WorkingDirectory = $desktop
            $shortcut.Description = $item[2]
            $shortcut.IconLocation = "$Output,0"
            $shortcut.Save()
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($shortcut)
        }
    } finally { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($shell) }
}
Write-Host $(if ($ShortcutOnly) { "Updated launcher shortcuts: $Output" } else { "Built launcher: $Output" })
