<#
Simulation test: the real quayflow.exe against a FAKE Claude usage/profile API
(tests/sim/fake_api.py) and a FAKE Claude environment. It scripts usage over time and checks
the engine's decisions end to end:

  observation of usage windows and per-model windows (FR-08), suggestion at the threshold and
  accept (FR-07, FR-09), Automatic switching, return to the Preferred account, stale data never
  counted as zero, backoff on a rate-limited usage endpoint, a rejected token, hard-limit
  recovery with one switch and one retry, concurrent failures sharing one switch, no recovery on
  an unconfirmed limit, the soonest-available account when every account is exhausted
  (FR-15, FR-16), and the refresh of an idle account's expiring token.

Safe to run anywhere: USERPROFILE, CLAUDE_CONFIG_DIR and QUAYFLOW_HOME point into a temporary
folder, and QUAYFLOW_SIM_API (loopback only) sends usage and profile calls to the fake server.
Nothing reaches api.anthropic.com and no real login is read or written.

What this does NOT prove: that the real service's responses and Claude Code's real behavior
match these fakes. The owner-run real-login scripts remain the release evidence.

Usage: pwsh -File tests/e2e/sim-usage.ps1 [-Exe path\to\quayflow.exe] [-Python python]
#>
param(
    [string]$Exe = (Join-Path $PSScriptRoot '..\..\target\debug\quayflow.exe'),
    [string]$Python = 'python'
)
$ErrorActionPreference = 'Continue'
$Exe = (Resolve-Path $Exe).Path
$root = Join-Path ([IO.Path]::GetTempPath()) ("qf-sim-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
$cfgDir = Join-Path $root 'home\.claude'
$saved = @{ USERPROFILE = $env:USERPROFILE; OFFLINE = $env:QUAYFLOW_OFFLINE }
$env:USERPROFILE = Join-Path $root 'home'
$env:CLAUDE_CONFIG_DIR = $cfgDir
$env:QUAYFLOW_HOME = Join-Path $root 'qf'
Remove-Item Env:QUAYFLOW_OFFLINE -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path (Join-Path $cfgDir 'sessions') | Out-Null

$results = [System.Collections.Generic.List[object]]::new()
function Check([string]$name, [bool]$ok, [string]$detail = '') {
    $results.Add([pscustomobject]@{ R = $(if ($ok) { 'PASS' } else { 'FAIL' }); Check = $name; Detail = ($detail -replace '\s+', ' ').Trim() })
}

# --- fake API ---
$psi = [Diagnostics.ProcessStartInfo]::new($Python, ('-I "{0}"' -f (Join-Path $PSScriptRoot '..\sim\fake_api.py')))
$psi.RedirectStandardOutput = $true
$psi.UseShellExecute = $false
$psi.CreateNoWindow = $true
$mock = [Diagnostics.Process]::Start($psi)
$port = ($mock.StandardOutput.ReadLine() -split ' ')[1]
$api = "http://127.0.0.1:$port"
$env:QUAYFLOW_SIM_API = $api
function Sim([hashtable]$accounts) {
    Invoke-RestMethod "$api/__sim/accounts" -Method Post -ContentType 'application/json' -Body ($accounts | ConvertTo-Json -Depth 6) | Out-Null
}
function SimLog { Invoke-RestMethod "$api/__sim/log" }
function SimRefresh([hashtable]$table) {
    Invoke-RestMethod "$api/__sim/refresh" -Method Post -ContentType 'application/json' -Body ($table | ConvertTo-Json -Depth 6) | Out-Null
}
function LiveRefresh { ((Get-Content -Raw (Join-Path $cfgDir '.credentials.json')) | ConvertFrom-Json).claudeAiOauth.refreshToken }
function SimResetLog { Invoke-RestMethod "$api/__sim/reset-log" -Method Post | Out-Null }

# --- fake Claude environment ---
$TA = 'sk-ant-oat-SIMA'; $TB = 'sk-ant-oat-SIMB'; $TC = 'sk-ant-oat-SIMC'
function Set-Login([string]$uuid, [string]$tok, [long]$ExpiresAt = 9999999999999) {
    @{ claudeAiOauth = @{ accessToken = $tok; refreshToken = "$tok-r"; expiresAt = $ExpiresAt; subscriptionType = 'max'; scopes = @('user:inference', 'user:profile') } } |
        ConvertTo-Json -Compress | Set-Content -NoNewline -Encoding utf8 (Join-Path $cfgDir '.credentials.json')
    $p = Join-Path $cfgDir '.claude.json'
    $c = if (Test-Path $p) { Get-Content -Raw $p | ConvertFrom-Json } else { [pscustomobject]@{ numStartups = 1 } }
    $c | Add-Member -Force -NotePropertyName oauthAccount -NotePropertyValue @{ accountUuid = $uuid; organizationUuid = "org-$uuid" }
    $c | ConvertTo-Json -Depth 10 | Set-Content -Encoding utf8 $p
}
function LiveToken { ((Get-Content -Raw (Join-Path $cfgDir '.credentials.json')) | ConvertFrom-Json).claudeAiOauth.accessToken }
function Qf { & $Exe @args 2>&1 | Out-String }
function State { Qf status --json | ConvertFrom-Json }
function Env0 { (State).envs[0] }
function Acct([string]$label) { (State).accounts | Where-Object { $_.label -eq $label } }
function WaitFor([scriptblock]$cond, [int]$seconds = 20) {
    $deadline = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $deadline) { if (& $cond) { return $true }; Start-Sleep -Milliseconds 500 }
    return [bool](& $cond)
}
function Api([string]$path, $body) {
    $conn = Get-Content -Raw (Join-Path $env:QUAYFLOW_HOME 'engine.json') | ConvertFrom-Json
    Invoke-RestMethod "http://127.0.0.1:$($conn.port)$path" -Method Post -Headers @{ 'x-quayflow-token' = $conn.token } -ContentType 'application/json' -Body ($body | ConvertTo-Json -Depth 6)
}
$sid = '11111111-2222-4333-8444-555555555555'
# The Mod's recovery call. -SessionPct is what the session itself reported (it sees the limit
# before the engine's next usage poll does).
function Recover([string]$turn, [double]$SessionPct = -1) {
    $body = @{ sessionId = $sid; turnId = $turn; index = 1; model = 'claude-sonnet'; errorKind = 'rate_limit' }
    if ($SessionPct -ge 0) { $body.rateLimits = @(@{ kind = 'five_hour'; percentUsed = $SessionPct; resetsAt = $null }) }
    Api '/v1/recover' $body
}

