# Installers: rules

- **Verify before anything runs or is moved into place**: hash and attestation (signer workflow, tag, source commit). This applies to everything downloaded and executed, including an installer run by `quayflow update`. A negative test (tampered file, failing attestation) accompanies each check: `tests/e2e/smoke-unix.sh`, `installer/test-install.sh`.
- `install.sh` runs under dash, bash and macOS `/bin/sh`: test with `QF_SH=dash` and `QF_SH=bash`. No bashisms; quote every variable.
- Piped into sh (`curl ... | sh`), standard input is the script: questions read `/dev/tty`, and child processes never read stdin.
- Removing things: compare a link's target before removing or replacing it; never build a path for `rm` from a variable that can be empty.
- Tests guard `mktemp` (`W=$(mktemp -d ...) && [ -n "$W" ] || exit 1`) before any trap uses the folder.
