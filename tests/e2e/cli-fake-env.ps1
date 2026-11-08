<#
CLI behavior test of the real quayflow.exe against a FAKE environment: argument validation,
error paths, groups, modes, projects, accounts, lanes, shell hook, policy, canary, update and
corrupted state. Complements smoke-fake-env.ps1.

Safe to run anywhere: USERPROFILE, CLAUDE_CONFIG_DIR and QUAYFLOW_HOME point into a temporary
folder and QUAYFLOW_OFFLINE is set. It never runs uninstall, add, reauth or onboard, and never
touches ~/.claude, the real PowerShell profiles or real logins.

Usage: pwsh -File tests/e2e/cli-fake-env.ps1 [-Exe path\to\quayflow.exe]
#>
param([string]$Exe = (Join-Path $PSScriptRoot '..\..\target\debug\quayflow.exe'))
$ErrorActionPreference = 'Continue'
$Exe = (Resolve-Path $Exe).Path
$root = Join-Path ([IO.Path]::GetTempPath()) ("qf-cli-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
$cfgDir = Join-Path $root 'home\.claude'
$realProfile = $env:USERPROFILE
$env:USERPROFILE = Join-Path $root 'home'
$env:CLAUDE_CONFIG_DIR = $cfgDir
$env:QUAYFLOW_HOME = Join-Path $root 'qf'
$env:QUAYFLOW_OFFLINE = '1'
New-Item -ItemType Directory -Force -Path (Join-Path $cfgDir 'sessions'), (Join-Path $env:USERPROFILE 'Documents') | Out-Null

$results = [System.Collections.Generic.List[object]]::new()
function Check([string]$name, [bool]$ok, [string]$detail = '') {
    $results.Add([pscustomobject]@{ R = $(if ($ok) { 'PASS' } else { 'FAIL' }); Check = $name; Detail = ($detail -replace '\s+', ' ').Trim() })
}
function Set-Login([string]$uuid, [string]$tok, [string]$sub) {
    $creds = @{ claudeAiOauth = @{ accessToken = "sk-ant-oat-$tok"; refreshToken = "sk-ant-ort-$tok"; expiresAt = 9999999999999; subscriptionType = $sub } }
    $creds | ConvertTo-Json -Depth 5 -Compress | Set-Content -NoNewline -Encoding utf8 (Join-Path $cfgDir '.credentials.json')
    $cfgPath = Join-Path $cfgDir '.claude.json'
    $cfg = if (Test-Path $cfgPath) { Get-Content -Raw $cfgPath | ConvertFrom-Json } else { [pscustomobject]@{ numStartups = 3 } }
    $cfg | Add-Member -Force -NotePropertyName oauthAccount -NotePropertyValue @{ accountUuid = $uuid; organizationUuid = "org-$uuid" }
    $cfg | ConvertTo-Json -Depth 10 | Set-Content -Encoding utf8 $cfgPath
}
function Live { (Get-Content -Raw (Join-Path $cfgDir '.claude.json') | ConvertFrom-Json).oauthAccount.accountUuid }
$script:code = 0
function Qf { $o = & $Exe @args 2>&1 | Out-String; $script:code = $LASTEXITCODE; $o }

try {
    # --- no engine running ---
    $o = Qf status; Check 'status without engine: clear message, nonzero' ($script:code -ne 0 -and $o -match '(?i)engine|start') "$($script:code) $o"
    $o = Qf frobnicate; Check 'unknown command: nonzero + hint' ($script:code -ne 0 -and $o -match '(?i)unknown|help') "$($script:code) $o"
    $cargoToml = Get-Content (Join-Path $PSScriptRoot '..\..\Cargo.toml') -Raw
    $null = $cargoToml -match '(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"'
    $o = Qf --version; Check '--version' ($o -match ('^quayflow ' + [regex]::Escape($Matches[1]) + ' ')) $o
    $o = Qf -h; Check '-h prints help' ($o -match 'Usage: quayflow') ''
    $o = Qf consent bogus on; Check 'consent unknown key refused' ($script:code -ne 0) "$($script:code) $o"
    $o = Qf consent terms maybe; Check 'consent bad value refused' ($script:code -ne 0) "$($script:code) $o"
    $o = Qf start --if-consented; Check 'start --if-consented without consent does not start' ($o -match 'not_consented|consent') $o
    $o = Qf last; Check 'last without a session refused' ($script:code -ne 0 -and $script:code -ne 3 -and $o -match 'session') "$($script:code) $o"
    $o = Qf last --session no-such-session --json; $j = $null; try { $j = $o | ConvertFrom-Json } catch {}
    Check 'last for an unknown session: exit 3, JSON no_limit, engine not needed' ($script:code -eq 3 -and $j.found -eq $false -and $j.outcome -eq 'no_limit') "$($script:code) $o"

    @{ pid = $PID; sessionId = '11111111-2222-4333-8444-555555555555'; cwd = $root; version = '9.9.9'; status = 'idle' } |
        ConvertTo-Json | Set-Content (Join-Path $cfgDir "sessions\$PID.json")
    New-Item -ItemType Directory -Force -Path (Join-Path $env:QUAYFLOW_HOME 'compat') | Out-Null
    '{"version":"9.9.9","checkedAt":0,"staticPass":true}' | Set-Content (Join-Path $env:QUAYFLOW_HOME 'compat\9.9.9.json')
    Set-Login 'uuid-a' 'A1' 'max'
    foreach ($c in 'credential-storage', 'terms', 'engine-start') { Qf consent $c on | Out-Null }
    $o = Qf start; Check 'start' ($o -match 'started|running') $o

    # --- registration and arguments ---
    $o = Qf register --group Default; Check 'register without --label refused' ($script:code -ne 0) "$($script:code) $o"
    $o = Qf register --label a; Check 'register without --group refused (group mandatory)' ($script:code -ne 0) "$($script:code) $o"
    $o = Qf register --label a --group NoSuch; Check 'register into missing group refused' ($script:code -ne 0) "$($script:code) $o"
    $o = Qf register --label a --group Default; Check 'register a' ($script:code -eq 0) $o
    $o = Qf register --label a2 --group Default; Check 'duplicate account detected' ($script:code -ne 0 -or $o -match '(?i)already') "$($script:code) $o"
    Set-Login 'uuid-b' 'B1' 'max'; Start-Sleep 3
    $o = Qf register --label a --group Default; Check 'duplicate label refused' ($script:code -ne 0 -or $o -match '(?i)already|in use|exists') "$($script:code) $o"
    $o = Qf register --label b --group Default; Check 'register b' ($script:code -eq 0) $o

    # --- groups ---
    $o = Qf group create Work; Check 'group create' ($script:code -eq 0) $o
    $o = Qf group create Work; Check 'group create duplicate refused' ($script:code -ne 0) "$($script:code) $o"
    $o = Qf group create ''; Check 'group create empty name refused' ($script:code -ne 0) "$($script:code) $o"
    $o = Qf group add Work a; Check 'group add' ($script:code -eq 0) $o
    $o = Qf group add Work nobody; Check 'group add unknown account refused' ($script:code -ne 0) "$($script:code) $o"
    $o = Qf group preferred Work a; Check 'group preferred' ($script:code -eq 0) $o
    $o = Qf group rank Work a backup; Check 'group rank' ($script:code -eq 0) $o
    $o = Qf group rank Work a sideways; Check 'group rank bad value refused' ($script:code -ne 0) "$($script:code) $o"
    $o = Qf group tuning Work --threshold 90 --margin 5; Check 'tuning valid' ($script:code -eq 0) $o
    $o = Qf group tuning Work --threshold 150; Check 'tuning threshold >100 refused' ($script:code -ne 0) "$($script:code) $o"
    $o = Qf group tuning Work --margin; Check 'tuning flag without value refused' ($script:code -ne 0) "$($script:code) $o"
    $o = Qf group tuning Work --threshold abc; Check 'tuning non-number refused' ($script:code -ne 0) "$($script:code) $o"
    $o = Qf group tuning Work --threshold -5; Check 'tuning negative refused' ($script:code -ne 0) "$($script:code) $o"
    $o = Qf group list; Check 'group list shows Work' ($o -match 'Work') ''
    $o = Qf group delete Default; Check 'delete default group refused' ($script:code -ne 0) "$($script:code) $o"
    $o = Qf group remove Work a; Check 'group remove' ($script:code -eq 0) $o

    # --- modes ---
    $o = Qf group list; Check 'a new Default group starts in Automatic, a created group in Suggest' (($o -match '(?m)Default.*(?i)automatic') -and ($o -match '(?m)Work.*(?i)suggest')) $o
    $o = Qf mode automatic --group Work; Check 'automatic without --approve refused' ($script:code -ne 0) "$($script:code) $o"
    $o = Qf mode turbo; Check 'mode bad value refused' ($script:code -ne 0) "$($script:code) $o"
    $o = Qf mode automatic --group Work --approve; Check 'automatic with approve' ($script:code -eq 0) $o
    $o = Qf mode suggest --group Work; Check 'back to suggest' ($script:code -eq 0) $o

    # --- projects ---
    $proj = Join-Path $root 'proj'; New-Item -ItemType Directory $proj | Out-Null
    $o = Qf project assign $proj Work; Check 'project assign' ($script:code -eq 0) $o
    $o = Qf project assign $proj NoSuch; Check 'project assign to missing group refused' ($script:code -ne 0) "$($script:code) $o"
    $o = Qf project list; Check 'project list' ($o -match [regex]::Escape('proj')) $o
    $o = Qf project unassign $proj; Check 'project unassign' ($script:code -eq 0) $o

    # --- accounts ---
    $o = Qf label a alpha; Check 'label rename' ($script:code -eq 0) $o
    $o = '' | & $Exe remove b 2>&1 | Out-String; $script:code = $LASTEXITCODE; Check 'remove without --yes and no input: cancelled' ($script:code -ne 0 -and $o -match 'Cancelled') "$($script:code) $o"
    $o = Qf remove b --yes; Check 'remove live account blocked' ($script:code -ne 0 -and (Test-Path (Join-Path $cfgDir '.credentials.json'))) "$($script:code) $o"
    $o = Qf use nobody; Check 'use unknown account refused' ($script:code -ne 0) "$($script:code) $o"
    $o = Qf use alpha; Start-Sleep 2; Check 'use alpha switches' ((Live) -eq 'uuid-a') "$(Live) $o"
    $o = Qf unpin; Check 'unpin' ($script:code -eq 0) $o
    $o = Qf accept; Check 'accept with nothing suggested: graceful' ($o -notmatch '(?i)panick') "$($script:code) $o"
    $o = Qf remove b --yes; Check 'remove non-live account' ($script:code -eq 0) $o
    $o = Qf status --json; $ok = $true; try { $j = $o | ConvertFrom-Json } catch { $ok = $false }; Check 'status --json is valid JSON' $ok ''
    Check 'status --json has no token' ($o -notmatch 'sk-ant') ''

    # --- engine controls ---
    $o = Qf pause; Check 'pause' ($script:code -eq 0) $o
    $o = Qf status; Check 'status shows paused' ($o -match '(?i)paused') ''
    $o = Qf resume; Check 'resume' ($script:code -eq 0) $o

    # --- lanes ---
    'shared' | Set-Content (Join-Path $cfgDir 'CLAUDE.md')
    $o = Qf lanes create side; Check 'lane create' ($script:code -eq 0) $o
    $o = Qf lanes create side; Check 'lane create duplicate refused' ($script:code -ne 0) "$($script:code) $o"
    $o = Qf lanes create '..\escape'; Check 'lane name with path traversal refused' ($script:code -ne 0) "$($script:code) $o"
    $o = Qf lanes check side; Check 'lane check' ($script:code -eq 0) $o
    $o = Qf lanes list; Check 'lane list' ($o -match 'side') $o
    $o = Qf lanes delete side; Check 'lane delete' ($script:code -eq 0) $o
    Check 'lane delete leaves shared setup intact' ((Get-Content (Join-Path $cfgDir 'CLAUDE.md')) -eq 'shared') ''

    # --- handoff, policy, canary, update, shell ---
    $o = Qf handoff prepare --session nope --to Default; Check 'handoff with bad session refused' ($script:code -ne 0) "$($script:code) $o"
    $o = Qf policy; Check 'policy' ($script:code -eq 0) $o
    $o = Qf canary; Check 'canary static, no installed version: graceful' ($o -notmatch '(?i)panick') "$($script:code) $o"
    $o = Qf update --check; Check 'update --check respects QUAYFLOW_OFFLINE' ($script:code -ne 0 -and $o -match 'network access is disabled') "$($script:code) $o"
    # update hands the install to the release's install.ps1 (-Update): a local release folder,
    # installed into a sandboxed LOCALAPPDATA (engine only, no PATH, no consents, no Mod).
    $rel = Join-Path $root 'release'
    New-Item -ItemType Directory -Force $rel | Out-Null
    Copy-Item $Exe, (Join-Path (Split-Path $Exe) 'quayflow-bg.exe') $rel
    Copy-Item (Join-Path $PSScriptRoot '..\..\installer\install.ps1') $rel
    $sums = Get-ChildItem $rel -File | Sort-Object Name | ForEach-Object { '{0}  {1}' -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant(), $_.Name }
    [IO.File]::WriteAllText((Join-Path $rel 'SHA256SUMS'), (($sums -join "`n") + "`n"))
    $o = Qf update --source $rel --skip-attestation; Check 'update refuses a data folder the installer does not use' ($script:code -ne 0 -and $o -match 'installs into') "$($script:code) $o"
    $o = Qf update --source $rel --version v1.2.3; Check 'update: --source and --version refused together' ($script:code -ne 0) "$($script:code) $o"
    $savedLad = $env:LOCALAPPDATA; $savedHome = $env:QUAYFLOW_HOME
    $env:LOCALAPPDATA = Join-Path $root 'lad'
    $env:QUAYFLOW_HOME = Join-Path $env:LOCALAPPDATA 'Quayflow'
    try {
        New-Item -ItemType Directory -Force (Join-Path $env:QUAYFLOW_HOME 'shell') | Out-Null
        Set-Content (Join-Path $env:QUAYFLOW_HOME 'shell\Quayflow.psm1') 'old module'
        $o = Qf update --source $rel --skip-attestation
        $binNew = Join-Path $env:QUAYFLOW_HOME 'bin\quayflow.exe'
        Check 'update installs a local release through install.ps1' ($script:code -eq 0 -and $o -match 'Updating Quayflow' -and $o -match 'updated to the local build' -and (Test-Path $binNew) -and (Get-FileHash $binNew).Hash -eq (Get-FileHash $Exe).Hash) "$($script:code) $o"
        Check 'update: no consent text, Mod left alone' ($o -notmatch 'not affiliated with' -and $o -match 'The Mod was left as it is') $o
        Check 'update refreshed the installed shell module' ((Get-Content -Raw (Join-Path $env:QUAYFLOW_HOME 'shell\Quayflow.psm1')) -match 'function') ''
        Check 'update left no download folder' (-not (Test-Path (Join-Path $env:QUAYFLOW_HOME 'update'))) ''
    } finally { $env:LOCALAPPDATA = $savedLad; $env:QUAYFLOW_HOME = $savedHome }
    $o = Qf shell install; Check 'shell install (sandboxed profile)' ($script:code -eq 0) $o
    $prof = Get-ChildItem -Recurse (Join-Path $env:USERPROFILE 'Documents') -Filter '*profile.ps1' -ErrorAction SilentlyContinue
    Check 'shell install wrote only the sandboxed profile' ($prof.Count -ge 1) ($prof.FullName -join ';')
    $o = Qf shell install; Check 'shell install idempotent' (((Get-Content -Raw $prof[0].FullName) -split 'Quayflow.psm1').Count -le 2) ''
    $o = Qf shell uninstall; $left = [IO.File]::ReadAllText($prof[0].FullName); Check 'shell uninstall' ($script:code -eq 0 -and $left -notmatch 'quayflow') "$($script:code) [$left]"
    $o = Qf migrate-poc --group Default; Check 'migrate-poc with no PoC vault: explains' ($o -match 'No PoC logins found') "$($script:code) $o"

    # --- corrupted state ---
    Qf stop | Out-Null; Start-Sleep 2
    Set-Content (Join-Path $env:QUAYFLOW_HOME 'state.json') '{ not json' -ErrorAction SilentlyContinue
    $o = Qf start; Start-Sleep 2; $o2 = Qf status
    Check 'corrupted state file: engine still starts or explains' ($o2 -notmatch '(?i)panick') "$o $o2"
    $logs = Get-ChildItem -Recurse $env:QUAYFLOW_HOME -File | Where-Object { $_.Extension -in '.log', '.jsonl', '.json' } | Get-Content -Raw
    Check 'no token anywhere in logs/journal/state' (-not (($logs -join '') -match 'sk-ant')) ''
}
finally {
    try { & $Exe stop 2>&1 | Out-Null } catch {}
    Start-Sleep 1
    $env:USERPROFILE = $realProfile
    Remove-Item Env:CLAUDE_CONFIG_DIR, Env:QUAYFLOW_HOME, Env:QUAYFLOW_OFFLINE -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
}
$results | Format-Table -AutoSize -Wrap | Out-String -Width 220 | Write-Output
$f = @($results | Where-Object R -eq 'FAIL').Count
Write-Output ("{0} passed, {1} failed" -f ($results.Count - $f), $f)
exit $f