try {
    @{ pid = $PID; sessionId = $sid; cwd = $root; version = '9.9.9'; status = 'idle' } | ConvertTo-Json | Set-Content (Join-Path $cfgDir "sessions\$PID.json")
    New-Item -ItemType Directory -Force -Path (Join-Path $env:QUAYFLOW_HOME 'compat') | Out-Null
    '{"version":"9.9.9","checkedAt":0,"staticPass":true}' | Set-Content (Join-Path $env:QUAYFLOW_HOME 'compat\9.9.9.json')
    Sim @{
        $TA = @{ uuid = 'uuid-a'; five_hour = 40; seven_day = 20; models = @(@{ name = 'Opus'; percent = 30; reset_s = 7200 }) }
        $TB = @{ uuid = 'uuid-b'; five_hour = 10; seven_day = 5 }
        $TC = @{ uuid = 'uuid-c'; five_hour = 5; seven_day = 5 }
    }
    foreach ($c in 'credential-storage', 'terms', 'engine-start') { Qf consent $c on | Out-Null }
    # Fast polling, short freshness and cooldown (allowed only in simulation mode).
    $cp = Join-Path $env:QUAYFLOW_HOME 'config.json'
    $cfg = Get-Content -Raw $cp | ConvertFrom-Json
    $cfg.tuning.livePollS = 1; $cfg.tuning.idlePollS = 1; $cfg.tuning.cooldownS = 3
    $cfg.tuning.liveFreshS = 8; $cfg.tuning.idleFreshS = 8; $cfg.tuning.backoffMaxS = 30
    $cfg.tuning.concurrentWindowS = 5
    $cfg | ConvertTo-Json -Depth 20 | Set-Content $cp

    Set-Login 'uuid-a' $TA
    Qf start | Out-Null
    # A new Default group starts in Automatic; the Suggest checks below need Suggest.
    Qf mode suggest --group Default | Out-Null
    $st = Qf status
    Check 'simulation mode is announced in status' ($st -match 'SIMULATION MODE') ''
    $r = Qf register --label a --group Default
    Check 'register verifies identity through the fake profile endpoint' ($r -match 'Registered a') $r
    Set-Login 'uuid-x' 'sk-ant-oat-UNKNOWN'
    Start-Sleep 2
    $r = Qf register --label x --group Default
    Check 'register refuses a token the profile endpoint rejects' ($r -notmatch 'Registered') $r
    Set-Login 'uuid-b' $TB; Start-Sleep 2; Qf register --label b --group Default | Out-Null
    Set-Login 'uuid-c' $TC; Start-Sleep 2; Qf register --label c --group Default | Out-Null
    Qf use a | Out-Null
    Check 'manual switch back to a' (WaitFor { (LiveToken) -eq $TA }) (LiveToken)
    Qf unpin | Out-Null

    # 1. Observation
    $ok = WaitFor { $a = Acct 'a'; ($a | ConvertTo-Json -Depth 8) -match '40' }
    Check 'usage observed from the fake API (a 40% 5h)' $ok ((Acct 'a') | ConvertTo-Json -Depth 8 -Compress)
    Check 'per-model window observed (Opus 30%)' (((Acct 'a') | ConvertTo-Json -Depth 8) -match 'Opus') ''

    # 2. Suggest at threshold, then accept
    Sim @{ $TA = @{ five_hour = 90 } }
    $ok = WaitFor { $e = Env0; $e.suggestion -and $e.suggestion.to }
    $e = Env0
    $toLabel = ((State).accounts | Where-Object { $_.id -eq $e.suggestion.to }).label
    Check 'suggestion at 85% threshold' $ok "$($e.suggestion.reason) | decision: $($e.decision) | transition: $($e.transition.state)"
    Check 'suggestion picks the account with most room (c)' ($toLabel -eq 'c') "to=$toLabel"
    Check 'Suggest mode does not switch by itself' ((LiveToken) -eq $TA) ''
    Qf accept | Out-Null
    Check 'accept switches to the suggestion' (WaitFor { (LiveToken) -eq $TC }) (LiveToken)

    # 3. Automatic mode and return home
    Qf mode automatic --group Default --approve | Out-Null
    Qf group preferred Default b | Out-Null
    Start-Sleep 4  # cooldown after the accepted switch
    Check 'Automatic returns to the Preferred account when it has room' (WaitFor { (LiveToken) -eq $TB } 20) (LiveToken)
    Sim @{ $TB = @{ five_hour = 92 } }
    Check 'Automatic switches away from b at the threshold' (WaitFor { (LiveToken) -ne $TB } 20) (LiveToken)
    Check 'Automatic does not pick a (still at 90%)' ((LiveToken) -eq $TC) (LiveToken)

    # 4. Stale data never counts as zero
    Qf mode suggest --group Default | Out-Null
    Sim @{ $TA = @{ five_hour = 0; status = 500 } }
    Start-Sleep 10  # a's last good reading (90%) ages past idleFreshS
    $a = (Acct 'a') | ConvertTo-Json -Depth 8 -Compress
    Check 'failing usage endpoint: a is not shown as 0%' ($a -notmatch '"pct":0[,}]' -and $a -notmatch '\b0% 5h') $a
    Sim @{ $TC = @{ five_hour = 95 } }
    $ok = WaitFor { $e = Env0; $e.suggestion -and $e.suggestion.to }
    $e = Env0
    $toLabel = if ($e.suggestion) { ((State).accounts | Where-Object { $_.id -eq $e.suggestion.to }).label }
    Check 'stale account a is never suggested' ($toLabel -ne 'a') "to=$toLabel"
    Sim @{ $TA = @{ status = 200; five_hour = 30 } }

    # 5. Rejected token
    Sim @{ $TB = @{ status = 401 } }
    $ok = WaitFor { ((Acct 'b') | ConvertTo-Json -Depth 8) -match '401|not accepted' } 15
    Check 'rejected token is reported for b' $ok ((Acct 'b') | ConvertTo-Json -Depth 8 -Compress)
    Sim @{ $TB = @{ status = 200 } }

    # 6. Backoff on a rate-limited usage endpoint
    Sim @{ $TB = @{ status = 429; retry_after = 20 } }
    Start-Sleep 2; SimResetLog; Start-Sleep 10
    $n = (SimLog).$TB.'/api/oauth/usage'
    Check 'usage endpoint 429 with Retry-After 20: no polling for 10 s' ([int]$n -le 1) "requests=$n"
    Sim @{ $TB = @{ status = 200; five_hour = 92 } }

    # 7. Hard-limit recovery (the Mod's call, made directly)
    # Usage first, then Automatic: enabling Automatic while the live account still reads 95%
    # would switch at once, and a recovery right after a switch shares it by design.
    Sim @{ $TA = @{ five_hour = 30 }; $TB = @{ five_hour = 92 }; $TC = @{ five_hour = 60 } }
    Start-Sleep 3  # fresh readings (1 s polling)
    Qf mode automatic --group Default --approve | Out-Null
    Start-Sleep 6  # past cooldown and the concurrent-failure window
    $before = LiveToken
    $r = Recover 'turn-unconfirmed'
    Check 'no recovery on an unconfirmed limit (live at 60%)' (-not $r.retry -and -not $r.switched -and (LiveToken) -eq $before) ($r | ConvertTo-Json -Compress)
    $r1 = Recover 'turn-1' 100
    $r2 = Recover 'turn-2' 100
    Check 'confirmed limit: one switch and one retry' ($r1.retry -and $r1.switched) ($r1 | ConvertTo-Json -Compress)
    Check 'recovery picks an account with room (a)' (WaitFor { (LiveToken) -eq $TA }) (LiveToken)
    Check 'concurrent failure shares the switch (no second switch)' ($r2.retry -and -not $r2.switched) ($r2 | ConvertTo-Json -Compress)
    $r3 = Recover 'turn-1'
    Check 'same request is not retried twice' (-not $r3.retry) ($r3 | ConvertTo-Json -Compress)

    # 8. Every account exhausted: soonest available
    # The retried request succeeded and the task moved on (the Mod reports each step).
    Api '/v1/step' @{ sessionId = $sid; turnId = 'turn-2'; index = 2; ok = $true; stopReason = 'end_turn'; model = 'claude-sonnet' } | Out-Null
    Start-Sleep 6  # past the concurrent-failure window
    Sim @{
        $TA = @{ five_hour = 100; five_hour_reset_s = 7200 }
        $TB = @{ five_hour = 100; five_hour_reset_s = 600 }
        $TC = @{ five_hour = 100; five_hour_reset_s = 3600 }
    }
    # Each account's reading must show its limit: an out-of-date reading says nothing about
    # when an account is available (b may still be slowed down after the 429 in step 6).
    $seen = WaitFor { @('a', 'b', 'c') | ForEach-Object { $u = (Acct $_).usage; $u -and $u.fresh -and ($u.windows | Where-Object { $_.pct -ge 100 }) } | Where-Object { -not $_ } | Measure-Object | ForEach-Object { $_.Count -eq 0 } } 60
    Check 'all exhausted: every reading shows the limit' $seen ((State).accounts | ConvertTo-Json -Depth 8 -Compress)
    $r = Recover 'turn-exhausted' 100
    Check 'all exhausted: switches to the account available soonest (b)' (WaitFor { (LiveToken) -eq $TB }) "$(LiveToken) $($r | ConvertTo-Json -Compress)"
    Check 'all exhausted: no retry; Claude''s own wait continues' (-not $r.retry) ($r | ConvertTo-Json -Compress)

    # 9. Idle account whose access token expires: the engine refreshes the saved copy (only
    # that copy), stores it, reads usage with the new token, and a later switch writes it.
    Qf mode manual --group Default | Out-Null
    $TD = 'sk-ant-oat-SIMD'; $TD2 = 'sk-ant-oat-SIMD2'
    Sim @{ $TD = @{ uuid = 'uuid-d'; five_hour = 10; seven_day = 5 }; $TD2 = @{ uuid = 'uuid-d'; five_hour = 11; seven_day = 6 } }
    SimRefresh @{ "$TD-r" = @{ access = $TD2; refresh = "$TD-r2" } }
    $soon = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() + 240000
    Set-Login 'uuid-d' $TD -ExpiresAt $soon
    Start-Sleep 2
    $r = Qf register --label d --group Default
    Check 'register d (token expiring in 4 min)' ($r -match 'Registered d') $r
    SimResetLog
    $u = Qf use b
    Check 'switch away from d' (WaitFor { (LiveToken) -eq $TB }) "$(LiveToken) $u"
    $ok = WaitFor { $l = SimLog; [int]$l.$TD2.'/api/oauth/usage' -ge 1 } 20
    $l = SimLog
    Check 'idle d: saved token refreshed, usage read with the new token' $ok ($l | ConvertTo-Json -Depth 4 -Compress)
    Check 'idle d: exactly one refresh request' ([int]$l."$TD-r".'/v1/oauth/token' -eq 1) ($l | ConvertTo-Json -Depth 4 -Compress)
    Check 'live b is never refreshed by Quayflow' (-not $l."$TB-r") ''
    $d = (Acct 'd') | ConvertTo-Json -Depth 8 -Compress
    Check 'd shows fresh usage (11% 5h)' ($d -match '"pct":11') $d
    Qf use d | Out-Null
    Check 'switch to d writes the refreshed login' (WaitFor { (LiveToken) -eq $TD2 -and (LiveRefresh) -eq "$TD-r2" }) "$(LiveToken)"
    # A refused refresh token marks the account for a new sign-in.
    $TE = 'sk-ant-oat-SIME'
    Sim @{ $TE = @{ uuid = 'uuid-e'; five_hour = 10; seven_day = 5 } }
    Set-Login 'uuid-e' $TE -ExpiresAt ([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() + 240000)
    Start-Sleep 2
    Qf register --label e --group Default | Out-Null
    Qf use d | Out-Null
    $ok = WaitFor { ((Acct 'e') | ConvertTo-Json -Depth 8) -match 'refresh_rejected' } 20
    Check 'refused refresh token: e is marked for a new sign-in' $ok ((Acct 'e') | ConvertTo-Json -Depth 8 -Compress)

    $all = (Get-ChildItem -Recurse $env:QUAYFLOW_HOME -File | Where-Object { $_.Extension -in '.log', '.jsonl', '.json' } | Get-Content -Raw) -join ''
    Check 'no token in logs, journal or state' (-not ($all -match 'sk-ant-oat-SIM')) ''
}
finally {
    try { & $Exe stop 2>&1 | Out-Null } catch { Write-Verbose 'engine already stopped' }
    Start-Sleep 1
    if ($mock -and -not $mock.HasExited) { $mock.Kill() }
    $env:USERPROFILE = $saved.USERPROFILE
    if ($saved.OFFLINE) { $env:QUAYFLOW_OFFLINE = $saved.OFFLINE }
    Remove-Item Env:CLAUDE_CONFIG_DIR, Env:QUAYFLOW_HOME, Env:QUAYFLOW_SIM_API -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
}
$results | Format-Table -AutoSize -Wrap | Out-String -Width 220 | Write-Output
$f = @($results | Where-Object R -eq 'FAIL').Count
Write-Output ("{0} passed, {1} failed" -f ($results.Count - $f), $f)
exit $f
