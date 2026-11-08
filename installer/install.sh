#!/bin/sh
# Install, upgrade or remove Quayflow for the current Linux or macOS user (no administrator
# rights).
#
# Downloads the Linux or macOS program from a GitHub Release of minhdaohuu/quayflow, verifies it
# against the release's SHA256SUMS and the GitHub build-provenance attestation of the
# release workflow (this needs the GitHub CLI, signed in; --skip-attestation skips that
# check explicitly), installs it to ~/.local/share/quayflow/bin (or $XDG_DATA_HOME), links
# it as ~/.local/bin/quayflow, installs the Quayflow Mod into Claude Code from the same
# release tag (refused if the tag no longer points at the commit the release was built
# from), and records your consent for on-demand engine start.
#
# Re-running it upgrades. It never reads, prints or changes a Claude login.
#
#   curl -fsSL https://github.com/minhdaohuu/quayflow/releases/latest/download/install.sh | sh
#   curl -fsSL .../install.sh | sh -s -- --version v0.1.6 --startup
#
# Options:
#   --version vX.Y.Z        Release to install (default: the latest release).
#   --source DIR            Local folder with the program, SHA256SUMS (and SOURCE_COMMIT):
#                           testing, offline or private use. The Mod is then added from the
#                           repository containing DIR, or from --marketplace-source.
#   --marketplace-source S  Claude plugin marketplace source for the Mod (a local repository
#                           path, owner/repo or owner/repo#tag).
#   --no-path               Do not link ~/.local/bin/quayflow.
#   --startup               Also start the engine when you sign in (a systemd user unit on
#                           Linux, a LaunchAgent on macOS).
#   --no-plugin             Install the engine only; do not touch Claude Code plugins.
#   --skip-attestation      Do not run `gh attestation verify` (builds you made yourself,
#                           releases before v0.1.5, or no GitHub CLI). Hashes are still checked.
#   --yes                   Accept the consent text without asking (it is still shown).
#   --update                Used by `quayflow update`: upgrade without the consent text,
#                           without touching PATH and without changing any consent. A Mod
#                           installed from a local folder is left as it is.
#   --uninstall [--delete-data]  Remove Quayflow (delegates to `quayflow uninstall`).
#
# quayflow-installer: update-mode
set -eu

REPO=minhdaohuu/quayflow
VERSION="" SOURCE="" MARKET_SOURCE="" NO_PATH=0 STARTUP=0 NO_PLUGIN=0 SKIP_ATTEST=0 YES=0 UPDATE=0 UNINSTALL=0 DELETE_DATA=0

step() { printf '==> %s\n' "$*"; }
info() { printf '    %s\n' "$*"; }
warn() { printf 'WARNING: %s\n' "$*" >&2; }
die() { printf '%s\n' "$*" >&2; exit 1; }

while [ $# -gt 0 ]; do
  case "$1" in
    --version) [ $# -ge 2 ] || die "--version needs a value"; VERSION=$2; shift ;;
    --source) [ $# -ge 2 ] || die "--source needs a folder"; SOURCE=$2; shift ;;
    --marketplace-source) [ $# -ge 2 ] || die "--marketplace-source needs a value"; MARKET_SOURCE=$2; shift ;;
    --no-path) NO_PATH=1 ;;
    --startup) STARTUP=1 ;;
    --no-plugin) NO_PLUGIN=1 ;;
    --skip-attestation) SKIP_ATTEST=1 ;;
    --yes|-y) YES=1 ;;
    --update) UPDATE=1 ;;
    --uninstall) UNINSTALL=1 ;;
    --delete-data) DELETE_DATA=1 ;;
    -h|--help) sed -n '2,35p' "$0" 2>/dev/null | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "Unknown option: $1 (see --help)" ;;
  esac
  shift
done
# An update keeps every earlier choice: no consent prompt, no PATH change.
if [ "$UPDATE" = 1 ]; then YES=1; NO_PATH=1; fi

