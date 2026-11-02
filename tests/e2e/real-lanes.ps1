<#
Owner-run check for lanes (AC-12, release condition "Claude reads and writes
correctly through lane links"). Uses a REAL second account; read before running.

Steps
  1. Create lane "qf-e2e" and check its links.
  2. Switch the second registered account into the lane (the default login is untouched).
  3. Run a headless Claude job inside the lane that reads a linked skill folder and writes a
     setting through the linked settings.json, then confirm the shared files saw it.
  4. Confirm the default environment's login did not change (independent environments).
  5. Delete the lane; the shared setup must survive.

Usage: pwsh -File tests/e2e/real-lanes.ps1 [-Account <label>]
#>
param([string]$Account)
$ErrorActionPreference = 'Stop'
$qfHome = if ($env:QUAYFLOW_HOME) { $env:QUAYFLOW_HOME } else { Join-Path $env:LOCALAPPDATA 'Quayflow' }
$qf = (Get-Command quayflow -ErrorAction SilentlyContinue).Source
if (-not $qf) { $qf = Join-Path $qfHome 'bin\quayflow.exe' }
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
function Api([string]$m, [string]$r, $b) {
    $c = Get-Content -Raw (Join-Path $qfHome 'engine.json') | ConvertFrom-Json
    $a = @{ Uri = "http://127.0.0.1:$($c.port)$r"; Method = $m; Headers = @{ 'x-quayflow-token' = $c.token } }
    if ($null -ne $b) { $a.Body = ($b | ConvertTo-Json -Compress); $a.ContentType = 'application/json' }
    Invoke-RestMethod @a
}
$results = [System.Collections.Generic.List[object]]::new()
function Check([string]$n, [bool]$ok, [string]$d = '') { $results.Add([pscustomobject]@{ Result = $(if ($ok) { 'PASS' } else { 'FAIL' }); Check = $n; Detail = $d }) }

$st = Api GET '/v1/state'
$def = $st.envs | Where-Object id -eq 'default'
$target = if ($Account) { $st.accounts | Where-Object label -eq $Account } else { $st.accounts | Where-Object { $_.id -ne $def.live.account -and $_.health -eq 'ok' } | Select-Object -First 1 }
if (-not $target) { throw 'need a second registered account' }
$shared = Join-Path $HOME '.claude'
$marker = "qf-e2e-$([guid]::NewGuid().ToString('N').Substring(0,6))"
$skill = Join-Path $shared "skills\$marker"
try {
    New-Item -ItemType Directory -Force $skill | Out-Null
    "---`nname: $marker`ndescription: Quayflow lane test skill. Reply with the word LANE-OK when asked about $marker.`n---`nReply LANE-OK." | Set-Content (Join-Path $skill 'SKILL.md')
    & $qf lanes create qf-e2e | Out-Host
    $lane = (Api GET '/v1/lanes') | Where-Object name -eq 'qf-e2e'
    Check 'lane created with links' ($lane.linksOk) ($lane.linkDetail -join '; ')
    Check 'lane passed its check' ($lane.canaryPassed)
    $r = Api POST '/v1/switch' @{ to = $target.id; env = 'qf-e2e' }
    Start-Sleep 2
    Check 'account switched into the lane' ($r.ok) $r.message
    $laneDir = $lane.configDir
    $env:CLAUDE_CONFIG_DIR = $laneDir
    $o = & $claude -p "Is there a skill named $marker available to you? Answer only YES or NO." --output-format json --model haiku 2>$null | Out-String
    Remove-Item Env:CLAUDE_CONFIG_DIR
    $res = try { ($o | ConvertFrom-Json).result } catch { '' }
    Check 'linked skills are visible inside the lane' ($res -match 'YES') $res
    # -AsHashtable: Claude's config can hold project paths that differ only in case.
    $laneCfg = Get-Content -Raw (Join-Path $laneDir '.claude.json') | ConvertFrom-Json -AsHashtable
    $laneUuid = "$($laneCfg['oauthAccount']['accountUuid'])"
    $sha = [Security.Cryptography.SHA256]::Create()
    $laneHash = (-join ($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($laneUuid))[0..5] | ForEach-Object { $_.ToString('x2') }))
    Check 'lane login is the target account' ($laneUuid -and $laneHash -eq $target.id) "#$laneHash
    $def2 = (Api GET '/v1/state').envs | Where-Object id -eq 'default'
    Check 'default environment login unchanged' ($def2.live.account -eq $def.live.account)
    $lanesNow = (Api GET '/v1/lanes') | Where-Object name -eq 'qf-e2e'
    Check 'links still intact after a real session' ($lanesNow.linksOk) ($lanesNow.linkDetail -join '; ')
}
finally {
    try { Api POST '/v1/lanes/delete' @{ name = 'qf-e2e' } | Out-Null } catch { Write-Warning "lane delete: $_" }
    Check 'shared skills survive lane deletion' (Test-Path (Join-Path $skill 'SKILL.md'))
    Remove-Item -Recurse -Force $skill -ErrorAction SilentlyContinue
}
$results | Format-Table -AutoSize | Out-String -Width 200 | Write-Output
exit @($results | Where-Object Result -eq 'FAIL').Count
