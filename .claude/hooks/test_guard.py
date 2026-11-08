#!/usr/bin/env python3
"""Cases for guard.py: python3 .claude/hooks/test_guard.py (exit 1 on a wrong answer)."""
import json, os, subprocess, sys

GUARD = os.path.join(os.path.dirname(os.path.abspath(__file__)), "guard.py")
BLOCK, ALLOW = 2, 0
cases = [
    (BLOCK, "pwsh -File tests/e2e/real-switching.ps1 -Case H"),
    (BLOCK, "pwsh -NoProfile -File ./tests/e2e/real-headless.ps1 -Exhausted x"),
    (BLOCK, "& ./tests/e2e/real-lanes.ps1"),
    (BLOCK, "python3 tests/e2e/real-macos-keychain.py --auto"),
    (BLOCK, "cd /x && python3 tests/e2e/real-macos-keychain.py"),
    (ALLOW, "cat tests/e2e/real-switching.ps1"),
    (ALLOW, "grep -n Case tests/e2e/real-headless.ps1"),
    (ALLOW, "git push origin main && echo real-lanes.ps1"),
    (ALLOW, "cat > x.md <<'EOF'\npwsh -File tests/e2e/real-switching.ps1\nEOF\necho done"),
    (BLOCK, "cat > x.md <<'EOF'\nharmless\nEOF\npwsh -File tests/e2e/real-switching.ps1"),
    (BLOCK, 'security delete-generic-password -s "Claude Code-credentials" -a me'),
    (BLOCK, "security add-generic-password -U -a me -s 'Claude Code-credentials' -w x"),
    (ALLOW, 'security delete-generic-password -s "Claude Code-credentials-1a2b3c4d" -a me'),
    (ALLOW, 'security find-generic-password -s "Claude Code-credentials"'),
    (BLOCK, "rm ~/.claude/.credentials.json"),
    (BLOCK, "rm -f $HOME/.claude.json"),
    (BLOCK, "echo {} > ~/.claude/.credentials.json"),
    (ALLOW, "rm -rf /tmp/x/.claude/.credentials.json"),
    (ALLOW, "cat ~/.claude.json | jq .oauthAccount.accountUuid"),
    (ALLOW, "QUAYFLOW_KEYCHAIN=off cargo test --workspace"),
    (ALLOW, "cargo test -p qf-adapter keychain"),
    (ALLOW, "git status"),
]
if sys.platform == "darwin":
    cases.append((BLOCK, "cargo test --workspace"))
wrong = 0
for want, cmd in cases:
    r = subprocess.run(
        [sys.executable, GUARD],
        input=json.dumps({"tool_name": "Bash", "tool_input": {"command": cmd}}),
        capture_output=True,
        text=True,
        env={k: v for k, v in os.environ.items() if k != "QUAYFLOW_KEYCHAIN"},
    )
    if r.returncode != want:
        wrong += 1
        print(f"WRONG ({r.returncode}, wanted {want}): {cmd!r}")
print(f"{len(cases)} cases, {wrong} wrong")
sys.exit(1 if wrong else 0)
