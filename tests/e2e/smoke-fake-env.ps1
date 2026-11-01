<#
Smoke test of the real quayflow.exe process against a FAKE Claude environment.
Safe to run anywhere: it never touches ~/.claude or real logins (CLAUDE_CONFIG_DIR and
QUAYFLOW_HOME point at a temporary folder; QUAYFLOW_OFFLINE disables network calls).

Covers: single instance (AC-28), start without a window, handshake (AC-42), consent-gated
registration (AC-03), mixed-identity disclosure (AC-35), manual switch with pin (AC-36),
lock held leaves the login unchanged (AC-16), unknown login pauses automation (AC-39),
support report without secrets (AC-30), intentional stop is respected (FR-19).

Usage: pwsh -File tests/e2e/smoke-fake-env.ps1 [-Exe path\to\quayflow.exe]
#>
param([string]$Exe = (Join-Path $PSScriptRoot '..\..\target\debug\quayflow.exe'))
$ErrorActionPreference = 'Stop'
$Exe = (Resolve-Path $Exe).Path
$root = Join-Path ([IO.Path]::GetTempPath()) ("qf-smoke-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
$cfgDir = Join-Path $root 'claude'
$env:CLAUDE_CONFIG_DIR = $cfgDir
$env:QUAYFLOW_HOME = Join-Path $root 'qf'
$env:QUAYFLOW_OFFLINE = '1'
New-Item -ItemType Directory -Force -Path (Join-Path $cfgDir 'sessions') | Out-Null

$results = [System.Collections.Generic.List[object]]::new()
function Check([string]$name, [bool]$ok, [string]$detail = '') {
    $results.Add([pscustomobject]@{ Result = $(if ($ok) { 'PASS' } else { 'FAIL' }); Check = $name; Detail = $detail })
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
function Qf { & $Exe @args 2>&1 | Out-String }

try {
    # A running "session" (this PowerShell process) on a fake Claude version that passes the static check.
    @{ pid = $PID; sessionId = '11111111-2222-4333-8444-555555555555'; cwd = $root; version = '9.9.9'; status = 'idle' } |
        ConvertTo-Json | Set-Content (Join-Path $cfgDir "sessions\$PID.json")
    New-Item -ItemType Directory -Force -Path (Join-Path $env:QUAYFLOW_HOME 'compat') | Out-Null
    '{"version":"9.9.9","checkedAt":0,"staticPass":true}' | Set-Content (Join-Path $env:QUAYFLOW_HOME 'compat\9.9.9.json')
    Set-Login 'uuid-personal' 'P1' 'max'

    $s1 = Qf start | ConvertFrom-Json
    $s2 = Qf start | ConvertFrom-Json
    Check 'engine starts in the background' ($s1.state -eq 'started') $s1.state
    Check 'second start finds the running engine (single instance)' ($s2.state -eq 'running') $s2.state
    $engines = @(Get-Process quayflow -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Exe })
    Check 'exactly one engine process' ($engines.Count -eq 1) "$($engines.Count)"
    Check 'engine has no window' (($engines | Where-Object { $_.MainWindowHandle -ne 0 }).Count -eq 0)

    $conn = Get-Content -Raw (Join-Path $env:QUAYFLOW_HOME 'engine.json') | ConvertFrom-Json
    $h = @{ 'x-quayflow-token' = $conn.token }
    $base = "http://127.0.0.1:$($conn.port)"
    $bad = try { Invoke-RestMethod "$base/v1/state" -Headers @{ 'x-quayflow-token' = 'nope' } ; $false } catch { $true }
    Check 'API refuses a wrong token' $bad
    $hello = Invoke-RestMethod "$base/v1/hello" -Method Post -Headers $h -ContentType 'application/json' -Body '{"protocol":7}'
    Check 'incompatible Mod protocol is refused with update advice' (-not $hello.compatible -and $hello.message -match 'quayflow update')

    $r = Qf register --label personal --group Default
    Check 'registration needs credential-storage consent' ($r -match 'not consented') $r.Trim()
    Qf consent credential-storage on | Out-Null
    Qf consent terms on | Out-Null
    Qf consent engine-start on | Out-Null
    $r = Qf register --label personal --group Default
    Check 'register current login' ($r -match 'Nothing was switched') $r.Trim()
    Set-Login 'uuid-org' 'O1' 'team'
    Start-Sleep -Seconds 3
    $st = Qf status
    Check 'unknown live login pauses automation' ($st -match 'not registered') ''
    $r = Qf register --label org --group Default
    Check 'mixed-identity disclosure required once' ($r -match 'may be sent to any of these accounts') ''
    $r = Qf register --label org --group Default --accept-disclosure
    Check 'register after disclosure' ($r -match 'Nothing was switched') $r.Trim()

    $r = Qf use personal
    Start-Sleep -Seconds 2
    Check 'manual switch writes the destination login' ((Live) -eq 'uuid-personal') (Live)
    $cfg = Get-Content -Raw (Join-Path $cfgDir '.claude.json') | ConvertFrom-Json
    Check 'other keys of .claude.json kept' ($cfg.numStartups -eq 3)
    $st = Qf status
    Check 'manual switch pins' ($st -match 'pinned') ''
    Check 'running session shows Pending next request' ($st -match 'Pending next request') ''

    New-Item -ItemType Directory (Join-Path $cfgDir '.oauth_refresh.lock') | Out-Null
    $r = Qf use org
    Start-Sleep -Seconds 13
    Check 'lock held: login unchanged' ((Live) -eq 'uuid-personal') (Live)
    Remove-Item (Join-Path $cfgDir '.oauth_refresh.lock')

    $rep = Qf report
    $path = ($rep -split "`n" | Where-Object { $_ -match 'Saved for your review: (.+)$' } | ForEach-Object { $Matches[1].Trim() })
    $reportText = Get-Content -Raw $path
    Check 'support report has no secrets' (-not ($reportText -match 'sk-ant')) $path
    $journal = Get-Content -Raw (Join-Path $env:QUAYFLOW_HOME 'journal.jsonl')
    Check 'journal has no secrets' (-not ($journal -match 'sk-ant'))
    $vaultText = (Get-ChildItem (Join-Path $env:QUAYFLOW_HOME 'vault') | Get-Content -Raw) -join ''
    Check 'vault has no plaintext' (-not ($vaultText -match 'sk-ant'))

    Qf stop | Out-Null
    Start-Sleep -Seconds 3
    $s3 = Qf start --if-consented | ConvertFrom-Json
    Check 'intentional stop is respected by on-demand start' ($s3.state -eq 'stopped_by_user') $s3.state
    Check 'engine connection file removed on stop' (-not (Test-Path (Join-Path $env:QUAYFLOW_HOME 'engine.json')))
}
finally {
    try { & $Exe stop 2>&1 | Out-Null } catch {}
    Start-Sleep -Seconds 1
    Remove-Item Env:CLAUDE_CONFIG_DIR, Env:QUAYFLOW_HOME, Env:QUAYFLOW_OFFLINE -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
}
$results | Format-Table -AutoSize | Out-String -Width 200 | Write-Output
$failed = @($results | Where-Object Result -eq 'FAIL').Count
Write-Output ("{0} passed, {1} failed" -f ($results.Count - $failed), $failed)
exit $failed