# ------------------------------------------------------------------ platform
OS=$(uname -s)
case "$OS/$(uname -m)" in
  Linux/x86_64|Linux/amd64) ASSET=quayflow-linux-x86_64 ;;
  Linux/*) die "Quayflow has no Linux build for $(uname -m) yet (x86_64 only)." ;;
  Darwin/arm64) ASSET=quayflow-macos-arm64 ;;
  Darwin/x86_64) ASSET=quayflow-macos-x86_64 ;;
  *) die "This installer is for Linux and macOS; on Windows use install.ps1." ;;
esac
[ -n "${HOME:-}" ] || die "HOME is not set; cannot choose an install folder."
DATA_ROOT="${XDG_DATA_HOME:-$HOME/.local/share}/quayflow"
BIN_DIR="$DATA_ROOT/bin"
EXE="$BIN_DIR/quayflow"
LINK="$HOME/.local/bin/quayflow"

have() { command -v "$1" >/dev/null 2>&1; }
if have sha256sum; then
  sha256() { sha256sum "$1" | cut -d' ' -f1; }
else
  sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
fi
find_claude() {
  if have claude; then command -v claude
  elif [ -x "$HOME/.local/bin/claude" ]; then printf '%s\n' "$HOME/.local/bin/claude"
  fi
}
CLAUDE=$(find_claude || true)

# --------------------------------------------------------------- uninstall
if [ "$UNINSTALL" = 1 ]; then
  step 'Removing Quayflow'
  if [ -x "$EXE" ]; then
    set -- uninstall
    [ "$DELETE_DATA" = 1 ] && set -- "$@" --delete-data
    [ "$YES" = 1 ] && set -- "$@" --yes
    # Piped into sh (`curl ... | sh -s -- --uninstall`), standard input is this script: the
    # question is asked on the terminal instead.
    if [ "$YES" = 0 ] && { : </dev/tty; } 2>/dev/null; then
      "$EXE" "$@" </dev/tty || die "quayflow uninstall failed (exit $?). Nothing else was changed by this script."
    else
      "$EXE" "$@" || die "quayflow uninstall failed (exit $?). Nothing else was changed by this script."
    fi
  else
    warn "quayflow not found at $EXE; removing the plugin and link only."
    if [ -n "$CLAUDE" ] && [ "$NO_PLUGIN" = 0 ]; then
      "$CLAUDE" plugin uninstall quayflow@quayflow --scope user >/dev/null 2>&1 || true
      "$CLAUDE" plugin marketplace remove quayflow >/dev/null 2>&1 || true
    fi
  fi
  if [ -L "$LINK" ]; then rm -f "$LINK"; info "Removed $LINK."; fi
  info 'Your current Claude login was left as it is.'
  [ "$DELETE_DATA" = 1 ] || info "Your vault, configuration and logs (if kept) are in $DATA_ROOT."
  exit 0
fi

# ----------------------------------------------------------------- consent
if [ "$UPDATE" = 1 ]; then
  step 'Updating Quayflow (your settings and consents are kept)'
else
  if [ "$STARTUP" = 1 ]; then
    if [ "$OS" = Darwin ]; then how='a LaunchAgent'; else how='a systemd user unit'; fi
    startup_text="  - Engine start at sign-in: ENABLED by --startup ($how; no window)."
  else
    startup_text="  - Engine start at sign-in: not enabled (re-run with --startup to enable)."
  fi
  if [ "$OS" = Darwin ]; then
    who="your macOS user"
    claude_keeps="Claude Code keeps its own login in your Keychain"
  else
    who="your Linux user"
    claude_keeps="as Claude Code keeps its own login in ~/.claude/.credentials.json"
  fi
  cat <<EOF

Quayflow - manage approved Claude accounts and switch between them.
Not affiliated with or endorsed by Anthropic.

What this installs, for $who only (no administrator rights):
  - the quayflow program in $BIN_DIR, linked as $LINK
  - the "quayflow" Mod (a Claude Code plugin, user scope): /quay, the status
    line, usage-limit detection and one retry of a failed request.
  - the Quayflow engine (quayflow), one per user. After you agree below, the
    Mod starts it when a Claude session finds it not running. It watches the
    usage of the accounts you register and switches the login when your
    group's mode allows it. It does not change how you start Claude: you keep
    typing \`claude\`.
$startup_text

How your logins are handled:
  - Nothing is stored by this installer. When you register an account later
    (in Claude: /quay), Quayflow keeps a reusable copy of that login in its
    vault, a folder readable by your user only ($claude_keeps).
    It is not encrypted.
  - Claude refreshes the live login and Quayflow saves the result. Quayflow
    refreshes only the saved copy of an idle account, so its usage stays readable.
    Logins never go into logs, reports or the Mod. There is no telemetry.

What a switch does:
  - A switch replaces the login of a Claude environment (by default
    ~/.claude). EVERY Claude session using that environment moves to the new
    account at its next request; requests in progress finish on the old one.
  - Groups decide which accounts may serve which projects. Putting accounts in
    one group approves sending that work to any of them.

Modes (per group):
  - Suggest (default): Quayflow proposes; you accept with /quay accept.
  - Automatic (opt-in): Quayflow switches by itself within the group's rules.
  - Manual: only switches you ask for.
  Unattended headless jobs (claude -p) only switch in Automatic mode.

Your responsibility:
  You are responsible for complying with your provider's terms and your
  organization's rules, including rules on which accounts may process your
  organization's work. Use only accounts that are yours to use. Quayflow does
  not make sharing accounts, or getting around a provider's limits, allowed
  where its terms forbid it. Quayflow is not affiliated with or endorsed by
  Anthropic.

EOF
  if [ "$YES" = 1 ]; then
    info 'Consent given with --yes.'
  else
    # Piped into sh, standard input is the script: ask on the terminal.
    if [ -r /dev/tty ]; then
      printf 'Install Quayflow and allow the Mod to start the engine on demand? (y/N) '
      read -r answer < /dev/tty || answer=""
    else
      die "No terminal to ask for consent; run again with --yes after reading the text above."
    fi
    case "$answer" in y|Y|yes|YES|Yes) ;; *) echo 'Nothing changed.'; exit 0 ;; esac
  fi
fi

# ------------------------------------------------------- resolve the release
TAG=""
if [ -n "$VERSION" ]; then
  TAG=$VERSION
  case "$TAG" in v*) ;; *) TAG="v$TAG" ;; esac
  printf '%s\n' "$TAG" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$' || die "Not a release version: $VERSION"
fi

GH_READY=0
if have gh && gh auth status --hostname github.com >/dev/null 2>&1; then GH_READY=1; fi
# Build provenance is required unless skipped explicitly: refuse before downloading anything.
if [ "$SKIP_ATTEST" = 0 ] && [ "$GH_READY" = 0 ]; then
  if have gh; then
    why="the GitHub CLI is not signed in"
    fix="    gh auth login"
  else
    why="the GitHub CLI (gh) is not installed"
    if [ "$OS" = Darwin ]; then
      fix="    brew install gh   (or see https://cli.github.com)
    gh auth login"
    else
      fix="    sudo apt install gh   (or see https://cli.github.com)
    gh auth login"
    fi
  fi
  die "REFUSED: the build-provenance attestation cannot be verified because $why. Nothing was installed.
The attestation proves the program was built by this repository's release workflow. To check it, run:
$fix
then run the installer again.
Or install with the SHA-256 check alone (it proves the files match this release's SHA256SUMS, not who built them) by adding --skip-attestation:
    curl -fsSL https://github.com/$REPO/releases/latest/download/install.sh | sh -s -- --skip-attestation"
fi
have sha256sum || have shasum || die "sha256sum or shasum is needed to verify the download."

WORK=$(mktemp -d "${TMPDIR:-/tmp}/quayflow-install-XXXXXXXX")
STOPPED_FOR_UPGRADE=0
RESTARTED=0
cleanup() {
  rm -rf "$WORK"
  if [ "$STOPPED_FOR_UPGRADE" = 1 ] && [ "$RESTARTED" = 0 ]; then
    # The upgrade failed after stopping the engine: do not leave the user without one.
    if [ -x "$EXE" ]; then
      state=$(start_engine)
      warn "The upgrade did not finish; the engine was restarted ($state)."
    else
      rm -f "$DATA_ROOT/engine.stopped"
    fi
  fi
}
trap cleanup EXIT
trap 'exit 130' INT TERM

start_engine() {
  # `quayflow start` prints one JSON line with the state (started, running, failed, ...).
  out=$("$EXE" start 2>/dev/null || true)
  state=$(printf '%s\n' "$out" | sed -n 's/.*"state":"\([a-z_]*\)".*/\1/p' | tail -n 1)
  printf '%s\n' "${state:-unknown}"
}

