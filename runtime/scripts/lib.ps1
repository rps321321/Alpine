$ErrorActionPreference = 'Stop'

function Get-NativeVersionText {
    [CmdletBinding()]
    param([Parameter(Mandatory = $true)][string]$FilePath)

    if (-not (Test-Path -LiteralPath $FilePath -PathType Leaf)) {
        throw "Native version probe executable is missing: $FilePath"
    }
    $start = New-Object System.Diagnostics.ProcessStartInfo
    $start.FileName = [IO.Path]::GetFullPath($FilePath)
    $start.Arguments = '--version'
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $start
    try {
        if (-not $process.Start()) { throw "Native version probe did not start: $FilePath" }
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        $process.WaitForExit()
        $text = ([string]$stdout.Result) + ([string]$stderr.Result)
        if ($process.ExitCode -ne 0) {
            throw "Native version probe exited with code $($process.ExitCode): $FilePath`n$text"
        }
        if (-not $text.Trim()) { throw "Native version probe returned no version text: $FilePath" }
        return $text
    } finally {
        $process.Dispose()
    }
}

function Get-FileSha256([string]$Path) {
    $stream = [IO.File]::OpenRead($Path)
    $algorithm = [Security.Cryptography.SHA256]::Create()
    try {
        ([BitConverter]::ToString($algorithm.ComputeHash($stream)) -replace '-', '').ToLowerInvariant()
    } finally {
        $algorithm.Dispose()
        $stream.Dispose()
    }
}

function Get-RequiredString {
    param($Object, [string]$Name, [string]$Source)
    $value = Get-PropertyValue $Object $Name $null
    if ($null -eq $value -or -not ([string]$value).Trim()) {
        throw "$Source`: required value '$Name' must be a non-empty string."
    }
    return [string]$value
}

function Test-RequiredInteger {
    param($Value, [int64]$Minimum = [int64]::MinValue, [int64]$Maximum = [int64]::MaxValue)
    if ($null -eq $Value -or $Value -is [bool] -or $Value -is [string]) { return $false }
    try { $number = [decimal]$Value } catch { return $false }
    return ($number -eq [decimal]::Truncate($number) -and $number -ge $Minimum -and $number -le $Maximum)
}

