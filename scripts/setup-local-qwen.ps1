[CmdletBinding()]
param(
    [string]$InstallRoot,
    [string]$Profile,
    [string]$Runtime,
    [string]$ReuseArtifactsFrom,
    [switch]$InstallPrerequisites,
    [switch]$SkipVision,
    [switch]$VerifyOnly,
    [switch]$NoShortcut
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repoRoot = Split-Path $PSScriptRoot -Parent
$manifestPath = Join-Path $repoRoot 'config\artifacts.json'
$manifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json
$InstallRoot = [IO.Path]::GetFullPath($InstallRoot)
if ($ReuseArtifactsFrom) { $ReuseArtifactsFrom = [IO.Path]::GetFullPath($ReuseArtifactsFrom) }
$profileSource = Join-Path $repoRoot "config\profiles\$Profile.json"
if (-not (Test-Path -LiteralPath $profileSource -PathType Leaf)) { throw "Unknown profile: $Profile" }

function Get-PropertyValueForSetup($Object, [string]$Name, $Default = $null) {
    if ($null -eq $Object) { return $Default }
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property -or $null -eq $property.Value) { return $Default }
    return $property.Value
}

function Get-Sha256([string]$Path) {
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Assert-Artifact([string]$Path, $Artifact) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw "Artifact missing: $Path" }
    $item = Get-Item -LiteralPath $Path
    if ([int64]$item.Length -ne [int64]$Artifact.bytes) {
        throw "Artifact size mismatch: $Path (got $($item.Length), expected $($Artifact.bytes))"
    }
    $hash = Get-Sha256 $Path
    if ($hash -ne ([string]$Artifact.sha256).ToLowerInvariant()) {
        throw "Artifact SHA-256 mismatch: $Path (got $hash)"
    }
}

function Move-AsideInvalid([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return }
    $suffix = (Get-Date).ToUniversalTime().ToString('yyyyMMdd-HHmmss')
    $backup = "$Path.invalid-$suffix"
    Move-Item -LiteralPath $Path -Destination $backup
    Write-Warning "Preserved invalid artifact as $backup"
}

