#!/bin/sh
# Sandbox test for installer/install.sh. Runs the installer with HOME redirected to a
# temporary folder, a fake quayflow program and fake gh and claude programs on a reduced
# PATH, so it changes nothing outside the temporary folder: no real PATH link, no Claude
# plugins, no downloads, no Claude data. Exit code 0 when every check passes.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
INSTALLER="$HERE/install.sh"
W=$(mktemp -d "${TMPDIR:-/tmp}/qf-installer-test-XXXXXXXX")
PASS=0 FAIL=0
check() { # condition-result name [detail]
  if [ "$1" = 0 ]; then PASS=$((PASS + 1)); printf '  PASS %s\n' "$2"
  else FAIL=$((FAIL + 1)); printf '  FAIL %s %s\n' "$2" "${3:-}"; fi
}
ok() { "$@" >/dev/null 2>&1; echo $?; }
trap 'pkill -f "$W/" 2>/dev/null; rm -rf "$W"' EXIT

ASSET=quayflow-linux-x86_64
ASSETS="$W/assets" FAKEBIN="$W/fakebin" NOGH="$W/nogh"
mkdir -p "$ASSETS" "$FAKEBIN" "$NOGH"

# Fake quayflow: logs its arguments; mimics the engine's stop/start contract (`stop` writes
# the "stopped by user" marker and ends the engine in engine.json; plain `start` clears it).
cat > "$ASSETS/$ASSET" <<'EOF'
#!/bin/sh
[ -n "${QF_FAKE_LOG:-}" ] && printf '%s\n' "$*" >> "$QF_FAKE_LOG"
root="${XDG_DATA_HOME:-$HOME/.local/share}/quayflow"
alive() { pid=$(sed -n 's/.*"pid": *\([0-9]*\).*/\1/p' "$root/engine.json" 2>/dev/null); [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; }
case "${1:-}" in
  version) [ "${QF_FAKE_VERSION_FAIL:-}" = 1 ] && { echo broken; exit 1; }; echo 'quayflow 0.0.0-test' ;;
  engine-sim) sleep 120 ;;
  stop) echo 1 > "$root/engine.stopped"; [ "${QF_FAKE_STOP_HANG:-}" = 1 ] && exit 0
        pid=$(sed -n 's/.*"pid": *\([0-9]*\).*/\1/p' "$root/engine.json" 2>/dev/null); [ -n "$pid" ] && kill "$pid" 2>/dev/null; true ;;
  start) if alive; then echo '{"state":"running"}'; exit 0; fi
         [ "${2:-}" = --if-consented ] || rm -f "$root/engine.stopped"
         s=${QF_FAKE_START_STATE:-started}; echo "{\"state\":\"$s\",\"pid\":4242}"; [ "$s" != failed ] ;;
esac
exit 0
EOF
chmod 755 "$ASSETS/$ASSET"

# Fake gh: auth status and attestation verify by environment; logs to the same log.
cat > "$FAKEBIN/gh" <<'EOF'
#!/bin/sh
[ -n "${QF_FAKE_LOG:-}" ] && printf 'gh: %s\n' "$*" >> "$QF_FAKE_LOG"
case "$1 $2" in
  "auth status") [ "${QF_FAKE_GH_AUTH:-}" = fail ] && exit 1; exit 0 ;;
  "attestation verify") [ "${QF_FAKE_GH_ATTEST:-}" = fail ] && { echo 'no matching attestation' >&2; exit 1; }; echo 'Verification succeeded' ;;
