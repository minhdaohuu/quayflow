<#
Quayflow PowerShell integration: same-terminal Replace Current Session.

Opt-in. Works in PowerShell 7 and Windows PowerShell 5.1. It does not redefine
`claude` or change command resolution; it only wraps the `prompt` function.

-------------------------------------------------------------------------------
INSTALLATION CONTRACT (quayflow.exe embeds this file; keep both sides in sync)
-------------------------------------------------------------------------------
`quayflow shell install` writes this file to
    %LOCALAPPDATA%\Quayflow\shell\Quayflow.psm1
and appends the block below to each CurrentUserCurrentHost profile:
    <Documents>\PowerShell\Microsoft.PowerShell_profile.ps1          (PowerShell 7)
    <Documents>\WindowsPowerShell\Microsoft.PowerShell_profile.ps1   (Windows PowerShell 5.1)
where <Documents> is the known folder FOLDERID_Documents (often redirected to
OneDrive). The block is ASCII, CRLF line endings, exactly these five lines:

# >>> quayflow >>>
# Managed by `quayflow shell install`; removed by `quayflow shell uninstall`. Do not edit inside this block.
$QuayflowModule = Join-Path $env:LOCALAPPDATA 'Quayflow\shell\Quayflow.psm1'
if (Test-Path -LiteralPath $QuayflowModule) { Import-Module $QuayflowModule -DisableNameChecking; Enable-QuayflowPromptHook }
Remove-Variable QuayflowModule -ErrorAction SilentlyContinue
# <<< quayflow <<<

Install is idempotent: if a line equal to `# >>> quayflow >>>` exists, the
existing block is replaced. `quayflow shell uninstall` removes every line from
`# >>> quayflow >>>` through `# <<< quayflow <<<` inclusive and deletes the
.psm1. Windows PowerShell 5.1 only loads profiles when the execution policy
allows local scripts (RemoteSigned for CurrentUser is enough).

-------------------------------------------------------------------------------
CLAIM CONTRACT
-------------------------------------------------------------------------------
The engine (POST /v1/handoff/replace) writes, atomically (tmp + rename),
    %LOCALAPPDATA%\Quayflow\handoff\claims\<shell-pid>.json
UTF-8 JSON:
    {"sessionId":"<uuid>","configDir":"<abs path>"|null,"cwd":"<abs path>",
     "createdAt":"<RFC 3339 UTC>","expiresAt":"<RFC 3339 UTC>"}
configDir null means the default environment (CLAUDE_CONFIG_DIR removed).
Times may also be Unix epoch milliseconds.

On each prompt the hook checks whether claims\$PID.json exists (nothing else
runs when it does not). If it exists:
  1. Consume it: `quayflow.exe shell claim --pid <PID>` (exit 0 + one JSON line =
     claim, exit 0 + empty = none, non-zero = error with the claim left in place;
     retried 3 times). Without quayflow.exe the file is consumed directly:
     rename to a unique name, read, delete.
  2. Ignore and report it if expired.
  3. Wait up to 10 s for <configDir>\projects\*\<sessionId>.jsonl (history
     ready); otherwise print the manual command and do not launch.
  4. Run `claude --resume <sessionId>` in this terminal with
     Start-Process -NoNewWindow -Wait, in the claim's cwd, with CLAUDE_CONFIG_DIR
     set (lane) or removed (default), then restore the previous value.
  5. Repeat while a new claim for this shell appears (a resumed session may
     request another replacement), at most 10 times per prompt.
#>

Set-StrictMode -Version 2.0

# Same data folder rule as the engine: QUAYFLOW_HOME (tests) or %LOCALAPPDATA%\Quayflow.
if ($env:QUAYFLOW_HOME) { $script:QfRoot = $env:QUAYFLOW_HOME }
else { $script:QfRoot = Join-Path $env:LOCALAPPDATA 'Quayflow' }
$script:QfOriginalPrompt = $null
$script:QfHookPrompt = $null
$script:QfQuayflowExe = $null
$script:QfClaudeExe = $null
$script:QfSkipClaimStamp = $null
$script:QfHistoryWaitMs = 10000
$script:QfMaxChain = 10
$script:QfMarker = 'QuayflowPromptHook-7c1f'

function Test-QfPromptIsHook {
    param($FunctionInfo)
    return ($null -ne $FunctionInfo -and $FunctionInfo.ScriptBlock.ToString().Contains($script:QfMarker))
}

function Write-QfNote {
    param([string]$Message, [ConsoleColor]$Color = [ConsoleColor]::Cyan)
    Write-Host "Quayflow: $Message" -ForegroundColor $Color
}