# A process that exists and has not ended (a zombie waiting for its parent has ended).
if [ "$OS" = Darwin ]; then
  running() { kill -0 "$1" 2>/dev/null && ! ps -o stat= -p "$1" 2>/dev/null | grep -q '^Z'; }
  # The program's name; for a script, the script's name rather than its interpreter.
  comm_of() {
    # shellcheck disable=SC2046 # split the argument list on purpose
    set -- $(ps -o args= -p "$1" 2>/dev/null)
    case "$(basename "${1:-x}")" in
      sh|bash|dash|zsh) basename "${2:-x}" ;;
      *) basename "${1:-x}" ;;
    esac
  }
else
  running() { [ -d "/proc/$1" ] && ! sed 's/.*) //' "/proc/$1/stat" 2>/dev/null | grep -q '^Z'; }
  comm_of() { cat "/proc/$1/comm" 2>/dev/null; }
fi

engine_pid() {
  # The engine named in engine.json, when that process is alive and is quayflow.
  f="$DATA_ROOT/engine.json"
  [ -f "$f" ] || return 1
  pid=$(sed -n 's/.*"pid"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p' "$f" | head -n 1)
  [ -n "$pid" ] && running "$pid" || return 1
  case "$(comm_of "$pid")" in quayflow*) printf '%s\n' "$pid" ;; *) return 1 ;; esac
}

