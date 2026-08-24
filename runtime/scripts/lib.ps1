$ErrorActionPreference = 'Stop'

function Get-NativeVersionText {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [string]$Arguments = '--version',
        [ValidateRange(1, 120000)][int]$TimeoutMilliseconds = 30000
    )

    if (-not (Test-Path -LiteralPath $FilePath -PathType Leaf)) {
        throw "Native version probe executable is missing: $FilePath"
    }
    $start = New-Object System.Diagnostics.ProcessStartInfo
    $start.FileName = [IO.Path]::GetFullPath($FilePath)
    $start.Arguments = $Arguments
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
        if (-not $process.WaitForExit($TimeoutMilliseconds)) {
            try { $process.Kill() } catch {}
            $process.WaitForExit()
            throw "Native version probe exceeded $TimeoutMilliseconds ms: $FilePath"
        }
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
    if ($value -isnot [string] -or -not $value.Trim()) {
        throw "$Source`: required value '$Name' must be a non-empty string."
    }
    return $value
}

function Get-ExactProperty {
    param($Object, [string]$Name)
    if ($null -eq $Object) { return $null }
    foreach ($property in $Object.PSObject.Properties) {
        if ($property.Name -ceq $Name) { return $property }
    }
    return $null
}

function Test-RequiredInteger {
    param($Value, [int64]$Minimum = [int64]::MinValue, [int64]$Maximum = [int64]::MaxValue)
    $integerTypes = @([byte], [sbyte], [int16], [uint16], [int32], [uint32], [int64], [uint64])
    if ($null -eq $Value -or -not @($integerTypes | Where-Object { $Value -is $_ }).Count) { return $false }
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
    $sessionFields = @('schema', 'root', 'host', 'port', 'active_profile', 'runtimes', 'model', 'mmproj', 'chat_template', 'api_key_file', 'base_url_file', 'state_file', 'cleanup')
    $unknownSessionFields = @($session.PSObject.Properties.Name | Where-Object { $_ -cnotin $sessionFields })
    if ($unknownSessionFields.Count) {
        throw "Unknown Session Config fields in $path`: $($unknownSessionFields -join ', ')."
    }
    $schemaValue = Get-PropertyValue $session 'schema' $null
    if (-not (Test-RequiredInteger $schemaValue 3 5)) {
        throw "Unsupported Session Config schema '$((Get-PropertyValue $session 'schema' $null))' in $path; expected 3, 4 or 5."
    }
    $configuredRoot = [IO.Path]::GetFullPath((Get-RequiredString $session 'root' $path))
    if ($configuredRoot.TrimEnd('\') -ine ([IO.Path]::GetFullPath($root)).TrimEnd('\')) {
        throw "Session Config root '$configuredRoot' does not match install root '$root'."
    }
    $sessionHost = Get-RequiredString $session 'host' $path
    $parsedAddress = $null
    $parsedIp = [Net.IPAddress]::TryParse($sessionHost, [ref]$parsedAddress)
    if ($sessionHost -cne 'localhost' -and (-not $parsedIp -or -not [Net.IPAddress]::IsLoopback($parsedAddress))) {
        throw "Session Config host must resolve explicitly to loopback: $path"
    }
    $schema = [int]$schemaValue
    $port = Get-PropertyValue $session 'port' $null
    if (-not (Test-RequiredInteger $port 1 65535)) {
        throw "Session Config port must be between 1 and 65535: $path"
    }
    if ($schema -eq 3) { Get-RequiredString $session 'active_profile' $path | Out-Null }
    if ($schema -in @(4, 5) -and (Get-ExactProperty $session 'active_profile')) {
        throw "Schema $schema stores deployment roles outside Session Config; remove active_profile from $path."
    }
    foreach ($name in @('model', 'mmproj', 'chat_template', 'api_key_file', 'base_url_file', 'state_file')) {
        Get-RequiredString $session $name $path | Out-Null
    }
    $runtimes = Get-PropertyValue $session 'runtimes' $null
    if ($null -eq $runtimes -or $runtimes -isnot [pscustomobject]) {
        throw "Session Config requires a runtimes object: $path"
    }
    $capabilities = Get-ProfileCapabilityContract
    $supportedRuntimes = @($capabilities.runtimes.PSObject.Properties.Name)
    $unknownRuntimes = @($runtimes.PSObject.Properties.Name | Where-Object { $_ -cnotin $supportedRuntimes })
    if ($unknownRuntimes.Count) {
        throw "Unsupported runtime names in $path`: $($unknownRuntimes -join ', '); expected $($supportedRuntimes -join ', ')."
    }
    foreach ($runtimeProperty in $runtimes.PSObject.Properties) {
        if ($null -ne $runtimeProperty.Value -and $runtimeProperty.Value -isnot [string]) {
            throw "Session Config runtime '$($runtimeProperty.Name)' must be a string path or null: $path"
        }
    }
    $cleanup = Get-PropertyValue $session 'cleanup' $null
    if ($schema -lt 5) {
        if ($null -ne $cleanup) {
            $cleanupFields = @($cleanup.PSObject.Properties.Name)
            $unknownCleanupFields = @($cleanupFields | Where-Object { $_ -cne 'enabled' })
            $cleanupEnabled = Get-PropertyValue $cleanup 'enabled' $false
            if ($cleanupEnabled -isnot [bool]) { throw "Session Config schema $schema cleanup.enabled must be a Boolean: $path" }
            if ($cleanupEnabled) { throw "Session Config schema $schema uses the retired cleanup start_script contract; migrate it to schema 5: $path" }
            if ($unknownCleanupFields.Count) { throw "Session Config schema $schema cleanup contains unknown fields: $($unknownCleanupFields -join ', '): $path" }
        }
    } else {
        if ($null -eq $cleanup -or $cleanup -isnot [pscustomobject]) { throw "Session Config schema 5 cleanup must be an object: $path" }
        $cleanupFields = @('enabled', 'port', 'executable', 'arguments', 'stdout', 'stderr', 'health')
        $unknownCleanupFields = @($cleanup.PSObject.Properties.Name | Where-Object { $_ -cnotin $cleanupFields })
        if ($unknownCleanupFields.Count) { throw "Session Config schema 5 cleanup contains unknown fields: $($unknownCleanupFields -join ', '): $path" }
        $cleanupEnabled = Get-PropertyValue $cleanup 'enabled' $false
        if ($cleanupEnabled -isnot [bool]) { throw "Session Config schema 5 cleanup.enabled must be a Boolean: $path" }
        $portProperty = Get-ExactProperty $cleanup 'port'
        if ($portProperty -and $null -ne $portProperty.Value -and -not (Test-RequiredInteger $portProperty.Value 0 65535)) {
            throw "Session Config schema 5 cleanup.port must be an unsigned 16-bit integer: $path"
        }
        foreach ($field in @('executable', 'stdout', 'stderr', 'health')) {
            $property = Get-ExactProperty $cleanup $field
            if ($property -and $null -ne $property.Value -and $property.Value -isnot [string]) {
                throw "Session Config schema 5 cleanup.$field must be a string or null: $path"
            }
        }
        $argumentsProperty = Get-ExactProperty $cleanup 'arguments'
        if ($argumentsProperty -and ($argumentsProperty.Value -isnot [array] -or @($argumentsProperty.Value | Where-Object { $_ -isnot [string] }).Count)) {
            throw "Session Config schema 5 cleanup.arguments must be an array of strings: $path"
        }
    }
    return $session
}

function Read-ProfileCapabilityContract([string]$Path) {
    $path = [IO.Path]::GetFullPath($Path)
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Profile capability contract missing: $path" }
    try { $contract = Get-Content -Raw -LiteralPath $path | ConvertFrom-Json }
    catch { throw "Malformed Profile capability contract $path`: $($_.Exception.Message)" }
    if ($contract -isnot [pscustomobject]) { throw "Invalid Profile capability contract object: $path" }
    $contractFields = @($contract.PSObject.Properties.Name)
    $expectedContractFields = @('schema', 'maximum_threads', 'runtimes')
    if (@($contractFields | Where-Object { $_ -cnotin $expectedContractFields }).Count -or
        @($expectedContractFields | Where-Object { -not (Get-ExactProperty $contract $_) }).Count -or
        -not (Test-RequiredInteger (Get-PropertyValue $contract 'schema' $null) 1 1) -or
        -not (Test-RequiredInteger (Get-PropertyValue $contract 'maximum_threads' $null) 1)) {
        throw "Invalid Profile capability contract: $path"
    }
    $runtimes = Get-PropertyValue $contract 'runtimes' $null
    if ($runtimes -isnot [pscustomobject] -or -not @($runtimes.PSObject.Properties).Count) {
        throw "Invalid Profile capability runtimes: $path"
    }
    foreach ($runtimeProperty in $runtimes.PSObject.Properties) {
        if ([string]::IsNullOrWhiteSpace($runtimeProperty.Name)) {
            throw "Invalid Profile runtime name: $path"
        }
        $runtime = $runtimeProperty.Value
        $runtimeFields = @('kv_cache', 'request_local_ngram')
        if ($runtime -isnot [pscustomobject] -or
            @($runtime.PSObject.Properties.Name | Where-Object { $_ -cnotin $runtimeFields }).Count -or
            @($runtimeFields | Where-Object { -not (Get-ExactProperty $runtime $_) }).Count) {
            throw "Invalid Profile runtime capability '$($runtimeProperty.Name)': $path"
        }
        $kvCache = Get-PropertyValue $runtime 'kv_cache' $null
        if ($kvCache -isnot [array] -or -not $kvCache.Count -or @($kvCache | Where-Object { $_ -isnot [string] -or [string]::IsNullOrWhiteSpace($_) }).Count) {
            throw "Invalid Profile runtime kv_cache capability '$($runtimeProperty.Name)': $path"
        }
        $seenKv = New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::Ordinal)
        foreach ($value in $kvCache) {
            if (-not $seenKv.Add($value)) { throw "Duplicate Profile runtime kv_cache capability '$value': $path" }
        }
        if ((Get-PropertyValue $runtime 'request_local_ngram' $null) -isnot [bool]) {
            throw "Invalid Profile runtime request_local_ngram capability '$($runtimeProperty.Name)': $path"
        }
    }
    return $contract
}

