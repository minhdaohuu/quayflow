# End-to-end tests

| Script | Logins | Runs in CI | What it covers |
|---|---|---|---|
| `smoke-fake-env.ps1` | fake | yes (Windows) | Single instance, start without a window, handshake, consent-gated registration, mixed-identity disclosure, manual switch with pin, a held lock leaving the login unchanged, unknown login pausing automation, report without secrets, intentional stop |
| `cli-fake-env.ps1` | fake | yes (Windows) | CLI argument validation and error paths: groups, modes, projects, accounts, lanes, shell hook, policy, canary, update and corrupted state. Never runs `uninstall`, `add`, `reauth` or `onboard` |
| `sim-usage.ps1` | fake, with a fake usage and profile API (`tests/sim/fake_api.py`, needs Python) | yes (Windows) | Usage over time: 5-hour and per-model windows, suggestion at the threshold and accept, Automatic, return home, stale data never counted as zero, a rejected token, backoff after a refused read (429), recovery with one switch and one retry, concurrent failures sharing one switch, no recovery on an unconfirmed limit, all exhausted, an idle account's saved token refreshed (never the live one), a refused refresh token marking the account for a new sign-in |
| `smoke-unix.sh` | fake | yes (Linux, macOS) | The real Linux or macOS program (the macOS Keychain turned off with `QUAYFLOW_KEYCHAIN=off`): `install.sh` from a local release folder (program, `~/.local/bin` link, owner-only data folder), engine start in its own process group, `engine.json` owner-only, status, `quayflow update` handing off to `install.sh` while the engine runs, stop, uninstall |
| `real-switching.ps1` | **real** | no | Mid-job switch (H), switch during two parallel subagents (S), simulated limit in Automatic (R1) and Suggest (R2) |
| `real-headless.ps1` | **real**, with one account at a real usage limit | no | Headless `claude -p`: Automatic recovery (X1), three parallel runs sharing one switch (X2), Suggest leaves Claude's result (X3), engine stopped (X4), a run failing at the limit then a fresh `claude -p` succeeding on another account after `quayflow last` names a time to run again (X5). `quayflow last --session <id> --json` is checked after X1, X3 and X5. In every case stdout is exactly Claude's own JSON and no Quayflow line appears |
| `real-lanes.ps1` | **real** | no | Lanes: links, a real headless Claude job inside a lane reading and writing through the links, independent logins, the shared setup surviving lane deletion |
| `real-macos-keychain.py` | **real**, two accounts signed in during the run | no | macOS, without Quayflow: the Keychain switch Quayflow uses ([tech spec §1b](../../docs/tech-spec.md#1b-macos)), in a throwaway environment. Two interactive sessions and a headless run follow at their next request, the item's other entries are kept, the `.credentials.json` copy is removed safely, no Keychain dialog, locks held under 3 s. Never touches your default login. `python3 tests/e2e/real-macos-keychain.py [--auto]` (`--auto` drives the sessions itself) |

The real-login scripts are PowerShell and also run on Linux and macOS with PowerShell 7 (`sudo snap install powershell --classic` on Ubuntu, `brew install powershell` on macOS): `pwsh -File tests/e2e/real-switching.ps1`. `real-switching.ps1` and `real-headless.ps1` take `-Case` as a comma-separated list (`-Case S,R1,R2`). They find the data folder by themselves (`~/.local/share/quayflow`, or `QUAYFLOW_HOME`).

The fake-environment scripts point `CLAUDE_CONFIG_DIR` and `QUAYFLOW_HOME` (and, for `cli-fake-env.ps1` and `sim-usage.ps1`, `USERPROFILE`) at a temporary folder, and never touch your real logins. `smoke-fake-env.ps1` and `cli-fake-env.ps1` turn network calls off (`QUAYFLOW_OFFLINE`); `sim-usage.ps1` sends usage and profile calls to its loopback fake (`QUAYFLOW_SIM_API`). Run them with `pwsh -File <script> [-Exe path\to\quayflow.exe]`; the default is `target\debug\quayflow.exe`. These three are Windows scripts. On Linux and macOS, run `sh tests/e2e/smoke-unix.sh [path/to/quayflow]` (default `target/debug/quayflow`); it points `HOME`, `CLAUDE_CONFIG_DIR` and the data folder at a temporary folder, turns the network off, and on macOS keeps Quayflow out of the Keychain (`QUAYFLOW_KEYCHAIN=off`).

## Running the real-login scenarios

These switch your **live Claude login** between your accounts and put it back. Read the scripts first. Prompts are synthetic and use the Haiku model.

Always run them with `pwsh -File`, never dot-sourced (`. script`): they end with `exit`, which would close your PowerShell window.

### One-time setup

To test a published release (v0.1.3 or later on Windows, v0.1.5 or later on Linux, v0.1.6 or later on macOS), install it as the [user guide](../../docs/user-guide.md#install) describes and go to step 3. To test a local build, start at step 1.

1. Stage a local build in `dist\` (ignored by git). On Windows:

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

   On Linux:

   ```sh
   cargo build --release --locked -p quayflow
   mkdir -p dist
   cp target/release/quayflow dist/quayflow-linux-x86_64
   cp installer/install.sh dist/
   (cd dist && sha256sum quayflow-linux-x86_64 install.sh > SHA256SUMS)
   ```

   On macOS (Apple silicon; on Intel name it `quayflow-macos-x86_64`):

   ```sh
   cargo build --release --locked -p quayflow
   mkdir -p dist
   cp target/release/quayflow dist/quayflow-macos-arm64
   cp installer/install.sh dist/
   (cd dist && shasum -a 256 quayflow-macos-arm64 install.sh > SHA256SUMS)
   ```

2. Install from that folder. The installer asks for consent, verifies the hashes and adds the Mod from this repository. On Windows:

   ```powershell
   pwsh -File installer\install.ps1 -Source dist -MarketplaceSource (Get-Location).Path -SkipAttestation
   ```

   On Linux and macOS:

   ```sh
   sh installer/install.sh --source dist --marketplace-source "$PWD" --skip-attestation
   ```

   Open a new terminal afterwards so `quayflow` is on your PATH.

3. Register the login you are using now, then add a second account (a window opens with Claude's own sign-in):

   ```powershell
   quayflow onboard
   quayflow register --label work --group Default
   quayflow add --label personal --group Default
   ```

   If the two accounts are an organization and a personal account, `add` shows a one-time notice and asks you to accept it; the sign-in is kept while you decide.

4. Allow the engine to record which account served each request during these tests, then check:

   ```powershell
   quayflow consent live-canary on
   quayflow status
   ```

   `status` must list both accounts in `Default`, with the one you use now live.

### Run

```powershell
pwsh -File tests/e2e/real-switching.ps1              # all cases: H, S, R1, R2
pwsh -File tests/e2e/real-switching.ps1 -Case H      # one case
pwsh -File tests/e2e/real-headless.ps1 -Exhausted <label>             # all cases: X1-X5
pwsh -File tests/e2e/real-headless.ps1 -Exhausted <label> -Case X1,X5 # selected cases
pwsh -File tests/e2e/real-lanes.ps1 [-Account <label>]
```

`real-switching.ps1` and `real-headless.ps1` take `-Group` (default `Default`) and `-PluginDir mod\quayflow` to load the repository's Mod instead of the installed one.

`real-headless.ps1` needs, in one group, an account that is at its usage limit now (`-Exhausted`) and at least one other account with room. It moves the live login to the exhausted account, sets the group to Automatic (with `--approve`) or Suggest per case, and stops the engine for X4 (then starts it again). The `quayflow last` checks need v0.1.3 or later.

`real-lanes.ps1` uses a second registered account (`-Account`, or the first other account in good health) in a lane named `qf-e2e`; your default login is not changed.

Each script prints a PASS/FAIL table. `real-switching.ps1` and `real-headless.ps1` end with your original account live and no pin, and `real-headless.ps1` also puts back the group's mode; if it cannot, it prints the commands to do it by hand. Record the results in [docs/acceptance.md](../../docs/acceptance.md).

### Undo

```powershell
quayflow uninstall              # keeps the vault and logs
quayflow uninstall --delete-data
```

Your current Claude login keeps working after uninstall.