function Get-QfClaimPath {
    Join-Path (Join-Path $script:QfRoot 'handoff\claims') ("{0}.json" -f $PID)
}

function Get-QfQuayflowExe {
    if ($script:QfQuayflowExe) { return $script:QfQuayflowExe }
    $exe = Join-Path $script:QfRoot 'bin\quayflow.exe'
    if ([System.IO.File]::Exists($exe)) { return $exe }
    return $null
}

function ConvertTo-QfUtc {
    # Accepts an RFC 3339 string, a DateTime (PowerShell 7's ConvertFrom-Json
    # converts ISO strings), or Unix epoch milliseconds. Returns $null if invalid.
    param($Value)
    if ($null -eq $Value) { return $null }
    if ($Value -is [datetime]) {
        if ($Value.Kind -eq [DateTimeKind]::Unspecified) { return [datetime]::SpecifyKind($Value, [DateTimeKind]::Utc) }
        return $Value.ToUniversalTime()
    }
    if ($Value -is [datetimeoffset]) { return $Value.UtcDateTime }
    if ($Value -is [int] -or $Value -is [long] -or $Value -is [double] -or $Value -is [decimal]) {
        return [DateTimeOffset]::FromUnixTimeMilliseconds([long]$Value).UtcDateTime
    }
    $parsed = [DateTimeOffset]::MinValue
    $styles = [System.Globalization.DateTimeStyles]::AssumeUniversal
    if ([DateTimeOffset]::TryParse([string]$Value, [System.Globalization.CultureInfo]::InvariantCulture, $styles, [ref]$parsed)) {
        return $parsed.UtcDateTime
    }
    return $null
}

function ConvertFrom-QfClaimJson {
    # Returns a validated claim object or $null.
    param([string]$Json)
    if ([string]::IsNullOrWhiteSpace($Json)) { return $null }
    try { $raw = $Json | ConvertFrom-Json -ErrorAction Stop } catch { return $null }
    if ($null -eq $raw) { return $null }
    $names = @($raw.PSObject.Properties | ForEach-Object { $_.Name })
    foreach ($required in 'sessionId', 'cwd', 'expiresAt') {
        if ($names -notcontains $required) { return $null }
    }
    $sessionId = [string]$raw.sessionId
    if ($sessionId -notmatch '^[0-9A-Za-z-]{8,64}$') { return $null }
    $configDir = $null
    if ($names -contains 'configDir' -and $null -ne $raw.configDir -and [string]$raw.configDir -ne '') {
        $configDir = [string]$raw.configDir
        if (-not [System.IO.Path]::IsPathRooted($configDir)) { return $null }
    }
    $createdAt = $null
    if ($names -contains 'createdAt') { $createdAt = ConvertTo-QfUtc $raw.createdAt }
    $expiresAt = ConvertTo-QfUtc $raw.expiresAt
    if ($null -eq $expiresAt) { return $null }
    [pscustomobject]@{
        SessionId = $sessionId
        ConfigDir = $configDir
        Cwd       = [string]$raw.cwd
        CreatedAt = $createdAt
        ExpiresAt = $expiresAt
        Stamp     = $Json.Trim()
    }
}

function Receive-QfClaimDirect {
    # Consume the claim file without quayflow.exe: rename (atomic, so a second
    # reader cannot also get it), read, delete.
    param([string]$Path)
    $taken = '{0}.{1}.claimed' -f $Path, ([guid]::NewGuid().ToString('N'))
    try { [System.IO.File]::Move($Path, $taken) } catch { return $null }
    try {
        return [System.IO.File]::ReadAllText($taken, [System.Text.Encoding]::UTF8)
    } catch {
        return $null
    } finally {
        try { [System.IO.File]::Delete($taken) } catch { $null = $_ }
    }
}

