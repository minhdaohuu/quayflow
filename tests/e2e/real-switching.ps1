<#
Owner-run end-to-end scenarios against REAL Claude logins (product build).

THIS SWITCHES YOUR LIVE CLAUDE LOGIN between two registered accounts and puts it back.
Synthetic prompts only (model haiku). Run it yourself; read it first.

Prerequisites
  - quayflow installed, engine running, two accounts registered in one group
    (quayflow status), Claude Code with the Quayflow plugin installed
    (or pass -PluginDir mod\quayflow to load the repository copy).
  - quayflow consent live-canary on    (lets the engine record per-request identity)

Cases (default: all)
  H   mid-job switch, main agent, sequential tools      AC-13, AC-15, AC-18, AC-25
  S   switch while two parallel subagents run           AC-14, AC-18
  R1  simulated usage limit, Automatic: one switch, one retry, one JSON   AC-21, AC-25
  R2  simulated usage limit, Suggest: no switch in headless             AC-08

Usage: pwsh -File tests/e2e/real-switching.ps1 [-Case H,S,R1,R2] [-PluginDir path] [-Group Default]
#>
param(
    [string[]]$Case = @('H', 'S', 'R1', 'R2'),
    [string]$PluginDir,
    [string]$Group = 'Default'
)
$ErrorActionPreference = 'Stop'
# `pwsh -File script -Case A,B` passes "A,B" as one string: split it into the cases.
$Case = @($Case | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim().ToUpperInvariant() } | Where-Object { $_ })
# The engine's data folder: %LOCALAPPDATA%\Quayflow on Windows, $XDG_DATA_HOME/quayflow
# (~/.local/share/quayflow) on Linux; QUAYFLOW_HOME overrides it.
$qfHome = if ($env:QUAYFLOW_HOME) { $env:QUAYFLOW_HOME }
elseif ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA 'Quayflow' }
elseif ($env:XDG_DATA_HOME) { Join-Path $env:XDG_DATA_HOME 'quayflow' }
else { Join-Path $HOME '.local/share/quayflow' }
$qf = (Get-Command quayflow -ErrorAction SilentlyContinue).Source
if (-not $qf) { $qf = Join-Path (Join-Path $qfHome 'bin') $(if ($IsWindows -or $env:LOCALAPPDATA) { 'quayflow.exe' } else { 'quayflow' }) }
$claude = (Get-Command claude -CommandType Application | Select-Object -First 1).Source

if ($MyInvocation.InvocationName -eq '.') {
    Write-Error 'Run this script with: pwsh -File <script> (dot-sourcing would close your shell when it exits).'
    return
}
$engineFile = Join-Path $qfHome 'engine.json'
$alive = $false
if (Test-Path $engineFile) {
    try {
        $c0 = Get-Content -Raw $engineFile | ConvertFrom-Json
        Invoke-RestMethod "http://127.0.0.1:$($c0.port)/v1/state" -Headers @{ 'x-quayflow-token' = $c0.token } -TimeoutSec 5 | Out-Null
        $alive = $true
    } catch { $alive = $false }
}
if (-not $alive) {
    Write-Host @"
The Quayflow engine is not running for this user ($qfHome).
Set up first (see tests/e2e/README.md):
  1. Install:   pwsh -File installer/install.ps1 -Source dist -MarketplaceSource <repo path>
  2. Register:  quayflow onboard ; quayflow register --label <name> --group Default
  3. Add:       quayflow add --label <name2> --group Default
  4. Consent:   quayflow consent live-canary on
  5. Check:     quayflow status
"@
    exit 2
}
$out = Join-Path $qfHome 'e2e'
New-Item -ItemType Directory -Force $out | Out-Null