esac
exit 0
EOF
# Fake claude: market.txt (the quayflow marketplace's source) and plugin.txt (installed).
cat > "$FAKEBIN/claude" <<'EOF'
#!/bin/sh
dir=$QF_FAKE_CLAUDE_STATE; mkdir -p "$dir"
market=$dir/market.txt plugin=$dir/plugin.txt
[ -n "${QF_FAKE_LOG:-}" ] && printf 'claude: %s\n' "$*" >> "$QF_FAKE_LOG"
case "$*" in
  "plugin list --json") if [ -f "$plugin" ]; then echo '[{"id":"quayflow@quayflow","scope":"user"}]'; else echo '[]'; fi ;;
  "plugin marketplace list --json")
    [ -f "$market" ] || { echo '[]'; exit 0; }
    src=$(cat "$market")
    case "$src" in
      /*) echo "[{\"name\":\"quayflow\",\"source\":\"directory\",\"path\":\"$src\",\"installLocation\":\"$src\"}]" ;;
      *) repo=${src%%#*}; ref=${src#*#}
         echo "[{\"name\":\"quayflow\",\"source\":\"github\",\"repo\":\"$repo\",\"ref\":\"$ref\",\"installLocation\":\"$QF_FAKE_CLAUDE_CLONE\"}]" ;;
    esac ;;
  "plugin marketplace remove quayflow") rm -f "$plugin"; [ -f "$market" ] && rm -f "$market" || exit 1 ;;
  "plugin marketplace add "*)
    [ -f "$market" ] && { echo 'Cannot add marketplace "quayflow": its source does not match' >&2; exit 1; }
    case "$4" in *"${QF_FAKE_CLAUDE_ADD_FAIL:-@none@}"*) echo 'Failed to clone marketplace repository' >&2; exit 1 ;; esac
    printf '%s' "$4" > "$market" ;;
  "plugin install quayflow@quayflow --scope user") [ -f "$market" ] || exit 1; echo 1 > "$plugin" ;;
esac
exit 0
EOF
chmod 755 "$FAKEBIN/gh" "$FAKEBIN/claude"
cp "$FAKEBIN/claude" "$NOGH/claude"

# A git checkout standing in for the marketplace clone; its HEAD is the release's commit.
CLONE="$W/clone"
mkdir -p "$CLONE"
git -C "$CLONE" init -q
echo mod > "$CLONE/README"
git -C "$CLONE" add README
git -C "$CLONE" -c user.name=t -c user.email=t@example.invalid -c commit.gpgsign=false commit -q -m init
HEAD_COMMIT=$(git -C "$CLONE" rev-parse HEAD)

write_sums() { # dir [name=hash override]
  d=$1; shift
  ( cd "$d" && for f in "$ASSET" SOURCE_COMMIT; do
      [ -f "$f" ] || continue
      h=$(sha256sum "$f" | cut -d' ' -f1)
      for o in "$@"; do case "$o" in "$f="*) h=${o#*=} ;; esac; done
      [ "$h" = omit ] || printf '%s  %s\n' "$h" "$f"
    done ) > "$d/SHA256SUMS"
}

# The system's programs without a real gh (CI runners ship one): links in one folder, so
# "no gh" means no gh, and only the fake gh in $FAKEBIN answers when a case wants one.
SYS="$W/sys"
mkdir -p "$SYS"
for d in /usr/local/bin /usr/bin /bin; do
  for f in "$d"/*; do
    n=${f##*/}
    [ "$n" = gh ] || [ -e "$SYS/$n" ] || ln -s "$f" "$SYS/$n"
  done
done
export HOME="$W/home" QF_FAKE_LOG="$W/fake.log" QF_FAKE_CLAUDE_STATE="$W/claude-state" QF_FAKE_CLAUDE_CLONE="$CLONE"
unset XDG_DATA_HOME XDG_CONFIG_HOME
mkdir -p "$HOME"
ROOT="$HOME/.local/share/quayflow" BIN="$HOME/.local/share/quayflow/bin" EXE="$HOME/.local/share/quayflow/bin/quayflow"
LOGL() { cat "$QF_FAKE_LOG" 2>/dev/null; }
run() { OUT=$(PATH="$NOGH:$SYS" sh "$INSTALLER" --source "$ASSETS" --yes --no-plugin --skip-attestation "$@" 2>&1); CODE=$?; }
runwith() { dirs=$1; shift; OUT=$(PATH="$dirs:$SYS" sh "$INSTALLER" --yes "$@" 2>&1); CODE=$?; }
sim() { # a running "engine": a quayflow* process named in engine.json, not our child
  cp "$ASSETS/$ASSET" "$W/quayflow-engine"; mkdir -p "$ROOT"
  ( "$W/quayflow-engine" engine-sim >/dev/null 2>&1 & echo $! > "$W/sim.pid" )
  SIM=$(cat "$W/sim.pid")
  printf '{"pid": %s}' "$SIM" > "$ROOT/engine.json"
}
gone() { i=0; while kill -0 "$1" 2>/dev/null && [ $i -lt 50 ]; do sleep 0.1; i=$((i + 1)); done; }

