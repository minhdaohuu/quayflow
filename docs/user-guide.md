# Quayflow user guide

Quayflow v0.3.0. This guide covers installing, setting up accounts and groups, how switching behaves in interactive sessions and headless `claude -p` jobs, lanes, and how to remove Quayflow. Quayflow runs on Windows x64, Linux x86_64 and, from v0.1.6, macOS (Apple silicon and Intel). Paths below are the Windows ones; [Linux](#linux) and [macOS](#macos) list what differs there.

Linux is not fully tested. Recovery from a real usage limit, Open New Session, an install or update from a published release, and distributions other than Ubuntu 24.04 have not been run there. macOS is not fully tested either; on an Apple silicon Mac, switching, lanes and recovery from a simulated limit passed with real accounts, while recovery from a real limit, Open New Session, Intel Macs and an install from a published release have not been run. See [acceptance results](acceptance.md).

> **Your responsibility.** You are responsible for complying with your provider's terms and your organization's rules, including rules about which accounts may process your organization's work. Use only accounts that are yours to use. Quayflow does not make sharing accounts, or getting around a provider's limits, allowed where its terms forbid it. Quayflow is not affiliated with or endorsed by Anthropic.

Contents: [Requirements](#requirements) · [Install](#install) · [First use](#first-use) · [Accounts](#accounts) · [Groups](#groups-are-data-approvals) · [Modes](#modes) · [Status line](#the-status-line) · [What a switch does](#what-a-switch-does) · [Manual switch and pin](#manual-switch-and-pin) · [Return home](#return-home) · [Usage limits](#when-a-request-hits-a-usage-limit) · [Several machines](#using-an-account-on-several-machines) · [Headless jobs](#headless-jobs-claude--p) · [Lanes](#lanes) · [Open and Replace](#open-new-session-and-replace-current-session) · [PowerShell integration](#powershell-integration) · [Unknown logins](#unknown-logins-and-other-switchers) · [Managed login](#organization-managed-login) · [Engine control](#engine-control) · [Troubleshooting](#troubleshooting) · [Testing](#testing-your-setup) · [Update](#update) · [Uninstall](#uninstall) · [Linux](#linux) · [macOS](#macos) · [Files](#files-and-folders)

## Requirements

- Windows 10 or 11, x64; or Linux x86_64 with glibc 2.35 and OpenSSL 3 or newer (Ubuntu 22.04, Debian 12, Fedora 36 and later); or macOS on Apple silicon or Intel (from v0.1.6).
- Claude Code installed and working (`claude --version`). Quayflow was validated against Claude Code 2.1.316 on Windows and 2.1.319 on Linux and macOS; other versions are checked automatically (see [Version checks](#version-checks)).
- Windows: Windows PowerShell 5.1 or PowerShell 7 for the installer. Linux and macOS: `sh`, `curl`, `sha256sum` (or `shasum` on macOS), and `python3` for the Mod step. No administrator rights.
- Claude subscription logins (claude.ai sign-in). Quayflow never switches a subscription session to API-key billing.

## Install

On Windows (PowerShell):

```powershell
irm https://github.com/minhdaohuu/quayflow/releases/latest/download/install.ps1 | iex
```

To check the script before running it, see [Verifying a release](verify-release.md).

The installer:

1. Shows what Quayflow does and asks for your consent.
2. Downloads `quayflow.exe` and `quayflow-bg.exe` and refuses to continue if their SHA-256 hashes do not match the release's `SHA256SUMS`. It also verifies the GitHub build-provenance attestation with the GitHub CLI (`gh`, installed and signed in), accepting only an attestation made by this repository's release workflow for that tag and commit, and refuses on failure. Without a signed-in `gh` it refuses before downloading, unless you pass `-SkipAttestation`.
3. Installs them to `%LOCALAPPDATA%\Quayflow\bin` and adds that folder to your user PATH (skip with `-NoPath`).
4. Adds the `quayflow` plugin marketplace pinned to the same release tag and installs the Mod (`quayflow@quayflow`, user scope). The release records the commit it was built from (`SOURCE_COMMIT`); the installer refuses the Mod if the tag's checkout is at a different commit (for example, a moved tag). If the new Mod cannot be added or is refused, the previous one is put back.
5. Records your consent for on-demand engine start. With `-Startup`, it also records consent for starting the engine when you sign in. These are recorded even when the Mod step failed; the installer then ends with an error that says what to fix.

It does not read, copy or change any Claude login. Running Claude sessions load the Mod after you restart them.

| Option | Effect |
|---|---|
| `-Version v0.1.5` | Install a specific release instead of the latest |
| `-Startup` | Also start the engine at Windows sign-in (per-user startup entry, no window) |
| `-NoPath` | Do not change your PATH |
| `-NoPlugin` | Engine only; do not touch Claude Code plugins |
| `-Yes` | Accept the consent text without a prompt (it is still printed) |
| `-Source <folder>` | Install from local files (`quayflow.exe`, `quayflow-bg.exe`, `SHA256SUMS`). The Mod is then added from the repository holding that folder, or from `-MarketplaceSource` |
| `-MarketplaceSource <source>` | Add the Mod from this plugin marketplace source instead (a local repository path, `owner/repo` or `owner/repo#tag`) |
| `-SkipAttestation` | Skip `gh attestation verify` (builds you made yourself, releases before v0.1.5, or no GitHub CLI). Without it, files whose attestation cannot be verified are refused. Hashes are still checked |
| `-Uninstall [-DeleteData]` | Remove Quayflow (see [Uninstall](#uninstall)) |

With `irm | iex` you cannot pass options. Use `& ([scriptblock]::Create((irm https://github.com/minhdaohuu/quayflow/releases/latest/download/install.ps1))) -Startup` instead. Plain `irm | iex` therefore needs the GitHub CLI signed in; without it, pass `-SkipAttestation` this way to rely on the hash check alone.

On Linux and macOS:

```sh
curl -fsSL https://github.com/minhdaohuu/quayflow/releases/latest/download/install.sh | sh
curl -fsSL https://github.com/minhdaohuu/quayflow/releases/latest/download/install.sh | sh -s -- --startup   # with options
```

`install.sh` makes the same checks and steps, with long options (`--version`, `--startup`, `--no-path`, `--no-plugin`, `--yes`, `--source`, `--marketplace-source`, `--skip-attestation`, `--uninstall [--delete-data]`). It installs to `~/.local/share/quayflow/bin` (or `$XDG_DATA_HOME/quayflow/bin`) and links `~/.local/bin/quayflow`; `--startup` adds a systemd user unit (`quayflow.service`) on Linux and a LaunchAgent (`com.quayflow.engine`) on macOS. See [Linux](#linux) and [macOS](#macos).

**GitHub CLI.** Install the [GitHub CLI](https://cli.github.com/) and sign in (`gh auth login`) before installing, so the installer can verify the attestations. Releases from v0.1.5 on are attested; v0.1.0 to v0.1.4 were built while the repository was private and are not attested, so they install only with `-SkipAttestation`. If Claude Code cannot clone the marketplace and you have no GitHub SSH key, set `CLAUDE_CODE_PLUGIN_PREFER_HTTPS=1` before installing.

## First use

Start Claude as usual (`claude`) and run `/quay`. The first time, Quayflow asks for each consent separately:

| Consent | What it allows | Can be changed with |
|---|---|---|
| Engine start | The Mod starts the engine (`quayflow.exe`; `quayflow` on Linux and macOS) in the background when a session finds it not running. Given at install | `quayflow consent engine-start on\|off` |
| Sign-in startup | The engine also starts when you sign in (a Windows startup entry, a systemd user unit on Linux, a LaunchAgent on macOS) | `quayflow consent signin-startup on\|off` |
| Credential storage | Quayflow keeps a reusable copy of each login you register, in its vault (DPAPI on Windows, owner-only files on Linux and macOS) | `quayflow consent credential-storage on\|off` |
| Terms | You confirm you are responsible for your provider's terms and your organization's rules | `quayflow consent terms on\|off` |

Onboarding happens only in interactive sessions. A headless job (`claude -p`) never shows a prompt. Every group starts in **Suggest** mode.

Quayflow creates one group, **Default**, which is empty at first.

After the consents, `/quay` shows a readiness table: engine and Mod protocol, the protected vault, the running Claude version, the static version check, whether the current login is known, and *Live adoption: on next actual switch*. None of these checks switches an account.

### The /quay pane

`/quay` opens a pane with five tabs (keys 1-5): **Overview**, **Accounts**, **Groups**, **Activity** and **Settings**. Tab and the arrow keys move between controls, a key shown on a button presses it, and Esc returns the keys to the prompt.

Under the tabs, a one-line **status strip** shows the live account, the mode (with *(paused)* when automation is paused), the project's group, the environment when it is a lane, and, when it differs, the account this session last used. A colored dot before it says whether all is well.

**Hints.** Explanations are hidden until you press **? Hints** (key `h`); press it again to hide them. Rules you must see before committing, such as the one-executor rule of Session actions and the mixed-identity approval, always show.

The **Overview** shows, top to bottom:

- anything that needs your attention (an unknown login, another switcher, a pause and its remedy), and a switch in progress;
- a card for the **live account** with its 5-hour and 7-day usage gauges (and a per-model weekly gauge when there is one), colored by the group's threshold, with the threshold marked on each gauge and a countdown to each reset (`↻ 2 h 10 min`);
- the other accounts. In the terminal they are a **fleet chart**: a 5-hour and a 7-day column per account, the threshold as a dashed line, **▼** over the live account and **★** over the recommended one, and the account names under it as buttons. Elsewhere, and when the accounts do not fit the width, they are rows with small gauges, a ★ before the recommended one, LIVE and HOME badges, and a short reason when the account is not a switch target now (see below);
- the **recommendation**: *Switch to work-02?* with **Switch** and **Why?**, or *Staying on work-01*;
- a pin, if any, with **Unpin**, and the last switch;
- the **home account's** state when you are away from it (see below);
- the number of sessions on this environment, and **Sessions**, **Actions**, **Limits** and **Group**.

Every figure is written out beside its graphic. Usage is shown only while the reading is current: otherwise the account says *reading…*, *not read yet*, *usage unknown*, or *unreadable:* and the reason, never 0%. The true-color graphics are drawn in the terminal; other surfaces show text gauges.

**Why an account is not a switch target.** Account rows (Overview and Accounts) give the reason in a word or two: *no room to gain* (it would not leave enough margin below the threshold), *usage unknown* (no current reading), *at its limit*, *not in this group*, *group conflict* (a running session's group does not allow it), *blocked by your organization* (managed login), or *plan lacks this model*. The live account, and an account whose badge already says what is wrong (SIGN IN, VAULT), get no reason. The full reason is in the account's detail and in **Why?**.

- **Why?** ("Why this account?") lists every account as *Eligible*, *Excluded* or *Unknown* (no fresh reading, never 0%), with the reason and the observation age, plus the threshold, the required margin and the cooldown (with any time left).
- **Switch** on the recommendation opens a short review (destination, scope, reason, checks, an optional **Wait for idle**) before anything is switched. Accepting does not pin. A switch shows its phases as a step track (prepared, quiet moment, apply, verify), a participant table while it waits for a quiet moment, and which clients have adopted it.
- **Home account** reads *Home account*, *Home ready*, *Home ready, return deferred (pinned)*, *Home ready, return deferred (cooldown)* or *Away from home*. **Pin** pins the live account without a switch.
- **Sessions** lists each client of the environment with its mode, group, version and the identity of its last request.
- **Limits** explains the recovery checks, shows the all-exhausted table (each account's exhausted-window resets and earliest usable time; unknown resets stay unknown), each account's endpoint state and next check, and states plainly that Claude's own wait setting and waiting state are not reported to Quayflow.

**`/quay` commands.** Every action is also a subcommand: `/quay help` lists them. An option `/quay` does not know is refused, never ignored: `/quay mode automatic --aprove` answers `unknown option --aprove (did you mean --approve?)` and changes nothing.

## Accounts

**Register the login you are using now.** In `/quay`, choose **Register login** (Accounts), or run `quayflow register --label work-01 --group Default`. Quayflow shows the identity of the live login (its e-mail address and organization, read locally for your review and never stored or logged), with a name suggested from the e-mail address (change it if you like) and the group; **+ New group** creates one there. The first time you save a login anywhere, Quayflow asks once for the credential storage consent (a box on the screen); after that, Enter on the name or **Save** saves. The identity is verified by a profile lookup when you save, and a problem (a name already in use, the box not ticked) stays on the screen until you fix it. Registering does not switch anything, does not change your running work, and does not turn on Automatic mode.

**Add another account.** In `/quay` choose Add account (or run `/quay add`), or run `quayflow add --label work-02 --group Default`. You sign in first and name the account after: pick the group (or **+ New group**), tick the credential storage box if Quayflow has never asked for it, and choose **Sign in with Claude**. A new window opens with Claude's own sign-in (`claude auth login`) in a temporary environment; Quayflow never asks you to paste a token, and the pane follows the sign-in by itself. If you are signed in to an account Quayflow does not know yet, the screen also offers to register it. After you sign in, the pane shows the account (its e-mail address, for your review only) with a name suggested from it; change the name if you like, and press Enter or **Save**. If the identity is already registered, the screen says so, and **Update** refreshes that registration under its existing name; this never counts as extra capacity. Saving retires the temporary sign-in copy without signing out, because signing out could revoke the token Quayflow just saved. Cancelling or failing the sign-in leaves your current login unchanged. With `quayflow add` you give the name and group first.

**Other account actions:**

- `quayflow label <account> <new label>` renames an account.
- `quayflow remove <account>` deletes Quayflow's saved copy. Your conversation history is not touched, and the provider-side authorization is not revoked. An account that is live right now cannot be removed.
- `quayflow reauth <account>` signs in again. Use it when an account's refresh token stops working: Quayflow sets that account aside and never retries it silently. The new sign-in must be the same identity. If it is a different one, nothing is replaced; you can save it as a new account with its own name and group.
- Registering the same identity twice updates the registration; it never counts as extra capacity.

## Groups are data approvals

When Quayflow switches accounts, the following requests of your conversation go to the new account. A **group** is your approval for that. Putting accounts in one group approves processing the work of any project that uses that group under any of those accounts. Switching within a group then needs no further confirmation.

- **Default group.** A project without an explicit assignment uses the designated default group. The default is a fallback, not a union of all your accounts. Adding an account to Default makes it eligible for every unassigned project, including projects you open in the future.
- **Project assignment.** `quayflow project assign C:\src\client-x ClientX` (Linux and macOS: `~/src/client-x`) makes that folder use group `ClientX` instead of the default. An explicit assignment replaces the default; it does not merge with it. `quayflow project list` shows the assignments and `quayflow project unassign <path>` removes one. Content inside a repository can never grant itself accounts.
- **Mixed identities.** If a change would put an organization account and a personal account in one group, or accounts of different organizations, Quayflow shows a one-time disclosure first: work in projects using this group may be sent to any of these accounts. Your acceptance is recorded with the group. Removing an account from a group withdraws the approval for future switches. Whether your organization allows its work to be processed by other accounts is your responsibility.
- **Home and ranks.** One account per group can be **Preferred**: it is the group's home (see [Return home](#return-home)). Accounts can also be ranked Preferred, Normal or Backup. Ranks only break close choices.
- **Commands:** `quayflow group list | create <G> | delete <G> | add <G> <account..> | remove <G> <account..> | preferred <G> <account|none> | rank <G> <account> <rank> | default <G> | tuning <G> --threshold N --margin N`.

**Conflicts between running sessions.** A switch moves every session of the environment (see below), so the destination must be allowed by the group of every running session. If one running session's project does not allow it, Quayflow does not switch. It names the blocking session and offers to move that session into a [lane](#lanes) so that the others can switch. Automatic mode never moves a session into a lane on its own.

## Modes

Each group has a mode. When several groups govern an environment, the most restrictive mode applies.

| Mode | Interactive sessions | Headless jobs (`claude -p`) |
|---|---|---|
| Manual | Only switches you ask for | No switching |
| **Suggest** (default) | Shows a suggested account in the status line; `/quay accept` switches | Records a suggestion; no prompt, no pause, no switch |
| Automatic (opt-in per group) | Switches by itself under the group's rules | Switches, returns home and recovers from usage limits |

**Unattended headless jobs need Automatic mode on their group.** In Suggest mode a headless job never switches, and a usage limit ends or pauses it exactly as Claude would without Quayflow.

Turning on Automatic needs explicit approval: `quayflow mode automatic --group Default --approve` (or the Groups tab in `/quay`).

Defaults per group (Groups tab, or `quayflow group tuning`): proactive threshold 85% of the account's binding usage window; a candidate must be at least 10 points below the threshold. For every group (`/quay` > Settings > Tuning): 5 minutes between proactive switches. The binding window is the fullest of the five-hour, seven-day and per-model weekly windows. Quayflow uses only fresh usage readings: at most 5 minutes old for the live account and 60 minutes for idle ones. A missing or stale reading is never treated as zero and never triggers an automatic switch.

**Idle accounts and expired tokens.** Claude refreshes the live account's login; Quayflow never refreshes the live login. When an idle account's access token expires, Quayflow refreshes that account's saved copy (with Claude's own token service and the login's own scopes) and reads its usage with the new token, so the account stays a candidate for suggestions, automatic switching and recovery. The refresh runs under the same credential lock every switch holds, and only when no environment's login uses the same refresh token; the new login is stored before it is used. If the sign-in service refuses an idle account's saved login, the account is marked for a new sign-in (**Sign in again** in its detail, or `quayflow reauth <account>`).

**Stale readings are never shown as current.** A usage figure is shown only while it is fresh. Otherwise `quayflow status` prints `usage unknown (last read 25 min ago)`, and `/quay` says *reading…* or *usage unknown*, or why the account cannot be read.

**How often usage is read.** The live account is read every 60 seconds and idle accounts every 10 minutes. The usage endpoint's budget is per login and shared with Claude Code, so after it refuses a read (HTTP 429), the live account is read every 2 minutes for the next hour, with a longer backoff when refusals repeat. A refresh from `/quay` (`/quay poll`, or the pane asking for a fresh read) reads at once, except during that slower pace or a backoff: then it waits for the next scheduled read instead of forcing one.

## The status line

In interactive sessions Quayflow keeps one line under the prompt:

```
Quayflow · <account> · <mode> · <status>
```

In the terminal, the everyday line (usage, home or away, a pin, a switch under way) is drawn dim at the end of the hint line under the prompt. A line that needs you is pinned as a status of its own, which Claude Code draws as a warning: a suggestion to accept, a sign-in or registration needed, automation paused, another switcher, a problem notice, setup not finished, or the engine off. Other surfaces show every line as a pinned status.

`<account>` is the live account of this session's environment (`Unknown login` or `Signed out` when there is no registered one), `<mode>` is Suggest, Manual or Automatic. `<status>` is the single most important thing to know right now, in this order:

| Priority | Example `<status>` | Meaning |
|---|---|---|
| 1. Problem | `mixed login files: switch to an account to repair`, `in-place switching off for this Claude version`, `a session did not adopt the switch: /quay` | Quayflow cannot switch safely here until this is fixed |
| 2. Live login | `not registered: /quay register`, `sign in to Claude (/login)`, `another switcher may be active`, `automation paused (/quay resume)` | The login is not one Quayflow knows, or automation is paused |
| 3. Switch | `switching to work-02`, `pending next request` | A switch is in progress, or this session adopts it at its next request |
| 4. Suggestion | `5h 91% · 7d 40% \| work-02 suggested` | The live account's usage, and the account `/quay accept` would switch to |
| 5. Pin | `pinned until 16:32` | A manual switch holds this account until then |
| 6. Override | `managed-login override ON` | The managed-login override setting is on (see [Security notes](security.md#managed-login-enforcement-and-the-override-setting)) |
| 7. Warning | `work-02 needs a new sign-in` | An account needs attention |
| 8. Usage unknown | `usage unknown (read 25 min ago)` | The live account's usage reading is too old to act on |
| 9. Home | `Home account · 5h 45% · 7d 30%`, `Away from home · 5h 45% · 7d 30%` | Whether the live account is the group's Preferred account (only when the group has one), with its usage when there is room |
| 10. Usage | `5h 45% · 7d 30%` | The live account's usage |

The usage figures are the 5-hour and 7-day windows (`5h 45% · 7d 30%`); a missing window is left out. When a per-model weekly window applies to the session's model (its family word, such as Opus, appears in the model id) and is the fullest window, it is added: `5h 45% · 7d 30% · Opus 7d 88%`.

Informational notices (for example that your organization enforces its login) are not shown here; `/quay` lists them. The line is kept to the terminal's width (80 characters until `/quay` has measured it): usage figures are dropped first (the per-model one before the others), then the status is shortened; the account name and mode never are. When Quayflow is off the line says why, for example `Quayflow · off · engine not running (Claude works as usual)` or `Quayflow · not set up · run /quay to review and start`.

**Usage figures shown once.** If your own Claude Code status line (the `statusLine` setting) already shows the 5-hour / 7-day limits, Quayflow leaves all its usage figures out, so they are not shown twice: a suggestion then reads `Personal suggested` and the home text stands alone. Quayflow decides this by reading the `statusLine` setting as Claude Code merges it (user, project, local and managed settings) and, when its command names a script file, that file's text, looking for the rate-limit fields (`rate_limits`, `five_hour`, `seven_day`). It never runs the command. The check is repeated every few minutes. To override it, set the plugin option **Status line usage figures** (`statusLineUsage`) in `/config`: `auto` (default), `show` (always show the figures) or `hide` (never show them).

**Transcript notes.** Besides the status line, Quayflow writes a short note into the transcript once for each event of this session's environment: a suggestion (also as a toast), a switch, a session adopting a switch, and notices such as a login changed outside Quayflow, a sign-out, a switch back after `model_not_found`, or the outcome of an interrupted switch after a restart. Each note appears once per session, events from before the session started are not replayed, and headless runs (`claude -p`) never get them.

## What a switch does

- A switch targets a **credential environment**, not a terminal window. By default this is `~/.claude`, which all your Claude sessions share. **Every session using it moves to the new account at its next request.** `/quay` lists the running sessions it can see. Other clients, such as editor extensions, may also use the environment and may not be listed.
- A request already in progress finishes on the old account. The next request, including a subagent's, goes to the new one. Nothing is restarted, no tool is interrupted and nothing is replayed.
- Quayflow checks the account each session's first request actually used and shows it per session. Until then the session shows *Pending next request*. A mismatch is reported and recorded, and it disables in-place switching for that Claude version.
- **Prompt-cache cost.** Claude's prompt cache is not shared between accounts. The first request after a switch processes the whole conversation again, which is slower and uses more of the new account's allowance on long conversations. Quayflow avoids unnecessary switches for this reason: a healthy job stays on its account, and there is a cooldown and a margin.
- Quayflow never changes the model. An account whose plan cannot serve the session's model or context is not a candidate. If the first request after a switch fails with `model_not_found`, Quayflow marks that model unavailable for that account and switches back.

## Manual switch and pin

`/quay use <account>` or `quayflow use <account>` switches now. Add `--idle-wait` to wait up to 60 seconds for a moment when no listed session is busy. A manual switch **pins** the account: no suggestion or automatic switch moves away from it, including return home. The pin ends when you run `/quay unpin` (or `quayflow unpin`), when the pinned account's binding window resets, or after 2 hours. When the pinned account reaches 98% of a window, normal Suggest or Automatic behavior resumes.

`/quay accept` (or `quayflow accept`) takes the current suggestion. A confirmed hard-limit recovery ignores the pin and the cooldown, never permissions or safety checks. The pin length (2 hours by default) and the proactive cooldown (5 minutes by default) are set in `/quay` > Settings > Tuning, for every group.

## Return home

When Quayflow has moved away from a group's Preferred account and that account's window has reset, Automatic mode switches back and Suggest mode suggests it. "Reset" means a fresh reading below the threshold minus the margin. Pin, cooldown, plan compatibility and group conflicts still apply. Without a Preferred account, Quayflow stays on the current account until something triggers a switch. To go back now, open the group in `/quay` and choose **Switch home**: it opens the switch review for the Preferred account.

## When a request hits a usage limit

When a request ends with no response, the Mod asks the engine whether the account's usage limit was reached. It checks this from the limits Claude itself reported, or from a fresh engine reading at 99% or more. A generic error is not a usage limit: Quayflow then does nothing and Claude's own result stands.

- In **Automatic** mode Quayflow switches to the eligible account in the group with the most remaining allowance and retries the same request **once**, in the same process. When several sessions fail at once, they share one switch.
- In **Suggest** mode it records a suggestion and does not switch.
- **When every account in the group is exhausted**, Quayflow switches to the account that becomes available soonest, then leaves the rest to Claude. Claude reports the limit, waits, and resumes automatically if your Claude settings allow it; otherwise the job ends with Claude's own result. Quayflow adds no wait and no time limit of its own. If another account frees up first, Automatic mode switches to it. (Whether Claude's waiting client picks up a switch made during the wait is a condition for 1.0 still being confirmed. A new `claude -p` started after the wait does pick it up; see [Headless jobs](#headless-jobs-claude--p).)

Limits: one switch and at most one retry per failed request. Cancellation is never recovered. Quayflow never replays completed actions (edits, commits, messages, deployments) and never starts a second process for a job that has ended.

## Using an account on several machines

The same account used on several machines shares one allowance and one usage-reading rate limit. Each machine's Quayflow reads usage on its own schedule: about once a minute for the live account and every ten minutes for idle ones, slowing down and backing off when the usage endpoint refuses a read (see [Modes](#modes)). An idle account's reading from this machine does not include what another machine has used since, so it is never treated as guaranteed capacity. Its age is shown, and a usage-limit failure on the new account is handled as normal recovery.

## Headless jobs (`claude -p`)

Quayflow works with unattended `claude -p` jobs (scheduled tasks, scripts, agent pipelines) on your own machine, with no change to how you run them.

**Setup.** Put the group your jobs' projects belong to in **Automatic** (`quayflow mode automatic --group <G> --approve`, a one-time approval). In Suggest mode nobody is there to accept a suggestion, so a job that hits a limit simply ends with Claude's own result.

**Output is never changed.** Quayflow writes nothing to a headless run's stdout or stderr and never changes its exit code. What a job sees:

| Situation | What `claude -p` returns |
|---|---|
| No limit reached | Claude's normal result |
| Limit reached, another approved account has room (Automatic) | Claude's normal result: Quayflow switched and retried the request once inside the run (a few seconds longer) |
| Every approved account at its limit | Claude's own limit result (for example `is_error: true`, exit code 1, "You've hit your weekly limit · resets 11am"). Quayflow switched to the account available soonest; the reset time in Claude's message is the failed account's, which is never earlier |
| Suggest or Manual mode, or Quayflow off | Claude's own result, as without Quayflow |

**A job that waits and runs again** (for example after reading the reset time from Claude's message) needs nothing from Quayflow: a new `claude -p` process reads the login when it starts, so it runs on whichever account Quayflow left live.

**Why did a run fail, and when to run again.** Pass the `session_id` from Claude's JSON output:

```sh
quayflow last --session <session_id> --json
```

```json
{
  "session": "c35454cf-…", "found": true, "outcome": "all_exhausted",
  "limit": { "account": "sdk", "window": "7d", "pct": 100, "resetsAt": "2026-11-08T04:00:00Z" },
  "switchedTo": "personal", "retried": false,
  "availableAt": "2026-11-08T01:00:00Z",
  "why": "every approved account is at its limit; personal is available soonest (…)"
}
```

- `outcome`: `recovered` (switched and retried; the run answered), `all_exhausted` (switched to the account available soonest, no retry), `not_recovered` (a confirmed limit without a recovery: Suggest or Manual mode, a group conflict, a pause, or a retry that failed too; `why` says which), or `no_limit` (nothing recorded for the session).
- `availableAt`: the earliest time worth running again, when the account now live frees up. It can be earlier than the reset in Claude's message.
- Exit code 0 when the session is found, 3 when nothing is recorded for it. It reads the engine's journal, so it also works while the engine is stopped. Account names appear only in this local output; logs keep ids.

**Your interactive sessions move too.** A switch changes the login of the whole environment, so a recovery made for a job also moves Claude windows open in the same environment, at their next request (a request in progress finishes on its account). The strictest mode among all running sessions applies: an interactive session in a Suggest group keeps a job in that environment from recovering while it runs.

## Lanes

A **lane** is an optional separate credential environment, for sessions that need a different account from the rest. The most common reason is the remedy for a group conflict.

- A lane has its own login, its own `.claude.json`, its own session folder and its own organization policy files.
- It links your shared Claude setup from `~/.claude`: `settings.json`, `CLAUDE.md`, `plugins`, `skills`, `agents`, `commands` and the memory folders. Folders use junctions on Windows and symbolic links on Linux and macOS; files use hard links. No administrator rights are needed. Inside a lane, Claude behaves like your normal Claude, including the Quayflow Mod, except for the account. `quayflow lanes check <name>` repairs a broken file link.
- A switch inside a lane moves only that lane's sessions. A lane does not give an account extra allowance and is not a security sandbox.
- In-place switching inside a lane is enabled only after the lane passes the version check.
- Commands: `quayflow lanes list | create <name> | delete <name> | check <name>`.

Starting a session in a lane, or moving one into it, is always explicit. See the next section.

## Open New Session and Replace Current Session

Moving a session into or between lanes, and the fallback where in-place switching is disabled for a Claude version, use one of two actions. Both carry **full history**: the transcript and its side files, copied with integrity checks. A stale copy never overwrites newer history, and the original is kept.

- **Open New Session** opens a new console window that resumes the session in the destination environment. Where a window cannot be opened, Quayflow shows the exact command to run.
- **Replace Current Session** ends the current session and resumes it in the same terminal under the destination. This needs the PowerShell integration below. In other shells (cmd, Git Bash, editor terminals without it), and on Linux and macOS, Quayflow offers Open New Session instead.

In `/quay` choose **Actions** on the Overview (or **Session actions** under Lanes in the Groups tab): Open New Session is preselected. Each action has a review that names the destination (environment and login), the context (full retained history) and the window, and asks you to confirm that only the destination continues the task. Replace names the shell first: in a shell other than PowerShell 7 or Windows PowerShell 5.1 it says same-terminal replacement is unavailable before anything is prepared, and offers a new window or the engine's prepared command. Without the prompt hook, a separate box installs it (it loads in PowerShell windows opened afterwards). A group conflict offers **Move to lane**, which names the restricted session and the destination lane; the move is made from that session.

Replace prepares the destination before the source exits. If the transfer fails, the original session is left as it was and the failure is reported. Quayflow never silently starts a fresh session or substitutes a summary. Only one copy of a task ever runs.

CLI: `quayflow handoff open|replace|prepare --session <id> --to <env>`.

## PowerShell integration

The integration is opt-in and Windows only, for PowerShell 7 and Windows PowerShell 5.1 (Linux and macOS have no same-terminal Replace yet):

```powershell
quayflow shell install     # adds a marked block to both PowerShell profiles
quayflow shell uninstall   # removes the block and the module
quayflow shell refresh     # rewrites an installed module from this version (updates do this)
```

`shell install` writes `%LOCALAPPDATA%\Quayflow\shell\Quayflow.psm1` and adds this block to your PowerShell 7 and Windows PowerShell 5.1 profiles:

```powershell
# >>> quayflow >>>
# Managed by `quayflow shell install`; removed by `quayflow shell uninstall`. Do not edit inside this block.
$QuayflowModule = Join-Path $env:LOCALAPPDATA 'Quayflow\shell\Quayflow.psm1'
if (Test-Path -LiteralPath $QuayflowModule) { Import-Module $QuayflowModule -DisableNameChecking; Enable-QuayflowPromptHook }
Remove-Variable QuayflowModule -ErrorAction SilentlyContinue
# <<< quayflow <<<
```

It wraps your `prompt` function and keeps your own prompt as it is. It does not redefine `claude`. When you choose Replace in a session started from this terminal, Quayflow leaves a one-time request for this shell; you then exit Claude. At the next prompt the hook takes the request and resumes the same session with `claude --resume <id>` in this terminal and folder. For a lane it sets `CLAUDE_CONFIG_DIR` to the lane; for the default environment it removes the variable. Afterwards it restores the variable to its previous value. Requests expire after a few minutes and stale ones are ignored. If the history is not ready, nothing starts and the exact command is shown.

For one shell only: `Disable-QuayflowPromptHook`, and `Enable-QuayflowPromptHook` to turn it back on.

**Windows PowerShell 5.1 and execution policy.** On Windows client editions, Windows PowerShell 5.1's default execution policy is *Restricted*, which stops profiles from loading at all. If `Get-ExecutionPolicy` shows Restricted or AllSigned, run `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned`, or use PowerShell 7, whose default is RemoteSigned. Check your organization's policy before changing this. The module file is written locally, so RemoteSigned is enough.

## Unknown logins and other switchers

If the live login of an environment is not one you registered, Quayflow pauses automatic switching and recovery for that environment and tells you. This happens, for example, after running `/login` yourself or signing out. You can register the login (`/quay register`) or ignore it (`quayflow unknown ignore`). Ignoring acknowledges the notice, and automation stays paused while that login is live. A manual switch is still possible, but Quayflow first stores a protected backup of the unknown login so it is never lost.

If the login keeps changing without Quayflow, another account-switching tool is probably working on the same environment. Quayflow warns you and pauses automation. Do not run two switchers on one environment.

## Organization-managed login

If your organization enforces its login (`forceLoginOrgUUID` or `forceLoginMethod` in managed settings), Quayflow respects it. Accounts that do not match are not candidates in that environment, and `/quay` shows the reason. Switching between matching accounts of the same organization works normally. `quayflow policy` shows the enforcement per environment. See [Security notes](security.md#managed-login-enforcement-and-the-override-setting) about the related setting.

## Engine control

- `quayflow status` shows environments, accounts, usage and observation age (`--json` for scripts).
- `quayflow pause` and `quayflow resume` pause and resume automation. Manual switches keep working. Resume ends only a pause you made (or a hold left by an unresolved switch). When automation is paused for another reason (an unknown login, another switcher, a signed-out or inconsistent login, an incompatible Claude version), `/quay` names that reason and its remedy instead of offering Resume.
- `quayflow stop` stops the engine on purpose. The Mod then does not restart it until you run `quayflow start`. Stopping the engine never stops Claude: without the engine, Claude works exactly as it would without Quayflow.
- `/quay` Activity shows recent decisions and the last switches; open one for its local trail (opaque ids and sanitized reasons). **Report**, then **Build preview**, builds the sanitized report without saving it; you review it, tick *I reviewed this report before saving it for sharing*, then **Save locally**. `/quay report` prints the preview without saving (`/quay report --save` saves), and `quayflow report --preview` does the same in a terminal; `quayflow report` saves at once. Nothing is sent automatically.

### Version checks

When the engine sees a new Claude version, it checks automatically that the parts it relies on are still present. If the check fails, in-place switching is disabled for that version, and Quayflow uses Open/Replace instead with a warning. The first real switch on a new version is verified per session as described above. `quayflow canary --live` runs an optional synthetic check, with your consent; `/quay` > Settings > **Version** shows what it does, its consent and its status before you run it. When several Claude versions run at once, the oldest one decides.

## Troubleshooting

| Symptom | What to do |
|---|---|
| `/quay` is unknown | Restart Claude. Check `claude plugin list` shows `quayflow@quayflow` enabled. Re-run the installer |
| Status line says the engine is not running | `quayflow start`. If you stopped it on purpose, the Mod does not restart it |
| "update needed: incompatible with engine" in the status line | The Mod and engine versions do not match. Run `quayflow update`. Meanwhile Claude works normally without switching |
| Installer: `REFUSED: hash mismatch` | The download is not the published file. Do not bypass it. Download again; if it repeats, report it |
| Installer: attestation failed | The file was not built by this repository's release workflow. Do not install it. Releases before v0.1.5 are not attested; see [Install](#install) |
| `claude plugin marketplace add` fails | Without a GitHub SSH key set `CLAUDE_CODE_PLUGIN_PREFER_HTTPS=1`, then run the installer again |
| Windows blocks `quayflow.exe` (SmartScreen, Smart App Control, company policy) or antivirus flags it | See [Verifying a release](verify-release.md#windows-protections-and-quayflow) |
| `/quay` shows "Connecting to the engine…" | Wait a few seconds: the Mod is shaking hands, or starting the engine. If it stays, run `quayflow status`, then `/quay start` |
| `/quay` says "unknown option" | A mistyped option is refused, never ignored. Use the suggested one (`--approve`, `--accept-disclosure`, `--idle-wait`, `--save`) |
| No switch happens near the limit | Check the mode (Suggest only suggests), a pin, the cooldown, group conflicts (`/quay` names the blocking session), stale readings, or a paused engine. Each account row in `/quay` says in a word or two why it is not a target; **Why?** gives the full reason |
| An idle account is never chosen | Look at its row in `/quay`. *usage unknown*: its reading is not current yet or cannot be read (its detail says why). SIGN IN: its saved login was refused; run `quayflow reauth <account>`. See [Modes](#modes) |
| Headless job stopped at a usage limit | Its group is not in Automatic mode, or every account was exhausted and Claude's own wait was not enabled. `quayflow last --session <session_id>` says which, and when to run again (see [Headless jobs](#headless-jobs-claude--p)) |
| An account shows SIGN IN or "set aside" | Its refresh token was refused. Run `quayflow reauth <account>`, or **Sign in again** in its detail in `/quay` |
| "Unknown login" notice | See [Unknown logins](#unknown-logins-and-other-switchers) |
| Replace did nothing in this terminal | The PowerShell hook is not loaded: `quayflow shell install`, open a new terminal, check the execution policy (5.1). Other shells, and Linux and macOS, use Open New Session |
| Replace said history is not ready | Run the command it printed once Claude has fully exited |

Logs are in `%LOCALAPPDATA%\Quayflow\logs` (Linux and macOS: `~/.local/share/quayflow/logs`, or under `$XDG_DATA_HOME/quayflow` when that is set), sanitized and kept 7 days.

## Testing your setup

The repository's `tests/e2e/` folder holds scripts that exercise the product build; [tests/e2e/README.md](../tests/e2e/README.md) lists them all and what each needs. Read each one before you run it.

- **Fake environment (safe anywhere).** `smoke-fake-env.ps1`, `cli-fake-env.ps1`, `sim-usage.ps1` and, on Linux and macOS, `smoke-unix.sh` run the real program against a temporary fake Claude environment. They never touch `~/.claude` or a real login. CI runs them on every change.
- **Real logins (owner-run).** `real-switching.ps1`, `real-lanes.ps1` and `real-headless.ps1` **switch your live Claude login** and put it back; on macOS, `real-macos-keychain.py` checks the Keychain switch in a throwaway environment, with accounts you sign in during the run. Run them only on your own machine, never in CI. Recorded results are in [acceptance.md](acceptance.md).

## Update

```sh
quayflow update --check   # is a newer release available?
quayflow update           # install it with the release's own installer and all its checks
quayflow update --version v0.1.5   # a specific release instead of the latest
```

`quayflow update` downloads the release's installer (`install.ps1`, or `install.sh` on Linux and macOS), checks it against the release's `SHA256SUMS`, and runs it in update mode. The installer then makes every check of a first install: the SHA-256 hashes, the build-provenance attestation of the release workflow for that tag and commit, and the Mod's source commit. The attestation check needs the GitHub CLI, signed in; without it the update is refused, unless you add `--skip-attestation` to accept the hashes alone (needed for releases before v0.1.5).

An update never runs in the middle of a switch. It stops the engine, replaces the programs, refreshes the PowerShell prompt hook module if you installed it, moves the Mod to the same release, and restarts the engine if it was running. Your consents, PATH and startup setting are kept. A Mod installed from a local folder (a development checkout) is left as it is, with a note. `quayflow update --source <folder>` installs a local release folder (`SHA256SUMS`, the installer and the programs) and leaves the Mod as it is. Re-running the installer also upgrades. Restart Claude sessions to load the new Mod.

## Uninstall

```powershell
quayflow uninstall                  # keeps vault, configuration and logs
quayflow uninstall --delete-vault   # also deletes the protected vault copies
quayflow uninstall --delete-logs    # also deletes routine logs and saved reports
quayflow uninstall --delete-data    # deletes the vault, logs, settings and lanes
```

Each deletion is a separate choice and off by default (`/quay` > Settings > **Uninstall** shows them and the matching command). An unresolved recovery record in the journal is kept and reported. Or use `install.ps1 -Uninstall [-DeleteData]` (Linux and macOS: `install.sh --uninstall [--delete-data]`). Uninstall removes the Mod, the sign-in startup entry, the PowerShell profile block, the engine and the PATH entry (Linux and macOS: the `~/.local/bin/quayflow` link). **The login that is live at that moment stays live**, so your current account keeps working in Claude. Conversation history is never deleted.

## Linux

Quayflow works the same way on Linux, with these differences:

| Topic | Windows | Linux |
|---|---|---|
| Install | `install.ps1` | `install.sh` (same checks; long options) |
| Program and data | `%LOCALAPPDATA%\Quayflow` | `~/.local/share/quayflow` (or `$XDG_DATA_HOME/quayflow`), folder `0700`; program linked as `~/.local/bin/quayflow` |
| Saved logins | Encrypted with DPAPI for your user | Owner-only files (`vault/<id>.login`, `0600`), not encrypted: the same protection Claude Code gives its own `~/.claude/.credentials.json` on Linux |
| Engine start at sign-in | A per-user Run entry | A systemd user unit, `~/.config/systemd/user/quayflow.service` (`loginctl enable-linger` to start it at boot) |
| Sign-in window, Open New Session | A new console window | A new window of your desktop's terminal (`x-terminal-emulator`, GNOME Terminal, Konsole, xterm), on the desktop of the Claude session that asked, even when the engine started over SSH; without a desktop, the exact command to run in another terminal |
| Lane links | Junctions for folders, hard links for files | Symbolic links for folders, hard links for files |
| Replace Current Session | PowerShell prompt hook | Not yet: use Open New Session, or run the prepared command after `/exit` |
| Update | `quayflow update` runs `install.ps1 -Update` | `quayflow update` runs `install.sh --update` |

If `quayflow` or `claude` is not found right after installing, `~/.local/bin` was created after you signed in: open a new login shell, or run `source ~/.profile`.

## macOS

macOS (Apple silicon and Intel) is supported from v0.1.6. It works like Linux, with these differences:

| Topic | Linux | macOS |
|---|---|---|
| Program | `quayflow-linux-x86_64` | `quayflow-macos-arm64` (Apple silicon) or `quayflow-macos-x86_64` (Intel), in the same folders as on Linux |
| Claude's own login | `~/.claude/.credentials.json` | The Keychain item `Claude Code-credentials`; with `CLAUDE_CONFIG_DIR` set, `Claude Code-credentials-` and 8 characters derived from the folder name |
| A switch | Rewrites the login file | Rewrites the Keychain item through `/usr/bin/security` (the login never appears on a command line), keeps the item's other entries such as MCP logins, and leaves an owner-only copy in `.credentials.json` for about 35 seconds. The copy makes every running session use the new account from its next request; without it a session can take up to 30 seconds, and a retry after a usage limit would still go to the old account |
| Saved logins | Owner-only files | Owner-only files, as on Linux (not in the Keychain) |
| Engine start at sign-in | systemd user unit | LaunchAgent `~/Library/LaunchAgents/com.quayflow.engine.plist`. macOS starts it some seconds after you sign in and shows a one-time "App Background Activity" notice (System Settings > General > Login Items & Extensions). The engine must run in your login session to use the Keychain: it cannot switch logins over SSH or as a system service |
| Sign-in window, Open New Session | Your desktop's terminal | A new Terminal window |
| Replace Current Session | Not yet | Not yet |

An environment that has no Keychain item yet (a new lane, or a Mac where Claude Code keeps its login in `.credentials.json` because the Keychain refused it) is switched through `.credentials.json`, as on Linux. Claude Code moves it into the Keychain at its next save; deleting a lane removes that item too.

Claude names the Keychain item after the exact text of `CLAUDE_CONFIG_DIR`. `~/work/claude` and `~/work/claude/` are two different items, and setting the variable to `~/.claude` is not the same as leaving it unset. A session started with another spelling than the one Quayflow uses does not follow a switch; `/quay` shows a notice naming the value to start it with.

## Files and folders

Everything lives in `%LOCALAPPDATA%\Quayflow` (Linux and macOS: `~/.local/share/quayflow`, with `/` paths):

| Path | Content |
|---|---|
| `bin\` | `quayflow.exe`, `quayflow-bg.exe` (Linux and macOS: `quayflow`) |
| `config.json` | Groups, projects, modes, tuning, consents. No credentials |
| `vault\` | Saved logins: encrypted with DPAPI for your Windows user; on Linux and macOS owner-only files |
| `backups\` | Protected backups of unknown logins taken before a manual switch |
| `observations.json` | Last usage readings with their times |
| `journal.jsonl` | Switches, recovery decisions, policy overrides, unresolved operations |
| `logs\` | Sanitized logs, 7 days |
| `lanes\<name>\` | Lane environments |
| `handoff\` | Pending same-terminal replacements and history backups |
| `shell\Quayflow.psm1` | PowerShell integration (Windows), when installed |
| `signin\` | Temporary environments for **Sign in with Claude**, removed after the sign-in |
| `reports\` | Support reports you saved |

Quayflow changes only the login entries (`claudeAiOauth` in the login store, which is `.credentials.json` or, on macOS, Claude's Keychain item; `oauthAccount` in `.claude.json`) of the environment it switches. It leaves every other setting alone.
