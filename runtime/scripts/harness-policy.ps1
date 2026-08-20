Set-StrictMode -Version Latest

function New-HarnessPolicy {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]$Session,
        [Parameter(Mandatory = $true)]$Profile,
        [bool]$Lean = $true,
        [bool]$SkillsEnabled = $false,
        [bool]$WithConvex = $false,
        [string]$CaptureEndpoint
    )

    $credentialRead = [ordered]@{
        '~/.ssh' = 'deny'; '~/.ssh/*' = 'deny'; '~/.ssh/**' = 'deny'
        '~/.aws/*' = 'deny'; '~/.azure/*' = 'deny'; '~/.kube/*' = 'deny'
        '~/.config/gcloud/*' = 'deny'; '~/.docker/config.json' = 'deny'
        '~/.config/gh/hosts.yml' = 'deny'; '~/AppData/Roaming/GitHub CLI/hosts.yml' = 'deny'
        '~/.git-credentials' = 'deny'; '~/.npmrc' = 'deny'; '~/.pypirc' = 'deny'
        '~/.local/share/opencode/auth.json' = 'deny'
        ([string]$Session.api_key_file -replace '\\', '/') = 'deny'
    }
    $external = [ordered]@{
        '*' = 'ask'
        '~/.ssh/*' = 'deny'; '~/.aws/*' = 'deny'; '~/.azure/*' = 'deny'; '~/.kube/*' = 'deny'
        '~/.config/gcloud/*' = 'deny'; '~/.docker/*' = 'deny'; '~/.config/gh/*' = 'deny'
        '~/AppData/Roaming/GitHub CLI/*' = 'deny'
        ([string]$Session.api_key_file -replace '\\', '/') = 'deny'
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
    $apiPath = [string]$Session.api_key_file -replace '\\', '/'
    $basePath = [string]$Session.base_url_file -replace '\\', '/'
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
    if (-not $SkillsEnabled) { $permission.skill = 'deny' }

    $config = [ordered]@{
        provider = [ordered]@{
            'local-models' = [ordered]@{
                npm = '@ai-sdk/openai-compatible'
                name = 'Pinned local Qwen'
                options = $providerOptions
                models = [ordered]@{
                    'Qwen3.8-27B-ABLITERATED' = [ordered]@{
                        name = "Qwen3.8-27B Abliterated ($([string]$Profile.name))"
                        limit = [ordered]@{ context = [int]$Profile.context; output = [int]$Profile.output }
                    }
                }
            }
        }
        agent = [ordered]@{ title = [ordered]@{ disable = $true } }
        tool_output = [ordered]@{ max_lines = 500; max_bytes = 12288 }
        mcp = [ordered]@{ convex = [ordered]@{ enabled = $WithConvex } }
        permission = $permission
    }
    if ($Lean) {
        $prompt = "Act as a production coding agent. Follow the user's request, inspect before editing, preserve unrelated work, use available tools when useful, and verify changes proportionately. Ask before destructive, irreversible, external-write, credential, privilege, or privacy-sensitive effects. Never expose secrets. This policy does not restrict topics, reasoning, or technical methods. Report failed or skipped checks."
        $config.agent.build = [ordered]@{ prompt = $prompt }
        $config.agent.plan = [ordered]@{ prompt = $prompt }
    }
    return $config
}

function Enter-HarnessEnvironment {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$ConfigJson,
        [bool]$SkillsEnabled = $false,
        [bool]$WithProjectConfig = $false
    )

    $saved = @{}
    $save = {
        param([string]$Name)
        if ($saved.ContainsKey($Name)) { return }
        $item = Get-Item -LiteralPath "Env:$Name" -ErrorAction SilentlyContinue
        $saved[$Name] = [pscustomobject]@{
            exists = ($null -ne $item)
            value = if ($null -ne $item) { [string]$item.Value } else { $null }
        }
    }
    $set = {
        param([string]$Name, [string]$Value)
        & $save $Name
        Set-Item -LiteralPath "Env:$Name" -Value $Value
    }
    $remove = {
        param([string]$Name)
        & $save $Name
        Remove-Item -LiteralPath "Env:$Name" -ErrorAction SilentlyContinue
    }

    & $set 'OPENCODE_DISABLE_CLAUDE_CODE_PROMPT' 'true'
    & $set 'OPENCODE_DISABLE_EXTERNAL_SKILLS' $(if ($SkillsEnabled) { 'false' } else { 'true' })
    & $set 'OPENCODE_DISABLE_PROJECT_CONFIG' $(if ($WithProjectConfig) { 'false' } else { 'true' })
    & $set 'OPENCODE_ENABLE_EXA' 'true'
    & $set 'OPENCODE_CONFIG_CONTENT' $ConfigJson

    $secretPattern = '(?i)(^|_)(TOKEN|SECRET|PASSWORD|PASSWD|API_?KEY|ACCESS_?KEY|CREDENTIALS?|AUTH)(_|$)'
    $secretNames = @(
        Get-ChildItem Env: | Where-Object { $_.Name -match $secretPattern } | Select-Object -ExpandProperty Name
        'SSH_AUTH_SOCK'; 'SSH_AGENT_PID'; 'GIT_ASKPASS'; 'SSH_ASKPASS'; 'GOOGLE_APPLICATION_CREDENTIALS'
    ) | Sort-Object -Unique
    foreach ($name in $secretNames) {
        if (Test-Path -LiteralPath "Env:$name") { & $remove $name }
    }
    return $saved
}

function Exit-HarnessEnvironment {
    [CmdletBinding()]
    param([Parameter(Mandatory = $true)]$State)

    foreach ($name in $State.Keys) {
        if ($State[$name].exists) {
            Set-Item -LiteralPath "Env:$name" -Value $State[$name].value
        } else {
            Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue
        }
    }
}

function Assert-EffectiveHarnessPolicy {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]$Effective,
        [Parameter(Mandatory = $true)]$Profile,
        [bool]$SkillsEnabled = $false
    )

    $local = $Effective.provider.'local-models'.models.'Qwen3.8-27B-ABLITERATED'
    if ([int]$local.limit.context -ne [int]$Profile.context) {
        throw 'OpenCode and server context limits differ.'
    }
    if ([int]$Effective.tool_output.max_lines -ne 500 -or [int]$Effective.tool_output.max_bytes -ne 12288) {
        throw 'OpenCode tool-output bounds do not match the 16K harness.'
    }
    foreach ($capability in @('webfetch', 'websearch')) {
        if ($Effective.permission.$capability -ne 'allow') {
            throw "$capability is not available for read-only research."
        }
    }
    foreach ($capability in @('task', 'todowrite')) {
        $property = $Effective.permission.PSObject.Properties[$capability]
        if ($null -ne $property -and $property.Value -eq 'deny') {
            throw "$capability is artificially denied in the production policy."
        }
    }
    $skillProperty = $Effective.permission.PSObject.Properties['skill']
    $skillPermission = if ($null -ne $skillProperty) { $skillProperty.Value } else { $null }
    if ($SkillsEnabled -and $skillPermission -eq 'deny') { throw 'Skills should be enabled for this profile.' }
    if (-not $SkillsEnabled -and $skillPermission -ne 'deny') { throw 'Skill catalog should be absent from this bounded-context profile.' }
    if ($Effective.permission.bash.'git push *' -ne 'ask') { throw 'External Git writes are not consent-gated.' }
    if ($Effective.permission.bash.'git remote *' -ne 'allow') { throw 'Read-only remote inspection is not allowed.' }
}