download() { # name -> $WORK/name
  if [ "$GH_READY" = 1 ]; then
    gh release download "$TAG" --repo "$REPO" --dir "$WORK" --clobber --pattern "$1" >/dev/null 2>&1
  else
    curl -fsSL -o "$WORK/$1" "https://github.com/$REPO/releases/download/$TAG/$1"
  fi
}

NEEDED="$ASSET SHA256SUMS"
if [ -n "$SOURCE" ]; then
  step "Using local files from $SOURCE"
  [ -d "$SOURCE" ] || die "Not a folder: $SOURCE"
  SRC_FULL=$(cd "$SOURCE" && pwd)
  for name in $NEEDED; do
    [ -f "$SRC_FULL/$name" ] || die "Missing $name in $SRC_FULL."
    cp "$SRC_FULL/$name" "$WORK/$name"
  done
  [ -f "$SRC_FULL/SOURCE_COMMIT" ] && cp "$SRC_FULL/SOURCE_COMMIT" "$WORK/SOURCE_COMMIT"
else
  if [ -z "$TAG" ]; then
    if [ "$GH_READY" = 1 ]; then
      TAG=$(gh release view --repo "$REPO" --json tagName --jq .tagName) || die "Could not find the latest release of $REPO with gh."
    else
      TAG=$(curl -fsSL -H 'User-Agent: quayflow-installer' "https://api.github.com/repos/$REPO/releases/latest" |
        sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)
      [ -n "$TAG" ] || die "Could not find the latest release of $REPO. If the repository is private, sign in with 'gh auth login' and run this again, or use --source."
    fi
  fi
  step "Downloading Quayflow $TAG"
  for name in $NEEDED; do
    download "$name" || die "Download failed: $name of $TAG (no such release, or no Linux build in it)."
  done
  # Older releases have no SOURCE_COMMIT; its absence is handled below.
  download SOURCE_COMMIT || rm -f "$WORK/SOURCE_COMMIT"
fi

# ------------------------------------------------------------------ verify
step 'Verifying SHA-256 hashes against SHA256SUMS'
sum_of() { # the SHA256SUMS entry for a name, lower-case
  sed -n "s/^\([0-9A-Fa-f]\{64\}\)[[:space:]][[:space:]]*\*\{0,1\}$1[[:space:]]*\$/\1/p" "$WORK/SHA256SUMS" | head -n 1 | tr 'A-F' 'a-f'
}
expected=$(sum_of "$ASSET")
[ -n "$expected" ] || die "REFUSED: SHA256SUMS has no entry for $ASSET. Nothing was installed."
actual=$(sha256 "$WORK/$ASSET")
[ "$actual" = "$expected" ] || die "REFUSED: hash mismatch for $ASSET (expected $expected, got $actual). Nothing was installed."
info "$ASSET  $actual  OK"