echo '=== install.sh ==='
write_sums "$ASSETS"
run
check "$([ $CODE = 0 ]; echo $?)" 'install succeeds' "$OUT"
check "$([ "$(sha256sum < "$EXE")" = "$(sha256sum < "$ASSETS/$ASSET")" ]; echo $?)" 'program installed'
check "$([ "$(readlink "$HOME/.local/bin/quayflow")" = "$EXE" ]; echo $?)" 'the ~/.local/bin link points to it'
check "$(LOGL | grep -qx 'consent engine-start on'; echo $?)" 'engine-start consent recorded'
check "$(LOGL | grep -q 'consent signin-startup on'; [ $? = 1 ]; echo $?)" 'no sign-in startup without --startup'
check "$(printf '%s' "$OUT" | grep -q 'Not affiliated with or endorsed by Anthropic'; echo $?)" 'consent text shown'
check "$(printf '%s' "$OUT" | grep -q 'skipped (--skip-attestation)'; echo $?)" 'attestation status reported'
check "$([ "$(stat -c %a "$ROOT")" = 700 ]; echo $?)" 'data folder owner-only'

run --startup
check "$([ $CODE = 0 ] && LOGL | grep -qx 'consent signin-startup on'; echo $?)" '--startup records sign-in consent' "$OUT"
check "$(LOGL | grep -qE '^(stop|start)$'; [ $? = 1 ]; echo $?)" 'engine not running: upgrade neither stops nor starts it'

sim; rm -f "$QF_FAKE_LOG"
run
check "$([ $CODE = 0 ]; echo $?)" 'upgrade with a running engine succeeds' "$OUT"
check "$(LOGL | grep -qx stop; echo $?)" 'engine stopped for the upgrade'
gone "$SIM"
check "$(LOGL | grep -qx start; echo $?)" 'engine restarted with plain start'
check "$([ ! -f "$ROOT/engine.stopped" ]; echo $?)" 'stopped-by-user marker cleared'
check "$(printf '%s' "$OUT" | grep -q 'Engine restarted (started)'; echo $?)" 'restart state reported' "$OUT"

sim; rm -f "$QF_FAKE_LOG"
QF_FAKE_VERSION_FAIL=1 run
gone "$SIM"
check "$([ $CODE != 0 ] && LOGL | grep -qx start && printf '%s' "$OUT" | grep -q 'engine was restarted (started)'; echo $?)" 'failed upgrade after stop restarts the engine' "$OUT"
check "$([ ! -f "$ROOT/engine.stopped" ]; echo $?)" 'no stopped-by-user marker after a failed upgrade'
rm -f "$ROOT/engine.json"

rm -rf "$BIN"
write_sums "$ASSETS" "$ASSET=$(printf '0%.0s' $(seq 64))"
run
check "$([ $CODE != 0 ] && printf '%s' "$OUT" | grep -q 'REFUSED: hash mismatch' && [ ! -e "$EXE" ]; echo $?)" 'hash mismatch refused, nothing installed' "$OUT"
write_sums "$ASSETS" "$ASSET=omit"
run
check "$([ $CODE != 0 ] && printf '%s' "$OUT" | grep -q 'REFUSED: SHA256SUMS has no entry'; echo $?)" 'missing hash entry refused' "$OUT"
write_sums "$ASSETS"

