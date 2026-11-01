<#
Self-test for shell/Quayflow.psm1 (no Pester needed).

    pwsh -NoProfile -File shell/test-Quayflow.ps1                 # runs in pwsh AND powershell.exe
    pwsh -NoProfile -File shell/test-Quayflow.ps1 -CurrentHostOnly

Everything happens in a temporary folder: LOCALAPPDATA and USERPROFILE are
redirected for the test process, `claude` and `quayflow.exe` are fake .cmd
scripts that log what they received. Nothing touches real Claude data.
Exit code 0 when every test passes.
#>
[CmdletBinding()]
param([switch]$CurrentHostOnly)

$ErrorActionPreference = 'Stop'
$ModulePath = Join-Path $PSScriptRoot 'Quayflow.psm1'

if (-not $CurrentHostOnly) {
    $hosts = @()
    foreach ($candidate in 'pwsh', 'powershell') {
        $cmd = Get-Command $candidate -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($cmd) { $hosts += $cmd.Source } else { Write-Host "SKIP host $candidate (not installed)" -ForegroundColor Yellow }
    }
    $failed = 0
    foreach ($exe in $hosts) {
        Write-Host "=== $exe ===" -ForegroundColor Cyan
        & $exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $PSCommandPath -CurrentHostOnly
        if ($LASTEXITCODE -ne 0) { $failed++ }
    }
    if ($failed -gt 0) { Write-Host "$failed host(s) failed" -ForegroundColor Red; exit 1 }
    Write-Host 'All hosts passed.' -ForegroundColor Green
    exit 0
}

