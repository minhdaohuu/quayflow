#!/bin/sh
# Linux and macOS end-to-end check of the real quayflow program against a FAKE environment: engine
# start, status, permissions and stop; install.sh from a local release folder; quayflow
# update handing off to install.sh while the engine runs; uninstall.
#
# Safe to run anywhere: HOME, CLAUDE_CONFIG_DIR and the data folder point into a temporary
# folder, the network is off (QUAYFLOW_OFFLINE), the macOS Keychain is not used
# (QUAYFLOW_KEYCHAIN=off) and no Claude plugin is touched.
# Usage: sh tests/e2e/smoke-unix.sh [path/to/quayflow]   (default target/debug/quayflow)
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
BIN=${1:-$REPO/target/debug/quayflow}
[ -x "$BIN" ] || { echo "no program at $BIN (cargo build -p quayflow)"; exit 1; }
W=$(mktemp -d "${TMPDIR:-/tmp}/qf-smoke-XXXXXXXX") && [ -n "$W" ] || { echo "mktemp failed" >&2; exit 1; }
PASS=0 FAIL=0
check() {
  if [ "$1" = 0 ]; then PASS=$((PASS + 1)); printf '  PASS %s\n' "$2"
  else FAIL=$((FAIL + 1)); printf '  FAIL %s %s\n' "$2" "${3:-}"; fi
}
mode() { stat -c %a "$1" 2>/dev/null || stat -f %Lp "$1" 2>/dev/null; }
sha() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }
case "$(uname -s)/$(uname -m)" in
  Darwin/arm64) ASSET=quayflow-macos-arm64 ;;
  Darwin/x86_64) ASSET=quayflow-macos-x86_64 ;;
  *) ASSET=quayflow-linux-x86_64 ;;
esac
export HOME="$W/home" QUAYFLOW_OFFLINE=1 QUAYFLOW_KEYCHAIN=off
unset XDG_DATA_HOME XDG_CONFIG_HOME QUAYFLOW_HOME CLAUDE_CONFIG_DIR
mkdir -p "$HOME/.claude/sessions"
ROOT="$HOME/.local/share/quayflow"
Q="$HOME/.local/bin/quayflow"
cleanup() { "$Q" stop >/dev/null 2>&1; "$BIN" stop >/dev/null 2>&1; sleep 1; rm -rf "$W"; }
trap cleanup EXIT

echo "=== $(uname -s) smoke ==="
# A local release folder, as the release workflow assembles it.
D="$W/dist"; mkdir -p "$D"
cp "$BIN" "$D/$ASSET"
cp "$REPO/installer/install.sh" "$D/"
( cd "$D" && sha "$ASSET" install.sh > SHA256SUMS )

OUT=$(sh "$REPO/installer/install.sh" --source "$D" --yes --no-plugin --skip-attestation 2>&1); CODE=$?
check "$([ $CODE = 0 ] && [ -x "$ROOT/bin/quayflow" ]; echo $?)" 'install.sh installs the program' "$OUT"
check "$([ "$(readlink "$Q")" = "$ROOT/bin/quayflow" ]; echo $?)" 'linked as ~/.local/bin/quayflow'
check "$([ "$(mode "$ROOT")" = 700 ]; echo $?)" 'data folder is owner-only'
check "$("$Q" version | grep -q '^quayflow '; echo $?)" 'quayflow version runs'

OUT=$("$Q" start 2>&1)
check "$(printf '%s' "$OUT" | grep -q '"state":"started"'; echo $?)" 'engine starts' "$OUT"
check "$([ "$(mode "$ROOT/engine.json")" = 600 ]; echo $?)" 'engine.json (API token) is owner-only' "$(mode "$ROOT/engine.json")"
PID=$(sed -n 's/.*"pid": *\([0-9]*\).*/\1/p' "$ROOT/engine.json")
check "$([ "$(ps -o pgid= -p "$PID" | tr -d ' ')" = "$PID" ]; echo $?)" 'engine runs in its own process group'
OUT=$("$Q" start 2>&1)
check "$(printf '%s' "$OUT" | grep -q '"state":"running"'; echo $?)" 'a second start finds it running'
OUT=$("$Q" status 2>&1)
check "$(printf '%s' "$OUT" | grep -q 'Environment default'; echo $?)" 'status answers from the engine' "$OUT"
check "$(printf '%s' "$OUT" | grep -q 'signed out'; echo $?)" 'no login in the fake environment: signed out'

OUT=$("$Q" update --source "$D" --skip-attestation 2>&1); CODE=$?
check "$([ $CODE = 0 ] && printf '%s' "$OUT" | grep -q 'Updating Quayflow' && printf '%s' "$OUT" | grep -q 'Engine restarted (started)'; echo $?)" 'quayflow update hands off to install.sh and restarts the engine' "$OUT"
check "$(printf '%s' "$OUT" | grep -q 'The Mod was left as it is'; echo $?)" 'update from a local build leaves the Mod'
NEW=$(sed -n 's/.*"pid": *\([0-9]*\).*/\1/p' "$ROOT/engine.json")
check "$([ -n "$NEW" ] && [ "$NEW" != "$PID" ] && kill -0 "$NEW" 2>/dev/null; echo $?)" 'a new engine process runs after the update'

OUT=$("$Q" stop 2>&1); sleep 1
check "$(! kill -0 "$NEW" 2>/dev/null; echo $?)" 'quayflow stop ends the engine' "$OUT"
OUT=$("$Q" start --if-consented 2>&1)
check "$(printf '%s' "$OUT" | grep -q 'stopped_by_user'; echo $?)" 'after stop, start --if-consented respects it' "$OUT"

OUT=$("$Q" uninstall --delete-data --yes 2>&1); CODE=$?; sleep 1
check "$([ $CODE = 0 ] && [ ! -e "$Q" ] && [ ! -e "$ROOT/vault" ] && [ ! -e "$ROOT/bin" ]; echo $?)" 'uninstall removes the program, link and data' "$OUT"

echo "$PASS passed, $FAIL failed"
[ "$FAIL" = 0 ]
