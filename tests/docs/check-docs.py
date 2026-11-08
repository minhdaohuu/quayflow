#!/usr/bin/env python3
"""Checks the Markdown docs against the repository. Runs in CI (Linux) and in pre-push.

- Every local link resolves, and every #anchor names a heading of the target file.
- The version is the same in every place CLAUDE.md lists.
- No phrase that only made sense before a release ("next release", "not released yet").
- Platform lists name macOS too: "Windows and Linux" only on a line that also says macOS or
  "every platform".
- Each release's version follows the CHANGELOG headings of that release (see CONTRIBUTING.md#versioning),
  and Cargo.toml has the newest released version.
Usage: python3 tests/docs/check-docs.py          (all checks)
       python3 tests/docs/check-docs.py --next   (prints the version the next release must have)
"""
import json, os, re, subprocess, sys, tomllib

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
os.chdir(ROOT)
problems = []

# Versioning (CONTRIBUTING.md#versioning): the CHANGELOG decides the next version.
# Before 1.0: Added, Changed, Deprecated or Removed -> minor; only Fixed or Security -> patch.
# From 1.0: Removed or a "**Breaking:**" entry -> major; Added, Changed or Deprecated -> minor;
# otherwise patch. Versions up to 0.1.6 predate this rule.
GRANDFATHERED = (0, 1, 6)


def changelog_sections():
    sections, cur = [], None
    for line in open("CHANGELOG.md", encoding="utf-8"):
        m = re.match(r"^## \[([^\]]+)\]", line)
        if m:
            cur = {"name": m.group(1), "kinds": set(), "breaking": False}
            sections.append(cur)
        elif cur and (k := re.match(r"^### (\w+)", line)):
            cur["kinds"].add(k.group(1))
        elif cur and re.match(r"^- \*\*Breaking:\*\*", line):
            cur["breaking"] = True
    return sections


def bump(prev, kinds, breaking):
    major, minor, patch = prev
    if major == 0:
        feature = breaking or bool(kinds & {"Added", "Changed", "Deprecated", "Removed"})
        return (0, minor + 1, 0) if feature else (0, minor, patch + 1)
    if breaking or "Removed" in kinds:
        return (major + 1, 0, 0)
    if kinds & {"Added", "Changed", "Deprecated"}:
        return (major, minor + 1, 0)
    return (major, minor, patch + 1)


def vtuple(s):
    return tuple(int(x) for x in s.split("."))


def vstr(t):
    return ".".join(map(str, t))


sections = changelog_sections()
released = [s for s in sections if re.fullmatch(r"\d+\.\d+\.\d+", s["name"])]
unreleased = next((s for s in sections if s["name"] == "Unreleased"), None)
if "--next" in sys.argv:
    # Prints the version the next release must have, from the [Unreleased] headings.
    if not unreleased or not unreleased["kinds"]:
        print("nothing under [Unreleased]: no release needed")
        sys.exit(1)
    print(vstr(bump(vtuple(released[0]["name"]), unreleased["kinds"], unreleased["breaking"])))
    sys.exit(0)
for newer, older in zip(released, released[1:]):
    new, old = vtuple(newer["name"]), vtuple(older["name"])
    if new <= GRANDFATHERED:
        continue
    want = bump(old, newer["kinds"], newer["breaking"])
    if new != want:
        kinds = ", ".join(sorted(newer["kinds"])) + (", Breaking" if newer["breaking"] else "")
        problems.append(f"CHANGELOG.md: [{newer['name']}] follows {older['name']} with {kinds}: it must be {vstr(want)} (CONTRIBUTING.md#versioning)")


def tracked(pattern):
    out = subprocess.run(["git", "ls-files", pattern], capture_output=True, text=True, check=True).stdout
    return [p for p in out.splitlines() if p]


def slug(heading):
    # GitHub's anchor: lower case, drop punctuation except - and _, spaces to -.
    s = re.sub(r"[`*_~]", "", heading.strip().lower())
    s = re.sub(r"[^\w\- ]", "", s)
    return s.replace(" ", "-")


anchors = {}


def anchors_of(path):
    if path not in anchors:
        found, seen, fence = set(), {}, False
        for line in open(path, encoding="utf-8"):
            if line.lstrip().startswith("```"):
                fence = not fence
            if fence:
                continue
            m = re.match(r"^(#{1,6})\s+(.*?)\s*#*\s*$", line)
            if m:
                s = slug(m.group(2))
                n = seen.get(s, 0)
                seen[s] = n + 1
                found.add(s if n == 0 else f"{s}-{n}")
        anchors[path] = found
    return anchors[path]


LINK = re.compile(r"\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")
docs = tracked("*.md")
for doc in docs:
    fence = False
    for no, line in enumerate(open(doc, encoding="utf-8"), 1):
        if line.lstrip().startswith("```"):
            fence = not fence
        if fence:
            continue
        for target in LINK.findall(line):
            if re.match(r"^[a-z]+:", target) or target.startswith("mailto:"):
                continue
            path, _, anchor = target.partition("#")
            dest = os.path.normpath(os.path.join(os.path.dirname(doc), path)) if path else doc
            if not os.path.exists(dest):
                problems.append(f"{doc}:{no}: link to missing {target}")
            elif anchor and dest.endswith(".md") and anchor not in anchors_of(dest):
                problems.append(f"{doc}:{no}: no heading for #{anchor} in {dest}")
        # A rule that quotes the phrase ("never 'the next release'") is not the phrase.
        bare = re.sub(r"\"[^\"]*\"|`[^`]*`", "", line)
        if re.search(r"(?i)\bnext release\b|not released yet", bare):
            problems.append(f"{doc}:{no}: says 'next release'/'not released yet' (name the version)")
        if re.search(r"(?i)windows (x64 )?and linux", line) and not re.search(r"(?i)macos|every platform", line):
            problems.append(f"{doc}:{no}: names Windows and Linux without macOS")

version = tomllib.load(open("Cargo.toml", "rb"))["workspace"]["package"]["version"]
places = {
    "mod/quayflow/.claude-plugin/plugin.json": json.load(open("mod/quayflow/.claude-plugin/plugin.json"))["version"],
    "mod/quayflow/hooks/context.ts": (re.search(r"MOD_VERSION = '([^']+)'", open("mod/quayflow/hooks/context.ts").read()) or [None, None])[1],
    "mod/quayflow/README.md": (re.search(r"version (\d+\.\d+\.\d+)", open("mod/quayflow/README.md").read()) or [None, None])[1],
    "mod/quayflow/tests/quayflow.test.ts": (re.search(r"modVersion: '([^']+)'", open("mod/quayflow/tests/quayflow.test.ts").read()) or [None, None])[1],
}
if released and version != released[0]["name"]:
    problems.append(f"Cargo.toml: version {version}, the newest CHANGELOG release is {released[0]['name']}")
for where, v in places.items():
    if v != version:
        problems.append(f"{where}: version {v}, Cargo.toml says {version}")
lock = open("Cargo.lock").read()
if f'name = "quayflow"\nversion = "{version}"' not in lock:
    problems.append(f"Cargo.lock: quayflow is not {version} (cargo update -w)")

for p in problems:
    print(p)
print(f"{len(docs)} docs checked, {len(problems)} problem(s)")
sys.exit(1 if problems else 0)
