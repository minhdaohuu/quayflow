<#
.SYNOPSIS
Install, upgrade or remove Quayflow for the current Windows user (no administrator rights).

.DESCRIPTION
Downloads quayflow.exe and quayflow-bg.exe from a GitHub Release of
minhdaohuu/quayflow, verifies them against the release's SHA256SUMS and the
GitHub build-provenance attestation of the release workflow (this needs the
GitHub CLI, signed in; -SkipAttestation skips that check explicitly), installs
them to %LOCALAPPDATA%\Quayflow\bin, installs the Quayflow Mod into Claude Code
from the same release tag (refused if the tag no longer points at the commit
the release was built from), and records your consent for on-demand engine start.

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
Do not run `gh attestation verify` (builds you made yourself, releases of a private
repository, or no GitHub CLI). Without it the installer refuses files whose
attestation it cannot verify. Hash verification still applies.
.PARAMETER Yes
Accept the consent text without asking (it is still shown).
.PARAMETER Update
Used by `quayflow update`: upgrade an existing install without the consent text,
without touching PATH and without changing any consent. A Mod installed from a
local folder (a development checkout) is left as it is.
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
    [switch]$Update,
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
        [switch]$Update,
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
    # An update keeps every earlier choice: no consent prompt, no PATH change (see below for the Mod).
    if ($Update) { $Yes = $true; $NoPath = $true }
    $PocPlugin = 'quayflow@quayflow-local'
    $PocMarketplace = 'quayflow-local'

    function Write-Step([string]$Text) { Write-Host "==> $Text" -ForegroundColor Cyan }
    function Write-Info([string]$Text) { Write-Host "    $Text" }
    function Write-Warn([string]$Text) { Write-Host "    $Text" -ForegroundColor Yellow }

    function Invoke-Native {
        # Runs a native command without throwing on stderr. Returns @{ Code; Out; Err; All }:
        # Out is stdout only (what callers parse), All is both streams (for messages).
        # PowerShell 5.1 and 7 both turn redirected stderr lines into ErrorRecords,
        # which is how the two streams are told apart here.
        param([string]$File, [string[]]$Arguments)
        $ErrorActionPreference = 'Continue'
        $out = New-Object System.Collections.Generic.List[string]
        $err = New-Object System.Collections.Generic.List[string]
        $all = New-Object System.Collections.Generic.List[string]
        & $File @Arguments 2>&1 | ForEach-Object {
            $line = "$_"
            if ($_ -is [System.Management.Automation.ErrorRecord]) { $err.Add($line) } else { $out.Add($line) }
            $all.Add($line)
        }
        return @{ Code = $LASTEXITCODE; Out = ($out -join "`n").Trim(); Err = ($err -join "`n").Trim(); All = ($all -join "`n").Trim() }
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

    function Start-Engine {
        # Runs `quayflow.exe start` and returns the state from its JSON line
        # (started, running, failed, ...), or a short description when there is none.
        $r = Invoke-Native $Exe @('start')
        $state = $null
        foreach ($line in @($r.Out -split "`n")) {
            $t = $line.Trim()
            if ($t.StartsWith('{')) {
                try { $state = [string]($t | ConvertFrom-Json).state } catch { $null = $_ }
            }
        }
        if ($state) { return $state }
        return "unknown, exit $($r.Code): $($r.All)"
    }

    function Get-ClaudeJson([string]$Claude, [string[]]$Arguments) {
        $r = Invoke-Native $Claude $Arguments
        if ($r.Code -ne 0) { return @() }
        # Piped through ForEach-Object: Windows PowerShell 5.1's ConvertFrom-Json emits a JSON
        # array as ONE object (an empty array would otherwise become a single bogus item).
        try { return @($r.Out | ConvertFrom-Json | ForEach-Object { $_ }) } catch { return @() }
    }

    function Remove-PocPlugin([string]$Claude) {
        $plugins = Get-ClaudeJson $Claude @('plugin', 'list', '--json')
        if (@($plugins | Where-Object { $_.id -eq $PocPlugin }).Count -gt 0) {
            $r = Invoke-Native $Claude @('plugin', 'uninstall', $PocPlugin, '--scope', 'user')
            if ($r.Code -eq 0) { Write-Info "Removed the PoC plugin $PocPlugin." } else { Write-Warn "Could not remove $PocPlugin`: $($r.All)" }
        }
        $markets = Get-ClaudeJson $Claude @('plugin', 'marketplace', 'list', '--json')
        if (@($markets | Where-Object { $_.name -eq $PocMarketplace }).Count -gt 0) {
            $r = Invoke-Native $Claude @('plugin', 'marketplace', 'remove', $PocMarketplace)
            if ($r.Code -eq 0) { Write-Info "Removed the PoC marketplace $PocMarketplace." } else { Write-Warn "Could not remove marketplace $PocMarketplace`: $($r.All)" }
        }
    }

    function Get-MarketplaceSource($Entry) {
        # The source `claude plugin marketplace add` accepts, rebuilt from a `list --json` entry.
        $names = @($Entry.PSObject.Properties.Name)
        $ref = $null
        if ($names -contains 'ref' -and $Entry.ref) { $ref = [string]$Entry.ref }
        foreach ($field in 'repo', 'url') {
            if ($names -contains $field -and $Entry.$field) {
                if ($ref) { return "$($Entry.$field)#$ref" }
                return [string]$Entry.$field
            }
        }
        if ($names -contains 'path' -and $Entry.path) { return [string]$Entry.path }
        return $null
    }

    function Install-Mod([string]$Claude, [string]$Market, [string]$ExpectedCommit) {
        # Replaces the "quayflow" marketplace with $Market and installs the plugin from it.
        # A marketplace name cannot be re-added from another source while it exists, so the
        # old one is removed first; if the new one cannot be added (or is refused), the old
        # one is put back, with the plugin when it was installed. Throws on failure.
        $markets = Get-ClaudeJson $Claude @('plugin', 'marketplace', 'list', '--json')
        $old = @($markets | Where-Object { $_.name -eq 'quayflow' }) | Select-Object -First 1
        $oldSource = $null
        if ($old) { $oldSource = Get-MarketplaceSource $old }
        $hadPlugin = @(Get-ClaudeJson $Claude @('plugin', 'list', '--json') | Where-Object { $_.id -eq 'quayflow@quayflow' }).Count -gt 0

        $restore = {
            param([string]$Why)
            if (-not $oldSource) { return $Why }
            $rr = Invoke-Native $Claude @('plugin', 'marketplace', 'add', $oldSource)
            if ($rr.Code -ne 0) { return "$Why`nThe previous Mod source ($oldSource) could not be restored: $($rr.All)" }
            if ($hadPlugin) {
                $rr = Invoke-Native $Claude @('plugin', 'install', 'quayflow@quayflow', '--scope', 'user')
                if ($rr.Code -ne 0) { return "$Why`nThe previous marketplace was restored, but the plugin was not: $($rr.All)" }
            }
            return "$Why`nThe previous Mod ($oldSource) was restored."
        }

        if ($old) {
            $r = Invoke-Native $Claude @('plugin', 'marketplace', 'remove', 'quayflow')
            if ($r.Code -ne 0) { throw "Could not replace the quayflow marketplace: $($r.All)" }
        }
        $r = Invoke-Native $Claude @('plugin', 'marketplace', 'add', $Market)
        if ($r.Code -ne 0) {
            throw (& $restore "claude plugin marketplace add $Market failed: $($r.All)`nFor a private repository, run 'gh auth login' and 'gh auth setup-git' first (or set CLAUDE_CODE_PLUGIN_PREFER_HTTPS=1 without an SSH key).")
        }

        if ($ExpectedCommit -and -not (Test-Path -LiteralPath $Market -PathType Container)) {
            # Tags can be moved: the Mod must come from the commit the release was built from.
            $entry = @(Get-ClaudeJson $Claude @('plugin', 'marketplace', 'list', '--json') | Where-Object { $_.name -eq 'quayflow' }) | Select-Object -First 1
            $head = $null
            $git = Get-App 'git'
            if ($entry -and $git -and (@($entry.PSObject.Properties.Name) -contains 'installLocation') -and $entry.installLocation) {
                $g = Invoke-Native $git @('-C', [string]$entry.installLocation, 'rev-parse', 'HEAD')
                if ($g.Code -eq 0) { $head = $g.Out.Trim().ToLowerInvariant() }
            }
            if ($head -ne $ExpectedCommit) {
                $null = Invoke-Native $Claude @('plugin', 'marketplace', 'remove', 'quayflow')
                $seen = $head; if (-not $seen) { $seen = 'unknown (could not read the checkout)' }
                throw (& $restore "REFUSED: the Mod source $Market is at commit $seen, but this release was built from $ExpectedCommit (the tag was moved, or the checkout could not be read). The Mod was not installed.")
            }
            Write-Info "Mod source verified: commit $ExpectedCommit"
        }

        $r = Invoke-Native $Claude @('plugin', 'install', 'quayflow@quayflow', '--scope', 'user')
        if ($r.Code -ne 0) { throw "claude plugin install quayflow@quayflow failed: $($r.All)" }
    }

    # ------------------------------------------------------------------ checks
    if ($PSVersionTable.PSVersion.Major -lt 5 -or ($PSVersionTable.PSVersion.Major -eq 5 -and $PSVersionTable.PSVersion.Minor -lt 1)) {
        throw 'Quayflow needs Windows PowerShell 5.1 or PowerShell 7.'
    }
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { throw 'This installer is for Windows; Quayflow does not support this platform yet.' }
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
    if ($Update) {
        Write-Step 'Updating Quayflow (your settings and consents are kept)'
    } else {
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
  - Claude refreshes the live login and Quayflow saves the result. Quayflow
    refreshes only the saved copy of an idle account, so its usage stays readable.
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
    }

    # ------------------------------------------------------- resolve the release
    # The requested version is checked first: a bad value is refused before anything else.
    $tag = $null
    if ($Version) {
        $tag = $Version.Trim()
        if ($tag -notmatch '^v') { $tag = "v$tag" }
        if ($tag -notmatch '^v\d+\.\d+\.\d+([-+][0-9A-Za-z.-]+)?$') { throw "Not a release version: $Version" }
    }

    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    $ghReady = $false
    if ($gh) { $ghReady = ((Invoke-Native $gh @('auth', 'status', '--hostname', 'github.com')).Code -eq 0) }
    # Build provenance is required unless skipped explicitly: refuse before downloading anything.
    if (-not $SkipAttestation -and -not $ghReady) {
        $why = 'the GitHub CLI (gh) is not installed'
        if ($gh) { $why = "the GitHub CLI is not signed in (run 'gh auth login')" }
        throw ("REFUSED: the build-provenance attestation cannot be verified because $why. Nothing was installed.`n" +
            "Install and sign in to the GitHub CLI and run this again, or accept the SHA-256 check alone by adding -SkipAttestation:`n" +
            "    & ([scriptblock]::Create((irm https://github.com/$Repo/releases/latest/download/install.ps1))) -SkipAttestation")
    }

    $work = Join-Path ([IO.Path]::GetTempPath()) ('quayflow-install-' + [guid]::NewGuid().ToString('N').Substring(0, 12))
    New-Item -ItemType Directory -Force -Path $work | Out-Null
    $stoppedForUpgrade = $false
    $restarted = $false
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
            $p = Join-Path $srcFull 'SOURCE_COMMIT'
            if (Test-Path -LiteralPath $p) { Copy-Item -LiteralPath $p -Destination (Join-Path $work 'SOURCE_COMMIT') }
        } else {
            if (-not $tag) {
                if ($ghReady) {
                    $r = Invoke-Native $gh @('release', 'view', '--repo', $Repo, '--json', 'tagName', '--jq', '.tagName')
                    if ($r.Code -ne 0) { throw "Could not find the latest release of $Repo with gh: $($r.All)" }
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
                if ($r.Code -ne 0) { throw "gh release download failed: $($r.All)" }
                # Older releases have no SOURCE_COMMIT; its absence is handled below.
                $null = Invoke-Native $gh @('release', 'download', $tag, '--repo', $Repo, '--dir', $work, '--clobber', '--pattern', 'SOURCE_COMMIT')
            } else {
                foreach ($name in $needed) {
                    $url = "https://github.com/$Repo/releases/download/$tag/$name"
                    try { Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile (Join-Path $work $name) }
                    catch { throw "Download failed: $url ($($_.Exception.Message))" }
                }
                try { Invoke-WebRequest -UseBasicParsing -Uri "https://github.com/$Repo/releases/download/$tag/SOURCE_COMMIT" -OutFile (Join-Path $work 'SOURCE_COMMIT') }
                catch { Remove-Item -LiteralPath (Join-Path $work 'SOURCE_COMMIT') -Force -ErrorAction SilentlyContinue }
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

        # The commit the release was built from (absent from older releases).
        $sourceCommit = $null
        $commitFile = Join-Path $work 'SOURCE_COMMIT'
        if (Test-Path -LiteralPath $commitFile) {
            if (-not $sums.ContainsKey('SOURCE_COMMIT')) { throw 'REFUSED: SHA256SUMS has no entry for SOURCE_COMMIT. Nothing was installed.' }
            $actual = Get-Sha256 $commitFile
            if ($actual -ne $sums['SOURCE_COMMIT']) { throw "REFUSED: hash mismatch for SOURCE_COMMIT (expected $($sums['SOURCE_COMMIT']), got $actual). Nothing was installed." }
            $sourceCommit = ([IO.File]::ReadAllText($commitFile)).Trim().ToLowerInvariant()
            if ($sourceCommit -notmatch '^([0-9a-f]{40}|[0-9a-f]{64})$') { throw "REFUSED: SOURCE_COMMIT is not a commit id. Nothing was installed." }
            Write-Info "SOURCE_COMMIT  $sourceCommit  OK"
        }

        Write-Step 'Verifying build provenance (GitHub artifact attestation)'
        $attestation = 'verified'
        if ($SkipAttestation) { $attestation = 'skipped (-SkipAttestation)' }
        else {
            # Only an attestation made by this repository's release workflow counts, for this
            # tag and commit when they are known.
            $verifyArgs = @('--repo', $Repo, '--signer-workflow', "$Repo/.github/workflows/release.yml")
            if ($tag) { $verifyArgs += @('--source-ref', "refs/tags/$tag") }
            if ($sourceCommit) { $verifyArgs += @('--source-digest', $sourceCommit) }
            $attested = @($Assets)
            if ($sourceCommit) { $attested += 'SOURCE_COMMIT' }
            foreach ($name in $attested) {
                $r = Invoke-Native $gh (@('attestation', 'verify', (Join-Path $work $name)) + $verifyArgs)
                if ($r.Code -ne 0) { throw "REFUSED: attestation verification failed for $name. Nothing was installed.`n$($r.All)" }
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
            # `stop` also writes the "stopped by user" marker; the engine is restarted below
            # with a plain `start`, which clears it (or by the finally block if this fails).
            Write-Info "Stopping the running engine (pid $($engine.Id)) for the upgrade."
            $stoppedForUpgrade = $true
            $null = Invoke-Native $Exe @('stop')
            if (-not $engine.WaitForExit(20000)) {
                # Asked to stop and still running: end it, or the old version would keep serving
                # (a later `start` would just report it as running).
                Write-Warn "The engine did not stop within 20 s; ending it (pid $($engine.Id))."
                try { $engine.Kill() } catch { $null = $_ }
                if (-not $engine.WaitForExit(5000)) { Write-Warn 'The old engine could not be ended; its files will be replaced by rename.' }
            }
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
        if ($r.Code -ne 0) { throw "quayflow.exe does not run: $($r.All)" }
        Write-Info "Installed: $($r.Out)"
        # The shell module is written by quayflow.exe; an installed copy follows the new program.
        if (Test-Path -LiteralPath (Join-Path $DataRoot 'shell\Quayflow.psm1')) {
            $r = Invoke-Native $Exe @('shell', 'refresh')
            if ($r.Code -ne 0) { Write-Warn "The PowerShell prompt hook module was not refreshed: $($r.All)" }
            else { Write-Info 'PowerShell prompt hook module refreshed.' }
        }

        if (-not $NoPath) {
            if (Add-UserPath $BinDir) { Write-Info "Added $BinDir to your user PATH (new terminals)." }
            else { Write-Info "$BinDir is already on your user PATH." }
        }

        # ---------------------------------------------------------- the Mod
        # A Mod failure does not undo the engine install: the consents and the engine restart
        # below still happen (the consent was given above, and a restored previous Mod uses
        # it too), and the installer reports the failure and exits with an error at the end.
        $modError = $null
        if ($NoPlugin) {
            Write-Warn 'Skipped the Claude Code plugin (-NoPlugin).'
        } elseif (-not $claude) {
            Write-Warn 'Claude Code (claude) was not found on PATH. Install it, then run this installer again to add the Mod.'
        } else {
            Write-Step 'Installing the Quayflow Mod into Claude Code'
            try {
                Remove-PocPlugin $claude
                $localMod = $null
                if ($Update -and -not $MarketplaceSource) {
                    # A Mod installed from a local folder is a development checkout: leave it alone.
                    $entry = @(Get-ClaudeJson $claude @('plugin', 'marketplace', 'list', '--json') | Where-Object { $_.name -eq 'quayflow' }) | Select-Object -First 1
                    $src = $null
                    if ($entry) { $src = Get-MarketplaceSource $entry }
                    if ($src -and (Test-Path -LiteralPath $src -PathType Container)) { $localMod = $src }
                }
                $market = $MarketplaceSource
                if ($localMod) {
                    $market = $null
                    Write-Info "The Mod is installed from $localMod and was not changed; it may not match the new engine."
                } elseif (-not $market -and $Source) {
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
                if (-not $market -and -not $localMod) {
                    $market = "$Repo#$tag"
                    if (-not $sourceCommit) {
                        Write-Warn "This release does not record its source commit (SOURCE_COMMIT), so the Mod from tag $tag cannot be checked against the commit the engine was built from."
                    }
                }
                if ($market) {
                    # Re-adding pins the Mod to the same release as the engine (matched pair).
                    Install-Mod $claude $market $sourceCommit
                    Write-Info "Marketplace: $market"
                    Write-Info 'Plugin quayflow@quayflow installed (user scope). Running Claude sessions load it after a restart.'
                }
            } catch {
                $modError = $_.Exception.Message
                Write-Warn "The Mod was not installed: $modError"
            }
        }

        # ---------------------------------------------------------- consents
        if (-not $Update) {
            Write-Step 'Recording your consent'
            $r = Invoke-Native $Exe @('consent', 'engine-start', 'on')
            if ($r.Code -ne 0) { throw "quayflow consent engine-start on failed: $($r.All)" }
            Write-Info 'On-demand engine start: on'
        }
        if ($Startup -and -not $Update) {
            $r = Invoke-Native $Exe @('consent', 'signin-startup', 'on')
            if ($r.Code -ne 0) { throw "quayflow consent signin-startup on failed: $($r.All)" }
            Write-Info 'Engine start at sign-in: on'
        }
        if ($wasRunning) {
            # Plain `start`: it clears the "stopped by user" marker the upgrade's `stop` wrote
            # (`start --if-consented` would honor that marker and leave the engine off).
            $state = Start-Engine
            $restarted = $true
            $now = Get-EngineProcess
            if ($now -and -not $engine.HasExited -and $now.Id -eq $engine.Id) {
                # `start` found the OLD engine still running: it is not the new version. Its
                # "stopped by user" marker would block later on-demand starts, so it goes.
                Remove-Item -LiteralPath (Join-Path $DataRoot 'engine.stopped') -Force -ErrorAction SilentlyContinue
                Write-Warn "The previous engine (pid $($engine.Id)) is still running the old version. Run 'quayflow stop' and then 'quayflow start', or sign out of Windows and back in."
            } elseif ($state -eq 'started' -or $state -eq 'running') { Write-Info "Engine restarted ($state)." }
            else { Write-Warn "The engine did not restart ($state). Run 'quayflow start' (log: $(Join-Path $DataRoot 'logs'))." }
        }

        $shown = $tag; if (-not $shown) { $shown = '(local files)' }
        if ($modError) { Write-Step 'Done, except the Mod' } else { Write-Step 'Done' }
        Write-Info "Version:     $shown"
        Write-Info "Attestation: $attestation"
        if (-not $Update) {
            Write-Info 'Next: start Claude as usual (claude) and run /quay to register your login and choose groups.'
            Write-Info 'Optional: quayflow shell install   (same-terminal session replacement in PowerShell)'
            Write-Info 'Uninstall:  quayflow uninstall'
        }
        if ($modError) { throw "The engine was installed, but the Mod was not: $modError`nFix the cause and run the installer again." }
    } finally {
        Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
        if ($stoppedForUpgrade -and -not $restarted) {
            # The upgrade failed after stopping the engine: do not leave the user without one.
            if (Test-Path -LiteralPath $Exe) {
                $state = Start-Engine
                Write-Warn "The upgrade did not finish; the engine was restarted ($state)."
            } else {
                Remove-Item -LiteralPath (Join-Path $DataRoot 'engine.stopped') -Force -ErrorAction SilentlyContinue
            }
        }
    }
}

Install-Quayflow @PSBoundParameters
