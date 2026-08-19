[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [string]$Project = (Get-Location).Path,
    [string]$Profile,
    [switch]$Lean,
    [switch]$FullPrompt,
    [switch]$WithVision,
    [switch]$WithConvex,
    [switch]$WithSkills,
    [switch]$WithProjectConfig,
    [switch]$WithPlugins,
    [switch]$KeepServer,
    [switch]$Check,
    [string]$CaptureEndpoint,
    [string]$RunPrompt,
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$OpenCodeArgs
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'lib.ps1')

$session = Get-SessionConfig
$profileConfig = Get-ProfileConfig $session $Profile
$Profile = [string]$profileConfig.name
$serverPath = Get-RuntimePath $session $profileConfig
$skillsEnabled = [bool](Get-PropertyValue $profileConfig 'external_skills' $false) -or [bool]$WithSkills
if ($Lean -and $FullPrompt) { throw 'Choose either -Lean or -FullPrompt, not both.' }
if (-not $FullPrompt) { $Lean = $true }
$projectPath = (Resolve-Path -LiteralPath $Project).Path
if (-not (Test-Path -LiteralPath $projectPath -PathType Container)) { throw "Project is not a directory: $projectPath" }
$openCode = Get-Command opencode -ErrorAction Stop
$modelId = 'local-models/Qwen3.8-27B-ABLITERATED'
$startedHere = $false
$saved = @{}
if ($OpenCodeArgs | Where-Object { $_ -match '^--auto(?:=|$)' }) {
    throw '--auto disables the consent boundary and is not accepted by this launcher.'
}

function Save-Environment([string]$Name) {
    if ($saved.ContainsKey($Name)) { return }
    $item = Get-Item -LiteralPath "Env:$Name" -ErrorAction SilentlyContinue
    $saved[$Name] = [pscustomobject]@{ exists = ($null -ne $item); value = if ($item) { $item.Value } else { $null } }
}
function Set-Environment([string]$Name, [string]$Value) {
    Save-Environment $Name
    Set-Item -LiteralPath "Env:$Name" -Value $Value
}
function Remove-Environment([string]$Name) {
    Save-Environment $Name
    Remove-Item -LiteralPath "Env:$Name" -ErrorAction SilentlyContinue
}
function Restore-Environment {
    foreach ($name in $saved.Keys) {
        if ($saved[$name].exists) { Set-Item -LiteralPath "Env:$name" -Value $saved[$name].value }
        else { Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue }
    }
}