# ---------------------------------------------------------------- fixtures
$Work = Join-Path ([System.IO.Path]::GetTempPath()) ("qf-shell-test-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Force $Work | Out-Null
$RealUserProfile = $env:USERPROFILE
$RealLocalAppData = $env:LOCALAPPDATA
$env:LOCALAPPDATA = Join-Path $Work 'localappdata'
$env:USERPROFILE = Join-Path $Work 'profile'
$DataRoot = Join-Path $env:LOCALAPPDATA 'Quayflow'
$ClaimsDir = Join-Path $DataRoot 'handoff\claims'
$ClaimPath = Join-Path $ClaimsDir "$PID.json"
$LaneDir = Join-Path $DataRoot 'lanes\work'
$DefaultDir = Join-Path $env:USERPROFILE '.claude'
$ProjectDir = Join-Path $Work 'project dir'
$Log = Join-Path $Work 'claude.log'
$QfLog = Join-Path $Work 'quayflow.log'
$NextClaim = Join-Path $Work 'next-claim.json'
New-Item -ItemType Directory -Force $ClaimsDir, $LaneDir, $DefaultDir, $ProjectDir | Out-Null

$env:QF_TEST_LOG = $Log
$env:QF_TEST_QFLOG = $QfLog
$env:QF_TEST_NEXT = $NextClaim
$env:QF_TEST_CLAIM = $ClaimPath
$env:QF_TEST_FAIL = ''

$FakeClaude = Join-Path $Work 'bin\claude.cmd'
New-Item -ItemType Directory -Force (Split-Path $FakeClaude) | Out-Null
@'
@echo off
>>"%QF_TEST_LOG%" echo args=%*
if defined CLAUDE_CONFIG_DIR (>>"%QF_TEST_LOG%" echo cfg=%CLAUDE_CONFIG_DIR%) else (>>"%QF_TEST_LOG%" echo cfg=UNSET)
>>"%QF_TEST_LOG%" echo cwd=%CD%
if exist "%QF_TEST_NEXT%" move /y "%QF_TEST_NEXT%" "%QF_TEST_CLAIM%" >nul
exit /b 0
'@ | Set-Content -Path $FakeClaude -Encoding ASCII

$FakeQuayflow = Join-Path $Work 'bin\quayflow.cmd'
@'
@echo off
>>"%QF_TEST_QFLOG%" echo args=%*
if "%QF_TEST_FAIL%"=="1" (echo engine unavailable 1>&2 & exit /b 3)
if not "%1 %2 %3"=="shell claim --pid" exit /b 2
set CLAIM=%LOCALAPPDATA%\Quayflow\handoff\claims\%4.json
if not exist "%CLAIM%" exit /b 0
type "%CLAIM%"
del "%CLAIM%"
exit /b 0
'@ | Set-Content -Path $FakeQuayflow -Encoding ASCII

function New-Claim {
    param([string]$SessionId = '11111111-2222-3333-4444-555555555555', $ConfigDir = $LaneDir,
        [int]$ExpiresInSec = 600, [string]$Path = $ClaimPath, [switch]$NoHistory, [switch]$EpochMs)
    $now = [DateTimeOffset]::UtcNow
    $created = $now.ToString("yyyy-MM-ddTHH:mm:ssZ")
    $expires = $now.AddSeconds($ExpiresInSec).ToString("yyyy-MM-ddTHH:mm:ssZ")
    if ($EpochMs) { $created = $now.ToUnixTimeMilliseconds(); $expires = $now.AddSeconds($ExpiresInSec).ToUnixTimeMilliseconds() }
    $obj = [ordered]@{ sessionId = $SessionId; configDir = $ConfigDir; cwd = $ProjectDir; createdAt = $created; expiresAt = $expires }
    if ($EpochMs) { $obj.sourcePid = 4242 }   # engine format: epoch ms plus fields the hook must ignore
    $json = $obj | ConvertTo-Json -Compress
    [System.IO.File]::WriteAllText($Path, $json, (New-Object System.Text.UTF8Encoding($false)))
    if (-not $NoHistory) {
        $base = $ConfigDir; if (-not $base) { $base = $DefaultDir }
        $proj = Join-Path $base 'projects\C--project-dir'
        New-Item -ItemType Directory -Force $proj | Out-Null
        Set-Content -Path (Join-Path $proj "$SessionId.jsonl") -Value '{}' -Encoding ASCII
    }
}

function Reset-State {
    foreach ($p in $Log, $QfLog, $NextClaim, $ClaimPath) { if (Test-Path $p) { Remove-Item $p -Force } }
    foreach ($d in (Join-Path $LaneDir 'projects'), (Join-Path $DefaultDir 'projects')) { if (Test-Path $d) { Remove-Item $d -Recurse -Force } }
    Get-ChildItem $ClaimsDir -Force | Remove-Item -Force
    $env:QF_TEST_FAIL = ''
}

function Get-LaunchLog { if (Test-Path $Log) { @(Get-Content $Log) } else { @() } }

$script:Pass = 0
$script:Fail = 0
function Assert {
    param([bool]$Condition, [string]$Name, [string]$Detail = '')
    if ($Condition) { $script:Pass++; Write-Host "  PASS $Name" -ForegroundColor Green }
    else { $script:Fail++; Write-Host "  FAIL $Name $Detail" -ForegroundColor Red }
}

function Invoke-Prompt { (prompt) 6>$null }

Write-Host "PowerShell $($PSVersionTable.PSVersion) ($($PSVersionTable.PSEdition))"

try {
    function global:prompt { "MYPROMPT[$?|$global:LASTEXITCODE]> " }
    $originalText = (Get-Item function:prompt).ScriptBlock.ToString()

    Import-Module $ModulePath -Force -DisableNameChecking
    $exported = (Get-Module Quayflow).ExportedFunctions.Keys | Sort-Object
    Assert (($exported -join ',') -eq 'Disable-QuayflowPromptHook,Enable-QuayflowPromptHook') 'exports exactly the two functions' ($exported -join ',')

    # Hidden test knobs: shorter history wait.
    & (Get-Module Quayflow) { $script:QfHistoryWaitMs = 600 }

    Enable-QuayflowPromptHook -ClaudePath $FakeClaude -QuayflowPath ''
    Enable-QuayflowPromptHook   # idempotent: must not wrap itself
    $hookText = (Get-Item function:prompt).ScriptBlock.ToString()
    Assert ($hookText -ne $originalText) 'prompt is wrapped'
    Assert ((& (Get-Module Quayflow) { $script:QfOriginalPrompt.ToString() }) -eq $originalText) 'second Enable keeps the user prompt as original'

    Write-Host 'T1 user prompt text, $? and $LASTEXITCODE preserved'
    Reset-State
    cmd.exe /c exit 0
    $out = Invoke-Prompt
    Assert ($out -eq 'MYPROMPT[True|0]> ') 'prompt output after a successful native command' $out
    cmd.exe /c exit 7
    $out = Invoke-Prompt
    Assert ($out -eq 'MYPROMPT[False|7]> ') 'prompt output after exit code 7' $out
    Get-Item (Join-Path $Work 'does-not-exist') -ErrorAction SilentlyContinue
    $out = Invoke-Prompt
    Assert ($out -like 'MYPROMPT`[False|*') '$? false is passed through' $out
    Assert ((Get-LaunchLog).Count -eq 0) 'no claim: claude not started'

    Write-Host 'T2 lane claim consumed directly (no quayflow.exe)'
    Reset-State
    New-Claim
    $env:CLAUDE_CONFIG_DIR = 'C:\previous-config'
    $out = Invoke-Prompt
    $lines = Get-LaunchLog
    Assert ($lines -contains 'args=--resume 11111111-2222-3333-4444-555555555555') 'claude --resume <id>' ($lines -join ' | ')
    Assert ($lines -contains "cfg=$LaneDir") 'CLAUDE_CONFIG_DIR set to the lane' ($lines -join ' | ')
    Assert ($lines -contains "cwd=$ProjectDir") 'started in the claim cwd' ($lines -join ' | ')
    Assert ($env:CLAUDE_CONFIG_DIR -eq 'C:\previous-config') 'previous CLAUDE_CONFIG_DIR restored' "$env:CLAUDE_CONFIG_DIR"
    Assert (-not (Test-Path $ClaimPath)) 'claim file consumed'
    Assert (@(Get-ChildItem $ClaimsDir -Force).Count -eq 0) 'no leftover .claimed file'
    Assert ($out -like 'MYPROMPT*') 'prompt text still returned'

    Write-Host 'T3 default-environment claim (configDir null), epoch-ms times, unknown field sourcePid'
    Reset-State
    New-Claim -ConfigDir $null -EpochMs
    $env:CLAUDE_CONFIG_DIR = 'C:\lane-shell'
    Invoke-Prompt | Out-Null
    $lines = Get-LaunchLog
    Assert ($lines -contains 'cfg=UNSET') 'CLAUDE_CONFIG_DIR removed for the default environment' ($lines -join ' | ')
    Assert ($env:CLAUDE_CONFIG_DIR -eq 'C:\lane-shell') 'shell value restored'
    Remove-Item Env:CLAUDE_CONFIG_DIR
    Reset-State
    New-Claim
    Invoke-Prompt | Out-Null
    Assert (-not (Test-Path Env:CLAUDE_CONFIG_DIR)) 'unset variable stays unset after a lane launch'

    Write-Host 'T4 expired claim ignored'
    Reset-State
    New-Claim -ExpiresInSec -5
    Invoke-Prompt | Out-Null
    Assert ((Get-LaunchLog).Count -eq 0) 'expired: claude not started'
    Assert (-not (Test-Path $ClaimPath)) 'expired claim removed'

    Write-Host 'T5 history not ready: nothing started'
    Reset-State
    New-Claim -NoHistory
    Invoke-Prompt | Out-Null
    Assert ((Get-LaunchLog).Count -eq 0) 'missing transcript: claude not started'
    Assert (-not (Test-Path $ClaimPath)) 'claim consumed (manual command shown)'

    Write-Host 'T6 malformed and hostile claims ignored'
    Reset-State
    New-Claim -SessionId 'x; Remove-Item C:\ -Recurse' -NoHistory
    Invoke-Prompt | Out-Null
    Assert ((Get-LaunchLog).Count -eq 0) 'bad sessionId rejected'
    Reset-State
    [System.IO.File]::WriteAllText($ClaimPath, 'not json')
    Invoke-Prompt | Out-Null
    Assert ((Get-LaunchLog).Count -eq 0) 'non-JSON rejected'
    Reset-State
    New-Claim -ConfigDir 'relative\dir' -NoHistory
    Invoke-Prompt | Out-Null
    Assert ((Get-LaunchLog).Count -eq 0) 'relative configDir rejected'

    Write-Host 'T7 chained replacement (resumed session requests another)'
    Reset-State
    New-Claim -SessionId 'aaaaaaaa-0000-0000-0000-000000000001'
    New-Claim -SessionId 'aaaaaaaa-0000-0000-0000-000000000002' -ConfigDir $null -Path $NextClaim
    Invoke-Prompt | Out-Null
    $lines = Get-LaunchLog
    $resumes = @($lines | Where-Object { $_ -like 'args=--resume*' })
    Assert ($resumes.Count -eq 2) 'two launches in one prompt' ($lines -join ' | ')
    Assert ($resumes[1] -eq 'args=--resume aaaaaaaa-0000-0000-0000-000000000002') 'second launch is the new session'

    Write-Host 'T8 claim consumed through quayflow.exe shell claim --pid'
    Enable-QuayflowPromptHook -QuayflowPath $FakeQuayflow
    Reset-State
    New-Claim
    Invoke-Prompt | Out-Null
    $qf = if (Test-Path $QfLog) { @(Get-Content $QfLog) } else { @() }
    Assert ($qf -contains "args=shell claim --pid $PID") 'CLI called with --pid' ($qf -join ' | ')
    Assert ((Get-LaunchLog) -contains 'args=--resume 11111111-2222-3333-4444-555555555555') 'claude launched from CLI output'

    Write-Host 'T9 CLI failure: no launch, claim kept, not retried on the next prompt'
    Reset-State
    New-Claim
    $env:QF_TEST_FAIL = '1'
    Invoke-Prompt | Out-Null
    $calls = @(Get-Content $QfLog).Count
    Assert ($calls -eq 3) 'three attempts' "$calls"
    Assert ((Get-LaunchLog).Count -eq 0) 'claude not started'
    Assert (Test-Path $ClaimPath) 'claim left in place'
    Invoke-Prompt | Out-Null
    Assert (@(Get-Content $QfLog).Count -eq 3) 'same claim not retried'
    $env:QF_TEST_FAIL = ''
    Start-Sleep -Milliseconds 50
    New-Claim   # rewritten claim (new timestamp) is tried again
    (Get-Item $ClaimPath).LastWriteTimeUtc = [DateTime]::UtcNow.AddSeconds(5)
    Invoke-Prompt | Out-Null
    Assert ((Get-LaunchLog).Count -gt 0) 'a rewritten claim is processed'

    Write-Host 'T10 claude resolved from PATH'
    Enable-QuayflowPromptHook -QuayflowPath '' -ClaudePath ''
    $savedPath = $env:Path
    $env:Path = (Split-Path $FakeClaude) + ';' + $env:Path
    Reset-State
    New-Claim
    Invoke-Prompt | Out-Null
    $env:Path = $savedPath
    Assert ((Get-LaunchLog).Count -gt 0) 'claude.cmd found on PATH and started'

    Write-Host 'T11 disable and module removal restore the user prompt'
    Disable-QuayflowPromptHook
    Assert ((Get-Item function:prompt).ScriptBlock.ToString() -eq $originalText) 'Disable restores the prompt'
    Reset-State
    New-Claim
    Invoke-Prompt | Out-Null
    Assert ((Get-LaunchLog).Count -eq 0) 'disabled hook does nothing'
    Enable-QuayflowPromptHook
    Remove-Module Quayflow
    Assert ((Get-Item function:prompt).ScriptBlock.ToString() -eq $originalText) 'Remove-Module restores the prompt'

    Write-Host 'T12 a prompt wrapped after ours is not clobbered by Disable'
    Import-Module $ModulePath -Force -DisableNameChecking
    Enable-QuayflowPromptHook -ClaudePath $FakeClaude
    $ours = $function:global:prompt
    function global:prompt { 'OTHER> ' }
    Disable-QuayflowPromptHook 3>$null
    Assert ((prompt) -eq 'OTHER> ') 'later wrapper kept'
    Set-Item function:global:prompt $ours
    Remove-Module Quayflow
} catch {
    $script:Fail++
    Write-Host "  ERROR $($_ | Out-String)" -ForegroundColor Red
} finally {
    $env:USERPROFILE = $RealUserProfile
    $env:LOCALAPPDATA = $RealLocalAppData
    Remove-Item Env:CLAUDE_CONFIG_DIR -ErrorAction SilentlyContinue
    Set-Location $PSScriptRoot
    Remove-Item $Work -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host ("{0} passed, {1} failed" -f $script:Pass, $script:Fail)
if ($script:Fail -gt 0) { exit 1 }
exit 0