function Install-Artifact($Artifact) {
    $destination = Join-Path $InstallRoot ([string]$Artifact.relative_path -replace '/', '\')
    if (Test-Path -LiteralPath $destination -PathType Leaf) {
        try {
            Assert-Artifact $destination $Artifact
            Write-Host "Verified: $destination"
            return $destination
        } catch {
            Move-AsideInvalid $destination
        }
    }

    $parent = Split-Path $destination -Parent
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    if ($ReuseArtifactsFrom) {
        $source = Join-Path $ReuseArtifactsFrom ([string]$Artifact.relative_path -replace '/', '\')
        if (Test-Path -LiteralPath $source -PathType Leaf) {
            Assert-Artifact $source $Artifact
            try {
                New-Item -ItemType HardLink -Path $destination -Target $source -ErrorAction Stop | Out-Null
                Write-Host "Reused with hard link: $destination"
            } catch {
                Copy-Item -LiteralPath $source -Destination $destination
                Write-Host "Reused with copy: $destination"
            }
            Assert-Artifact $destination $Artifact
            return $destination
        }
    }

    $partial = "$destination.part"
    Write-Host "Downloading $($Artifact.filename)"
    & curl.exe --fail --location --retry 6 --retry-all-errors --continue-at - --output $partial $Artifact.url
    if ($LASTEXITCODE -ne 0) { throw "Download failed: $($Artifact.url)" }
    Assert-Artifact $partial $Artifact
    Move-Item -LiteralPath $partial -Destination $destination
    return $destination
}

function Install-Tooling {
    if (-not $InstallPrerequisites) { return }
    $agreements = @('--accept-package-agreements', '--accept-source-agreements', '--disable-interactivity')
    & winget install --id Git.Git --exact @agreements
    & winget install --id Kitware.CMake --exact --version 4.2.3 @agreements
    & winget install --id Nvidia.CUDA --exact --version 13.2 @agreements
    & winget install --id Microsoft.VisualStudio.2022.BuildTools --exact @agreements `
        --override '--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended'
    if (-not (Get-Command npm -ErrorAction SilentlyContinue)) {
        & winget install --id OpenJS.NodeJS.LTS --exact @agreements
    }
    & npm install --global opencode-ai@1.18.18
    if ($LASTEXITCODE -ne 0) { throw 'OpenCode installation failed.' }
}

function Assert-CustomBuildTools {
    foreach ($command in @('git', 'cmake')) {
        if (-not (Get-Command $command -ErrorAction SilentlyContinue)) {
            throw "$command is required for the custom runtime. Re-run with -InstallPrerequisites."
        }
    }
    $cuda = 'C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.2'
    if (-not (Test-Path -LiteralPath (Join-Path $cuda 'bin\nvcc.exe'))) {
        throw 'CUDA Toolkit 13.2 is required for the pinned custom runtime. Re-run with -InstallPrerequisites.'
    }
    $vswhere = 'C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere)) {
        throw 'Visual Studio 2022 Build Tools with C++ are required. Re-run with -InstallPrerequisites.'
    }
}

function Assert-CustomRuntime([string]$RuntimeDir) {
    $buildManifestPath = Join-Path $RuntimeDir 'build-manifest.json'
    if (-not (Test-Path -LiteralPath $buildManifestPath)) { throw "Custom runtime manifest missing: $buildManifestPath" }
    $buildManifest = Get-Content -Raw -LiteralPath $buildManifestPath | ConvertFrom-Json
    if ($buildManifest.llama_cpp_commit -ne $manifest.llama_cpp.commit) { throw 'Custom runtime commit does not match the artifact manifest.' }
    $expectedPatch = (Get-FileHash -LiteralPath (Join-Path $repoRoot $manifest.llama_cpp.patch) -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($buildManifest.source_patch_sha256 -ne $expectedPatch) { throw 'Custom runtime patch hash does not match the repo.' }
    foreach ($property in $buildManifest.files.PSObject.Properties) {
        $path = Join-Path $RuntimeDir $property.Name
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Custom runtime file missing: $path" }
        $item = Get-Item -LiteralPath $path
        if ([int64]$item.Length -ne [int64]$property.Value.bytes) { throw "Custom runtime size mismatch: $path" }
        if ((Get-Sha256 $path) -ne $property.Value.sha256) { throw "Custom runtime hash mismatch: $path" }
    }
    $version = & (Join-Path $RuntimeDir 'llama-server.exe') --version 2>&1 | Out-String
    if ($version -notmatch '3cb7ffb') { throw "Custom runtime version mismatch:`n$version" }
}

function Install-OfficialRuntime {
    $cache = Join-Path $InstallRoot '.artifacts'
    New-Item -ItemType Directory -Force -Path $cache | Out-Null
    foreach ($entry in @($manifest.llama_cpp.official_runtime, $manifest.llama_cpp.official_cuda)) {
        $artifact = [pscustomobject]@{
            filename = $entry.filename
            relative_path = ".artifacts/$($entry.filename)"
            url = $entry.url
            sha256 = $entry.sha256
            bytes = $entry.bytes
        }
        if ($ReuseArtifactsFrom) {
            $legacySource = Join-Path $ReuseArtifactsFrom "runtime\$($entry.filename)"
            $cacheTarget = Join-Path $cache $entry.filename
            if (-not (Test-Path -LiteralPath $cacheTarget) -and (Test-Path -LiteralPath $legacySource)) {
                Assert-Artifact $legacySource $artifact
                try { New-Item -ItemType HardLink -Path $cacheTarget -Target $legacySource -ErrorAction Stop | Out-Null }
                catch { Copy-Item -LiteralPath $legacySource -Destination $cacheTarget }
            }
        }
        Install-Artifact $artifact | Out-Null
    }
    $runtimeDir = Join-Path $InstallRoot 'runtime-official'
    New-Item -ItemType Directory -Force -Path $runtimeDir | Out-Null
    Expand-Archive -LiteralPath (Join-Path $cache $manifest.llama_cpp.official_runtime.filename) -DestinationPath $runtimeDir -Force
    Expand-Archive -LiteralPath (Join-Path $cache $manifest.llama_cpp.official_cuda.filename) -DestinationPath $runtimeDir -Force
    $server = Get-ChildItem $runtimeDir -Filter llama-server.exe -Recurse | Select-Object -First 1
    if (-not $server) { throw 'Official runtime archive did not contain llama-server.exe.' }
    if ($server.DirectoryName -ne $runtimeDir) {
        Get-ChildItem $server.DirectoryName -File | Copy-Item -Destination $runtimeDir -Force
    }
    return (Join-Path $runtimeDir 'llama-server.exe')
}

function Install-CustomRuntime {
    $runtimeDir = Join-Path $InstallRoot 'runtime-custom'
    if ($ReuseArtifactsFrom) {
        $reusable = Join-Path $ReuseArtifactsFrom 'runtime-custom'
        if (Test-Path -LiteralPath (Join-Path $reusable 'build-manifest.json')) {
            Assert-CustomRuntime $reusable
            New-Item -ItemType Directory -Force -Path $runtimeDir | Out-Null
            Get-ChildItem -LiteralPath $reusable -File | ForEach-Object {
                $target = Join-Path $runtimeDir $_.Name
                if (Test-Path -LiteralPath $target) { return }
                try { New-Item -ItemType HardLink -Path $target -Target $_.FullName -ErrorAction Stop | Out-Null }
                catch { Copy-Item -LiteralPath $_.FullName -Destination $target }
            }
            Assert-CustomRuntime $runtimeDir
            return (Join-Path $runtimeDir 'llama-server.exe')
        }
    }
    Assert-CustomBuildTools
    $source = Join-Path $InstallRoot 'build\llama.cpp-b10453-ngram-reset'
    $build = Join-Path $source 'build-sm120'
    $expectedCommit = [string]$manifest.llama_cpp.commit
    if (-not (Test-Path -LiteralPath (Join-Path $source '.git'))) {
        New-Item -ItemType Directory -Force -Path (Split-Path $source -Parent) | Out-Null
        & git clone --no-checkout $manifest.llama_cpp.repo $source
        if ($LASTEXITCODE -ne 0) { throw 'llama.cpp clone failed.' }
        & git -C $source checkout --detach $expectedCommit
        if ($LASTEXITCODE -ne 0) { throw 'Pinned llama.cpp checkout failed.' }
        $patch = Join-Path $repoRoot $manifest.llama_cpp.patch
        & git -C $source apply --check $patch
        if ($LASTEXITCODE -ne 0) { throw 'The pinned n-gram reset patch does not apply cleanly.' }
        & git -C $source apply $patch
        if ($LASTEXITCODE -ne 0) { throw 'Applying the pinned n-gram reset patch failed.' }
    }
    $actualCommit = (& git -C $source rev-parse HEAD).Trim()
    if ($actualCommit -ne $expectedCommit) { throw "Custom source commit mismatch: $actualCommit" }
    $diff = & git -C $source diff -- common/speculative.cpp
    if (-not ($diff -match 'LLAMA_NGRAM_MOD_RESET_ON_BEGIN')) {
        throw 'Custom source tree does not contain the request-local n-gram patch.'
    }

    & cmake -S $source -B $build -G 'Visual Studio 17 2022' -A x64 `
        -DGGML_CUDA=ON -DGGML_NATIVE=ON -DGGML_CUDA_FA_ALL_QUANTS=ON `
        -DCMAKE_CUDA_ARCHITECTURES=120 -DLLAMA_CURL=OFF -DLLAMA_BUILD_TESTS=ON
    if ($LASTEXITCODE -ne 0) { throw 'Custom runtime CMake configuration failed.' }
    & cmake --build $build --config Release --target llama-server --parallel 16
    if ($LASTEXITCODE -ne 0) { throw 'Custom runtime build failed.' }

    $built = Join-Path $build 'bin\Release'
    & (Join-Path $repoRoot 'scripts\package-custom-runtime.ps1') -BuiltRuntime $built -Output $runtimeDir
    Assert-CustomRuntime $runtimeDir
    return (Join-Path $runtimeDir 'llama-server.exe')
}

function Copy-ControlPlane {
    foreach ($dir in @('scripts', 'launcher', 'profiles', 'logs')) {
        New-Item -ItemType Directory -Force -Path (Join-Path $InstallRoot $dir) | Out-Null
    }
    Get-ChildItem (Join-Path $repoRoot 'runtime\scripts') -File | Copy-Item -Destination (Join-Path $InstallRoot 'scripts') -Force
    Get-ChildItem (Join-Path $repoRoot 'runtime\launcher') -File | Copy-Item -Destination (Join-Path $InstallRoot 'launcher') -Force
    Get-ChildItem (Join-Path $repoRoot 'config\profiles') -File | Copy-Item -Destination (Join-Path $InstallRoot 'profiles') -Force
    Copy-Item -LiteralPath $manifestPath -Destination (Join-Path $InstallRoot 'config\artifacts.json') -Force
}

function Write-SessionConfig([string]$OfficialServer, [string]$CustomServer) {
    $configDir = Join-Path $InstallRoot 'config'
    New-Item -ItemType Directory -Force -Path $configDir | Out-Null
    $path = Join-Path $configDir 'session.json'
    $cleanup = [ordered]@{ enabled = $false }
    if (Test-Path -LiteralPath $path) {
        try {
            $old = Get-Content -Raw -LiteralPath $path | ConvertFrom-Json
            if ($old.cleanup) {
                $cleanup = [ordered]@{
                    enabled = $true
                    port = Get-PropertyValueForSetup $old.cleanup 'port' 0
                    exe = Get-PropertyValueForSetup $old.cleanup 'exe' ''
                    start_script = Get-PropertyValueForSetup $old.cleanup 'start_script' ''
                    health = Get-PropertyValueForSetup $old.cleanup 'health' ''
                }
            }
            $backup = "$path.backup-$((Get-Date).ToUniversalTime().ToString('yyyyMMdd-HHmmss'))"
            Copy-Item -LiteralPath $path -Destination $backup
        } catch { Write-Warning 'Existing session config could not be preserved; writing the canonical v3 config.' }
    }
    $selectedProfile = Get-Content -Raw -LiteralPath $profileSource | ConvertFrom-Json
    if ($selectedProfile.runtime -eq 'custom' -and -not $CustomServer) {
        throw "$Profile requires the custom runtime, but setup was asked for Official only."
    }
    $activeServer = if ($selectedProfile.runtime -eq 'custom') { $CustomServer } else { $OfficialServer }
    $config = [ordered]@{
        schema = 3
        root = $InstallRoot
        host = '127.0.0.1'
        port = 8100
        active_profile = $Profile
        runtimes = [ordered]@{
            official = $OfficialServer
            custom = $CustomServer
        }
        llama_server = $activeServer
        model = Join-Path $InstallRoot ([string]$manifest.model.relative_path -replace '/', '\')
        mmproj = Join-Path $InstallRoot ([string]$manifest.mmproj.relative_path -replace '/', '\')
        chat_template = Join-Path $InstallRoot ([string]$manifest.chat_template.relative_path -replace '/', '\')
        api_key_file = Join-Path $configDir 'api-key.txt'
        base_url_file = Join-Path $configDir 'base-url.txt'
        state_file = Join-Path $InstallRoot 'logs\session-state.json'
        cleanup = $cleanup
    }
    $json = $config | ConvertTo-Json -Depth 8
    [IO.File]::WriteAllText($path, $json + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
}

function Assert-Install {
    $sessionPath = Join-Path $InstallRoot 'config\session.json'
    if (-not (Test-Path -LiteralPath $sessionPath)) { throw "Session config missing: $sessionPath" }
    $session = Get-Content -Raw -LiteralPath $sessionPath | ConvertFrom-Json
    $profilePath = Join-Path $InstallRoot "profiles\$($session.active_profile).json"
    if (-not (Test-Path -LiteralPath $profilePath)) { throw "Profile missing: $profilePath" }
    $profileConfig = Get-Content -Raw -LiteralPath $profilePath | ConvertFrom-Json
    $runtimeProperty = $session.runtimes.PSObject.Properties[[string]$profileConfig.runtime]
    if (-not $runtimeProperty -or -not $runtimeProperty.Value) { throw "Runtime unavailable for profile $($profileConfig.name): $($profileConfig.runtime)" }
    $serverPath = [string]$runtimeProperty.Value
    Assert-Artifact $session.model $manifest.model
    if (-not $SkipVision) { Assert-Artifact $session.mmproj $manifest.mmproj }
    Assert-Artifact $session.chat_template $manifest.chat_template
    foreach ($path in @($serverPath, (Join-Path $InstallRoot 'scripts\start-session.ps1'), (Join-Path $InstallRoot 'scripts\open-local-opencode.ps1'))) {
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Install file missing: $path" }
    }
    $version = & $serverPath --version 2>&1 | Out-String
    if ($version -notmatch '3cb7ffb') { throw "llama-server is not pinned to commit 3cb7ffb:`n$version" }
    Write-Host "Verified install: $InstallRoot"
    Write-Host "Profile: $($session.active_profile) | Runtime: $($profileConfig.runtime)"
}

if ($VerifyOnly) {
    Assert-Install
    return
}

Install-Tooling
foreach ($dir in @('config', 'models', 'logs', '.artifacts')) {
    New-Item -ItemType Directory -Force -Path (Join-Path $InstallRoot $dir) | Out-Null
}
Install-Artifact $manifest.model | Out-Null
if (-not $SkipVision) { Install-Artifact $manifest.mmproj | Out-Null }
Install-Artifact $manifest.chat_template | Out-Null
$officialServer = Install-OfficialRuntime
$customServer = if ($Runtime -eq 'Custom') { Install-CustomRuntime } else { $null }
Copy-ControlPlane
Write-SessionConfig $officialServer $customServer

$builder = Join-Path $InstallRoot 'scripts\build-launcher.ps1'
if ($NoShortcut) { & $builder -NoShortcut } else { & $builder }
Assert-Install
Write-Host 'Setup complete. Launch "Open Local Qwen.exe" or a generated Desktop shortcut.'