function Get-Policy {
    $credentialRead = [ordered]@{
        '~/.ssh' = 'deny'; '~/.ssh/*' = 'deny'; '~/.ssh/**' = 'deny'
        '~/.aws/*' = 'deny'; '~/.azure/*' = 'deny'; '~/.kube/*' = 'deny'
        '~/.config/gcloud/*' = 'deny'; '~/.docker/config.json' = 'deny'
        '~/.config/gh/hosts.yml' = 'deny'; '~/AppData/Roaming/GitHub CLI/hosts.yml' = 'deny'
        '~/.git-credentials' = 'deny'; '~/.npmrc' = 'deny'; '~/.pypirc' = 'deny'
        '~/.local/share/opencode/auth.json' = 'deny'
        ($session.api_key_file -replace '\\', '/') = 'deny'
    }
    $external = [ordered]@{
        '*' = 'ask'
        '~/.ssh/*' = 'deny'; '~/.aws/*' = 'deny'; '~/.azure/*' = 'deny'; '~/.kube/*' = 'deny'
        '~/.config/gcloud/*' = 'deny'; '~/.docker/*' = 'deny'; '~/.config/gh/*' = 'deny'
        '~/AppData/Roaming/GitHub CLI/*' = 'deny'
        ($session.api_key_file -replace '\\', '/') = 'deny'
    }
    $bash = [ordered]@{
        'git push' = 'ask'; 'git push *' = 'ask'; 'git pull' = 'ask'; 'git pull *' = 'ask'
        'gh pr merge *' = 'ask'; 'gh repo create *' = 'ask'; 'gh release create *' = 'ask'
        'npm publish' = 'ask'; 'npm publish *' = 'ask'; 'pnpm publish *' = 'ask'; 'yarn npm publish *' = 'ask'
        'vercel deploy *' = 'ask'; 'vercel --prod *' = 'ask'; 'wrangler deploy *' = 'ask'
        'git reset --hard' = 'ask'; 'git reset --hard *' = 'ask'; 'git clean *' = 'ask'
        'git rebase' = 'ask'; 'git rebase *' = 'ask'; 'git filter-branch *' = 'ask'; 'git filter-repo *' = 'ask'
        'git checkout -- *' = 'ask'; 'git checkout -f *' = 'ask'; 'git restore *' = 'ask'
        'git commit --amend' = 'ask'; 'git commit --amend *' = 'ask'; 'git branch -D *' = 'ask'
        'git stash drop *' = 'ask'; 'git stash clear' = 'ask'; 'git worktree remove *' = 'ask'
        'git remote' = 'allow'; 'git remote *' = 'allow'; 'git remote add *' = 'ask'
        'git remote remove *' = 'ask'; 'git remote rename *' = 'ask'; 'git remote set-url *' = 'ask'
        'git config --global *' = 'ask'; 'git config --system *' = 'ask'
        'ssh' = 'ask'; 'ssh *' = 'ask'; 'scp *' = 'ask'; 'sftp *' = 'ask'; 'rsync *' = 'ask'
        'rm *' = 'ask'; 'remove-item *' = 'ask'; 'del *' = 'ask'; 'erase *' = 'ask'; 'rmdir *' = 'ask'; 'rd *' = 'ask'
        'stop-process *' = 'ask'; 'taskkill *' = 'ask'; 'stop-service *' = 'ask'; 'restart-service *' = 'ask'
        'restart-computer *' = 'ask'; 'shutdown *' = 'ask'; 'runas *' = 'ask'
    }
    $apiPath = $session.api_key_file -replace '\\', '/'
    $basePath = $session.base_url_file -replace '\\', '/'
    $providerOptions = if ($CaptureEndpoint) {
        [ordered]@{ baseURL = $CaptureEndpoint.TrimEnd('/'); apiKey = 'capture-only-not-a-secret' }
    } else {
        [ordered]@{ baseURL = "{file:$basePath}"; apiKey = "{file:$apiPath}" }
    }
    $permission = [ordered]@{
        webfetch = 'allow'; websearch = 'allow'
        external_directory = $external
        read = $credentialRead
        bash = $bash
    }
    if (-not $skillsEnabled) { $permission.skill = 'deny' }
    $config = [ordered]@{
        provider = [ordered]@{
            'local-models' = [ordered]@{
                npm = '@ai-sdk/openai-compatible'
                name = 'Pinned local Qwen'
                options = $providerOptions
                models = [ordered]@{
                    'Qwen3.8-27B-ABLITERATED' = [ordered]@{
                        name = "Qwen3.8-27B Abliterated ($Profile)"
                        limit = [ordered]@{ context = [int]$profileConfig.context; output = [int]$profileConfig.output }
                    }
                }
            }
        }
        agent = [ordered]@{ title = [ordered]@{ disable = $true } }
        mcp = [ordered]@{ convex = [ordered]@{ enabled = [bool]$WithConvex } }
        permission = $permission
    }
    if ($Lean) {
        $prompt = "Act as a production coding agent. Follow the user's request, inspect before editing, preserve unrelated work, use available tools when useful, and verify changes proportionately. Ask before destructive, irreversible, external, credential, or privacy-sensitive effects. Never expose secrets. Report failed or skipped checks."
        $config.agent.build = [ordered]@{ prompt = $prompt }
        $config.agent.plan = [ordered]@{ prompt = $prompt }
    }
    return $config
}