function Receive-QfClaim {
    # Returns the claim JSON text, '' when there is none, or $null on error.
    param([string]$Path)
    $exe = Get-QfQuayflowExe
    if (-not $exe) { return (Receive-QfClaimDirect -Path $Path) }
    # Windows PowerShell 5.1 turns redirected stderr into error records, which a
    # user's $ErrorActionPreference = 'Stop' would make terminating.
    $ErrorActionPreference = 'Continue'
    $lastError = ''
    for ($try = 1; $try -le 3; $try++) {
        # quayflow.exe writes UTF-8; PowerShell decodes native output with
        # [Console]::OutputEncoding (the OEM code page by default), which would
        # garble non-ASCII paths. Decode as UTF-8 for this call only.
        $savedEncoding = $null
        try { $savedEncoding = [Console]::OutputEncoding; [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false) } catch { $savedEncoding = $null }
        try {
            $output = & $exe shell claim --pid $PID 2>&1
            $code = $LASTEXITCODE
        } finally {
            if ($null -ne $savedEncoding) { try { [Console]::OutputEncoding = $savedEncoding } catch { $null = $_ } }
        }
        $stdout = @($output | Where-Object { $_ -isnot [System.Management.Automation.ErrorRecord] }) -join "`n"
        if ($code -eq 0) { return $stdout.Trim() }
        $lastError = (@($output | ForEach-Object { "$_" }) -join ' ').Trim()
        Start-Sleep -Milliseconds (500 * $try)
    }
    Write-QfNote "could not read the pending replacement (quayflow shell claim failed: $lastError)." Red
    return $null
}

function Get-QfClaudeExe {
    if ($script:QfClaudeExe) { return $script:QfClaudeExe }
    $cmd = Get-Command claude -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($cmd) { return $cmd.Source }
    return $null
}

