# Known limits of Quayflow v0.1

Quayflow v0.1 is a private-pilot release. This page lists what it does not do, what depends on undocumented Claude Code behavior, and what has not yet been confirmed. Requirement references (FR-xx, AC-xx) are to [requirements.md](requirements.md) and [acceptance.md](acceptance.md).

## Platform and scope

- **Windows only**, x64. macOS and Linux are planned later; the architecture does not prevent them.
- **Subscription logins only** (claude.ai sign-in). Quayflow never moves a session to API-key or other billing.
- **One Windows user.** The engine, vault and settings are per user. Nothing is shared between users or machines.
- **No telemetry, no cloud component.** Every decision is made locally, from usage readings taken on this machine.

## Depends on undocumented behavior

Quayflow relies on Claude Code behavior that was observed on Claude Code 2.1.316 but is not documented:

- the client reads the login again at its next model request, so an in-place swap takes effect without a restart;
- Claude's credential lock protocol (`.oauth_refresh.lock`, the legacy lock, the configuration lock), which Quayflow takes in Claude's order;
- the usage endpoint (`/api/oauth/usage`) and profile endpoint (`/api/oauth/profile`), and their response shapes;
- the session registry (`<env>\sessions\<pid>.json`).

Any Claude update can change these. Quayflow checks each new Claude version automatically. It disables in-place switching for a version that fails the check, or whose first real switch is not adopted, and falls back to Open/Replace with a warning (FR-24). If the usage endpoint changes or refuses, readings become stale. Stale readings never count as zero, so Quayflow stops proactive switching instead of guessing, and manual switching and confirmed-limit recovery keep working.

## Switching behavior

