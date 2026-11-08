#!/usr/bin/env python3
"""Stop hook: once per stop, remind Claude when code changed but CHANGELOG.md did not.

Compares the working tree (and the commits not yet pushed) with origin/main. Internal-only
changes can say [no changelog] in a commit message, as the CI check accepts.
"""
import json, re, subprocess, sys

try:
    data = json.load(sys.stdin)
except Exception:
    data = {}
if data.get("stop_hook_active"):
    sys.exit(0)


def git(*args):
    r = subprocess.run(["git", *args], capture_output=True, text=True)
    return r.stdout if r.returncode == 0 else ""


base = "origin/main" if git("rev-parse", "--verify", "-q", "origin/main") else "HEAD"
files = set(git("diff", "--name-only", base).split()) | set(git("ls-files", "--others", "--exclude-standard").split())
code = [
    f
    for f in files
    if re.match(r"^(crates/[^/]+/src/|mod/quayflow/hooks/|installer/install\.(sh|ps1)$|shell/Quayflow\.psm1$)", f)
    and not re.search(r"/tests?\.rs$|/invariants\.rs$", f)
]
if not code or "CHANGELOG.md" in files or "[no changelog]" in git("log", "--format=%B", f"{base}..HEAD"):
    sys.exit(0)
print(
    json.dumps(
        {
            "decision": "block",
            "reason": "Code changed without a CHANGELOG.md line under [Unreleased] ("
            + ", ".join(sorted(code)[:5])
            + "). Add a line written for users, or say [no changelog] in the commit message if the change is internal only.",
        }
    )
)
