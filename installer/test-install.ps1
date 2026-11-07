<#
Sandbox test for installer/install.ps1. Runs the installer in child processes
of PowerShell 7 and Windows PowerShell 5.1 with LOCALAPPDATA redirected to a
temporary folder, a fake quayflow.exe (compiled with the .NET Framework C#
compiler through Windows PowerShell) and -Source -NoPath, so it changes nothing
outside the temporary folder: no PATH, no Claude plugins, no downloads, no
Claude data. Attestation and Mod cases run with fake gh.exe and claude.exe (the
same program) on a reduced PATH that hides the real ones.
Exit code 0 when every check passes.
#>
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$Installer = Join-Path $PSScriptRoot 'install.ps1'
$Work = Join-Path ([IO.Path]::GetTempPath()) ('qf-installer-test-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Force $Work | Out-Null
$script:Pass = 0; $script:Fail = 0
function Assert([bool]$Condition, [string]$Name, [string]$Detail = '') {
    if ($Condition) { $script:Pass++; Write-Host "  PASS $Name" -ForegroundColor Green }
    else { $script:Fail++; Write-Host "  FAIL $Name $Detail" -ForegroundColor Red }
}

try {
    # Fake quayflow.exe: logs its arguments, prints a version, exits 0. It also mimics the
    # engine's stop/start contract: `stop` writes the "stopped by user" marker and ends the
    # process named in engine.json; `start --if-consented` honors the marker; plain `start`
    # clears it. `engine-sim` just stays alive (a stand-in for a running engine).
    $src = Join-Path $Work 'fake.cs'
    $assets = Join-Path $Work 'assets'
    New-Item -ItemType Directory -Force $assets | Out-Null
    @'
using System;
using System.Diagnostics;
using System.IO;
using System.Text.RegularExpressions;
using System.Threading;
public static class Program {
    static string Env(string name) { return Environment.GetEnvironmentVariable(name) ?? ""; }
    static string Json(string s) { return s.Replace("\\", "\\\\").Replace("\"", "\\\""); }
    static bool EngineAlive(string root) {
        try {
            Match m = Regex.Match(File.ReadAllText(Path.Combine(root, "engine.json")), "\"pid\"\\s*:\\s*(\\d+)");
            return !Process.GetProcessById(int.Parse(m.Groups[1].Value)).HasExited;
        } catch (Exception) { return false; }
    }
    public static int Main(string[] args) {
        // One program, three roles chosen by the file name: quayflow*.exe, gh.exe, claude.exe.
        string self = Path.GetFileNameWithoutExtension(System.Reflection.Assembly.GetEntryAssembly().Location).ToLowerInvariant();
        string prefix = (self == "gh" || self == "claude") ? self + ": " : "";
        string log = Env("QF_FAKE_LOG");
        if (log != "") File.AppendAllText(log, prefix + string.Join(" ", args) + Environment.NewLine);
        if (self == "gh") return Gh(args);
        if (self == "claude") return Claude(args);
        string root = Path.Combine(Env("LOCALAPPDATA"), "Quayflow");
        string marker = Path.Combine(root, "engine.stopped");
        string cmd = args.Length > 0 ? args[0] : "";
        if (cmd == "version") {
            if (Env("QF_FAKE_VERSION_FAIL") == "1") { Console.WriteLine("broken"); return 1; }
            Console.WriteLine("quayflow 0.0.0-test");
        } else if (cmd == "engine-sim") {
            Thread.Sleep(120000);
        } else if (cmd == "stop") {
            File.WriteAllText(marker, "1");
            if (Env("QF_FAKE_STOP_HANG") == "1") return 0;   // asked to stop, keeps running
            try {
                Match m = Regex.Match(File.ReadAllText(Path.Combine(root, "engine.json")), "\"pid\"\\s*:\\s*(\\d+)");
                Process.GetProcessById(int.Parse(m.Groups[1].Value)).Kill();
            } catch (Exception) { }
        } else if (cmd == "start") {
            Console.Error.WriteLine("{\"note\":\"stderr is not the answer\"}");
            if (EngineAlive(root)) { Console.WriteLine("{\"state\":\"running\"}"); return 0; }
            bool ifConsented = args.Length > 1 && args[1] == "--if-consented";
            if (ifConsented && File.Exists(marker)) { Console.WriteLine("{\"state\":\"stopped_by_user\"}"); return 0; }
            if (!ifConsented && File.Exists(marker)) File.Delete(marker);
            string state = Env("QF_FAKE_START_STATE");
            if (state == "") state = "started";
            Console.WriteLine("{\"state\":\"" + state + "\",\"pid\":4242}");
            return state == "failed" ? 1 : 0;
        }
        return 0;
    }
    static int Gh(string[] args) {
        string cmd = args.Length > 1 ? args[0] + " " + args[1] : "";
        if (cmd == "auth status") return Env("QF_FAKE_GH_AUTH") == "fail" ? 1 : 0;
        if (cmd == "attestation verify") {
            if (Env("QF_FAKE_GH_ATTEST") == "fail") { Console.Error.WriteLine("no matching attestation"); return 1; }
            Console.WriteLine("Verification succeeded");
        }
        return 0;
    }
    static int Claude(string[] args) {
        // State: market.txt (the quayflow marketplace's source) and plugin.txt (installed).
        string dir = Env("QF_FAKE_CLAUDE_STATE");
        Directory.CreateDirectory(dir);
        string market = Path.Combine(dir, "market.txt"), plugin = Path.Combine(dir, "plugin.txt");
        string cmd = string.Join(" ", args);
        Console.Error.WriteLine("[claude] a warning on stderr");
        if (cmd == "plugin list --json") {
            Console.WriteLine(File.Exists(plugin) ? "[{\"id\":\"quayflow@quayflow\",\"scope\":\"user\"}]" : "[]");
        } else if (cmd == "plugin marketplace list --json") {
            if (!File.Exists(market)) { Console.WriteLine("[]"); return 0; }
            string src = File.ReadAllText(market);
            if (src.Contains(":\\")) {
                Console.WriteLine("[{\"name\":\"quayflow\",\"source\":\"directory\",\"path\":\"" + Json(src) + "\",\"installLocation\":\"" + Json(src) + "\"}]");
            } else {
                string[] parts = src.Split('#');
                Console.WriteLine("[{\"name\":\"quayflow\",\"source\":\"github\",\"repo\":\"" + Json(parts[0]) + "\"" +
                    (parts.Length > 1 ? ",\"ref\":\"" + Json(parts[1]) + "\"" : "") +
                    ",\"installLocation\":\"" + Json(Env("QF_FAKE_CLAUDE_CLONE")) + "\"}]");
            }
        } else if (cmd == "plugin marketplace remove quayflow") {
            if (File.Exists(plugin)) File.Delete(plugin);
            if (File.Exists(market)) File.Delete(market); else return 1;
        } else if (args.Length == 4 && cmd.StartsWith("plugin marketplace add ")) {
            if (File.Exists(market)) { Console.Error.WriteLine("Cannot add marketplace \"quayflow\": its source doesn't match"); return 1; }
            string fail = Env("QF_FAKE_CLAUDE_ADD_FAIL");
            if (fail != "" && args[3].Contains(fail)) { Console.Error.WriteLine("Failed to clone marketplace repository"); return 1; }
            File.WriteAllText(market, args[3]);
        } else if (cmd == "plugin install quayflow@quayflow --scope user") {
            if (!File.Exists(market)) return 1;
            File.WriteAllText(plugin, "1");
        }
        return 0;
    }
}
'@ | Set-Content -Path $src -Encoding ASCII
    $exe = Join-Path $assets 'quayflow.exe'
    $compile = "Add-Type -TypeDefinition (Get-Content -Raw '$src') -OutputAssembly '$exe' -OutputType ConsoleApplication"
    & powershell.exe -NoProfile -NonInteractive -Command $compile
    if (-not (Test-Path $exe)) { throw 'Could not compile the fake quayflow.exe' }
    Copy-Item $exe (Join-Path $assets 'quayflow-bg.exe')
    $simExe = Join-Path $Work 'quayflow-engine-sim.exe'
    Copy-Item $exe $simExe
    # Make quayflow-bg.exe differ so the two hashes are distinct.
    [IO.File]::AppendAllText((Join-Path $assets 'quayflow-bg.exe'), 'bg')

    # Fake gh.exe and claude.exe (same program) in their own folder. Runs that use them get a
    # PATH of that folder, git's and the system folders only, so the real gh/claude are hidden.
    $fakeBin = Join-Path $Work 'fakebin'
    $noGhBin = Join-Path $Work 'nogh'
    New-Item -ItemType Directory -Force $fakeBin, $noGhBin | Out-Null
    Copy-Item $exe (Join-Path $fakeBin 'gh.exe')
    Copy-Item $exe (Join-Path $fakeBin 'claude.exe')
    Copy-Item $exe (Join-Path $noGhBin 'claude.exe')
    $git = (Get-Command git -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
    $sysPath = @((Split-Path $git), "$env:SystemRoot\System32", $env:SystemRoot, "$env:SystemRoot\System32\WindowsPowerShell\v1.0") -join ';'
    # A git checkout standing in for the marketplace clone Claude Code makes; its HEAD is the
    # commit the "release" records in SOURCE_COMMIT.
    $clone = Join-Path $Work 'clone'
    New-Item -ItemType Directory -Force $clone | Out-Null
    & $git -C $clone init -q 2>&1 | Out-Null
    Set-Content (Join-Path $clone 'README') 'mod' -Encoding ASCII
    & $git -C $clone add README 2>&1 | Out-Null
    & $git -C $clone -c user.name=t -c user.email=t@example.invalid -c commit.gpgsign=false commit -q -m init 2>&1 | Out-Null
    $cloneHead = (& $git -C $clone rev-parse HEAD).Trim()
    if ($cloneHead -notmatch '^[0-9a-f]{40}$') { throw 'Could not create the test git checkout' }

    function Write-Sums([string]$Dir, [hashtable]$Override = @{}, [string[]]$Omit = @()) {
        $names = @('quayflow.exe', 'quayflow-bg.exe')
        if (Test-Path (Join-Path $Dir 'SOURCE_COMMIT')) { $names += 'SOURCE_COMMIT' }
        $lines = foreach ($n in $names) {
            if ($Omit -contains $n) { continue }
            $h = (Get-FileHash (Join-Path $Dir $n) -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($Override.ContainsKey($n)) { $h = $Override[$n] }
            "$h  $n"
        }
        [IO.File]::WriteAllText((Join-Path $Dir 'SHA256SUMS'), (($lines -join "`n") + "`n"))
    }

    $hosts = @()
    foreach ($c in 'pwsh', 'powershell') {
        $cmd = Get-Command $c -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($cmd) { $hosts += $cmd.Source }
    }

    foreach ($ps in $hosts) {
        # Windows PowerShell 5.1 turns a child's stderr into error records; expected refusals must not abort the test.
        $ErrorActionPreference = 'Continue'
        Write-Host "=== $ps ===" -ForegroundColor Cyan
        $lad = Join-Path $Work ('lad-' + [guid]::NewGuid().ToString('N').Substring(0, 6))
        $log = Join-Path $Work ('fake-' + [guid]::NewGuid().ToString('N').Substring(0, 6) + '.log')
        $bin = Join-Path $lad 'Quayflow\bin'
        $savedLad = $env:LOCALAPPDATA
        $env:LOCALAPPDATA = $lad
        $env:QF_FAKE_LOG = $log
        function Run([string[]]$Extra) {
            # Windows PowerShell 5.1 turns a child's stderr into error records; expected refusals must not abort the test.
            $ErrorActionPreference = 'Continue'
            $out = & $ps -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $Installer -Source $assets -Yes -NoPath -NoPlugin -SkipAttestation @Extra 2>&1 | Out-String
            return @{ Code = $LASTEXITCODE; Out = $out }
        }
        function RunWith([string]$Dirs, [string[]]$Extra) {
            # The installer with a PATH of $Dirs plus git and the system folders only.
            $ErrorActionPreference = 'Continue'
            $savedPath = $env:PATH
            $env:PATH = "$Dirs;$sysPath"
            try { $out = & $ps -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $Installer -Yes -NoPath @Extra 2>&1 | Out-String; $code = $LASTEXITCODE }
            finally { $env:PATH = $savedPath }
            return @{ Code = $code; Out = $out }
        }
        function LogLines { if (Test-Path $log) { @(Get-Content $log) } else { @() } }
        $sims = @()
        function Start-Sim {
            # A running "engine": a quayflow* process named in engine.json.
            $p = Start-Process -FilePath $simExe -ArgumentList 'engine-sim' -PassThru -WindowStyle Hidden
            [IO.File]::WriteAllText((Join-Path $lad 'Quayflow\engine.json'), ('{"pid": ' + $p.Id + '}'))
            $script:sims += $p
            return $p
        }
        $marker = Join-Path $lad 'Quayflow\engine.stopped'
        try {
            Write-Sums $assets
            $r = Run @()
            Assert ($r.Code -eq 0) 'install succeeds' $r.Out
            Assert ((Get-FileHash (Join-Path $bin 'quayflow.exe')).Hash -eq (Get-FileHash $exe).Hash) 'quayflow.exe installed'
            Assert (Test-Path (Join-Path $bin 'quayflow-bg.exe')) 'quayflow-bg.exe installed'
            Assert ((LogLines) -contains 'consent engine-start on') 'engine-start consent recorded'
            Assert (-not ((LogLines) -contains 'consent signin-startup on')) 'no sign-in startup without -Startup'
            Assert ($r.Out -match 'not affiliated with or endorsed by Anthropic') 'consent text shown'
            Assert ($r.Out -match 'skipped \(-SkipAttestation\)') 'attestation status reported'

            $r = Run @('-Startup')
            Assert ($r.Code -eq 0) 're-run (upgrade) succeeds' $r.Out
            Assert ((LogLines) -contains 'consent signin-startup on') '-Startup records sign-in consent'
            Assert (@(Get-ChildItem $bin -Filter '*.new').Count -eq 0) 'no staged files left'
            Assert (-not ((LogLines) -contains 'stop') -and -not ((LogLines) -match '^start')) 'engine not running: upgrade neither stops nor starts it' ((LogLines) -join ' | ')

            # Upgrade while the engine runs: it is stopped for the swap and must come back on,
            # with the "stopped by user" marker written by that stop cleared.
            $sim = Start-Sim
            Remove-Item $log -ErrorAction SilentlyContinue
            $r = Run @()
            Assert ($r.Code -eq 0) 'upgrade with a running engine succeeds' $r.Out
            Assert ((LogLines) -contains 'stop') 'engine stopped for the upgrade' ((LogLines) -join ' | ')
            Assert ($sim.WaitForExit(5000)) 'old engine process ended'
            Assert ((LogLines) -contains 'start') 'engine restarted with plain start' ((LogLines) -join ' | ')
            Assert (-not ((LogLines) -contains 'start --if-consented')) 'restart does not use --if-consented' ((LogLines) -join ' | ')
            Assert (-not (Test-Path $marker)) 'stopped-by-user marker cleared after the upgrade'
            Assert ($r.Out -match 'Engine restarted \(started\)') 'real restart state reported' $r.Out

            # A failed restart is reported as failed, not as "restarted".
            $null = Start-Sim
            $env:QF_FAKE_START_STATE = 'failed'
            $r = Run @()
            Remove-Item Env:QF_FAKE_START_STATE -ErrorAction SilentlyContinue
            Assert ($r.Out -match 'did not restart \(failed\)' -and $r.Out -notmatch 'Engine restarted') 'failed restart reported truthfully' $r.Out

            # The upgrade fails after the engine was stopped: the engine is still restarted.
            $null = Start-Sim
            Remove-Item $log -ErrorAction SilentlyContinue
            $env:QF_FAKE_VERSION_FAIL = '1'
            $r = Run @()
            Remove-Item Env:QF_FAKE_VERSION_FAIL -ErrorAction SilentlyContinue
            Assert ($r.Code -ne 0) 'upgrade failure after stop is an error' $r.Out
            Assert ((LogLines) -contains 'start' -and $r.Out -match 'engine was restarted \(started\)') 'engine restarted after a failed upgrade' ($r.Out + ' | ' + ((LogLines) -join ' | '))
            Assert (-not (Test-Path $marker)) 'no stopped-by-user marker left after a failed upgrade'
            Remove-Item (Join-Path $lad 'Quayflow\engine.json') -ErrorAction SilentlyContinue

            Remove-Item $bin -Recurse -Force
            Write-Sums $assets @{ 'quayflow.exe' = ('0' * 64) }
            $r = Run @()
            Assert ($r.Code -ne 0 -and $r.Out -match 'REFUSED: hash mismatch') 'hash mismatch refused' $r.Out
            Assert (-not (Test-Path (Join-Path $bin 'quayflow.exe'))) 'nothing installed on mismatch'

            Write-Sums $assets @{} @('quayflow-bg.exe')
            $r = Run @()
            Assert ($r.Code -ne 0 -and $r.Out -match 'REFUSED: SHA256SUMS has no entry') 'missing hash entry refused' $r.Out

            Write-Sums $assets
            # ---- Attestation fails closed: without -SkipAttestation, files are installed only
            # when the release workflow's attestation verifies.
            Remove-Item $bin -Recurse -Force -ErrorAction SilentlyContinue
            $r = RunWith $noGhBin @('-Source', $assets, '-NoPlugin')
            Assert ($r.Code -ne 0 -and $r.Out -match 'REFUSED: the build-provenance attestation cannot be verified because the GitHub CLI \(gh\) is not installed' -and $r.Out -match '-SkipAttestation') 'no gh: refused, and told how to skip explicitly' $r.Out
            Assert (-not (Test-Path (Join-Path $bin 'quayflow.exe'))) 'no gh: nothing installed'
            $env:QF_FAKE_GH_AUTH = 'fail'
            $r = RunWith $fakeBin @('-Source', $assets, '-NoPlugin')
            Remove-Item Env:QF_FAKE_GH_AUTH
            Assert ($r.Code -ne 0 -and $r.Out -match 'not signed in' -and -not (Test-Path (Join-Path $bin 'quayflow.exe'))) 'gh signed out: refused, nothing installed' $r.Out
            $env:QF_FAKE_GH_ATTEST = 'fail'
            $r = RunWith $fakeBin @('-Source', $assets, '-NoPlugin')
            Remove-Item Env:QF_FAKE_GH_ATTEST
            Assert ($r.Code -ne 0 -and $r.Out -match 'REFUSED: attestation verification failed for quayflow.exe' -and -not (Test-Path (Join-Path $bin 'quayflow.exe'))) 'unattested binary refused' $r.Out

            # A release with SOURCE_COMMIT: every file is checked against the release workflow,
            # this tag and this commit.
            $rel = Join-Path $Work 'release'
            New-Item -ItemType Directory -Force $rel | Out-Null
            Copy-Item (Join-Path $assets '*.exe') $rel
            [IO.File]::WriteAllText((Join-Path $rel 'SOURCE_COMMIT'), "$cloneHead`n")
            Write-Sums $rel
            Remove-Item $log -ErrorAction SilentlyContinue
            $r = RunWith $fakeBin @('-Source', $rel, '-NoPlugin')
            $verifies = @(LogLines | Where-Object { $_ -like 'gh: attestation verify *' })
            Assert ($r.Code -eq 0 -and $r.Out -match 'Attestation: verified') 'attested release installs' $r.Out
            Assert ($verifies.Count -eq 3 -and @($verifies | Where-Object { $_ -like '*SOURCE_COMMIT*' }).Count -eq 1) 'both executables and SOURCE_COMMIT verified' ($verifies -join ' | ')
            Assert (@($verifies | Where-Object { $_ -notlike '*--repo minhdaohuu/quayflow --signer-workflow minhdaohuu/quayflow/.github/workflows/release.yml*' -or $_ -notlike "*--source-digest $cloneHead*" }).Count -eq 0) 'verified against the release workflow and the source commit' ($verifies -join ' | ')
            [IO.File]::WriteAllText((Join-Path $rel 'SOURCE_COMMIT'), ('f' * 40) + "`n")
            $r = RunWith $fakeBin @('-Source', $rel, '-NoPlugin')
            Assert ($r.Code -ne 0 -and $r.Out -match 'REFUSED: hash mismatch for SOURCE_COMMIT') 'edited SOURCE_COMMIT refused' $r.Out
            [IO.File]::WriteAllText((Join-Path $rel 'SOURCE_COMMIT'), "$cloneHead`n")

            # ---- The Mod: added from the tag only while the tag points at SOURCE_COMMIT; a
            # failed or refused replacement puts the previous Mod back.
            $claudeState = Join-Path $lad 'claude-state'
            $env:QF_FAKE_CLAUDE_STATE = $claudeState
            $env:QF_FAKE_CLAUDE_CLONE = $clone
            $r = RunWith $fakeBin @('-Source', $rel, '-SkipAttestation', '-MarketplaceSource', 'minhdaohuu/quayflow#v9.9.9')
            Assert ($r.Code -eq 0 -and $r.Out -match "Mod source verified: commit $cloneHead") 'Mod added from a tag at the release commit' $r.Out
            Assert ((Get-Content (Join-Path $claudeState 'market.txt') -Raw) -eq 'minhdaohuu/quayflow#v9.9.9' -and (Test-Path (Join-Path $claudeState 'plugin.txt'))) 'marketplace and plugin installed'

            # The tag was moved: the checkout is not at the release commit.
            Set-Content (Join-Path $clone 'README') 'moved' -Encoding ASCII
            & $git -C $clone -c user.name=t -c user.email=t@example.invalid -c commit.gpgsign=false commit -q -am moved 2>&1 | Out-Null
            Remove-Item $log -ErrorAction SilentlyContinue
            $r = RunWith $fakeBin @('-Source', $rel, '-SkipAttestation', '-MarketplaceSource', 'minhdaohuu/quayflow#v9.9.10')
            Assert ($r.Code -ne 0 -and $r.Out -match 'REFUSED: the Mod source minhdaohuu/quayflow#v9.9.10 is at commit' -and $r.Out -match 'Done, except the Mod') 'moved tag: Mod refused' $r.Out
            Assert ((Get-Content (Join-Path $claudeState 'market.txt') -Raw) -eq 'minhdaohuu/quayflow#v9.9.9' -and (Test-Path (Join-Path $claudeState 'plugin.txt'))) 'moved tag: previous Mod restored' $r.Out
            Assert ((LogLines) -contains 'consent engine-start on') 'Mod failure: engine consent still recorded' ((LogLines) -join ' | ')
            & $git -C $clone reset -q --hard $cloneHead 2>&1 | Out-Null

            # The new source cannot be added: the previous Mod is put back.
            $env:QF_FAKE_CLAUDE_ADD_FAIL = 'broken'
            $r = RunWith $fakeBin @('-Source', $rel, '-SkipAttestation', '-MarketplaceSource', 'minhdaohuu/quayflow#broken')
            Remove-Item Env:QF_FAKE_CLAUDE_ADD_FAIL
            Assert ($r.Code -ne 0 -and $r.Out -match 'marketplace add minhdaohuu/quayflow#broken failed' -and $r.Out -match 'previous Mod \(minhdaohuu/quayflow#v9.9.9\) was restored') 'failed add reported, previous Mod restored' $r.Out
            Assert ((Get-Content (Join-Path $claudeState 'market.txt') -Raw) -eq 'minhdaohuu/quayflow#v9.9.9' -and (Test-Path (Join-Path $claudeState 'plugin.txt'))) 'failed add: marketplace and plugin back'

            # ---- -Update (quayflow update): no consent text, no consent changes, the shell
            # module follows the new program, the Mod moves with the release.
            $shellDir = Join-Path $lad 'Quayflow\shell'
            New-Item -ItemType Directory -Force $shellDir | Out-Null
            Set-Content (Join-Path $shellDir 'Quayflow.psm1') 'old module' -Encoding ASCII
            Remove-Item $log -ErrorAction SilentlyContinue
            $r = RunWith $fakeBin @('-Source', $rel, '-SkipAttestation', '-Update', '-Startup', '-MarketplaceSource', 'minhdaohuu/quayflow#v9.9.9')
            Assert ($r.Code -eq 0 -and $r.Out -match 'Updating Quayflow' -and $r.Out -notmatch 'not affiliated with or endorsed') 'update: no consent text' $r.Out
            Assert (-not ((LogLines) -match '^consent ')) 'update: no consent changed (not even with -Startup)' ((LogLines) -join ' | ')
            Assert ((LogLines) -contains 'shell refresh') 'update: installed shell module refreshed' ((LogLines) -join ' | ')
            Assert ((Get-Content (Join-Path $claudeState 'market.txt') -Raw) -eq 'minhdaohuu/quayflow#v9.9.9' -and $r.Out -match "Mod source verified: commit $cloneHead") 'update: Mod moved to the release' $r.Out
            # A Mod installed from a local folder (a development checkout) is left alone.
            Set-Content (Join-Path $claudeState 'market.txt') $clone -NoNewline -Encoding ASCII
            Remove-Item $log -ErrorAction SilentlyContinue
            $r = RunWith $fakeBin @('-Source', $rel, '-SkipAttestation', '-Update')
            Assert ($r.Code -eq 0 -and $r.Out -match 'installed from .* and was not changed') 'update: local-folder Mod reported as kept' $r.Out
            Assert ((Get-Content (Join-Path $claudeState 'market.txt') -Raw) -eq $clone -and -not ((LogLines) -match '^claude: plugin marketplace (add|remove)')) 'update: local-folder Mod untouched' ((LogLines) -join ' | ')
            Remove-Item $shellDir -Recurse -Force
            Remove-Item Env:QF_FAKE_CLAUDE_STATE, Env:QF_FAKE_CLAUDE_CLONE

            # ---- An engine that ignores `stop` is ended, and the new one started.
            $sim = Start-Sim
            $env:QF_FAKE_STOP_HANG = '1'
            $r = Run @()
            Remove-Item Env:QF_FAKE_STOP_HANG
            Assert ($r.Out -match 'did not stop within 20 s; ending it' -and $sim.HasExited -and $r.Out -match 'Engine restarted \(started\)') 'hung engine ended, new engine started' $r.Out
            Remove-Item (Join-Path $lad 'Quayflow\engine.json') -ErrorAction SilentlyContinue

            $null = Run @()
            Remove-Item $log -ErrorAction SilentlyContinue
            $r = & $ps -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $Installer -Uninstall -Yes -DeleteData -NoPlugin 2>&1 | Out-String
            Assert ($LASTEXITCODE -eq 0) 'uninstall succeeds' $r
            Assert ((LogLines) -contains 'uninstall --delete-data --yes') 'uninstall delegated to quayflow.exe' ((LogLines) -join ' | ')

            $r = & $ps -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $Installer -Version 'not-a-version' -Yes -NoPath -NoPlugin 2>&1 | Out-String
            Assert ($LASTEXITCODE -ne 0 -and $r -match 'Not a release version') 'invalid -Version refused before any download' $r
        } finally {
            foreach ($p in $script:sims) { if (-not $p.HasExited) { try { $p.Kill() } catch { $null = $_ } } }
            $env:LOCALAPPDATA = $savedLad
            Remove-Item Env:QF_FAKE_LOG, Env:QF_FAKE_START_STATE, Env:QF_FAKE_VERSION_FAIL, Env:QF_FAKE_STOP_HANG, Env:QF_FAKE_GH_AUTH, Env:QF_FAKE_GH_ATTEST, Env:QF_FAKE_CLAUDE_STATE, Env:QF_FAKE_CLAUDE_CLONE, Env:QF_FAKE_CLAUDE_ADD_FAIL -ErrorAction SilentlyContinue
        }
    }
} catch {
    $script:Fail++
    Write-Host "  ERROR $($_ | Out-String)" -ForegroundColor Red
} finally {
    Remove-Item $Work -Recurse -Force -ErrorAction SilentlyContinue
}
Write-Host ("{0} passed, {1} failed" -f $script:Pass, $script:Fail)
if ($script:Fail -gt 0) { exit 1 }
exit 0