- **A switch moves every session of an environment.** The default environment `~/.claude` is shared by all your sessions. Use [lanes](user-guide.md#lanes) for sessions that must stay on a different account.
- **Not every client is visible.** `/quay` lists sessions from Claude's session registry. Editor extensions and other clients of the same environment may not appear, and they switch too.
- **Prompt cache is per account.** The first request after a switch reprocesses the whole conversation: slower, and it uses more allowance on long conversations.
- **Out-of-group switches are not offered.** Verification happens on each client's first request after the switch, so the destination must already be approved by the group. Pre-request verification for other cases is not built (AC-19).
- **Attribution is read after each request.** If a switch lands while a request is starting, that request may be attributed to the new account. The request itself always goes wholly to one account.
- **Idle accounts drop out of automatic switching after some hours.** Quayflow never refreshes tokens, so an idle account's access token expires after some hours. Its usage can then no longer be read, its last reading ages past the 60-minute idle freshness limit, and it stops being a candidate for automatic switching and usage-limit recovery until it is live again. Manual switching to it still works (`quayflow use <account>`), and Claude refreshes it once it is live. Alternatives are under review; see the technical specification, section 5.
- **No pre-request hold in v0.1.** The requirements allow the Mod to hold a request briefly at the request boundary to choose the switch moment (FR-11). This build does not do that. A switch lands at each client's next request as it comes. Automatic mode can still prefer a moment when no listed session is busy (`--idle-wait`, up to 60 s).
- **Headless jobs switch only in Automatic mode.** In Suggest mode a `claude -p` job never switches; a usage limit ends or pauses it as it would without Quayflow.
- **One retry per failed request.** Quayflow never replays completed actions and never starts a hidden second process. A request that failed after side effects, such as a tool that already ran, is not retried blindly (AC-22).
- **Quayflow does not wait for capacity.** When every account is exhausted, waiting and auto-resume are Claude's own behavior and depend on your Claude settings.
- **Model and plan.** Quayflow never changes your model. An account whose plan cannot serve the current model or context is skipped. Plan capability is partly learned from `model_not_found` after a switch, which costs one failed request.
- **Idle readings are local.** An account also used on another machine may have less allowance than this machine's last reading shows. See [Several machines](user-guide.md#using-an-account-on-several-machines).

## Organization accounts

- Managed login enforcement (`forceLoginOrgUUID`, `forceLoginMethod`) is respected. Accounts it refuses are not candidates. The managed-login override setting is recorded but **not implemented** in this build (see [Security notes](security.md#managed-login-enforcement-and-the-override-setting)).
- Changing the credential selector (`CLAUDE_CONFIG_DIR`) inside a running organization session is held by Claude Code. Quayflow never works around that, so organization sessions switch only by the in-place swap or by Open/Replace.
- Enforcement of an organization policy that differs from the cached one has not been observed; that needs an organization administrator.
- Company machines often allow only signed software (next section).

## Distribution

- **Not Authenticode-signed.** Smart App Control (On) will most likely block Quayflow. AppLocker and WDAC policies that allow only signed software, or only approved paths, will block it. SmartScreen may warn about a browser download, and antivirus tools may flag it. See [Verifying a release](verify-release.md#windows-protections-and-quayflow).
- **Private pilot releases are not attested.** GitHub issues artifact attestations only for public repositories (or GitHub Enterprise Cloud). Until the repository is public, releases are hash-verified and reproducible only. When the GitHub CLI is signed in, the installer refuses an unattested binary, so pilot users install with `-SkipAttestation`.
- **Mod from a private repository** needs git credentials that Claude Code can use without prompting (`gh auth setup-git`, or an SSH key in `ssh-agent`).
- **Matched pair.** The Mod and engine must be from compatible versions. On a mismatch, switching and recovery are disabled until you update, and Claude itself keeps working.

## PowerShell integration

- Same-terminal Replace works only in **PowerShell 7 and Windows PowerShell 5.1** with the opt-in prompt hook. In cmd, Git Bash, WSL and other shells, Quayflow opens a new window or shows the exact command.
- Windows PowerShell 5.1 does not load profiles under its default *Restricted* execution policy. The hook then needs `RemoteSigned` for your user.
- Replace runs when the prompt returns after Claude exits. If something else in your profile replaces `prompt` after Quayflow loads, the hook stops running. Quayflow then will not remove the other tool's prompt; start a new shell instead.
- The same-terminal relaunch was validated in PowerShell 7 (about 5 seconds). In Windows PowerShell 5.1 the hook passes its automated tests, but an interactive Claude session relaunched by it is a release check.

## Lanes

- Lanes link your shared Claude setup with directory junctions and file hard links. If Claude replaces a linked file by renaming, the link breaks. `quayflow lanes check` detects and repairs this, but until then the lane sees an older copy of that file.
- Auto-memory lives in each environment's project folders. Memory written in a lane may not appear in the default environment and the other way round, unless the memory folders are linked.
- A lane is not a security boundary and gives an account no extra allowance.

## Security boundaries

- The DPAPI vault does not protect against malware running as your Windows user. Such software can also read Claude's own login file. See [Security notes](security.md#the-vault-dpapi).
- Any program running as your user can call the engine's loopback API.

## Not yet confirmed (v0.1 release conditions)

These were not confirmed when this build was produced. Each is required before v0.1 leaves the pilot ([release conditions](requirements.md#release-conditions-for-v01)):

- A real usage-limit failure handled end to end. Recovery has been tested with a simulated limit, and Claude's own retry must not compete with Quayflow's (AC-21).
- Claude's own usage-limit wait adopting a login switched during the wait (AC-23).
- Claude reading and writing correctly through lane links (AC-12).
- Engine crash reconciliation, version gating, and dead refresh token handling against the product build (AC-28, AC-37, AC-05).
- The remaining acceptance scenarios AC-01 to AC-43 against the product build. The proof of concept demonstrated many of them, but the product build has its own results, which are recorded separately.

## Recovery procedures

| Situation | Recovery |
|---|---|
| A switch failed after the write, or adoption could not be confirmed | Quayflow enters recovery and reports it. It never claims success. `quayflow status` shows the live account; `quayflow use <account>` sets it explicitly |
| The engine crashed during a switch | On restart it reads the journal and the real login, and resolves the unfinished switch without replaying it |
| Quayflow is removed or broken | Claude keeps working with whatever login is live. `claude /login` signs in natively as usual |
| An unknown login appeared | Register it or ignore it (automation stays paused while it is live). A backup is taken before any manual switch |
| An account is "set aside" | `quayflow reauth <account>` |
| A Replace failed | The original session is untouched; resume it with the command Quayflow printed |
| The vault cannot be decrypted (for example after a Windows profile reset) | Quayflow refuses to switch. Remove the affected accounts and register them again with a native sign-in |
