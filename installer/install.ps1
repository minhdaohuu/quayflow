<#
.SYNOPSIS
Install, upgrade or remove Quayflow for the current Windows user (no administrator rights).

.DESCRIPTION
Downloads quayflow.exe and quayflow-bg.exe from a GitHub Release of
minhdaohuu/quayflow, verifies them against the release's SHA256SUMS (and the
GitHub build-provenance attestation when the GitHub CLI is available), installs
them to %LOCALAPPDATA%\Quayflow\bin, installs the Quayflow Mod into Claude Code
from the same release tag, and records your consent for on-demand engine start.

Re-running it upgrades. It never reads, prints or changes a Claude login.

    irm https://github.com/minhdaohuu/quayflow/releases/latest/download/install.ps1 | iex

With parameters:

    & ([scriptblock]::Create((irm https://github.com/minhdaohuu/quayflow/releases/latest/download/install.ps1))) -Version v0.1.0 -Startup

.PARAMETER Version
Release tag to install (for example v0.1.0 or 0.1.0). Default: the latest release.
.PARAMETER Source
Local folder holding quayflow.exe, quayflow-bg.exe and SHA256SUMS (testing,
offline or private use). The Mod is then added from the repository containing
that folder, or from -MarketplaceSource.
.PARAMETER MarketplaceSource
Explicit Claude plugin marketplace source (a local repository path, owner/repo or owner/repo#tag).
.PARAMETER NoPath
Do not add %LOCALAPPDATA%\Quayflow\bin to your user PATH.
.PARAMETER Startup
Also start the engine when you sign in to Windows (separate consent).
.PARAMETER NoPlugin
Install the engine only; do not touch Claude Code plugins.
.PARAMETER SkipAttestation
Do not run `gh attestation verify` (for builds you made yourself). Hash verification still applies.
.PARAMETER Yes
Accept the consent text without asking (it is still shown).
.PARAMETER Uninstall
Remove Quayflow (delegates to `quayflow.exe uninstall`) and its PATH entry.
.PARAMETER DeleteData
With -Uninstall: also delete the vault, configuration and logs.
#>
[CmdletBinding()]
param(
    [string]$Version,
    [string]$Source,
    [string]$MarketplaceSource,
    [switch]$NoPath,
    [switch]$Startup,
    [switch]$NoPlugin,
    [switch]$SkipAttestation,
    [switch]$Yes,
    [switch]$Uninstall,
    [switch]$DeleteData
)

function Install-Quayflow {
    [CmdletBinding()]
    param(
        [string]$Version,
        [string]$Source,
        [string]$MarketplaceSource,
        [switch]$NoPath,
        [switch]$Startup,
        [switch]$NoPlugin,
        [switch]$SkipAttestation,
        [switch]$Yes,
        [switch]$Uninstall,
        [switch]$DeleteData
    )

    Set-StrictMode -Version 2.0
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'

    $Repo = 'minhdaohuu/quayflow'
    $Assets = @('quayflow.exe', 'quayflow-bg.exe')
    $DataRoot = Join-Path $env:LOCALAPPDATA 'Quayflow'
    $BinDir = Join-Path $DataRoot 'bin'
    $Exe = Join-Path $BinDir 'quayflow.exe'
    $PocPlugin = 'quayflow@quayflow-local'
    $PocMarketplace = 'quayflow-local'

    function Write-Step([string]$Text) { Write-Host "==> $Text" -ForegroundColor Cyan }
    function Write-Info([string]$Text) { Write-Host "    $Text" }
    function Write-Warn([string]$Text) { Write-Host "    $Text" -ForegroundColor Yellow }

    function Invoke-Native {
        # Runs a native command; returns @{ Code; Out } without throwing on stderr (5.1).
        param([string]$File, [string[]]$Arguments)
        $ErrorActionPreference = 'Continue'
        $out = & $File @Arguments 2>&1 | ForEach-Object { "$_" }
        return @{ Code = $LASTEXITCODE; Out = (@($out) -join "`n").Trim() }
    }

    function Get-Sha256([string]$Path) {
        # .NET directly: Get-FileHash may fail to autoload when PSModulePath is
        # inherited from another PowerShell edition.
        $sha = [Security.Cryptography.SHA256]::Create()
        $stream = [IO.File]::OpenRead($Path)
        try { return ([BitConverter]::ToString($sha.ComputeHash($stream)) -replace '-', '').ToLowerInvariant() }
        finally { $stream.Dispose(); $sha.Dispose() }
    }

    function Get-App([string]$Name) {
        $cmd = Get-Command $Name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($cmd) { return $cmd.Source }
        return $null
    }

    function Get-UserPathRaw {
        $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment')
        try { return [string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) }
        finally { $key.Close() }
    }

    function Set-UserPathRaw([string]$Value) {
        # Keep REG_EXPAND_SZ so entries like %USERPROFILE%\bin keep working.
        $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
        try { $key.SetValue('Path', $Value, [Microsoft.Win32.RegistryValueKind]::ExpandString) }
        finally { $key.Close() }
        # Setting a user variable through .NET broadcasts WM_SETTINGCHANGE so new
        # terminals see the change; the temporary variable is removed again.
        [Environment]::SetEnvironmentVariable('QUAYFLOW_PATH_REFRESH', '1', 'User')
        [Environment]::SetEnvironmentVariable('QUAYFLOW_PATH_REFRESH', $null, 'User')
    }

    function Test-SamePath([string]$A, [string]$B) {
        $x = [Environment]::ExpandEnvironmentVariables($A).TrimEnd('\')
        $y = [Environment]::ExpandEnvironmentVariables($B).TrimEnd('\')
        return [string]::Equals($x, $y, [StringComparison]::OrdinalIgnoreCase)
    }

    function Add-UserPath([string]$Dir) {
        $parts = @((Get-UserPathRaw) -split ';' | Where-Object { $_ -ne '' })
        if (@($parts | Where-Object { Test-SamePath $_ $Dir }).Count -gt 0) { return $false }
        Set-UserPathRaw ((@($parts) + $Dir) -join ';')
        if (@($env:Path -split ';' | Where-Object { $_ -ne '' -and (Test-SamePath $_ $Dir) }).Count -eq 0) {
            $env:Path = $env:Path.TrimEnd(';') + ';' + $Dir
        }
        return $true
    }

    function Remove-UserPath([string]$Dir) {
        $parts = @((Get-UserPathRaw) -split ';' | Where-Object { $_ -ne '' })
        $kept = @($parts | Where-Object { -not (Test-SamePath $_ $Dir) })
        if ($kept.Count -eq $parts.Count) { return $false }
        Set-UserPathRaw ($kept -join ';')
        $env:Path = (@($env:Path -split ';' | Where-Object { $_ -ne '' -and -not (Test-SamePath $_ $Dir) }) -join ';')
        return $true
    }

    function Get-EngineProcess {
        $info = Join-Path $DataRoot 'engine.json'
        if (-not (Test-Path -LiteralPath $info)) { return $null }
        try {
            $enginePid = [int](Get-Content -LiteralPath $info -Raw | ConvertFrom-Json).pid
            $proc = Get-Process -Id $enginePid -ErrorAction Stop
            if ($proc.ProcessName -like 'quayflow*') { return $proc }
        } catch { $null = $_ }
        return $null
    }

    function Get-ClaudeJson([string]$Claude, [string[]]$Arguments) {
        $r = Invoke-Native $Claude $Arguments
        if ($r.Code -ne 0) { return @() }
        try { return @($r.Out | ConvertFrom-Json) } catch { return @() }
    }

    function Remove-PocPlugin([string]$Claude) {
        $plugins = Get-ClaudeJson $Claude @('plugin', 'list', '--json')
        if (@($plugins | Where-Object { $_.id -eq $PocPlugin }).Count -gt 0) {
            $r = Invoke-Native $Claude @('plugin', 'uninstall', $PocPlugin, '--scope', 'user')
            if ($r.Code -eq 0) { Write-Info "Removed the PoC plugin $PocPlugin." } else { Write-Warn "Could not remove $PocPlugin`: $($r.Out)" }
        }
        $markets = Get-ClaudeJson $Claude @('plugin', 'marketplace', 'list', '--json')
        if (@($markets | Where-Object { $_.name -eq $PocMarketplace }).Count -gt 0) {
            $r = Invoke-Native $Claude @('plugin', 'marketplace', 'remove', $PocMarketplace)
            if ($r.Code -eq 0) { Write-Info "Removed the PoC marketplace $PocMarketplace." } else { Write-Warn "Could not remove marketplace $PocMarketplace`: $($r.Out)" }
        }
    }

    # ------------------------------------------------------------------ checks
    if ($PSVersionTable.PSVersion.Major -lt 5 -or ($PSVersionTable.PSVersion.Major -eq 5 -and $PSVersionTable.PSVersion.Minor -lt 1)) {
        throw 'Quayflow needs Windows PowerShell 5.1 or PowerShell 7.'
    }
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { throw 'Quayflow v0.1 runs on Windows only.' }
    if (-not [Environment]::Is64BitOperatingSystem) { throw 'Quayflow needs 64-bit Windows.' }
    if (-not $env:LOCALAPPDATA) { throw 'LOCALAPPDATA is not set; cannot choose an install folder.' }

    $claude = Get-App 'claude'
    $gh = Get-App 'gh'

    # --------------------------------------------------------------- uninstall
    if ($Uninstall) {
        Write-Step 'Removing Quayflow'
        if (Test-Path -LiteralPath $Exe) {
            $cliArgs = @('uninstall')
            if ($DeleteData) { $cliArgs += '--delete-data' }
            if ($Yes) { $cliArgs += '--yes' }
            # Interactive: quayflow.exe asks its own questions in this console.
            & $Exe @cliArgs
            if ($LASTEXITCODE -ne 0) { throw "quayflow.exe uninstall failed (exit $LASTEXITCODE). Nothing else was changed by this script." }
        } else {
            Write-Warn "quayflow.exe not found at $Exe; removing the plugin and PATH entry only."
            if ($claude -and -not $NoPlugin) {
                $null = Invoke-Native $claude @('plugin', 'uninstall', 'quayflow@quayflow', '--scope', 'user')
                $null = Invoke-Native $claude @('plugin', 'marketplace', 'remove', 'quayflow')
            }
        }
        if ($claude -and -not $NoPlugin) { Remove-PocPlugin $claude }
        if (Remove-UserPath $BinDir) { Write-Info "Removed $BinDir from your user PATH." }
        Write-Info 'Your current Claude login was left as it is.'
        if (-not $DeleteData) { Write-Info "Your vault, configuration and logs (if kept) are in $DataRoot." }
        return
    }

    # ----------------------------------------------------------------- consent
    $startupText = '  - Engine start at Windows sign-in: not enabled (re-run with -Startup to enable).'
    if ($Startup) { $startupText = '  - Engine start at Windows sign-in: ENABLED by -Startup (a per-user startup entry; no window).' }
    Write-Host @"

Quayflow - manage approved Claude accounts and switch between them.
Not affiliated with or endorsed by Anthropic.

What this installs, for your Windows user only (no administrator rights):
  - quayflow.exe and quayflow-bg.exe in $BinDir
  - the "quayflow" Mod (a Claude Code plugin, user scope): /quay, the status
    line, usage-limit detection and one retry of a failed request.
  - the Quayflow engine (quayflow.exe), one per Windows user. After you agree
    below, the Mod starts it when a Claude session finds it not running. It
    watches the usage of the accounts you register and switches the login when
    your group's mode allows it. It opens no windows and does not change how
    you start Claude: you keep typing ``claude``.
$startupText

How your logins are handled:
  - Nothing is stored by this installer. When you register an account later
    (in Claude: /quay), Quayflow keeps a reusable copy of that login in a vault
    protected by Windows DPAPI for your user, on this machine only.
  - Quayflow never refreshes tokens; Claude does, and Quayflow saves the result.
    Logins never go into logs, reports or the Mod. There is no telemetry.

What a switch does:
  - A switch replaces the login of a Claude environment (by default
    ~/.claude). EVERY Claude session using that environment moves to the new
    account at its next request; requests in progress finish on the old one.
  - Groups decide which accounts may serve which projects. Putting accounts in
    one group approves sending that work to any of them.

Modes (per group):
  - Suggest (default): Quayflow proposes; you accept with /quay accept.
  - Automatic (opt-in): Quayflow switches by itself within the group's rules.
  - Manual: only switches you ask for.
  Unattended headless jobs (claude -p) only switch in Automatic mode.

Your responsibility:
  You are responsible for complying with your provider's terms and your
  organization's rules, including rules on which accounts may process your
  organization's work. Quayflow is not affiliated with or endorsed by Anthropic.

"@
    if (-not $Yes) {
        $answer = Read-Host 'Install Quayflow and allow the Mod to start the engine on demand? (y/N)'
        if ($answer -notmatch '^(y|yes)$') { Write-Host 'Nothing changed.'; return }
    } else {
        Write-Info 'Consent given with -Yes.'
    }

    # ------------------------------------------------------- resolve the release
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    $ghReady = $false
    if ($gh) { $ghReady = ((Invoke-Native $gh @('auth', 'status', '--hostname', 'github.com')).Code -eq 0) }

    $tag = $null
    if ($Version) {
        $tag = $Version.Trim()
        if ($tag -notmatch '^v') { $tag = "v$tag" }
        if ($tag -notmatch '^v\d+\.\d+\.\d+([-+][0-9A-Za-z.-]+)?$') { throw "Not a release version: $Version" }
    }

    $work = Join-Path ([IO.Path]::GetTempPath()) ('quayflow-install-' + [guid]::NewGuid().ToString('N').Substring(0, 12))
    New-Item -ItemType Directory -Force -Path $work | Out-Null
    try {
        $needed = @($Assets) + 'SHA256SUMS'
        if ($Source) {
            Write-Step "Using local files from $Source"
            $srcFull = (Resolve-Path -LiteralPath $Source).Path
            foreach ($name in $needed) {
                $p = Join-Path $srcFull $name
                if (-not (Test-Path -LiteralPath $p)) { throw "Missing $name in $srcFull." }
                Copy-Item -LiteralPath $p -Destination (Join-Path $work $name)
            }
        } else {
            if (-not $tag) {
                if ($ghReady) {
                    $r = Invoke-Native $gh @('release', 'view', '--repo', $Repo, '--json', 'tagName', '--jq', '.tagName')
                    if ($r.Code -ne 0) { throw "Could not find the latest release of $Repo with gh: $($r.Out)" }
                    $tag = $r.Out.Trim()
                } else {
                    try {
                        $tag = (Invoke-RestMethod -UseBasicParsing -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers @{ 'User-Agent' = 'quayflow-installer' }).tag_name
                    } catch {
                        throw "Could not find the latest release of $Repo ($($_.Exception.Message)). If the repository is private, sign in with 'gh auth login' and run this again, or use -Source."
                    }
                }
            }
            Write-Step "Downloading Quayflow $tag"
            if ($ghReady) {
                $ghArgs = @('release', 'download', $tag, '--repo', $Repo, '--dir', $work, '--clobber')
                foreach ($name in $needed) { $ghArgs += @('--pattern', $name) }
                $r = Invoke-Native $gh $ghArgs
                if ($r.Code -ne 0) { throw "gh release download failed: $($r.Out)" }
            } else {
                foreach ($name in $needed) {
                    $url = "https://github.com/$Repo/releases/download/$tag/$name"
                    try { Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile (Join-Path $work $name) }
                    catch { throw "Download failed: $url ($($_.Exception.Message))" }
                }
            }
        }

        # ------------------------------------------------------------- verify
        Write-Step 'Verifying SHA-256 hashes against SHA256SUMS'
        $sums = @{}
        foreach ($line in Get-Content -LiteralPath (Join-Path $work 'SHA256SUMS')) {
            if ($line -match '^\s*([0-9A-Fa-f]{64})\s+\*?(.+?)\s*$') { $sums[$Matches[2]] = $Matches[1].ToLowerInvariant() }
        }
        $hashes = @{}
        foreach ($name in $Assets) {
            if (-not $sums.ContainsKey($name)) { throw "REFUSED: SHA256SUMS has no entry for $name. Nothing was installed." }
            $actual = Get-Sha256 (Join-Path $work $name)
            if ($actual -ne $sums[$name]) {
                throw "REFUSED: hash mismatch for $name (expected $($sums[$name]), got $actual). Nothing was installed."
            }
            $hashes[$name] = $actual
            Write-Info "$name  $actual  OK"
        }

        Write-Step 'Verifying build provenance (GitHub artifact attestation)'
        $attestation = 'verified'
        if ($SkipAttestation) { $attestation = 'skipped (-SkipAttestation)' }
        elseif (-not $gh) { $attestation = 'skipped (GitHub CLI not installed)' }
        elseif (-not $ghReady) { $attestation = "skipped (GitHub CLI not signed in; run 'gh auth login' to enable)" }
        else {
            foreach ($name in $Assets) {
                $r = Invoke-Native $gh @('attestation', 'verify', (Join-Path $work $name), '--repo', $Repo)
                if ($r.Code -ne 0) { throw "REFUSED: attestation verification failed for $name. Nothing was installed.`n$($r.Out)" }
                Write-Info "$name  attestation OK"
            }
        }
        if ($attestation -ne 'verified') { Write-Warn "Attestation: $attestation" }

        # ------------------------------------------------------------ install
        Write-Step "Installing to $BinDir"
        New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
        Get-ChildItem -LiteralPath $BinDir -Filter '*.old-*' -ErrorAction SilentlyContinue | ForEach-Object {
            Remove-Item -LiteralPath $_.FullName -Force -ErrorAction SilentlyContinue
        }

        $engine = Get-EngineProcess
        $wasRunning = $null -ne $engine
        if ($wasRunning -and (Test-Path -LiteralPath $Exe)) {
            Write-Info "Stopping the running engine (pid $($engine.Id)) for the upgrade."
            $null = Invoke-Native $Exe @('stop')
            if (-not $engine.WaitForExit(20000)) { Write-Warn 'The engine did not stop within 20 s; its files will be replaced by rename.' }
        }

        foreach ($name in $Assets) {
            $target = Join-Path $BinDir $name
            $staged = "$target.new"
            Copy-Item -LiteralPath (Join-Path $work $name) -Destination $staged -Force
            try {
                Move-Item -LiteralPath $staged -Destination $target -Force
            } catch {
                # A running executable cannot be overwritten but can be renamed.
                $old = '{0}.old-{1}' -f $target, [DateTime]::UtcNow.Ticks
                Move-Item -LiteralPath $target -Destination $old -Force
                Move-Item -LiteralPath $staged -Destination $target -Force
            }
            $installed = Get-Sha256 $target
            if ($installed -ne $hashes[$name]) { throw "Installed $name does not match the verified hash." }
        }
        $r = Invoke-Native $Exe @('version')
        if ($r.Code -ne 0) { throw "quayflow.exe does not run: $($r.Out)" }
        Write-Info "Installed: $($r.Out)"

        if (-not $NoPath) {
            if (Add-UserPath $BinDir) { Write-Info "Added $BinDir to your user PATH (new terminals)." }
            else { Write-Info "$BinDir is already on your user PATH." }
        }

        # ---------------------------------------------------------- the Mod
        if ($NoPlugin) {
            Write-Warn 'Skipped the Claude Code plugin (-NoPlugin).'
        } elseif (-not $claude) {
            Write-Warn 'Claude Code (claude) was not found on PATH. Install it, then run this installer again to add the Mod.'
        } else {
            Write-Step 'Installing the Quayflow Mod into Claude Code'
            Remove-PocPlugin $claude
            $market = $MarketplaceSource
            if (-not $market -and $Source) {
                $dir = Get-Item -LiteralPath (Resolve-Path -LiteralPath $Source).Path
                while ($dir) {
                    $manifest = Join-Path $dir.FullName '.claude-plugin\marketplace.json'
                    if ((Test-Path -LiteralPath $manifest) -and ((Get-Content -LiteralPath $manifest -Raw | ConvertFrom-Json).name -eq 'quayflow')) {
                        $market = $dir.FullName; break
                    }
                    $dir = $dir.Parent
                }
                if (-not $market) { throw "No Quayflow marketplace found above $Source; pass -MarketplaceSource <repository folder>." }
            }
            if (-not $market) { $market = "$Repo#$tag" }

            # Re-adding pins the Mod to the same release as the engine (matched pair).
            $markets = Get-ClaudeJson $claude @('plugin', 'marketplace', 'list', '--json')
            if (@($markets | Where-Object { $_.name -eq 'quayflow' }).Count -gt 0) {
                $r = Invoke-Native $claude @('plugin', 'marketplace', 'remove', 'quayflow')
                if ($r.Code -ne 0) { throw "Could not replace the quayflow marketplace: $($r.Out)" }
            }
            $r = Invoke-Native $claude @('plugin', 'marketplace', 'add', $market)
            if ($r.Code -ne 0) {
                throw "claude plugin marketplace add $market failed: $($r.Out)`nFor a private repository, run 'gh auth login' and 'gh auth setup-git' first (or set CLAUDE_CODE_PLUGIN_PREFER_HTTPS=1 without an SSH key)."
            }
            Write-Info "Marketplace: $market"
            $r = Invoke-Native $claude @('plugin', 'install', 'quayflow@quayflow', '--scope', 'user')
            if ($r.Code -ne 0) { throw "claude plugin install quayflow@quayflow failed: $($r.Out)" }
            Write-Info 'Plugin quayflow@quayflow installed (user scope). Running Claude sessions load it after a restart.'
        }

        # ---------------------------------------------------------- consents
        Write-Step 'Recording your consent'
        $r = Invoke-Native $Exe @('consent', 'engine-start', 'on')
        if ($r.Code -ne 0) { throw "quayflow consent engine-start on failed: $($r.Out)" }
        Write-Info 'On-demand engine start: on'
        if ($Startup) {
            $r = Invoke-Native $Exe @('consent', 'signin-startup', 'on')
            if ($r.Code -ne 0) { throw "quayflow consent signin-startup on failed: $($r.Out)" }
            Write-Info 'Engine start at sign-in: on'
        }
        if ($wasRunning) {
            $r = Invoke-Native $Exe @('start', '--if-consented')
            Write-Info "Engine restarted: $($r.Out)"
        }

        Write-Step 'Done'
        $shown = $tag; if (-not $shown) { $shown = '(local files)' }
        Write-Info "Version:     $shown"
        Write-Info "Attestation: $attestation"
        Write-Info 'Next: start Claude as usual (claude) and run /quay to register your login and choose groups.'
        Write-Info 'Optional: quayflow shell install   (same-terminal session replacement in PowerShell)'
        Write-Info 'Uninstall:  quayflow uninstall'
    } finally {
        Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Install-Quayflow @PSBoundParameters
