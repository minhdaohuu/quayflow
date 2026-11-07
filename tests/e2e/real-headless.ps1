<#
Owner-run headless (`claude -p`) scenarios against REAL Claude logins and a REAL usage limit.

THIS SWITCHES YOUR LIVE CLAUDE LOGIN. It moves the login to an account you name that is
already at its usage limit, runs `claude -p`, and puts everything back at the end (live
account, pin, group mode). Synthetic prompts only (model haiku). Run it yourself; read it first.

Prerequisites
  - quayflow installed, engine running, the Quayflow plugin installed in Claude Code
    (or pass -PluginDir mod\quayflow to load the repository copy).
  - In one group: the live account, the exhausted account (-Exhausted) and at least one
    other account with room (quayflow status).

Cases (default: all)
  X1  real limit, Automatic: one switch, one retry, one clean JSON result, exit 0
  X2  real limit, Automatic, three parallel runs: one shared switch, every run answers
  X3  real limit, Suggest: no switch, no question, no hang; Claude's own result stands
  X4  engine stopped: `claude -p` behaves as without Quayflow (and the cost of Quayflow on)
  X5  the agent loop: a run fails at the limit (Suggest), `quayflow last` says when to run
      again, the login moves, and a FRESH `claude -p` succeeds on the other account

`quayflow last --session <id> --json` is checked after X1, X3 and X5 (needs v0.1.3 or later).

In every case stdout must be exactly Claude's own JSON, and neither stdout nor stderr may
carry a Quayflow line (headless output is never changed).

Usage: pwsh -File tests/e2e/real-headless.ps1 -Exhausted <label> [-Case X1,X2,X3,X4,X5] [-PluginDir path] [-Group Default]
#>
param(
    [Parameter(Mandatory)][string]$Exhausted,
    [string[]]$Case = @('X1', 'X2', 'X3', 'X4', 'X5'),
    [string]$PluginDir,
    [string]$Group = 'Default'
)
$ErrorActionPreference = 'Stop'
if ($MyInvocation.InvocationName -eq '.') {
    Write-Error 'Run this script with: pwsh -File <script> (dot-sourcing would close your shell when it exits).'
    return
}
$qfHome = if ($env:QUAYFLOW_HOME) { $env:QUAYFLOW_HOME } else { Join-Path $env:LOCALAPPDATA 'Quayflow' }
$qf = (Get-Command quayflow -ErrorAction SilentlyContinue).Source
if (-not $qf) { $qf = Join-Path $qfHome 'bin\quayflow.exe' }
$claude = (Get-Command claude -CommandType Application | Select-Object -First 1).Source
$out = Join-Path $qfHome 'e2e'
New-Item -ItemType Directory -Force $out | Out-Null

function Api([string]$method, [string]$route, $body) {
    $c = Get-Content -Raw (Join-Path $qfHome 'engine.json') | ConvertFrom-Json
    $a = @{ Uri = "http://127.0.0.1:$($c.port)$route"; Method = $method; Headers = @{ 'x-quayflow-token' = $c.token }; TimeoutSec = 30 }
    if ($null -ne $body) { $a.Body = ($body | ConvertTo-Json -Depth 6 -Compress); $a.ContentType = 'application/json' }
    Invoke-RestMethod @a
}
try { $state = Api GET '/v1/state' } catch {
    Write-Host 'The Quayflow engine is not running for this user. Start it (quayflow start) and check quayflow status.'
    exit 2
}
$envd = $state.envs | Where-Object id -eq 'default'
if (-not $envd.live.known) { throw 'the live login must be a registered account' }
$home0 = $envd.live.label
$mode0 = ($state.groups | Where-Object name -eq $Group).mode
$ex = $state.accounts | Where-Object { $_.label -eq $Exhausted }
if (-not $ex) { throw "no registered account named $Exhausted" }
if (-not ($ex.groups -contains $Group)) { throw "$Exhausted is not in group $Group" }
$room = @($state.accounts | Where-Object { $_.id -ne $ex.id -and $_.health -eq 'ok' -and $_.groups -contains $Group })
if (-not $room) { throw "need another healthy account in group $Group" }
Write-Host "Live now: $home0. Exhausted: $Exhausted. Group $Group (mode $mode0). Others: $($room.label -join ', ')"