function Get-ProfileCapabilityContract {
    $parent = Split-Path $PSScriptRoot -Parent
    $candidates = @(
        (Join-Path $parent 'config\profile-capabilities.json'),
        (Join-Path (Split-Path $parent -Parent) 'config\profile-capabilities.json')
    )
    $path = $candidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
    if (-not $path) { throw "Profile capability contract missing; checked: $($candidates -join ', ')" }
    return Read-ProfileCapabilityContract $path
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
            throw "Schema $($Session.schema) default selection is Rust-owned and requires the installed alpine.exe."
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
    if (Get-ExactProperty $profile 'status') {
        throw "Profile field 'status' is not a setting; lifecycle and deployment roles belong to append-only deployment history: $path"
    }
    $profileFields = @('name', 'runtime', 'context', 'output', 'parallel', 'threads', 'batch_size', 'ubatch_size', 'kv_cache', 'tensor_cpu_through_block', 'mtp_depth', 'ngram_mod', 'ngram_reset_on_begin', 'external_skills', 'skill_tool', 'vision_fit', 'fit_target_mib')
    $unknownProfileFields = @($profile.PSObject.Properties.Name | Where-Object { $_ -cnotin $profileFields })
    if ($unknownProfileFields.Count) {
        throw "Unknown Profile fields in $path`: $($unknownProfileFields -join ', ')."
    }
    $missingProfileFields = @($profileFields | Where-Object { -not (Get-ExactProperty $profile $_) })
    if ($missingProfileFields.Count) {
        throw "Missing Profile fields in $path`: $($missingProfileFields -join ', ')."
    }
    if ((Get-RequiredString $profile 'name' $path) -cne $selected) {
        throw "Profile name '$($profile.name)' does not match selected name '$selected'."
    }
    foreach ($field in @('runtime', 'kv_cache')) { Get-RequiredString $profile $field $path | Out-Null }
    $capabilities = Get-ProfileCapabilityContract
    $runtimeProperty = Get-ExactProperty $capabilities.runtimes ([string]$profile.runtime)
    if (-not $runtimeProperty) { throw "Profile runtime '$($profile.runtime)' is unsupported: $path" }
    $runtimeCapabilities = $runtimeProperty.Value
    if ([string]$profile.kv_cache -cnotin @($runtimeCapabilities.kv_cache)) {
        throw "Profile kv_cache '$($profile.kv_cache)' is unsupported by runtime '$($profile.runtime)': $path"
    }
    foreach ($field in @('context', 'output', 'parallel', 'threads', 'batch_size', 'ubatch_size', 'mtp_depth', 'fit_target_mib')) {
        $value = Get-PropertyValue $profile $field $null
        if (-not (Test-RequiredInteger $value 1)) { throw "Profile value '$field' must be a positive integer: $path" }
    }
    $block = Get-PropertyValue $profile 'tensor_cpu_through_block' $null
    if (-not (Test-RequiredInteger $block 0)) { throw "Profile value 'tensor_cpu_through_block' must be a non-negative integer: $path" }
    if ([int64]$profile.output -gt [int64]$profile.context) { throw "Profile output must not exceed context: $path" }
    if ([int64]$profile.ubatch_size -gt [int64]$profile.batch_size) { throw "Profile ubatch_size must not exceed batch_size: $path" }
    if ([int64]$profile.threads -gt [int64]$capabilities.maximum_threads) { throw "Profile threads exceeds Alpine's supported sanity limit of $($capabilities.maximum_threads): $path" }
    foreach ($field in @('ngram_mod', 'ngram_reset_on_begin', 'external_skills', 'skill_tool', 'vision_fit')) {
        if ((Get-PropertyValue $profile $field $null) -isnot [bool]) { throw "Profile value '$field' must be a Boolean: $path" }
    }
    if ($profile.ngram_reset_on_begin -and -not $profile.ngram_mod) { throw "Profile ngram_reset_on_begin requires ngram_mod: $path" }
    if ($profile.ngram_mod -and -not $runtimeCapabilities.request_local_ngram) { throw "Profile runtime '$($profile.runtime)' does not support request-local ngram_mod: $path" }
    return $profile
}

function Get-RuntimePath {
    param($Session, $Profile)
    $runtimeName = [string](Get-PropertyValue $Profile 'runtime' '')
    $property = Get-ExactProperty $Session.runtimes $runtimeName
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
            if ([string]$key -ceq $Name) {
                $value = $Object[$key]
                if ($null -eq $value) { return $Default }
                return $value
            }
        }
        return $Default
    }
    $property = Get-ExactProperty $Object $Name
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
