[CmdletBinding()]
param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Arguments)
$ErrorActionPreference = 'Stop'
$env:PYTHONUTF8 = '1'
& python.exe -m localmodel.cli @Arguments
exit $LASTEXITCODE