# ---- attestation fails closed
runwith "$NOGH" --source "$ASSETS" --no-plugin
check "$([ $CODE != 0 ] && printf '%s' "$OUT" | grep -q 'gh) is not installed' && printf '%s' "$OUT" | grep -q 'sudo apt install gh' && printf '%s' "$OUT" | grep -q -- '--skip-attestation' && [ ! -e "$EXE" ]; echo $?)" 'no gh: refused, told how to install gh and how to skip' "$OUT"
QF_FAKE_GH_AUTH=fail runwith "$FAKEBIN" --source "$ASSETS" --no-plugin
check "$([ $CODE != 0 ] && printf '%s' "$OUT" | grep -q 'not signed in' && [ ! -e "$EXE" ]; echo $?)" 'gh signed out: refused, nothing installed' "$OUT"
QF_FAKE_GH_ATTEST=fail runwith "$FAKEBIN" --source "$ASSETS" --no-plugin
check "$([ $CODE != 0 ] && printf '%s' "$OUT" | grep -q "REFUSED: attestation verification failed for $ASSET" && [ ! -e "$EXE" ]; echo $?)" 'unattested program refused' "$OUT"

REL="$W/release"; mkdir -p "$REL"
cp "$ASSETS/$ASSET" "$REL/"; printf '%s\n' "$HEAD_COMMIT" > "$REL/SOURCE_COMMIT"; write_sums "$REL"
rm -f "$QF_FAKE_LOG"
runwith "$FAKEBIN" --source "$REL" --no-plugin
V=$(LOGL | grep '^gh: attestation verify')
check "$([ $CODE = 0 ] && printf '%s' "$OUT" | grep -q 'Attestation: verified'; echo $?)" 'attested release installs' "$OUT"
check "$([ "$(printf '%s\n' "$V" | grep -c .)" = 2 ] && printf '%s\n' "$V" | grep -q SOURCE_COMMIT; echo $?)" 'program and SOURCE_COMMIT verified' "$V"
check "$(printf '%s\n' "$V" | grep -v -- "--repo minhdaohuu/quayflow --signer-workflow minhdaohuu/quayflow/.github/workflows/release.yml" | grep -q .; [ $? = 1 ] && printf '%s\n' "$V" | grep -q -- "--source-digest $HEAD_COMMIT"; echo $?)" 'verified against the release workflow and the source commit' "$V"
printf 'ffffffffffffffffffffffffffffffffffffffff\n' > "$REL/SOURCE_COMMIT"
runwith "$FAKEBIN" --source "$REL" --no-plugin
check "$([ $CODE != 0 ] && printf '%s' "$OUT" | grep -q 'REFUSED: hash mismatch for SOURCE_COMMIT'; echo $?)" 'edited SOURCE_COMMIT refused' "$OUT"
printf '%s\n' "$HEAD_COMMIT" > "$REL/SOURCE_COMMIT"

# ---- the Mod: added from the tag only while the tag points at SOURCE_COMMIT; a failed or
# refused replacement puts the previous Mod back.
STATE=$QF_FAKE_CLAUDE_STATE
runwith "$FAKEBIN" --source "$REL" --skip-attestation --marketplace-source 'minhdaohuu/quayflow#v9.9.9'
check "$([ $CODE = 0 ] && printf '%s' "$OUT" | grep -q "Mod source verified: commit $HEAD_COMMIT"; echo $?)" 'Mod added from a tag at the release commit' "$OUT"
check "$([ "$(cat "$STATE/market.txt")" = 'minhdaohuu/quayflow#v9.9.9' ] && [ -f "$STATE/plugin.txt" ]; echo $?)" 'marketplace and plugin installed'
echo moved > "$CLONE/README"; git -C "$CLONE" -c user.name=t -c user.email=t@example.invalid -c commit.gpgsign=false commit -q -am moved
rm -f "$QF_FAKE_LOG"
runwith "$FAKEBIN" --source "$REL" --skip-attestation --marketplace-source 'minhdaohuu/quayflow#v9.9.10'
check "$([ $CODE != 0 ] && printf '%s' "$OUT" | grep -q 'REFUSED: the Mod source minhdaohuu/quayflow#v9.9.10 is at commit' && printf '%s' "$OUT" | grep -q 'Done, except the Mod'; echo $?)" 'moved tag: Mod refused' "$OUT"
check "$([ "$(cat "$STATE/market.txt")" = 'minhdaohuu/quayflow#v9.9.9' ] && [ -f "$STATE/plugin.txt" ]; echo $?)" 'moved tag: previous Mod restored'
check "$(LOGL | grep -qx 'consent engine-start on'; echo $?)" 'Mod failure: engine consent still recorded'
git -C "$CLONE" reset -q --hard "$HEAD_COMMIT"
QF_FAKE_CLAUDE_ADD_FAIL=broken runwith "$FAKEBIN" --source "$REL" --skip-attestation --marketplace-source 'minhdaohuu/quayflow#broken'
check "$([ $CODE != 0 ] && printf '%s' "$OUT" | grep -q 'marketplace add minhdaohuu/quayflow#broken failed' && printf '%s' "$OUT" | grep -q 'previous Mod (minhdaohuu/quayflow#v9.9.9) was restored'; echo $?)" 'failed add reported, previous Mod restored' "$OUT"
check "$([ "$(cat "$STATE/market.txt")" = 'minhdaohuu/quayflow#v9.9.9' ] && [ -f "$STATE/plugin.txt" ]; echo $?)" 'failed add: marketplace and plugin back'

