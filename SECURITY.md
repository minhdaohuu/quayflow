# Security policy

## Supported versions

Only the latest release (currently v0.1.5) receives security fixes during the 0.x series.

## Reporting a vulnerability

Please **do not open a public issue** for a security problem. Use GitHub's private vulnerability reporting ("Report a vulnerability" on the repository's Security tab). Include the version (`quayflow version`), what you observed, and steps to reproduce. Never include real tokens or account details; redact them.

You will get an acknowledgement within 7 days. Fixes are released as soon as practical and credited in the changelog unless you prefer otherwise.

## Scope

In scope: the engine and CLI (`quayflow.exe`, Linux `quayflow`), the background launcher (`quayflow-bg.exe`), the Mod, the PowerShell integration, the installers (`install.ps1`, `install.sh`), `quayflow update` and the release pipeline. Examples: credential exposure in logs, the journal, reports or the API; a saved refresh token sent anywhere but Claude's own token service; the loopback API accepting requests without its token; a path that bypasses consent, group permissions or managed-login enforcement; tampering with release artifacts.

Out of scope: malware already running as the same user (neither DPAPI user scope nor Linux file permissions can protect against it; see `docs/security.md`), and the behavior of Claude Code itself.

See `docs/security.md` for the threat model and what the engine touches.
