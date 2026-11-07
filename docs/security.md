# Quayflow security notes

What Quayflow v0.1 touches on your machine, how it protects saved logins, and where that protection ends. Requirement references (FR-xx) are to [requirements.md](requirements.md).

## Managed login enforcement and the override setting

> [!CAUTION]
> **The managed-login override setting works around your organization's access control.** Turning it on can violate your employment or organization policy and expose company work to non-organization accounts. **Responsibility is yours.**
>
> **Not implemented in this build.** Quayflow v0.1 records the setting and shows its state, but contains no code path that bypasses enforcement. With the setting on or off, v0.1 behaves the same way: managed login enforcement is respected.

Organizations can enforce which login Claude Code accepts, through `forceLoginOrgUUID` and `forceLoginMethod` in managed settings. Quayflow reads these from every source Claude reads:

- registry policy `HKLM\SOFTWARE\Policies\ClaudeCode` and `HKCU\SOFTWARE\Policies\ClaudeCode`;
- `C:\Program Files\ClaudeCode\managed-settings.json` and `managed-settings.d\*.json`;
- server-managed settings that Claude has stored in the environment.

It checks again after each policy fetch (FR-06a).

**Default: respected.** In an environment with enforcement, accounts that do not match the enforced organization or method are not candidates. `/quay` states the reason and that your organization's administrator controls it. Switching between matching accounts of the same organization works normally.

**The override setting.** The configuration has an opt-in setting that would disable this detection, so that Quayflow would swap to accounts the organization's policy refuses at sign-in. Its rules:

- is off by default;
- can be turned on only with an explicit confirmation that names the enforcing organization and states the risk (`quayflow` API `POST /v1/policy/dorg` takes the organization and a typed acknowledgment);
- is shown in `/quay` and in the status line while on;
- is recorded in the journal every time it allows a switch the policy would have refused.

In v0.1 the confirmation, display and journal hooks exist, but the bypass itself was deliberately not written. Claude Code's restriction on changing the credential selector inside organization sessions is never worked around.

## What Quayflow touches

| Component | Reads | Writes |
|---|---|---|
| Engine (`quayflow.exe`) | Claude's login entries (`claudeAiOauth` in `<env>\.credentials.json`, `oauthAccount` in `.claude.json`); session registry `<env>\sessions\*.json`; managed policy (above); the Claude binary, scanned for the markers it relies on | Only those two login entries, by atomic replace under Claude's own locks, leaving every other key unchanged; its own data folder `%LOCALAPPDATA%\Quayflow` |
| Engine, network | Two provider endpoints with each saved account's access token: usage (`/api/oauth/usage`) and profile (`/api/oauth/profile`) on `api.anthropic.com`; GitHub Releases for `quayflow update` | Nothing else. No telemetry |
| Mod (Claude Code plugin) | Per-request results, the session's own rate-limit information, and the identity its request used (an opaque handle and a profile lookup, hashed) | Calls to the engine's loopback API only. It never receives a credential |
| PowerShell integration (opt-in) | Its own claim file `handoff\claims\<pid>.json` | Your two PowerShell profiles (a marked block), `CLAUDE_CONFIG_DIR` only for the resumed process |
| Installer | Release assets, Claude plugin lists | `%LOCALAPPDATA%\Quayflow\bin`, user PATH, Claude plugin and marketplace registration |

The engine calls Windows APIs directly (DPAPI, file locks, process control). It does not run PowerShell or other shells, opens no console windows, and runs as your user without administrator rights. Model requests are always made by the unmodified Claude client.

## The vault (DPAPI)

Each saved login is sealed separately in `%LOCALAPPDATA%\Quayflow\vault\<account-id>.dpapi` with Windows DPAPI (`CryptProtectData`). It uses current-user scope, the Quayflow-specific entropy `quayflow-vault-v1`, and no UI (FR-21). There is no separate vault password, so unattended switching works while you are signed in.

What this protects against:

- other Windows users on the machine, and offline copies of the files, such as a backup or a copied disk without your Windows credentials;
- logins accidentally ending up in logs, reports, the Mod, command-line arguments or transcripts. They never go there.

What it does **not** protect against:

- **Malware running as your Windows user.** It can ask DPAPI to decrypt the vault just as Quayflow does, and it can read Claude's own login file directly. Nothing that runs unattended as you can prevent this.
- **Claude's own copy.** The live login sits in Claude's `.credentials.json` as Claude stores it; Quayflow does not encrypt Claude's files.
- **Anyone who has your Windows password**, or an administrator who can act as you or reset your password in a way that keeps DPAPI keys.

If DPAPI fails, Quayflow refuses to store or switch. It never falls back to plaintext. Proof-of-concept plaintext entries are sealed on first read (`quayflow migrate-poc`, with consent) and never written back as plaintext.

## Tokens and refresh

**Quayflow never refreshes a live login.** Claude refreshes the live account itself. The engine saves each refresh into the vault: before every switch it stores the live login back first, and it watches the login file. Quayflow never restores an older snapshot over a newer refresh.

**Idle accounts.** When an idle account's access token has expired, the engine refreshes the saved copy in the vault, through the same token service and public client that Claude Code uses, so that account's usage stays readable. This is safe only while the vault holds the only copy of that refresh token, because the service may rotate it. So the refresh runs under the credential lock that every switch holds while writing a login. Under that lock the engine reads every environment's login files and skips the refresh if any of them holds the same refresh token or cannot be read. The new login is stored before it is used, and a switch reads the saved login again under the lock, so it always writes the newest copy. A refused refresh token marks the account for a new sign-in. Tokens are never logged; the journal records only that an account was refreshed.

When you add an account, the temporary sign-in copy is retired **without signing out**. Signing out could revoke the refresh token the vault now holds, and two holders of one refresh token can invalidate each other. That temporary environment is never used for sessions.

## Logs, journal and reports

- **Logs** (`logs\quayflow-YYYYMMDD.jsonl`, 7 days, 20 MB per day) contain versions, timestamps, durations, opaque IDs, transition states and sanitized reasons. They exclude tokens, prompt and response content, source code, email addresses, friendly labels, raw arguments and raw provider responses (FR-22).
- **Journal** (`journal.jsonl`) is append-only and flushed to disk before each switch begins. It holds switches, recovery decisions, policy overrides, unresolved operations and active waits, with the same sanitization. If Quayflow cannot record the state before a switch, it does not switch.
- **Accounts in records** are identified by the first 12 hex characters of SHA-256 of the account UUID, not by email or label.
- **Support reports** (`quayflow report`) are built locally from the same sanitized data. You review them before saving and share them yourself. Nothing is uploaded.

## Loopback API

The engine listens on `127.0.0.1` on a random port only. Each run creates a random token, which it writes with the port to `engine.json` in the data folder; that folder is readable only by your user. Every request must carry the token (`x-quayflow-token`). The API has bounded operations and validates input. There is no endpoint that runs arbitrary commands, and no response carries a credential (FR-20). Duplicate mutating requests (same `requestId`) return the first result.

Any program running as your user can read `engine.json` and call the API, for example to trigger a switch among your registered accounts. That is the same trust boundary as the vault: it does not protect against software already running as you.

## Same-terminal replacement

The PowerShell hook acts only on a claim file named for its own shell's process ID, which the engine creates when you choose Replace. It rejects expired or malformed claims, including session IDs that are not plain identifiers. It runs only `claude --resume <id>` with `Start-Process`, never through `Invoke-Expression`. Claims live in your user's data folder.

## Release integrity

Releases are reproducible, hashed and provenance-attested, but **not Authenticode-signed** in v0.1 . See [Verifying a release](verify-release.md).

The installer always checks the SHA-256 hashes. It also requires a build-provenance attestation made by this repository's release workflow (`.github/workflows/release.yml`) for the release tag and commit, verified with a signed-in GitHub CLI; if it cannot verify one, it refuses to install, unless you pass `-SkipAttestation`, which leaves the hash check as the only protection. Private pilot releases are not attested and need that switch. The Mod is added from the release tag and refused if the checkout is not at the commit the release records in `SOURCE_COMMIT`; the gaps that remain are in [Known limits](known-limits.md#distribution).

The release workflow pins every third-party GitHub Action to a full commit SHA and checks out without keeping the job token in the git configuration.

## Reporting a vulnerability

Do not open a public issue for a security problem. Follow [SECURITY.md](../SECURITY.md): use GitHub's private vulnerability reporting, and redact tokens and account details.