try {
    if ($CaptureEndpoint -and -not $RunPrompt) { throw '-CaptureEndpoint requires -RunPrompt.' }
    if (-not $CaptureEndpoint) {
        Ensure-LocalApiKey $session
        [IO.File]::WriteAllText($session.base_url_file, "http://$($session.host):$($session.port)/v1", [Text.Encoding]::ASCII)
    }
    # Claude Code's ambient global prompt contains hosted-model routing and
    # delegation instructions that are unrelated to this local OpenCode worker.
    # Skills remain available on demand; only the foreign harness prompt is off.
    Set-Environment 'OPENCODE_DISABLE_CLAUDE_CODE_PROMPT' 'true'
    Set-Environment 'OPENCODE_DISABLE_EXTERNAL_SKILLS' $(if ($skillsEnabled) { 'false' } else { 'true' })
    Set-Environment 'OPENCODE_DISABLE_PROJECT_CONFIG' $(if ($WithProjectConfig) { 'false' } else { 'true' })
    Set-Environment 'OPENCODE_CONFIG_CONTENT' ((Get-Policy) | ConvertTo-Json -Depth 14 -Compress)

    $secretPattern = '(?i)(^|_)(TOKEN|SECRET|PASSWORD|PASSWD|API_?KEY|ACCESS_?KEY|CREDENTIALS?|AUTH)(_|$)'
    $secretNames = @(
        Get-ChildItem Env: | Where-Object { $_.Name -match $secretPattern } | Select-Object -ExpandProperty Name
        'SSH_AUTH_SOCK'; 'SSH_AGENT_PID'; 'GIT_ASKPASS'; 'SSH_ASKPASS'; 'GOOGLE_APPLICATION_CREDENTIALS'
    ) | Sort-Object -Unique
    foreach ($name in $secretNames) { if (Test-Path -LiteralPath "Env:$name") { Remove-Environment $name } }

    [string[]]$pureArgs = if ($WithPlugins) { @() } else { @('--pure') }
    if ($Check) {
        $raw = & $openCode.Source @pureArgs debug config
        if (-not $?) { throw 'OpenCode effective-config check failed.' }
        $effective = $raw | ConvertFrom-Json
        $local = $effective.provider.'local-models'.models.'Qwen3.8-27B-ABLITERATED'
        if ([int]$local.limit.context -ne [int]$profileConfig.context) { throw 'OpenCode and server context limits differ.' }
        foreach ($capability in @('task', 'todowrite')) {
            $value = Get-PropertyValue $effective.permission $capability $null
            if ($value -eq 'deny') { throw "$capability is artificially denied in the production policy." }
        }
        $skillPermission = Get-PropertyValue $effective.permission 'skill' $null
        if ($skillsEnabled -and $skillPermission -eq 'deny') { throw 'Skills should be enabled for this profile.' }
        if (-not $skillsEnabled -and $skillPermission -ne 'deny') { throw 'Skill catalog should be absent from this bounded-context profile.' }
        if ($effective.permission.bash.'git push *' -ne 'ask') { throw 'External Git writes are not consent-gated.' }
        if ($effective.permission.bash.'git remote *' -ne 'allow') { throw 'Read-only remote inspection is not allowed.' }
        Write-Host "OpenCode check passed: $Profile context=$($profileConfig.context) lean=$([bool]$Lean) skills=$skillsEnabled plugins=$([bool]$WithPlugins)"
        Write-Host 'Core agent capabilities are inherited; only safety-sensitive effects are gated.'
        return
    }

    $listener = if ($CaptureEndpoint) { $null } else { Get-Listener $session.port }
    if ($CaptureEndpoint) {
        Write-Host "Capturing one OpenCode request at $CaptureEndpoint"
    } elseif ($listener) {
        $process = Get-ProcessOnPort $session.port
        if (-not $process -or -not $process.Path -or ($process.Path -ine $serverPath)) {
            throw "Port $($session.port) is not owned by the configured local Qwen runtime."
        }
        $state = Read-SessionState $session
        $runningProfile = if ($state) { [string](Get-PropertyValue $state 'profile' '') } else { '' }
        $runningVision = if ($state) { [bool](Get-PropertyValue $state 'vision' $false) } else { $false }
        if ($runningProfile -ne $Profile -or $runningVision -ne [bool]$WithVision) {
            & (Join-Path $PSScriptRoot 'stop-session.ps1')
            if ($WithVision) { & (Join-Path $PSScriptRoot 'start-session.ps1') -Profile $Profile -Vision }
            else { & (Join-Path $PSScriptRoot 'start-session.ps1') -Profile $Profile }
            $startedHere = $true
        }
    } elseif (-not $CaptureEndpoint) {
        if ($WithVision) { & (Join-Path $PSScriptRoot 'start-session.ps1') -Profile $Profile -Vision }
        else { & (Join-Path $PSScriptRoot 'start-session.ps1') -Profile $Profile }
        $startedHere = $true
    }

    Write-Host "Opening OpenCode: $Profile | context=$($profileConfig.context) | project=$projectPath"
    Write-Host "Capabilities: core agent tools on | external skills=$skillsEnabled | lean prompt=$([bool]$Lean) | plugins=$([bool]$WithPlugins) | Convex=$([bool]$WithConvex)"
    Write-Host 'Boundary: consent tripwires and credential shielding; this is not a hostile-code sandbox.'
    if ($RunPrompt) {
        & $openCode.Source run @pureArgs --model $modelId --agent build --format json --dir $projectPath $RunPrompt @OpenCodeArgs
    } else {
        & $openCode.Source @pureArgs --model $modelId $projectPath @OpenCodeArgs
    }
    $exitCode = if ($?) { 0 } elseif ($null -ne $LASTEXITCODE) { $LASTEXITCODE } else { 1 }
} finally {
    if ($startedHere -and -not $KeepServer) {
        try { & (Join-Path $PSScriptRoot 'stop-session.ps1') } catch { Write-Warning $_.Exception.Message }
    }
    Restore-Environment
}
exit $exitCode
