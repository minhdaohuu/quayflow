# Quayflow security notes

What Quayflow v0.1 touches on your machine, how it protects saved logins, where that protection ends, and how it treats your organization's managed login. Requirement references (FR-xx) are to [requirements.md](requirements.md).

## What Quayflow touches

The data folder is `%LOCALAPPDATA%\Quayflow` on Windows and `~/.local/share/quayflow` on Linux.

| Component | Reads | Writes |
|---|---|---|
| Engine (`quayflow.exe` on Windows, `quayflow` on Linux) | Claude's login entries (`claudeAiOauth` in `<env>\.credentials.json`, `oauthAccount` in `.claude.json`); session registry `<env>\sessions\*.json`; managed policy ([below](#managed-login-enforcement-and-the-override-setting)); the Claude binary, scanned for the markers it relies on | Those two login entries, by atomic replace under Claude's own locks, leaving every other key unchanged; a session's history copied into the destination environment when you choose Open New Session or Replace (newer history is never overwritten); its own data folder |
| Engine, network | Two provider endpoints with each saved account's access token: usage (`/api/oauth/usage`) and profile (`/api/oauth/profile`) on `api.anthropic.com`; the token service (`platform.claude.com/v1/oauth/token`) with an idle account's saved refresh token, only to refresh that saved copy; GitHub Releases for `quayflow update` | Nothing else. No telemetry |
| Mod (Claude Code plugin) | Per-request results, the session's own rate-limit information, and the identity its request used (an opaque handle and a profile lookup, hashed). On Linux, an interactive session's desktop variables (display, session bus), sent to the engine at the handshake so a sign-in window opens on your desktop; the engine keeps them in memory only | Calls to the engine's loopback API only. It never receives a credential |
| Command-line tool (`quayflow`) | The data folder, release assets for `quayflow update` | The sign-in startup entry when you turn it on (`quayflow consent signin-startup on`, or the installer's startup option): the `Quayflow` value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` on Windows, the systemd user unit `~/.config/systemd/user/quayflow.service` on Linux |
| PowerShell integration (opt-in, Windows) | Its own claim file `handoff\claims\<pid>.json` | Your two PowerShell profiles (a marked block), `CLAUDE_CONFIG_DIR` only for the resumed process |
| Installer | Release assets, Claude plugin lists | Windows (`install.ps1`): `%LOCALAPPDATA%\Quayflow\bin`, user PATH, Claude plugin and marketplace registration, the startup entry with `-Startup`. Linux (`install.sh`): `~/.local/share/quayflow/bin`, the link `~/.local/bin/quayflow`, Claude plugin and marketplace registration, the systemd user unit with `--startup` |

The engine calls the operating system directly (Windows: DPAPI, file locks, process control; Linux: `/proc`, `flock`, file modes). It does not run PowerShell or other shells. It opens a window only when you ask for one: a sign-in, or Open New Session. It runs as your user without administrator rights. The command-line tool runs a shell only for `quayflow update`, which runs the release's installer with PowerShell (Windows) or `sh` (Linux). Model requests are always made by the unmodified Claude client.

## The vault on Windows (DPAPI)

Each saved login is sealed separately in `%LOCALAPPDATA%\Quayflow\vault\<account-id>.dpapi` with Windows DPAPI (`CryptProtectData`). It uses current-user scope, the Quayflow-specific entropy `quayflow-vault-v1`, and no UI (FR-21). There is no separate vault password, so unattended switching works while you are signed in.

What this protects against:

- other Windows users on the machine, and offline copies of the files, such as a backup or a copied disk without your Windows credentials;
- logins accidentally ending up in logs, reports, the Mod, command-line arguments or transcripts. They never go there.

What it does **not** protect against:

- **Malware running as your Windows user.** It can ask DPAPI to decrypt the vault just as Quayflow does, and it can read Claude's own login file directly. Nothing that runs unattended as you can prevent this.
- **Claude's own copy.** The live login sits in Claude's `.credentials.json` as Claude stores it; Quayflow does not encrypt Claude's files.
- **Anyone who has your Windows password**, or an administrator who can act as you or reset your password in a way that keeps DPAPI keys.

If DPAPI fails, Quayflow refuses to store or switch. It never falls back to plaintext. Proof-of-concept plaintext entries are sealed on first read (`quayflow migrate-poc`, with consent) and never written back as plaintext.

## The vault on Linux

On Linux each saved login is a file `~/.local/share/quayflow/vault/<account-id>.login`, readable and writable by your user only (`0600`), in a data folder only you can open (`0700`). It is **not encrypted**. This is the same protection Claude Code gives its own login on Linux (`~/.claude/.credentials.json`, `0600`), so the vault adds no weaker copy than the one Claude already keeps.

It protects against other (non-root) users of the machine. It does not protect against malware running as you, root, or an offline copy of the disk (use disk encryption for that). An encrypted vault through the desktop keyring (Secret Service) is planned; it is not used now because unattended jobs on a server usually have no unlocked keyring, and a vault that fails when nobody is signed in would break them.

Files Quayflow replaces atomically (saved logins, Claude's login files, its own settings) keep the permissions of the file they replace, or are created `0600`. A switch therefore never leaves Claude's `.credentials.json` readable by others. Other files, such as logs and the journal, are created with your umask, inside the `0700` data folder.

## Tokens and refresh

**Quayflow never refreshes a live login.** Claude refreshes the live account itself. The engine saves each refresh into the vault: before every switch it stores the live login back first, and it watches the login file. Quayflow never restores an older snapshot over a newer refresh.

**Idle accounts.** When an idle account's access token has expired, the engine refreshes the saved copy in the vault, through the same token service and public client that Claude Code uses, so that account's usage stays readable. This is safe only while the vault holds the only copy of that refresh token, because the service may rotate it. So the refresh runs under the credential lock that every switch holds while writing a login. Under that lock the engine reads every environment's login files and skips the refresh if any of them holds the same refresh token or cannot be read. The new login is stored before it is used, and a switch reads the saved login again under the lock, so it always writes the newest copy. A refused refresh token marks the account for a new sign-in. Tokens are never logged; the journal records only that an account was refreshed.

When you add an account, the temporary sign-in copy is retired **without signing out**. Signing out could revoke the refresh token the vault now holds, and two holders of one refresh token can invalidate each other. That temporary environment is never used for sessions.

## Logs, journal and reports

- **Logs** (`logs\quayflow-YYYYMMDD.jsonl`, 7 days, 20 MB per day) contain versions, timestamps, durations, opaque IDs, transition states and sanitized reasons. They exclude tokens, prompt and response content, source code, email addresses, friendly labels, raw arguments and raw provider responses (FR-22).
- **Journal** (`journal.jsonl`) is append-only and flushed to disk before each switch begins. It holds switches, recovery decisions (with the Claude session id and account ids), policy overrides, unresolved operations and active waits, with the same sanitization. If Quayflow cannot record the state before a switch, it does not switch. `quayflow last` reads it locally and adds account labels from `config.json` only to its own output.
- **Accounts in records** are identified by the first 12 hex characters of SHA-256 of the account UUID, not by email or label.
- **Support reports** (`quayflow report`) are built locally from the same sanitized data. You review them before saving and share them yourself. Nothing is uploaded.

## Loopback API

The engine listens on `127.0.0.1` on a random port only. Each run creates a random token, which it writes with the port to `engine.json` in the data folder; that folder is readable only by your user. Every request must carry the token (`x-quayflow-token`). The API has bounded operations and validates input. There is no endpoint that runs arbitrary commands, and no response carries a credential (FR-20). A repeated mutating request (same `requestId`) returns the first successful result and does not run again.

Any program running as your user can read `engine.json` and call the API, for example to trigger a switch among your registered accounts. That is the same trust boundary as the vault: it does not protect against software already running as you.

## Managed login enforcement and the override setting

> [!CAUTION]
> **The managed-login override setting works around your organization's access control.** Turning it on can violate your employment or organization policy and expose company work to non-organization accounts. **Responsibility is yours.**
>
> **Not implemented in this build.** Quayflow v0.1 records the setting and shows its state, but contains no code path that bypasses enforcement. With the setting on or off, v0.1 behaves the same way: managed login enforcement is respected.

Organizations can enforce which login Claude Code accepts, through `forceLoginOrgUUID` and `forceLoginMethod` in managed settings. Quayflow reads these from every source Claude reads:

- Windows: registry policy `HKLM\SOFTWARE\Policies\ClaudeCode` and `HKCU\SOFTWARE\Policies\ClaudeCode`;
- Windows: `C:\Program Files\ClaudeCode\managed-settings.json` and `managed-settings.d\*.json`;
- Linux: `/etc/claude-code/managed-settings.json` and `managed-settings.d/*.json`;
- server-managed settings that Claude has stored in the environment.

It checks again after each policy fetch (FR-06a).

**Default: respected.** In an environment with enforcement, accounts that do not match the enforced organization or method are not candidates. `/quay` states the reason and that your organization's administrator controls it. Switching between matching accounts of the same organization works normally.

**The override setting.** The configuration has an opt-in setting that would disable this detection, so that Quayflow would swap to accounts the organization's policy refuses at sign-in. Its rules:

- is off by default;
- can be turned on only with an explicit confirmation that names the enforcing organization and states the risk (`quayflow` API `POST /v1/policy/dorg` takes the organization and a typed acknowledgment);
- is shown in `/quay` and in the status line while on;
- is recorded in the journal every time it allows a switch the policy would have refused.

In v0.1 the confirmation, display and journal hooks exist, but the bypass itself was deliberately not written. Claude Code's restriction on changing the credential selector inside organization sessions is never worked around.

## Same-terminal replacement

Same-terminal replacement exists on Windows only (PowerShell). The PowerShell hook acts only on a claim file named for its own shell's process ID, which the engine creates when you choose Replace. It rejects expired or malformed claims, including session IDs that are not plain identifiers. It runs only `claude --resume <id>` with `Start-Process`, never through `Invoke-Expression`. Claims live in your user's data folder.

## Release integrity

Releases are reproducible, hashed and, from v0.1.5 on, attested, but **not code-signed** in v0.1 (no Authenticode signature on Windows, no signature on Linux). See [Verifying a release](verify-release.md).

The installer always checks the SHA-256 hashes. It also requires a build-provenance attestation made by this repository's release workflow (`.github/workflows/release.yml`) for the release tag and commit, verified with a signed-in GitHub CLI. If it cannot verify one, it refuses to install, unless you pass `-SkipAttestation` (`install.ps1`) or `--skip-attestation` (`install.sh`). That option leaves the hash check as the only protection. v0.1.0 to v0.1.4 were built while the repository was private. They are not attested and need that option. The Mod is added from the release tag and refused if the checkout is not at the commit the release records in `SOURCE_COMMIT`; the gaps that remain are in [Known limits](known-limits.md#distribution).

`quayflow update` downloads the release's installer (`install.ps1` on Windows, `install.sh` on Linux), checks it against `SHA256SUMS`, and runs it, so an update makes the installer's full checks. Like the installer, it refuses when the attestation cannot be verified, unless `--skip-attestation` is given.

The release workflow pins every third-party GitHub Action to a full commit SHA and checks out without keeping the job token in the git configuration.

## Reporting a vulnerability

Do not open a public issue for a security problem. Follow [SECURITY.md](../SECURITY.md): use GitHub's private vulnerability reporting, and redact tokens and account details.
