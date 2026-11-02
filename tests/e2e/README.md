# End-to-end tests

| Script | Logins | Runs in CI | What it covers |
|---|---|---|---|
| `smoke-fake-env.ps1` | fake | yes | Engine start, handshake, registration, manual switch, locks, unknown login, report, stop |
| `cli-fake-env.ps1` | fake | yes | CLI argument validation and error paths for every command that is safe to sandbox |
| `sim-usage.ps1` | fake, with a fake usage API | yes | Usage over time: suggestions, Automatic, return home, stale data, backoff, recovery, all exhausted |
| `real-switching.ps1` | **real** | no | Mid-job switch (H), switch during parallel subagents (S), simulated limit in Automatic (R1) and Suggest (R2) |
| `real-lanes.ps1` | **real** | no | Lanes: links, a real Claude session inside a lane, independent logins |

The fake-environment scripts point `USERPROFILE`, `CLAUDE_CONFIG_DIR` and `QUAYFLOW_HOME` at a temporary folder and never touch your real logins. Run them with `pwsh -File <script> [-Exe path\to\quayflow.exe]`.

## Running the real-login scenarios

These switch your **live Claude login** between two of your accounts and put it back. Read the scripts first. Prompts are synthetic and use the Haiku model.

Always run them with `pwsh -File`, never dot-sourced (`. script`): they end with `exit`, which would close your PowerShell window.

### One-time setup from a local build (before a release exists)

1. Remove any earlier Quayflow PoC plugin, which uses the same plugin name:

   ```powershell
   claude plugin list
   claude plugin uninstall quayflow@quayflow-local
   claude plugin marketplace remove quayflow-local
   ```

2. Stage a local build in `dist\` (ignored by git):

   ```powershell
   cargo build --release --locked -p quayflow
   New-Item -ItemType Directory -Force dist | Out-Null
   Copy-Item target\release\quayflow.exe, target\release\quayflow-bg.exe, installer\install.ps1, shell\Quayflow.psm1 dist\ -Force
   Push-Location dist
   (Get-ChildItem -File | Where-Object Name -ne 'SHA256SUMS' | Sort-Object Name |
       ForEach-Object { '{0}  {1}' -f (Get-FileHash $_.Name -Algorithm SHA256).Hash.ToLower(), $_.Name }) -join "`n" |
       Set-Content -NoNewline SHA256SUMS
   Pop-Location
   ```

3. Install from that folder. The installer asks for consent, verifies the hashes and adds the Mod from this repository:

   ```powershell
   pwsh -File installer\install.ps1 -Source dist -MarketplaceSource (Get-Location).Path -SkipAttestation
   ```

   Open a new terminal afterwards so `quayflow` is on your PATH.

4. Register the login you are using now, then add a second account (a window opens with Claude's own sign-in):

   ```powershell
   quayflow onboard
   quayflow register --label work --group Default
   quayflow add --label personal --group Default
   ```

   If the two accounts are an organization and a personal account, `add` shows a one-time notice and asks you to accept it; the sign-in is kept while you decide.

5. Allow the engine to record which account served each request during these tests, then check:

   ```powershell
   quayflow consent live-canary on
   quayflow status
   ```

   `status` must list both accounts in `Default`, with the one you use now live.

### Run

```powershell
pwsh -File tests/e2e/real-switching.ps1              # all cases: H, S, R1, R2
pwsh -File tests/e2e/real-switching.ps1 -Case H      # one case
pwsh -File tests/e2e/real-lanes.ps1
```

Each script prints a PASS/FAIL table and ends with your original account live. Record the results in [docs/acceptance.md](../../docs/acceptance.md).

### Undo

```powershell
quayflow uninstall              # keeps the vault and logs
quayflow uninstall --delete-data
```

Your current Claude login keeps working after uninstall.
