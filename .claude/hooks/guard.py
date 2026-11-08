#!/usr/bin/env python3
"""PreToolUse guard for Bash: stops commands that can change the user's live Claude login.

Exit 2 blocks the command and shows the reason to Claude. Rules (see CLAUDE.md):
- the real-login scripts are run by the user, never by Claude;
- the bare `Claude Code-credentials` Keychain item is the user's live login;
- ~/.claude/.credentials.json and ~/.claude.json are the live login files;
- on macOS, cargo test runs with QUAYFLOW_KEYCHAIN=off unless the work is on the Keychain.
"""
import json, os, re, sys

try:
    data = json.load(sys.stdin)
except Exception:
    sys.exit(0)
if data.get("tool_name") != "Bash":
    sys.exit(0)
cmd = (data.get("tool_input") or {}).get("command", "")
# Text written to a file through a heredoc is data, not a command: leave it out.
cmd = re.sub(r"<<-?\s*(['\"]?)(\w+)\1[^\n]*\n.*?\n\s*\2\s*(?:\n|$)", "\n", cmd, flags=re.S)


def block(why):
    sys.stderr.write(f"Blocked by .claude/hooks/guard.py: {why}\n")
    sys.exit(2)


RUNNER = r"(?:(?<![\w./-])(?:pwsh|powershell(?:\.exe)?|python3?|py|sh|bash)\b|(?<![&\w])&)[^\n;|&]*"
if re.search(RUNNER + r"real-(?:switching|headless|lanes)\.ps1", cmd) or re.search(
    RUNNER + r"real-macos-keychain\.py", cmd
):
    block(
        "real-login scripts switch the user's live login. Give the user the command "
        "(they can run it with `! <command>`) and read the results they paste."
    )

if re.search(r"(?:add|delete|set)-generic-password|security\s+-i\b", cmd) and re.search(
    r"Claude Code-credentials(?![-\w])", cmd
):
    block("the bare `Claude Code-credentials` Keychain item is the user's live login; never write or delete it.")

LIVE = r"(?:~|\$HOME|\$\{HOME\}|/Users/[^/\s]+|/home/[^/\s]+)/\.claude(?:/\.credentials\.json|\.json)\b"
if re.search(r"(?:\brm\b|\bmv\b|\bcp\b|\btruncate\b|>>?)[^\n;|]*" + LIVE, cmd):
    block("~/.claude/.credentials.json and ~/.claude.json hold the live login; tests use a temporary folder.")

if sys.platform == "darwin" and re.search(r"\bcargo\s+(?:\+\S+\s+)?(?:test|nextest)\b", cmd):
    off = re.search(r"QUAYFLOW_KEYCHAIN=(?:off|0|false|no)\b", cmd) or os.environ.get(
        "QUAYFLOW_KEYCHAIN", ""
    ).lower() in ("off", "0", "false", "no")
    if not off and not re.search(r"(?i)keychain", cmd):
        block(
            "on this Mac, run tests with QUAYFLOW_KEYCHAIN=off (prefix the command), unless the "
            "work is on the Keychain backend (name the keychain tests in the command)."
        )
sys.exit(0)
