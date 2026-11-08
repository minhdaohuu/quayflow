# Known limits of Quayflow v0.1

Quayflow v0.1 (currently v0.1.5) is a pilot release. This page lists what it does not do, what depends on undocumented Claude Code behavior, and what has not yet been confirmed. Requirement references (FR-xx, AC-xx) are to [requirements.md](requirements.md) and [acceptance.md](acceptance.md).

## Platform and scope

- **Windows x64 and Linux x86_64.** Quayflow targets all platforms Claude Code runs on; v0.1 supports Windows 10/11 x64 and, from v0.1.5, Linux x86_64 with glibc 2.35 and OpenSSL 3 or newer (Ubuntu 22.04, Debian 12, Fedora 36 and later). macOS (Apple silicon and Intel) is built for the next release and passes the automated suites on a Mac; its CI job has not run yet; Linux arm64 is not built yet (see [requirements.md](requirements.md#platform-scope)).
- **Linux vault is not encrypted.** Saved logins are owner-only files (`0600` in a `0700` folder), as Claude Code keeps its own login on Linux; any program running as you, or root, can read them. An encrypted vault through the desktop keyring (Secret Service) is planned; it is not used now because unattended jobs on a server often have no unlocked keyring.
- **macOS is not fully tested.** The engine, installer and Mod pass their automated suites on a Mac. The Keychain switch passed with two real accounts on Claude Code 2.1.319 (`tests/e2e/real-macos-keychain.py`, 19/19): two running interactive sessions and a headless run used the new account from their next request (the first 0.5 s after the switch), the item's other entries were kept and no Keychain dialog appeared. Quayflow itself then passed `real-switching.ps1` 13/13 on macOS with two real accounts: switches in the middle of a headless job and of parallel subagents, and recovery from a simulated usage limit (one switch, one retry on the new account). Lanes passed `real-lanes.ps1` 8/8. The LaunchAgent started the engine at sign-in. Not yet run on macOS: recovery from a real usage limit (`real-headless.ps1`), Open New Session, an install from a published release, Intel Macs, and the macOS CI job on GitHub.
- **macOS: the Keychain switch depends on more undocumented behavior.** Claude's Keychain item name (a hash of the exact `CLAUDE_CONFIG_DIR` text), its 30-second Keychain cache, and its check of `.credentials.json`'s modification time before each request, which Quayflow uses to make running sessions read the new login at once. The engine must run in your login session; it cannot use the Keychain over SSH. A session started with another spelling of `CLAUDE_CONFIG_DIR` than Quayflow's (a trailing slash, a path through a symbolic link, or the variable set to `~/.claude` explicitly) reads another Keychain item and does not follow a switch; `/quay` names such sessions and the exact value to use. A `CLAUDE_CONFIG_DIR` with characters outside ASCII may name a different item than Claude's (Claude normalizes the text to Unicode NFC; Quayflow does not).
- **macOS vault is not encrypted.** Saved logins are owner-only files, as on Linux, while Claude Code keeps its own login in the Keychain there.
- **macOS: configuration profiles are not read.** Quayflow reads the managed login policy from `/Library/Application Support/ClaudeCode`, but not from MDM configuration profiles (`/Library/Managed Preferences/…/com.anthropic.claudecode.plist`), which Claude Code also reads. Claude still enforces such a policy.
- **Linux and macOS: no same-terminal Replace.** The prompt hook is PowerShell only. Open New Session needs a desktop terminal; over SSH, Quayflow prints the command to run.
- **Linux is not fully tested.** The engine, installer and Mod pass their automated suites on Linux, and on one Ubuntu 24.04 machine switching, lanes and a silent headless job were run with two real accounts. Not yet run on Linux: recovery from a real usage limit (headless cases X1, X2, X3, X5), an install and update from a published release, Open New Session, and any distribution other than Ubuntu 24.04. See [acceptance.md](acceptance.md).
- **Subscription logins only** (claude.ai sign-in). Quayflow never moves a session to API-key or other billing.
- **One user.** The engine, vault and settings are per user. Nothing is shared between users or machines.
- **No telemetry, no cloud component.** Every decision is made locally, from usage readings taken on this machine.

## Depends on undocumented behavior

Quayflow relies on Claude Code behavior that was observed on Claude Code 2.1.316 but is not documented:

- the client reads the login again at its next model request, so an in-place swap takes effect without a restart;
- Claude's credential lock protocol (`.oauth_refresh.lock`, the legacy lock, `.storage-write.lock`, the configuration lock), which Quayflow takes in Claude's order;
- on macOS, Claude's Keychain item for its login, its 30-second Keychain cache, and its check of `.credentials.json` before each request (see the macOS items above);
- the usage endpoint (`/api/oauth/usage`) and profile endpoint (`/api/oauth/profile`), and their response shapes;
- the token service and Claude Code's public client, used to refresh an idle account's saved login;
- the session registry (`<env>\sessions\<pid>.json`).

Any Claude update can change these. Quayflow checks each new Claude version automatically. It disables in-place switching for a version that fails the check, or whose first real switch is not adopted, and falls back to Open New Session (or same-terminal Replace, Windows only) with a warning (FR-24). If the usage endpoint changes or refuses, readings become stale. Stale readings never count as zero, so Quayflow stops proactive switching instead of guessing, and manual switching and confirmed-limit recovery keep working.

**Usage readings are paced.** The usage endpoint's budget is per login and shared with Claude Code itself. Quayflow reads the live account every minute and idle accounts every 10 minutes. After the endpoint refuses a read (HTTP 429), that account waits at least 2 minutes, doubling to at most 15 minutes for refusals in a row, and for an hour the live account is read every 2 minutes. A refresh from `/quay` waits for that pace too. So a figure can be a few minutes old; its age is always shown.

## Switching behavior

- **A switch moves every session of an environment.** The default environment `~/.claude` is shared by all your sessions. Use [lanes](user-guide.md#lanes) for sessions that must stay on a different account.
- **Not every client is visible.** `/quay` lists sessions from Claude's session registry. Editor extensions and other clients of the same environment may not appear, and they switch too.
- **Prompt cache is per account.** The first request after a switch reprocesses the whole conversation: slower, and it uses more allowance on long conversations.
- **Out-of-group switches are not offered.** Verification happens on each client's first request after the switch, so the destination must already be approved by the group. Pre-request verification for other cases is not built (AC-19).
- **Attribution is read after each request.** If a switch lands while a request is starting, that request may be attributed to the new account. The request itself always goes wholly to one account.
- **An idle account needs a working refresh token.** When an idle account's access token expires, the engine refreshes its saved copy so its usage stays readable. It skips the refresh while any environment's login files hold the same refresh token; the account then shows no current usage until it is live or the refresh can run. If the sign-in service refuses that refresh token (for example after a long absence or a sign-out elsewhere), the account is marked for a new sign-in and is not a candidate until you reauthenticate it. While a refresh fails for other reasons (network, service errors), the account shows no usage figure and is not a candidate.
- **No pre-request hold in v0.1.** The requirements allow the Mod to hold a request briefly at the request boundary to choose the switch moment (FR-11). This build does not do that. A switch lands at each client's next request as it comes. Automatic mode can still prefer a moment when no listed session is busy (`--idle-wait`, up to 60 s).
- **Headless jobs switch only in Automatic mode.** In Suggest mode a `claude -p` job never switches; a usage limit ends it as it would without Quayflow. Quayflow never changes a job's stdout, stderr or exit code, so a job learns what happened with `quayflow last --session <id>`, which also says when to run again.
- **One retry per failed request.** Quayflow never replays completed actions and never starts a hidden second process. A request that failed after side effects, such as a tool that already ran, is not retried blindly (AC-22).
- **Quayflow does not wait for capacity.** When every account is exhausted, Quayflow switches to the account available soonest (Automatic mode) without a retry, and Claude's own result for that request stands. Waiting and auto-resume are Claude's own behavior and depend on your Claude settings. Whether Claude's own wait picks up a login switched during the wait is not confirmed (see below).
- **Model and plan.** Quayflow never changes your model. An account whose plan cannot serve the current model or context is skipped. Plan capability is partly learned from `model_not_found` after a switch, which costs one failed request.
- **Idle readings are local.** An account also used on another machine may have less allowance than this machine's last reading shows. See [Several machines](user-guide.md#using-an-account-on-several-machines).

## Organization accounts

- Managed login enforcement (`forceLoginOrgUUID`, `forceLoginMethod`) is respected. Accounts it refuses are not candidates. The managed-login override setting is recorded but **not implemented** in this build (see [Security notes](security.md#managed-login-enforcement-and-the-override-setting)).
- Changing the credential selector (`CLAUDE_CONFIG_DIR`) inside a running organization session is held by Claude Code. Quayflow never works around that, so organization sessions switch only by the in-place swap, Open New Session or same-terminal Replace (Windows only).
- Enforcement of an organization policy that differs from the cached one has not been observed; that needs an organization administrator.
- Company machines often allow only signed software (next section).

## Distribution

- **Not Authenticode-signed.** Smart App Control (On) will most likely block Quayflow. AppLocker and WDAC policies that allow only signed software, or only approved paths, will block it. SmartScreen may warn about a browser download, and antivirus tools may flag it. See [Verifying a release](verify-release.md#windows-protections-and-quayflow).
- **Early releases are not attested.** v0.1.0 to v0.1.4 were built while the repository was private and are not attested: they are hash-verified and reproducible only, and install only with `-SkipAttestation`. Releases from v0.1.5 on are attested. The installer refuses files it cannot verify an attestation for, including when the GitHub CLI is missing or signed out.
- **The Mod comes from a git tag.** Claude Code adds a marketplace by tag, not by commit (`owner/repo#<commit>` is not accepted). The installer checks that the checkout is at the commit recorded in the release's `SOURCE_COMMIT` before installing the plugin, and refuses otherwise. Two gaps remain: releases made before `SOURCE_COMMIT` existed (v0.1.0) are installed without that check, with a warning; and a later marketplace update in Claude Code (`claude plugin marketplace update`, or an automatic one) fetches the tag again without this check, so a tag moved after installation can be picked up then. With `-SkipAttestation`, `SOURCE_COMMIT` is trusted as far as the release's `SHA256SUMS` is.
- **Replacing the Mod resets its saved options.** A marketplace cannot be re-added from a new source while it exists, so an upgrade removes it first; Claude Code then deletes the plugin's saved options and data. If the new source fails, the installer (which `quayflow update` runs) re-adds the previous one, but the options are not restored.
- **Matched pair.** The Mod and engine must be from compatible versions. On a mismatch, switching and recovery are disabled until you update, and Claude itself keeps working.

## PowerShell integration

- Same-terminal Replace works only in **PowerShell 7 and Windows PowerShell 5.1** with the opt-in prompt hook. In cmd, Git Bash, WSL and other shells, Quayflow opens a new window or shows the exact command.
- Windows PowerShell 5.1 does not load profiles under its default *Restricted* execution policy. The hook then needs `RemoteSigned` for your user.
- Replace runs when the prompt returns after Claude exits. If something else in your profile replaces `prompt` after Quayflow loads, the hook stops running. Quayflow then will not remove the other tool's prompt; start a new shell instead.
- The same-terminal relaunch was validated in PowerShell 7 (about 5 seconds). In Windows PowerShell 5.1 the hook passes its automated tests, but an interactive Claude session relaunched by it is a release check.

## Lanes

- Lanes link your shared Claude setup: folders with directory junctions on Windows and symbolic links on Linux and macOS, files with hard links on all three. If Claude replaces a linked file by renaming, the link breaks. `quayflow lanes check` detects and repairs this, but until then the lane sees an older copy of that file.
- Auto-memory lives in each environment's project folders. Memory written in a lane may not appear in the default environment and the other way round, unless the memory folders are linked.
- A lane is not a security boundary and gives an account no extra allowance.

## Security boundaries

- No vault (DPAPI on Windows, owner-only files on Linux and macOS) protects against malware running as your user. Such software can also read Claude's own login file. See [Security notes](security.md#the-vault-on-windows-dpapi).
- Any program running as your user can call the engine's loopback API.

## Not yet confirmed (v0.1 release conditions)

These are not confirmed yet. Each is required before v0.1 leaves the pilot ([release conditions](requirements.md#release-conditions-for-v01)):

- Claude's own in-process usage-limit wait adopting a login switched during the wait (AC-23). A job that fails when every account is exhausted is not affected: a fresh `claude -p` uses the login that is live when it starts.
- Claude reading and writing correctly through lane links on the release build. It passed on local builds on Windows (2026-11-03) and Linux (2026-11-08) (AC-12).
- Engine crash reconciliation, version gating, and dead refresh token handling against the product build (AC-28, AC-37, AC-05).
- The remaining acceptance scenarios AC-01 to AC-43 against the product build.

Recovery from a real usage limit passed on Windows, on a local build (AC-21). It has not been run on Linux or macOS; on both, recovery from a simulated limit passed (case R1). See [acceptance.md](acceptance.md).

## Recovery procedures

| Situation | Recovery |
|---|---|
| A switch failed after the write, or adoption could not be confirmed | Quayflow enters recovery and reports it. It never claims success. `quayflow status` shows the live account; `quayflow use <account>` sets it explicitly |
| The engine crashed during a switch | On restart it reads the journal and the real login, and resolves the unfinished switch without replaying it |
| Quayflow is removed or broken | Claude keeps working with whatever login is live. `claude /login` signs in natively as usual |
| An unknown login appeared | Register it or ignore it (automation stays paused while it is live). A backup is taken before any manual switch |
| An account is "set aside" | `quayflow reauth <account>` |
| A Replace failed | The original session is untouched; resume it with the command Quayflow printed |
| A `claude -p` job failed at a usage limit | `quayflow last --session <session_id> --json` says whether Quayflow recovered, switched to the account available soonest, or why not, and gives `availableAt`, the earliest time worth running again. It works with the engine stopped |
| The vault cannot be decrypted (for example after a Windows profile reset) | Quayflow refuses to switch. Remove the affected accounts and register them again with a native sign-in |