SOURCE_COMMIT=""
if [ -f "$WORK/SOURCE_COMMIT" ]; then
  expected=$(sum_of SOURCE_COMMIT)
  [ -n "$expected" ] || die 'REFUSED: SHA256SUMS has no entry for SOURCE_COMMIT. Nothing was installed.'
  actual=$(sha256 "$WORK/SOURCE_COMMIT")
  [ "$actual" = "$expected" ] || die "REFUSED: hash mismatch for SOURCE_COMMIT (expected $expected, got $actual). Nothing was installed."
  SOURCE_COMMIT=$(tr -d '[:space:]' < "$WORK/SOURCE_COMMIT" | tr 'A-F' 'a-f')
  printf '%s\n' "$SOURCE_COMMIT" | grep -Eq '^([0-9a-f]{40}|[0-9a-f]{64})$' || die 'REFUSED: SOURCE_COMMIT is not a commit id. Nothing was installed.'
  info "SOURCE_COMMIT  $SOURCE_COMMIT  OK"
fi

step 'Verifying build provenance (GitHub artifact attestation)'
ATTESTATION=verified
if [ "$SKIP_ATTEST" = 1 ]; then
  ATTESTATION='skipped (--skip-attestation)'
else
  # Only an attestation made by this repository's release workflow counts, for this tag and
  # commit when they are known.
  set -- --repo "$REPO" --signer-workflow "$REPO/.github/workflows/release.yml"
  [ -n "$TAG" ] && set -- "$@" --source-ref "refs/tags/$TAG"
  [ -n "$SOURCE_COMMIT" ] && set -- "$@" --source-digest "$SOURCE_COMMIT"
  attested=$ASSET
  [ -n "$SOURCE_COMMIT" ] && attested="$attested SOURCE_COMMIT"
  for name in $attested; do
    out=$(gh attestation verify "$WORK/$name" "$@" 2>&1) || die "REFUSED: attestation verification failed for $name. Nothing was installed.
$out"
    info "$name  attestation OK"
  done
fi
[ "$ATTESTATION" = verified ] || warn "Attestation: $ATTESTATION"

# ----------------------------------------------------------------- install
step "Installing to $BIN_DIR"
mkdir -p "$BIN_DIR"
chmod 700 "$DATA_ROOT"
WAS_RUNNING=0
if pid=$(engine_pid); then
  WAS_RUNNING=1
  if [ -x "$EXE" ]; then
    # `stop` also writes the "stopped by user" marker; the engine is restarted below with a
    # plain `start`, which clears it (or by the cleanup above if this fails).
    info "Stopping the running engine (pid $pid) for the upgrade."
    STOPPED_FOR_UPGRADE=1
    "$EXE" stop >/dev/null 2>&1 || true
    i=0
    while running "$pid" && [ $i -lt 40 ]; do sleep 0.5; i=$((i + 1)); done
    if running "$pid"; then
      warn "The engine did not stop within 20 s; ending it (pid $pid)."
      kill "$pid" 2>/dev/null || true
      sleep 1
    fi
  fi
fi
# Replace by rename: a running program keeps its old file until it exits.
cp "$WORK/$ASSET" "$EXE.new"
chmod 755 "$EXE.new"
# macOS: a copy of a file downloaded by a browser keeps its quarantine flag, and Gatekeeper
# would refuse the program. Its hash (and attestation) were verified above.
[ "$OS" = Darwin ] && xattr -d com.apple.quarantine "$EXE.new" 2>/dev/null || true
mv -f "$EXE.new" "$EXE"
[ "$(sha256 "$EXE")" = "$(sum_of "$ASSET")" ] || die 'Installed quayflow does not match the verified hash.'
version=$("$EXE" version 2>&1) || die "quayflow does not run: $version"
info "Installed: $version"
# The shell module is written by quayflow; an installed copy follows the new program.
if [ -f "$DATA_ROOT/shell/Quayflow.psm1" ]; then
  "$EXE" shell refresh >/dev/null 2>&1 && info 'PowerShell prompt hook module refreshed.' || warn 'The PowerShell prompt hook module was not refreshed.'