function Api([string]$method, [string]$route, $body) {
    $c = Get-Content -Raw (Join-Path $qfHome 'engine.json') | ConvertFrom-Json
    $args = @{ Uri = "http://127.0.0.1:$($c.port)$route"; Method = $method; Headers = @{ 'x-quayflow-token' = $c.token } }
    if ($null -ne $body) { $args.Body = ($body | ConvertTo-Json -Depth 6 -Compress); $args.ContentType = 'application/json' }
    Invoke-RestMethod @args
}
$state = Api GET '/v1/state'
$envd = $state.envs | Where-Object id -eq 'default'
if (-not $envd.live.known) { throw 'the live login must be a registered account' }
$from = $envd.live.account
$fromLabel = $envd.live.label
$other = $state.accounts | Where-Object { $_.id -ne $from -and $_.health -eq 'ok' -and $_.groups -contains $Group } | Select-Object -First 1
if (-not $other) { throw "need a second healthy account in group $Group" }
$label = @{ $from = $fromLabel; $other.id = $other.label }
Write-Host "Accounts: $fromLabel (#$from, live) and $($other.label) (#$($other.id))"
$results = [System.Collections.Generic.List[object]]::new()
function Check([string]$c, [string]$name, [bool]$ok, [string]$detail = '') {
    $results.Add([pscustomobject]@{ Case = $c; Result = $(if ($ok) { 'PASS' } else { 'FAIL' }); Check = $name; Detail = $detail })
}