function Get-SessionConfig {
    param([string]$InstallRoot)
    $root = if ($InstallRoot) { [IO.Path]::GetFullPath($InstallRoot) } else { Split-Path $PSScriptRoot -Parent }
    $publicationMarker = Join-Path $root '.setup-publishing.json'
    if (Test-Path -LiteralPath $publicationMarker) {
        throw "Setup publication is incomplete: $publicationMarker. Re-run setup to restore the prior installation before using it."
    }
    $path = Join-Path $root 'config\session.json'
    if (-not (Test-Path -LiteralPath $path)) { throw "Session config missing: $path" }
    try { $session = Get-Content -Raw -LiteralPath $path | ConvertFrom-Json }
    catch { throw "Malformed Session Config $path`: $($_.Exception.Message)" }
    $schema = [int](Get-PropertyValue $session 'schema' 0)
    if ($schema -notin @(3, 4)) {
        throw "Unsupported Session Config schema '$((Get-PropertyValue $session 'schema' $null))' in $path; expected 3 or 4."
    }
    $configuredRoot = [IO.Path]::GetFullPath((Get-RequiredString $session 'root' $path))
    if ($configuredRoot.TrimEnd('\') -ine ([IO.Path]::GetFullPath($root)).TrimEnd('\')) {
        throw "Session Config root '$configuredRoot' does not match install root '$root'."
    }
    Get-RequiredString $session 'host' $path | Out-Null
    $port = Get-PropertyValue $session 'port' $null
    if (-not (Test-RequiredInteger $port 1 65535)) {
        throw "Session Config port must be between 1 and 65535: $path"
    }
    if ($schema -eq 3) { Get-RequiredString $session 'active_profile' $path | Out-Null }
    if ($schema -eq 4 -and (Get-PropertyValue $session 'active_profile' $null)) {
        throw "Schema 4 stores deployment roles outside Session Config; remove active_profile from $path."
    }
    foreach ($name in @('model', 'mmproj', 'chat_template', 'api_key_file', 'base_url_file', 'state_file')) {
        Get-RequiredString $session $name $path | Out-Null
    }
    $runtimes = Get-PropertyValue $session 'runtimes' $null
    if ($null -eq $runtimes -or $runtimes -isnot [psobject]) {
        throw "Session Config requires a runtimes object: $path"
    }
    return $session
}

function Get-ProfileConfig {
    param($Session, [string]$Name)
    if ($Name) {
        $selected = $Name
    } elseif ([int]$Session.schema -eq 3) {
        $selected = [string]$Session.active_profile
    } else {
        $alpine = Join-Path ([string]$Session.root) 'alpine.exe'
        if (-not (Test-Path -LiteralPath $alpine -PathType Leaf)) {
            throw 'Schema 4 default selection is Rust-owned and requires the installed alpine.exe.'
        }
        $raw = & $alpine deployment-status --install-root ([string]$Session.root) --compact
        if ($LASTEXITCODE -ne 0) { throw 'Alpine could not derive the deployment daily_default.' }
        try { $deployment = $raw | ConvertFrom-Json }
        catch { throw "Alpine returned invalid deployment state: $($_.Exception.Message)" }
        $selected = Get-RequiredString $deployment.roles 'daily_default' 'Alpine deployment state'
    }
    $path = Join-Path $Session.root "profiles\$selected.json"
    if (-not (Test-Path -LiteralPath $path)) { throw "Profile missing: $path" }
    try { $profile = Get-Content -Raw -LiteralPath $path | ConvertFrom-Json }
    catch { throw "Malformed Profile $path`: $($_.Exception.Message)" }
    if ((Get-RequiredString $profile 'name' $path) -ne $selected) {
        throw "Profile name '$($profile.name)' does not match selected name '$selected'."
    }
    if ($profile.PSObject.Properties['status']) { $profile.PSObject.Properties.Remove('status') }
    foreach ($field in @('runtime', 'kv_cache')) { Get-RequiredString $profile $field $path | Out-Null }
    foreach ($field in @('context', 'output', 'parallel', 'threads', 'batch_size', 'ubatch_size', 'mtp_depth')) {
        $value = Get-PropertyValue $profile $field $null
        if (-not (Test-RequiredInteger $value 1)) { throw "Profile value '$field' must be a positive integer: $path" }
    }
    $block = Get-PropertyValue $profile 'tensor_cpu_through_block' $null
    if (-not (Test-RequiredInteger $block 0)) { throw "Profile value 'tensor_cpu_through_block' must be a non-negative integer: $path" }
    return $profile
}

function Get-RuntimePath {
    param($Session, $Profile)
    $runtimeName = [string](Get-PropertyValue $Profile 'runtime' '')
    $property = $Session.runtimes.PSObject.Properties[$runtimeName]
    if (-not $property -or -not $property.Value) { throw "Runtime '$runtimeName' is not installed." }
    return [string]$property.Value
}

function Get-ResolvedSession {
    param([string]$InstallRoot, [string]$Name, [switch]$RequireRuntime)
    $session = Get-SessionConfig $InstallRoot
    $profile = Get-ProfileConfig $session $Name
    $runtimeName = [string]$profile.runtime
    $server = Get-RuntimePath $session $profile
    if (-not $server) { throw "Runtime '$runtimeName' is unavailable for Profile '$($profile.name)'." }
    $serverPath = [IO.Path]::GetFullPath($server)
    if ($RequireRuntime -and -not (Test-Path -LiteralPath $serverPath -PathType Leaf)) {
        throw "Runtime '$runtimeName' is unavailable at $serverPath."
    }
    return [pscustomobject][ordered]@{
        InstallRoot = [IO.Path]::GetFullPath([string]$session.root)
        Session = $session
        ProfileName = [string]$profile.name
        Profile = $profile
        RuntimeName = $runtimeName
        ServerPath = $serverPath
        Model = [IO.Path]::GetFullPath([string]$session.model)
        Mmproj = [IO.Path]::GetFullPath([string]$session.mmproj)
        ChatTemplate = [IO.Path]::GetFullPath([string]$session.chat_template)
        ApiKeyFile = [IO.Path]::GetFullPath([string]$session.api_key_file)
        BaseUrlFile = [IO.Path]::GetFullPath([string]$session.base_url_file)
        StateFile = [IO.Path]::GetFullPath([string]$session.state_file)
        BaseUrl = "http://$($session.host):$($session.port)"
    }
}

function Get-PropertyValue {
    param($Object, [string]$Name, $Default = $null)
    if ($null -eq $Object) { return $Default }
    if ($Object -is [Collections.IDictionary]) {
        foreach ($key in $Object.Keys) {
            if ([string]$key -ieq $Name) {
                $value = $Object[$key]
                if ($null -eq $value) { return $Default }
                return $value
            }
        }
        return $Default
    }
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property -or $null -eq $property.Value) { return $Default }
    return $property.Value
}

function Get-TensorOverride([int]$ThroughBlock) {
    $pattern = ((0..$ThroughBlock) -join '|')
    return "blk\.($pattern)\.ffn_.*=CPU"
}