# ---- --update (quayflow update): no consent text, no consent changes, no link change, the
# Mod moves with the release; a Mod from a local folder is left alone.
rm -f "$HOME/.local/bin/quayflow" "$QF_FAKE_LOG"
runwith "$FAKEBIN" --source "$REL" --skip-attestation --update --startup --marketplace-source 'minhdaohuu/quayflow#v9.9.9'
check "$([ $CODE = 0 ] && printf '%s' "$OUT" | grep -q 'Updating Quayflow' && ! printf '%s' "$OUT" | grep -qi 'not affiliated with or endorsed'; echo $?)" 'update: no consent text' "$OUT"
check "$(LOGL | grep -q '^consent '; [ $? = 1 ]; echo $?)" 'update: no consent changed (not even with --startup)'
check "$([ ! -e "$HOME/.local/bin/quayflow" ]; echo $?)" 'update: PATH link untouched'
check "$(printf '%s' "$OUT" | grep -q "Mod source verified: commit $HEAD_COMMIT"; echo $?)" 'update: Mod moved to the release' "$OUT"
printf '%s' "$CLONE" > "$STATE/market.txt"; rm -f "$QF_FAKE_LOG"
runwith "$FAKEBIN" --source "$REL" --skip-attestation --update
check "$([ $CODE = 0 ] && printf '%s' "$OUT" | grep -q 'installed from .* and was not changed'; echo $?)" 'update: local-folder Mod reported as kept' "$OUT"
check "$([ "$(cat "$STATE/market.txt")" = "$CLONE" ] && ! LOGL | grep -qE '^claude: plugin marketplace (add|remove)'; echo $?)" 'update: local-folder Mod untouched'
runwith "$FAKEBIN" --source "$REL" --skip-attestation --update --no-plugin
check "$(printf '%s' "$OUT" | grep -q 'The Mod was left as it is (local build)'; echo $?)" 'update from a local build: Mod left as it is' "$OUT"

# ---- uninstall and argument checks
ln -sfn "$EXE" "$HOME/.local/bin/quayflow"; rm -f "$QF_FAKE_LOG"
OUT=$(PATH="$NOGH:$SYS" sh "$INSTALLER" --uninstall --delete-data --yes 2>&1); CODE=$?
check "$([ $CODE = 0 ] && LOGL | grep -qx 'uninstall --delete-data --yes' && [ ! -e "$HOME/.local/bin/quayflow" ]; echo $?)" 'uninstall delegated to quayflow, link removed' "$OUT"
OUT=$(PATH="$NOGH:$SYS" sh "$INSTALLER" --version bogus --yes 2>&1); CODE=$?
check "$([ $CODE != 0 ] && printf '%s' "$OUT" | grep -q 'Not a release version'; echo $?)" 'invalid --version refused before any download' "$OUT"
OUT=$(PATH="$NOGH:$SYS" sh "$INSTALLER" --frobnicate 2>&1); CODE=$?
check "$([ $CODE != 0 ] && printf '%s' "$OUT" | grep -q 'Unknown option'; echo $?)" 'unknown option refused' "$OUT"

echo "$PASS passed, $FAIL failed"
[ "$FAIL" = 0 ]