function Test-QfHistoryReady {
    param($Claim)
    $base = $Claim.ConfigDir
    if (-not $base) { $base = Join-Path $env:USERPROFILE '.claude' }
    $projects = Join-Path $base 'projects'
    $name = '{0}.jsonl' -f $Claim.SessionId
    $deadline = [DateTime]::UtcNow.AddMilliseconds($script:QfHistoryWaitMs)
    do {
        if ([System.IO.Directory]::Exists($projects)) {
            foreach ($dir in [System.IO.Directory]::GetDirectories($projects)) {
                if ([System.IO.File]::Exists((Join-Path $dir $name))) { return $true }
            }
        }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    return $false
}

function Write-QfManualCommand {
    param($Claim)
    if ($Claim.ConfigDir) {
        Write-QfNote ("to continue by hand: `$env:CLAUDE_CONFIG_DIR = '{0}'; Set-Location '{1}'; claude --resume {2}" -f $Claim.ConfigDir, $Claim.Cwd, $Claim.SessionId) Yellow
    } else {
        Write-QfNote ("to continue by hand: Remove-Item Env:CLAUDE_CONFIG_DIR -ErrorAction SilentlyContinue; Set-Location '{0}'; claude --resume {1}" -f $Claim.Cwd, $Claim.SessionId) Yellow
    }
}

function Start-QfResume {
    param($Claim)
    $exe = Get-QfClaudeExe
    if (-not $exe) {
        Write-QfNote 'the claude command was not found on PATH; the session was not resumed.' Red
        Write-QfManualCommand $Claim
        return
    }
    if (-not [System.IO.Directory]::Exists($Claim.Cwd)) {
        Write-QfNote "the session's folder no longer exists: $($Claim.Cwd)" Red
        Write-QfManualCommand $Claim
        return
    }
    if (-not (Test-QfHistoryReady $Claim)) {
        Write-QfNote "the history of session $($Claim.SessionId) is not ready in the destination; nothing was started." Red
        Write-QfManualCommand $Claim
        return
    }

    $target = 'the default environment'
    if ($Claim.ConfigDir) { $target = $Claim.ConfigDir }
    Write-QfNote "resuming session $($Claim.SessionId) in $target..."

    $hadPrevious = Test-Path Env:CLAUDE_CONFIG_DIR
    $previous = $env:CLAUDE_CONFIG_DIR
    try {
        if ($Claim.ConfigDir) { $env:CLAUDE_CONFIG_DIR = $Claim.ConfigDir }
        else { Remove-Item Env:CLAUDE_CONFIG_DIR -ErrorAction SilentlyContinue }
        # Inside `prompt` PowerShell captures a native command's output, so the
        # TUI would see a redirected stdout and quit. Start-Process -NoNewWindow
        # gives Claude this console's real handles (validated in PoC-A).
        $proc = Start-Process -FilePath $exe -ArgumentList @('--resume', $Claim.SessionId) `
            -WorkingDirectory $Claim.Cwd -NoNewWindow -Wait -PassThru
        if ($proc -and $proc.ExitCode -ne 0) {
            Write-QfNote "claude exited with code $($proc.ExitCode)." Yellow
        }
    } finally {
        # The destination applies to the resumed session only.
        if ($hadPrevious) { $env:CLAUDE_CONFIG_DIR = $previous }
        else { Remove-Item Env:CLAUDE_CONFIG_DIR -ErrorAction SilentlyContinue }
    }
}

function Invoke-QfPendingClaim {
    $path = Get-QfClaimPath
    for ($n = 0; $n -lt $script:QfMaxChain; $n++) {
        if (-not [System.IO.File]::Exists($path)) { return }
        $json = Receive-QfClaim -Path $path
        if ($null -eq $json) {
            # Error already shown; skip this claim until it changes.
            try { $script:QfSkipClaimStamp = [System.IO.File]::GetLastWriteTimeUtc($path) } catch { $null = $_ }
            return
        }
        if ($json -eq '') { return }
        $claim = ConvertFrom-QfClaimJson $json
        if ($null -eq $claim) {
            Write-QfNote 'ignored an unreadable replacement request.' Yellow
            continue
        }
        if ($claim.ExpiresAt -le [DateTime]::UtcNow) {
            Write-QfNote "ignored an expired replacement request for session $($claim.SessionId)." Yellow
            Write-QfManualCommand $claim
            continue
        }
        Start-QfResume $claim
    }
}

function Invoke-QfPromptHook {
    $path = Get-QfClaimPath
    if (-not [System.IO.File]::Exists($path)) { return }
    if ($null -ne $script:QfSkipClaimStamp) {
        try {
            if ([System.IO.File]::GetLastWriteTimeUtc($path) -eq $script:QfSkipClaimStamp) { return }
        } catch { return }
        $script:QfSkipClaimStamp = $null
    }
    try { Invoke-QfPendingClaim } catch { Write-QfNote "$_" Red }
}

function Enable-QuayflowPromptHook {
    <#
    .SYNOPSIS
    Wrap this shell's prompt so a pending Quayflow replacement resumes in this terminal.
    .DESCRIPTION
    Idempotent. The existing prompt function keeps producing the prompt text.
    QuayflowPath and ClaudePath override the executables (used by tests).
    #>
    [CmdletBinding()]
    param([string]$QuayflowPath, [string]$ClaudePath, [string]$DataRoot)
    if ($PSBoundParameters.ContainsKey('QuayflowPath')) { $script:QfQuayflowExe = $QuayflowPath }
    if ($PSBoundParameters.ContainsKey('ClaudePath')) { $script:QfClaudeExe = $ClaudePath }
    if ($PSBoundParameters.ContainsKey('DataRoot')) { $script:QfRoot = $DataRoot }

    $current = Get-Item function:prompt -ErrorAction SilentlyContinue
    if (Test-QfPromptIsHook $current) { return }

    if ($current) { $script:QfOriginalPrompt = $current.ScriptBlock }
    else { $script:QfOriginalPrompt = { "PS $($executionContext.SessionState.Path.CurrentLocation)$('>' * ($nestedPromptLevel + 1)) " } }

    # Defined inside the module, so it can call module functions; installed globally.
    $script:QfHookPrompt = {
        # QuayflowPromptHook-7c1f (marker used to recognise this wrapper)
        # Capture before any statement runs, so the user's prompt still sees them.
        $qfLastSuccess = $global:?
        $qfLastExit = $global:LASTEXITCODE
        Invoke-QfPromptHook
        $global:LASTEXITCODE = $qfLastExit
        if (-not $qfLastSuccess) { Write-Error -Message 'quayflow' -ErrorAction Ignore }
        & $script:QfOriginalPrompt
    }
    Set-Item -Path function:global:prompt -Value $script:QfHookPrompt
}

function Disable-QuayflowPromptHook {
    <#
    .SYNOPSIS
    Restore the prompt function that was active before Enable-QuayflowPromptHook.
    #>
    [CmdletBinding()]
    param()
    $current = Get-Item function:prompt -ErrorAction SilentlyContinue
    if (-not $script:QfHookPrompt) { return }
    if (-not (Test-QfPromptIsHook $current)) {
        # Something wrapped the prompt after us; restoring would remove it.
        Write-Warning 'Quayflow: the prompt was changed after the Quayflow hook was enabled; it was left as it is. Start a new shell without the hook to remove it.'
        return
    }
    if ($script:QfOriginalPrompt) { Set-Item -Path function:global:prompt -Value $script:QfOriginalPrompt }
    $script:QfHookPrompt = $null
}

$ExecutionContext.SessionState.Module.OnRemove = {
    $current = Get-Item function:prompt -ErrorAction SilentlyContinue
    if ((Test-QfPromptIsHook $current) -and $script:QfOriginalPrompt) {
        Set-Item -Path function:global:prompt -Value $script:QfOriginalPrompt
    }
}

Export-ModuleMember -Function Enable-QuayflowPromptHook, Disable-QuayflowPromptHook