function Get-Listener([int]$Port) {
    Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue | Select-Object -First 1
}

function Get-ProcessOnPort([int]$Port) {
    $listener = Get-Listener $Port
    if (-not $listener) { return $null }
    Get-Process -Id $listener.OwningProcess -ErrorAction SilentlyContinue
}

function Get-CommandLine([int]$Id) {
    (Get-CimInstance Win32_Process -Filter "ProcessId=$Id" -ErrorAction SilentlyContinue).CommandLine
}

function Test-HttpOk([string]$Url, [int]$TimeoutSec = 3) {
    try {
        $code = & curl.exe -sS -o NUL -w '%{http_code}' --connect-timeout $TimeoutSec --max-time $TimeoutSec $Url 2>$null
        return ($LASTEXITCODE -eq 0 -and [int]$code -ge 200 -and [int]$code -lt 300)
    } catch { return $false }
}

function Wait-HttpOk([string]$Url, [int]$TimeoutSec = 600) {
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    do {
        if (Test-HttpOk $Url 3) { return $true }
        Start-Sleep -Seconds 2
    } while ((Get-Date) -lt $deadline)
    return $false
}

function Wait-PortFree([int]$Port, [int]$TimeoutSec = 30) {
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    do {
        if (-not (Get-Listener $Port)) { return $true }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $deadline)
    return $false
}

function Enter-InterprocessLock {
    param([string]$Path, [int]$TimeoutMilliseconds = 15000)
    $parent = Split-Path $Path -Parent
    if ($parent) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMilliseconds)
    do {
        try {
            return [IO.File]::Open($Path, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        } catch [IO.IOException] {
            if ([DateTime]::UtcNow -ge $deadline) {
                throw "Timed out waiting for interprocess lock: $Path"
            }
            Start-Sleep -Milliseconds 50
        }
    } while ($true)
}

function Exit-InterprocessLock($Lock) {
    if ($null -ne $Lock) { $Lock.Dispose() }
}

function Write-AtomicText {
    param([string]$Path, [string]$Content, [Text.Encoding]$Encoding = ([Text.UTF8Encoding]::new($false)))
    $parent = Split-Path $Path -Parent
    if ($parent) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }
    $identity = [Guid]::NewGuid().ToString('N')
    $temporary = "$Path.$identity.tmp"
      $backup = "$Path.$identity.bak"
      try {
          [IO.File]::WriteAllText($temporary, $Content, $Encoding)
          for ($attempt = 1; $attempt -le 200; $attempt++) {
              try {
                  if (Test-Path -LiteralPath $Path) {
                      [IO.File]::Replace($temporary, $Path, $backup)
                  } else {
                      [IO.File]::Move($temporary, $Path)
                  }
                  break
              } catch [IO.IOException] {
                  if ($attempt -eq 200) { throw }
                  Start-Sleep -Milliseconds 10
              }
          }
      } finally {
        if (Test-Path -LiteralPath $temporary) { [IO.File]::Delete($temporary) }
        if (Test-Path -LiteralPath $backup) { [IO.File]::Delete($backup) }
    }
}

  function Read-SessionState($Session) {
      $lock = Enter-InterprocessLock "$($Session.state_file).write.lock"
      try {
          if (-not (Test-Path -LiteralPath $Session.state_file)) { return $null }
          return Get-Content -Raw -LiteralPath $Session.state_file | ConvertFrom-Json
      } finally { Exit-InterprocessLock $lock }
  }

function Save-SessionState($State, $Session) {
    $parent = Split-Path $Session.state_file -Parent
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    $json = $State | ConvertTo-Json -Depth 8
    $lock = Enter-InterprocessLock "$($Session.state_file).write.lock"
    try { Write-AtomicText $Session.state_file ($json + [Environment]::NewLine) }
    finally { Exit-InterprocessLock $lock }
}

function Ensure-LocalApiKey($Session) {
    $lock = Enter-InterprocessLock "$($Session.api_key_file).lock"
    try {
        if (Test-Path -LiteralPath $Session.api_key_file) { return }
        $bytes = New-Object byte[] 32
        $rng = [Security.Cryptography.RandomNumberGenerator]::Create()
        try { $rng.GetBytes($bytes) } finally { $rng.Dispose() }
        $key = 'sk-local-' + (([BitConverter]::ToString($bytes)) -replace '-', '').ToLowerInvariant()
        Write-AtomicText $Session.api_key_file $key
    } finally { Exit-InterprocessLock $lock }
}

function Test-CleanupEnabled($Session) {
    [bool](Get-PropertyValue $Session.cleanup 'enabled' $false)
}