fi

if [ "$NO_PATH" = 0 ]; then
  mkdir -p "$(dirname "$LINK")"
  if [ -e "$LINK" ] && [ ! -L "$LINK" ]; then
    warn "$LINK exists and is not a link; left as it is. Run $EXE directly."
  else
    ln -sfn "$EXE" "$LINK"
    info "Linked $LINK."
    case ":$PATH:" in
      *":$HOME/.local/bin:"*) ;;
      *) info "Add ~/.local/bin to your PATH to run quayflow by name (most Linux distributions do this at sign-in; on macOS add it to ~/.zprofile)." ;;
    esac
  fi
fi

# ----------------------------------------------------------------- the Mod
# A Mod failure does not undo the engine install: the consents and the engine restart below
# still happen, and the installer reports the failure and exits with an error at the end.
MOD_ERROR=""
claude_json() { "$CLAUDE" "$@" 2>/dev/null || printf '[]'; }
# Reads `claude plugin marketplace list --json` (stdin): prints the quayflow entry's source
# and checkout, tab-separated.
marketplace_entry() {
  python3 -c '
import json, sys
try:
    entries = json.load(sys.stdin)
except Exception:
    entries = []
for e in entries if isinstance(entries, list) else []:
    if e.get("name") == "quayflow":
        src = e.get("repo") or e.get("url") or ""
        if src and e.get("ref"):
            src = src + "#" + e["ref"]
        src = src or e.get("path") or ""
        print(src + "\t" + (e.get("installLocation") or ""))
        break
'
}
has_plugin() { claude_json plugin list --json | grep -q '"quayflow@quayflow"'; }

