# Security policy

## Supported versions

Only the latest release receives security fixes during the 0.x series.

## Reporting a vulnerability

Please **do not open a public issue** for a security problem. Use GitHub's private vulnerability reporting ("Report a vulnerability" on the repository's Security tab). Include the version (`quayflow version`), what you observed, and steps to reproduce. Never include real tokens or account details; redact them.

You will get an acknowledgement within 7 days. Fixes are released as soon as practical and credited in the changelog unless you prefer otherwise.

## Scope

In scope: the engine and CLI (`quayflow.exe`), the Mod, the PowerShell integration, the installer and the release pipeline. Examples: credential exposure in logs, reports or the API; the loopback API accepting requests without its token; a path that bypasses consent, group permissions or managed-login enforcement; tampering with release artifacts.

Out of scope: malware already running as the same Windows user (DPAPI user scope cannot protect against it; see `docs/security.md`), and the behavior of Claude Code itself.

See `docs/security.md` for the threat model and what the engine touches.