function Start-Job2([string]$name, [string]$prompt, [string]$tools) {
    $stdout = Join-Path $out "$name.json"; $stderr = Join-Path $out "$name.err"
    $a = @('-p', "`"$prompt`"", '--output-format', 'json', '--model', 'haiku', '--allowedTools', "`"$tools`"")
    if ($PluginDir) { $a += @('--plugin-dir', "`"$((Resolve-Path $PluginDir).Path)`"") }
    $p = Start-Process -FilePath $claude -NoNewWindow -PassThru -RedirectStandardOutput $stdout -RedirectStandardError $stderr -ArgumentList $a
    [pscustomobject]@{ Process = $p; Stdout = $stdout }
}
function Restore { & $qf use $fromLabel | Out-Null; Start-Sleep 2; & $qf unpin | Out-Null }
function OneJson($job, [string]$expect) {
    try { $r = Get-Content -Raw $job.Stdout | ConvertFrom-Json; return ($r.result -match $expect) } catch { return $false }
}

function Invoke-SwitchCase([string]$c, [string]$prompt, [string]$tools, [scriptblock]$when) {
    Write-Host "`n=== $c ===" -ForegroundColor Cyan
    Api POST '/v1/canary/start' @{} | Out-Null
    $job = Start-Job2 $c $prompt $tools
    $swapped = 0
    while (-not $job.Process.HasExited) {
        $steps = (Api GET '/v1/canary').steps
        if (-not $swapped -and (& $when $steps)) {
            $r = Api POST '/v1/canary/switch' @{ to = $other.id }
            if ($r.ok) {
                # The write time: requests are classified by when they STARTED (one in flight at
                # the write finishes on the previous account by design).
                $swapped = if ($r.transition.writtenAt) { [int64]$r.transition.writtenAt } else { [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() }
                Write-Host "switched to $($other.label)"
            }
        }
        Start-Sleep -Milliseconds 300
    }
    $job.Process.WaitForExit(); Start-Sleep 2
    $run = Api POST '/v1/canary/stop' @{}
    $ev = @($run.steps | Where-Object { $_.account })
    function Started($s) { if ($s.startedAt) { [int64]$s.startedAt } else { [int64]$s.ts } }
    $before = @($ev | Where-Object { (Started $_) -lt $swapped }); $after = @($ev | Where-Object { (Started $_) -ge $swapped })
    foreach ($s in $ev) { Write-Host ("  {0} {1}" -f $(if ((Started $s) -ge $swapped) { 'after ' } else { 'before' }), $label["$($s.account)"]) }
    Check $c 'swap happened mid-job' ($swapped -gt 0)
    Check $c 'requests before the swap used the first account' ($before.Count -gt 0 -and -not ($before | Where-Object account -ne $from)) "$($before.Count) request(s)"
    Check $c 'requests after the swap used the second account' ($after.Count -gt 0 -and -not ($after | Where-Object account -ne $other.id)) "$($after.Count) request(s)"
    Check $c 'same process, exit 0, exactly one JSON result' ($job.Process.ExitCode -eq 0 -and (OneJson $job '.')) "exit $($job.Process.ExitCode)"
    Restore
}

try {
    if ($Case -contains 'H') {
        Invoke-SwitchCase 'H' 'Run each of these Bash commands in its own separate tool call, one after another, never in parallel: echo h1 ; sleep 10 && echo h2 ; sleep 10 && echo h3 ; echo h4 . Then reply with exactly DONE.' 'Bash' `
        { param($s) @($s | Where-Object { $_.stopReason -eq 'tool_use' -and -not $_.agent }).Count -ge 2 }
    }
    if ($Case -contains 'S') {
        Invoke-SwitchCase 'S' 'Use the Agent tool to launch two general-purpose subagents in parallel, in one message. Subagent A must run the Bash command: sleep 20 && echo SUB-A , then reply with its output. Subagent B must run: sleep 20 && echo SUB-B , then reply with its output. When both are done, reply with exactly BOTH-DONE.' 'Bash Agent Task' `
        { param($s) @($s | Where-Object { $_.agent -and $_.stopReason -eq 'tool_use' } | Select-Object -ExpandProperty agent -Unique).Count -ge 2 }
    }
    foreach ($rc in @('R1', 'R2')) {
        if ($Case -notcontains $rc) { continue }
        Write-Host "`n=== $rc ===" -ForegroundColor Cyan
        $mode = if ($rc -eq 'R1') { 'automatic' } else { 'suggest' }
        $prev = ($state.groups | Where-Object name -eq $Group).mode
        & $qf mode $mode --group $Group --approve | Out-Null
        # Fresh observation of the other account so recovery has a known destination.
        Api POST '/v1/poll' @{} | Out-Null
        '{"atStep":1}' | Set-Content (Join-Path $qfHome 'simulate-limit.json')
        $since = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        # The Mod's simulated-limit hook is off unless the Claude process has QUAYFLOW_SIM_LIMIT=1.
        $env:QUAYFLOW_SIM_LIMIT = '1'
        try { $job = Start-Job2 $rc 'Run the Bash command: echo r1 . Then run the Bash command: echo r2 . Then reply with exactly RECOVERED.' 'Bash' }
        finally { Remove-Item Env:QUAYFLOW_SIM_LIMIT -ErrorAction SilentlyContinue }
        $job.Process.WaitForExit(); Start-Sleep 2
        $j = Get-Content (Join-Path $qfHome 'journal.jsonl') | ForEach-Object { $_ | ConvertFrom-Json } | Where-Object { $_.ts -ge $since }
        $dec = @($j | Where-Object event -eq 'recovery.decision')
        $live = (Api GET '/v1/state').envs | Where-Object id -eq 'default'
        if ($rc -eq 'R1') {
            Check $rc 'exactly one recovery decision, with a switch and a retry' ($dec.Count -eq 1 -and $dec[0].switched -and $dec[0].retry) ($dec | ConvertTo-Json -Compress)
            Check $rc 'job finished with one JSON result' ($job.Process.ExitCode -eq 0 -and (OneJson $job 'RECOVERED'))
            Check $rc 'the other account is live' ($live.live.account -eq $other.id)
        } else {
            Check $rc 'Suggest mode: no switch in headless' ($dec.Count -ge 1 -and -not ($dec | Where-Object switched)) ($dec.why -join '; ')
            Check $rc 'live account unchanged' ($live.live.account -eq $from)
        }
        Remove-Item (Join-Path $qfHome 'simulate-limit.json') -ErrorAction SilentlyContinue
        & $qf mode $prev --group $Group | Out-Null
        Restore
    }
}
finally {
    try { Api POST '/v1/canary/stop' @{} | Out-Null } catch {}
    Remove-Item (Join-Path $qfHome 'simulate-limit.json') -ErrorAction SilentlyContinue
}
$results | Format-Table -AutoSize | Out-String -Width 220 | Write-Output
$failed = @($results | Where-Object Result -eq 'FAIL').Count
Write-Output ("{0} passed, {1} failed. Live login now: {2}" -f ($results.Count - $failed), $failed, (((Api GET '/v1/state').envs | Where-Object id -eq 'default').live.label))
exit $failed
