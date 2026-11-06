<#
Sandbox test for installer/install.ps1. Runs the installer in child processes
of PowerShell 7 and Windows PowerShell 5.1 with LOCALAPPDATA redirected to a
temporary folder, a fake quayflow.exe (compiled with the .NET Framework C#
compiler through Windows PowerShell) and -Source -NoPath -NoPlugin
-SkipAttestation, so it changes nothing outside the temporary folder: no PATH,
no Claude plugins, no downloads, no Claude data.
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
    public static int Main(string[] args) {
        string log = Environment.GetEnvironmentVariable("QF_FAKE_LOG");
        if (!string.IsNullOrEmpty(log)) File.AppendAllText(log, string.Join(" ", args) + Environment.NewLine);
        string root = Path.Combine(Environment.GetEnvironmentVariable("LOCALAPPDATA"), "Quayflow");
        string marker = Path.Combine(root, "engine.stopped");
        string cmd = args.Length > 0 ? args[0] : "";
        if (cmd == "version") {
            if (Environment.GetEnvironmentVariable("QF_FAKE_VERSION_FAIL") == "1") { Console.WriteLine("broken"); return 1; }
            Console.WriteLine("quayflow 0.0.0-test");
        } else if (cmd == "engine-sim") {
            Thread.Sleep(120000);
        } else if (cmd == "stop") {
            File.WriteAllText(marker, "1");
            try {
                Match m = Regex.Match(File.ReadAllText(Path.Combine(root, "engine.json")), "\"pid\"\\s*:\\s*(\\d+)");
                Process.GetProcessById(int.Parse(m.Groups[1].Value)).Kill();
            } catch (Exception) { }
        } else if (cmd == "start") {
            bool ifConsented = args.Length > 1 && args[1] == "--if-consented";
            if (ifConsented && File.Exists(marker)) { Console.WriteLine("{\"state\":\"stopped_by_user\"}"); return 0; }
            if (!ifConsented && File.Exists(marker)) File.Delete(marker);
            string state = Environment.GetEnvironmentVariable("QF_FAKE_START_STATE");
            if (string.IsNullOrEmpty(state)) state = "started";
            Console.WriteLine("{\"state\":\"" + state + "\",\"pid\":4242}");
            return state == "failed" ? 1 : 0;
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

    function Write-Sums([string]$Dir, [hashtable]$Override = @{}, [string[]]$Omit = @()) {
        $lines = foreach ($n in 'quayflow.exe', 'quayflow-bg.exe') {
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
            # Without -SkipAttestation: a binary not built by the release workflow
            # must be refused when gh is signed in, or reported as skipped.
            Remove-Item $bin -Recurse -Force -ErrorAction SilentlyContinue
            $r = & $ps -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $Installer -Source $assets -Yes -NoPath -NoPlugin 2>&1 | Out-String
            $refused = ($LASTEXITCODE -ne 0 -and $r -match 'REFUSED: attestation verification failed' -and -not (Test-Path (Join-Path $bin 'quayflow.exe')))
            $skipped = ($LASTEXITCODE -eq 0 -and $r -match 'Attestation: skipped \(GitHub CLI')
            Assert ($refused -or $skipped) 'unattested binary refused (or attestation reported skipped without gh)' $r

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
            Remove-Item Env:QF_FAKE_LOG, Env:QF_FAKE_START_STATE, Env:QF_FAKE_VERSION_FAIL -ErrorAction SilentlyContinue
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