install_mod() { # market expected_commit
  market=$1 expected=$2
  entry=$(claude_json plugin marketplace list --json | marketplace_entry)
  old_source=$(printf '%s' "$entry" | cut -f1)
  had_plugin=0
  has_plugin && had_plugin=1
  restore() {
    [ -n "$old_source" ] || { printf '%s' "$1"; return; }
    if ! r=$("$CLAUDE" plugin marketplace add "$old_source" 2>&1); then
      printf '%s\nThe previous Mod source (%s) could not be restored: %s' "$1" "$old_source" "$r"; return
    fi
    if [ "$had_plugin" = 1 ] && ! r=$("$CLAUDE" plugin install quayflow@quayflow --scope user 2>&1); then
      printf '%s\nThe previous marketplace was restored, but the plugin was not: %s' "$1" "$r"; return
    fi
    printf '%s\nThe previous Mod (%s) was restored.' "$1" "$old_source"
  }
  if [ -n "$old_source" ]; then
    r=$("$CLAUDE" plugin marketplace remove quayflow 2>&1) || { MOD_ERROR="Could not replace the quayflow marketplace: $r"; return 1; }
  fi
  if ! r=$("$CLAUDE" plugin marketplace add "$market" 2>&1); then
    MOD_ERROR=$(restore "claude plugin marketplace add $market failed: $r
For a private repository, run 'gh auth login' and 'gh auth setup-git' first.")
    return 1
  fi
  if [ -n "$expected" ] && [ ! -d "$market" ]; then
    # Tags can be moved: the Mod must come from the commit the release was built from.
    checkout=$(claude_json plugin marketplace list --json | marketplace_entry | cut -f2)
    head=""
    [ -n "$checkout" ] && head=$(git -C "$checkout" rev-parse HEAD 2>/dev/null | tr 'A-F' 'a-f' || true)
    if [ "$head" != "$expected" ]; then
      "$CLAUDE" plugin marketplace remove quayflow >/dev/null 2>&1 || true
      MOD_ERROR=$(restore "REFUSED: the Mod source $market is at commit ${head:-unknown (could not read the checkout)}, but this release was built from $expected (the tag was moved, or the checkout could not be read). The Mod was not installed.")
      return 1
    fi
    info "Mod source verified: commit $expected"
  fi
  r=$("$CLAUDE" plugin install quayflow@quayflow --scope user 2>&1) || { MOD_ERROR="claude plugin install quayflow@quayflow failed: $r"; return 1; }
  return 0
}

if [ "$NO_PLUGIN" = 1 ]; then
  if [ "$UPDATE" = 1 ]; then info 'The Mod was left as it is (local build).'; else warn 'Skipped the Claude Code plugin (--no-plugin).'; fi
elif [ -z "$CLAUDE" ]; then
  warn 'Claude Code (claude) was not found. Install it, then run this installer again to add the Mod.'
elif ! have python3; then
  MOD_ERROR="python3 is needed to read Claude Code's plugin list; install it and run the installer again."
else
  step 'Installing the Quayflow Mod into Claude Code'
  market=$MARKET_SOURCE
  local_mod=""
  if [ "$UPDATE" = 1 ] && [ -z "$market" ]; then
    # A Mod installed from a local folder is a development checkout: leave it alone.
    src=$(claude_json plugin marketplace list --json | marketplace_entry | cut -f1)
    [ -n "$src" ] && [ -d "$src" ] && local_mod=$src
  fi
  if [ -n "$local_mod" ]; then
    info "The Mod is installed from $local_mod and was not changed; it may not match the new engine."
  else
    if [ -z "$market" ] && [ -n "$SOURCE" ]; then
      dir=$SRC_FULL
      while [ "$dir" != / ] && [ -n "$dir" ]; do
        if [ -f "$dir/.claude-plugin/marketplace.json" ] && grep -q '"name"[[:space:]]*:[[:space:]]*"quayflow"' "$dir/.claude-plugin/marketplace.json"; then
          market=$dir; break
        fi
        dir=$(dirname "$dir")
      done
      [ -n "$market" ] || MOD_ERROR="No Quayflow marketplace found above $SOURCE; pass --marketplace-source <repository folder>."
    fi
    if [ -z "$market" ] && [ -z "$MOD_ERROR" ]; then
      market="$REPO#$TAG"
      [ -n "$SOURCE_COMMIT" ] || warn "This release does not record its source commit (SOURCE_COMMIT), so the Mod from tag $TAG cannot be checked against the commit the engine was built from."
    fi
    if [ -z "$MOD_ERROR" ] && install_mod "$market" "$SOURCE_COMMIT"; then
      # Re-adding pins the Mod to the same release as the engine (matched pair).
      info "Marketplace: $market"
      info 'Plugin quayflow@quayflow installed (user scope). Running Claude sessions load it after a restart.'
    fi
  fi
fi
[ -z "$MOD_ERROR" ] || warn "The Mod was not installed: $MOD_ERROR"

# ---------------------------------------------------------------- consents
if [ "$UPDATE" = 0 ]; then
  step 'Recording your consent'
  "$EXE" consent engine-start on >/dev/null || die 'quayflow consent engine-start on failed.'
  info 'On-demand engine start: on'
  if [ "$STARTUP" = 1 ]; then
    if out=$("$EXE" consent signin-startup on 2>&1); then
      if [ "$OS" = Darwin ]; then
        info 'Engine start at sign-in: on (LaunchAgent com.quayflow.engine)'
      else
        info 'Engine start at sign-in: on (systemd user unit quayflow.service)'
      fi
    else
      warn "Engine start at sign-in was not set up: $out"
    fi
  fi
fi
if [ "$WAS_RUNNING" = 1 ]; then
  # Plain `start`: it clears the "stopped by user" marker the upgrade's `stop` wrote.
  state=$(start_engine)
  RESTARTED=1
  case "$state" in
    started|running) info "Engine restarted ($state)." ;;
    *) warn "The engine did not restart ($state). Run 'quayflow start' (log: $DATA_ROOT/logs)." ;;
  esac
fi

if [ -n "$MOD_ERROR" ]; then step 'Done, except the Mod'; else step 'Done'; fi
info "Version:     ${TAG:-(local files)}"
info "Attestation: $ATTESTATION"
if [ "$UPDATE" = 0 ]; then
  info 'Next: start Claude as usual (claude) and run /quay to register your login and choose groups.'
  info 'Uninstall:  quayflow uninstall'
fi
if [ -n "$MOD_ERROR" ]; then
  die "The engine was installed, but the Mod was not: $MOD_ERROR
Fix the cause and run the installer again."
fi