$results = [System.Collections.Generic.List[object]]::new()
function Check([string]$c, [string]$name, [bool]$ok, [string]$detail = '') {
    $results.Add([pscustomobject]@{ Case = $c; Result = $(if ($ok) { 'PASS' } else { 'FAIL' }); Check = $name; Detail = $detail })
}
function Now { [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() }
function Journal([int64]$since) {
    Get-Content (Join-Path $qfHome 'journal.jsonl') | ForEach-Object { try { $_ | ConvertFrom-Json } catch {} } | Where-Object { $_.ts -ge $since }
}
function Live { ((Api GET '/v1/state').envs | Where-Object id -eq 'default').live }

function Start-Run([string]$name, [string]$prompt) {
    $stdout = Join-Path $out "$name.json"; $stderr = Join-Path $out "$name.err"
    $a = @('-p', "`"$prompt`"", '--output-format', 'json', '--model', 'haiku')
    if ($PluginDir) { $a += @('--plugin-dir', "`"$((Resolve-Path $PluginDir).Path)`"") }
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $p = Start-Process -FilePath $claude -NoNewWindow -PassThru -RedirectStandardOutput $stdout -RedirectStandardError $stderr -ArgumentList $a
    [pscustomobject]@{ Name = $name; Process = $p; Stdout = $stdout; Stderr = $stderr; Clock = $sw }
}
function Finish($run, [int]$timeoutS = 180) {
    if (-not $run.Process.WaitForExit($timeoutS * 1000)) { $run.Process.Kill(); $run | Add-Member Hung $true }
    $run.Clock.Stop()
    $raw = Get-Content -Raw $run.Stdout -ErrorAction SilentlyContinue
    $json = $null; try { $json = $raw | ConvertFrom-Json } catch {}
    $run | Add-Member Json $json -Force
    $run | Add-Member Raw "$raw" -Force
    $run | Add-Member Err "$(Get-Content -Raw $run.Stderr -ErrorAction SilentlyContinue)" -Force
    $run | Add-Member Seconds ([math]::Round($run.Clock.Elapsed.TotalSeconds, 1)) -Force
    $run
}
function CleanOutput([string]$c, $run) {
    Check $c "$($run.Name): stdout is exactly one JSON result" ($null -ne $run.Json -and $run.Json.type -eq 'result') ($run.Raw.Substring(0, [Math]::Min(120, $run.Raw.Length)))
    Check $c "$($run.Name): no Quayflow text in stdout or stderr" (-not ("$($run.Raw)`n$($run.Err)" -match 'Quayflow|quayflow'))
}
function Last($run) {
    $raw = & $qf last --session $run.Json.session_id --json 2>$null
    try { ($raw -join "`n") | ConvertFrom-Json } catch { $null }
}
function UseExhausted {
    & $qf use $Exhausted | Out-Null
    Start-Sleep 2
    Check 'setup' "live is $Exhausted" ((Live).label -eq $Exhausted)
}
function Restore {
    & $qf mode $mode0 --group $Group | Out-Null
    if ((Live).label -ne $home0) { & $qf use $home0 | Out-Null; Start-Sleep 2 }
    & $qf unpin | Out-Null
}

$PROMPT = 'Reply with exactly: HEADLESS-OK'
try {
    foreach ($xc in @('X1', 'X2')) {
        if ($Case -notcontains $xc) { continue }
        Write-Host "`n=== $xc ===" -ForegroundColor Cyan
        & $qf mode automatic --group $Group --approve | Out-Null
        Api POST '/v1/poll' @{} | Out-Null
        UseExhausted
        $since = Now
        $n = if ($xc -eq 'X1') { 1 } else { 3 }
        $runs = @(1..$n | ForEach-Object { Start-Run "$xc-$_" $PROMPT })
        $runs = @($runs | ForEach-Object { Finish $_ })
        Start-Sleep 2
        $j = @(Journal $since)
        $switches = @($j | Where-Object { $_.event -eq 'transition.written' -and $_.how -eq 'recovery' })
        $dec = @($j | Where-Object event -eq 'recovery.decision')
        foreach ($r in $runs) {
            Check $xc "$($r.Name): exit 0 and Claude's answer" ($r.Process.ExitCode -eq 0 -and -not $r.Json.is_error -and $r.Json.result -match 'HEADLESS-OK') "exit $($r.Process.ExitCode), $($r.Seconds) s"
            CleanOutput $xc $r
        }
        Check $xc 'exactly one switch, by recovery' ($switches.Count -eq 1) "$($switches.Count) switch(es)"
        Check $xc 'every failed request retried at most once' (-not ($dec | Group-Object key | Where-Object Count -gt 1)) "$($dec.Count) decision(s): $(($dec | ForEach-Object { "switched=$($_.switched) retry=$($_.retry)" }) -join '; ')"
        Check $xc "live is no longer $Exhausted" ((Live).label -ne $Exhausted) "live: $((Live).label)"
        if ($xc -eq 'X1') {
            $l = Last $runs[0]
            Check $xc 'quayflow last: recovered, from the exhausted account' ($l.outcome -eq 'recovered' -and $l.retried -and $l.limit.account -eq $Exhausted) ($l | ConvertTo-Json -Compress)
        }
        Restore
    }
    if ($Case -contains 'X3') {
        Write-Host "`n=== X3 ===" -ForegroundColor Cyan
        & $qf mode suggest --group $Group | Out-Null
        UseExhausted
        $since = Now
        $r = Finish (Start-Run 'X3' $PROMPT) 120
        Start-Sleep 2
        $j = @(Journal $since)
        $dec = @($j | Where-Object event -eq 'recovery.decision')
        Check 'X3' 'no hang: the run ended on its own' (-not $r.Hung) "$($r.Seconds) s"
        Check 'X3' 'the limit was confirmed and nothing was switched' ($dec.Count -ge 1 -and -not ($dec | Where-Object switched)) (($dec.why) -join '; ')
        Check 'X3' "Claude's own result stands (an error result)" ($r.Json.is_error -or $r.Process.ExitCode -ne 0) "exit $($r.Process.ExitCode)"
        CleanOutput 'X3' $r
        Check 'X3' "live is still $Exhausted" ((Live).label -eq $Exhausted)
        $l = Last $r
        Check 'X3' 'quayflow last: not recovered, with the reset time to run again' ($l.outcome -eq 'not_recovered' -and $l.availableAt -and $l.limit.pct -ge 99) ($l | ConvertTo-Json -Compress)
        Restore
    }
    if ($Case -contains 'X5') {
        Write-Host "`n=== X5 ===" -ForegroundColor Cyan
        & $qf mode suggest --group $Group | Out-Null
        UseExhausted
        $first = Finish (Start-Run 'X5-first' $PROMPT) 120
        Check 'X5' 'the run fails at the limit with Claude''s own result' ($first.Json.is_error -and $first.Process.ExitCode -ne 0) "exit $($first.Process.ExitCode): $($first.Json.result)"
        $l = Last $first
        Check 'X5' 'quayflow last names a time to run again' ([bool]$l.availableAt) "$($l.outcome), run again at $($l.availableAt)"
        # What the engine does when every account is at its limit: the soonest one goes live.
        $dest = $room[0].label
        & $qf use $dest | Out-Null
        Start-Sleep 2
        $again = Finish (Start-Run 'X5-again' $PROMPT)
        Check 'X5' "a fresh claude -p succeeds on $dest" ($again.Process.ExitCode -eq 0 -and $again.Json.result -match 'HEADLESS-OK') "exit $($again.Process.ExitCode)"
        CleanOutput 'X5' $again
        Restore
    }
    if ($Case -contains 'X4') {
        Write-Host "`n=== X4 ===" -ForegroundColor Cyan
        $on = Finish (Start-Run 'X4-on' $PROMPT)
        & $qf stop | Out-Null
        Start-Sleep 2
        $off = Finish (Start-Run 'X4-off' $PROMPT)
        Check 'X4' 'engine stopped: exit 0 and the answer' ($off.Process.ExitCode -eq 0 -and $off.Json.result -match 'HEADLESS-OK') "exit $($off.Process.ExitCode)"
        CleanOutput 'X4' $off
        Check 'X4' 'engine stopped by you is not restarted by a headless run' (-not (Get-Process quayflow -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$qfHome*" }))
        & $qf start | Out-Null
        Start-Sleep 2
        Check 'X4' 'engine on: exit 0 and the answer' ($on.Process.ExitCode -eq 0 -and $on.Json.result -match 'HEADLESS-OK')
        CleanOutput 'X4' $on
        Write-Host ("  time with Quayflow on: {0} s, off: {1} s (one sample each; Claude's own time varies)" -f $on.Seconds, $off.Seconds)
    }
}
finally {
    try { Restore } catch { Write-Warning "Could not restore: $_. Run: quayflow mode $mode0 --group $Group ; quayflow use $home0 ; quayflow unpin" }
}
$results | Format-Table -AutoSize | Out-String -Width 220 | Write-Output
$failed = @($results | Where-Object Result -eq 'FAIL').Count
Write-Output ("{0} passed, {1} failed. Live login now: {2}. Outputs: {3}" -f ($results.Count - $failed), $failed, (Live).label, $out)
exit $failed
