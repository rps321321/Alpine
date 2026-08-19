[CmdletBinding()]
param(
    [string]$InstallRoot = (Join-Path $env:USERPROFILE 'local-models'),
    [string]$Output
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repoRoot = Split-Path $PSScriptRoot -Parent
if (-not $Output) {
    $inventory = Join-Path $repoRoot 'inventory'
    New-Item -ItemType Directory -Force -Path $inventory | Out-Null
    $Output = Join-Path $inventory ("hardware-{0}.json" -f (Get-Date -Format 'yyyy-MM-dd'))
}

function Get-CommandText([string]$Command, [string[]]$Arguments) {
    try { ((& $Command @Arguments 2>&1) | Out-String).Trim() } catch { $null }
}

$gpuFields = @(
    'name', 'driver_version', 'memory.total', 'pci.bus_id', 'pci.device_id',
    'pcie.link.gen.current', 'pcie.link.gen.max', 'pcie.link.width.current', 'pcie.link.width.max',
    'clocks.max.sm', 'clocks.max.memory', 'power.limit', 'vbios_version'
)
$gpuLines = & nvidia-smi ("--query-gpu=" + ($gpuFields -join ',')) '--format=csv,noheader,nounits'
$computeCaps = @(& nvidia-smi '--query-gpu=compute_cap' '--format=csv,noheader,nounits')
$gpus = @()
for ($i = 0; $i -lt @($gpuLines).Count; $i++) {
    $values = @($gpuLines)[$i] -split ',\s*'
    $gpus += [ordered]@{
        index = $i
        name = $values[0]
        vram_mib = [int]$values[2]
        driver = $values[1]
        compute_capability = @($computeCaps)[$i].Trim()
        pci = [ordered]@{
            bus_id = $values[3]
            device_id = $values[4]
            generation_current = [int]$values[5]
            generation_max = [int]$values[6]
            width_current = [int]$values[7]
            width_max = [int]$values[8]
        }
        clocks_max_mhz = [ordered]@{ sm = [int]$values[9]; memory = [int]$values[10] }
        power_limit_w = [double]$values[11]
        vbios = $values[12]
    }
}

$processor = Get-CimInstance Win32_Processor | Select-Object -First 1
$memoryModules = @(Get-CimInstance Win32_PhysicalMemory | ForEach-Object {
    [ordered]@{
        locator = $_.DeviceLocator.Trim()
        bytes = [int64]$_.Capacity
        configured_mhz = [int]$_.ConfiguredClockSpeed
        spd_mhz = [int]$_.Speed
        manufacturer = $_.Manufacturer.Trim()
        part = $_.PartNumber.Trim()
    }
})
$operatingSystem = Get-CimInstance Win32_OperatingSystem
$computer = Get-CimInstance Win32_ComputerSystem
$board = Get-CimInstance Win32_BaseBoard | Select-Object -First 1
$disks = @(Get-PhysicalDisk | ForEach-Object {
    [ordered]@{
        name = $_.FriendlyName
        media = [string]$_.MediaType
        bus = [string]$_.BusType
        bytes = [int64]$_.Size
        health = [string]$_.HealthStatus
    }
})
$pageFiles = @(Get-CimInstance Win32_PageFileUsage | ForEach-Object {
    [ordered]@{
        path = $_.Name
        allocated_mib = [int]$_.AllocatedBaseSize
        current_mib = [int]$_.CurrentUsage
        peak_mib = [int]$_.PeakUsage
    }
})
$power = Get-CommandText 'powercfg.exe' @('/getactivescheme')
$cudaToolkits = @()
$cudaRoot = 'C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA'
if (Test-Path -LiteralPath $cudaRoot) {
    $cudaToolkits = @(Get-ChildItem $cudaRoot -Directory | Sort-Object Name | ForEach-Object {
        $nvcc = Join-Path $_.FullName 'bin\nvcc.exe'
        [ordered]@{ path = $_.FullName; nvcc = if (Test-Path $nvcc) { Get-CommandText $nvcc @('--version') } else { $null } }
    })
}

$backend = $null
$sessionPath = Join-Path $InstallRoot 'config\session.json'
if (Test-Path -LiteralPath $sessionPath) {
    $session = Get-Content -Raw -LiteralPath $sessionPath | ConvertFrom-Json
    $serverPath = if ($session.PSObject.Properties['llama_server']) { $session.llama_server } elseif ($session.runtime) { $session.runtime.llama_server } else { $null }
    if ($serverPath -and (Test-Path -LiteralPath $serverPath)) {
        $backend = [ordered]@{
            path = $serverPath
            version = Get-CommandText $serverPath @('--version')
            sha256 = (Get-FileHash -LiteralPath $serverPath -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }
}

$repoCommit = Get-CommandText 'git.exe' @('-C', $repoRoot, 'rev-parse', 'HEAD')
$manifest = [ordered]@{
    schema = 1
    captured_at = (Get-Date).ToUniversalTime().ToString('o')
    repo_commit = $repoCommit
    machine = [ordered]@{
        manufacturer = $computer.Manufacturer
        model = $computer.Model
        motherboard = "$($board.Manufacturer) $($board.Product)".Trim()
    }
    os = [ordered]@{
        caption = $operatingSystem.Caption
        version = $operatingSystem.Version
        build = $operatingSystem.BuildNumber
        architecture = $operatingSystem.OSArchitecture
        timezone = [TimeZoneInfo]::Local.Id
    }
    cpu = [ordered]@{
        name = $processor.Name.Trim()
        physical_cores = [int]$processor.NumberOfCores
        logical_processors = [int]$processor.NumberOfLogicalProcessors
        max_clock_mhz_reported_by_cim = [int]$processor.MaxClockSpeed
    }
    memory = [ordered]@{
        physical_bytes = [int64]$computer.TotalPhysicalMemory
        module_count = $memoryModules.Count
        channel_inference = if ($memoryModules.Count -eq 1) { 'single-channel-likely-one-populated-dimm' } else { 'not-inferred' }
        modules = $memoryModules
        pagefiles = $pageFiles
    }
    gpus = $gpus
    storage = $disks
    power_plan = $power
    software = [ordered]@{
        nvidia_smi = Get-CommandText 'nvidia-smi.exe' @()
        cuda_toolkits = $cudaToolkits
        cmake = Get-CommandText 'cmake.exe' @('--version')
        git = Get-CommandText 'git.exe' @('--version')
        python = Get-CommandText 'python.exe' @('--version')
        opencode = Get-CommandText 'opencode.cmd' @('--version')
        backend = $backend
    }
}
$json = $manifest | ConvertTo-Json -Depth 12
$parent = Split-Path $Output -Parent
New-Item -ItemType Directory -Force -Path $parent | Out-Null
[IO.File]::WriteAllText($Output, $json + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
Write-Host "Hardware manifest: $Output"
