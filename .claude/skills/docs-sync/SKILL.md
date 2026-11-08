---
name: docs-sync
description: Bring Quayflow's docs in step with a code change (a command, option, default, file, platform, flow or test count). Use after changing user-visible behavior, before a release, or when asked to review or update the docs.
---

# Keeping the docs in step

1. **What changed?** `git diff <base> --stat`, and the CHANGELOG `[Unreleased]` lines.
2. **Where it is described:**

| Change | Docs |
|---|---|
| CLI command or option | `crates/quayflow/src/main.rs` HELP, `docs/user-guide.md`, `docs/tech-spec.md` (CLI section) |
| Default, threshold, timing | `qf-model` `Tuning`, `docs/user-guide.md`, `docs/tech-spec.md`, the README where quoted |
| API route or field | `docs/tech-spec.md` §6, `mod/quayflow/types/index.d.ts` |
| Pane screen, button, flow | `docs/user-guide.md`, README Quick start, `mod/quayflow/README.md` |
| File, folder, lock | `docs/tech-spec.md`, `docs/user-guide.md` (Files), `docs/security.md` |
| Platform or asset | README (platform table, Install), `docs/user-guide.md` (requirements, platform section), `docs/tech-spec.md` §1a/§1b, `docs/security.md`, `docs/known-limits.md`, `docs/requirements.md`, `docs/verify-release.md`, `CONTRIBUTING.md`, `tests/e2e/README.md`, `CLAUDE.md` |
| Test script or check count | `docs/acceptance.md` (counts), `tests/e2e/README.md`, `CONTRIBUTING.md` |
| Real-login result | the `real-test` skill |

3. **Read each affected section in full** and check it against the code, quoting the code's actual strings. Name all three platforms; say a version, never "the next release".
4. **For a large change or before a release**, have reviewers read the docs line by line against the code, split as: README; user guide; tech spec, security, requirements and verify-release; everything else. Verify each finding before editing.
5. `python3 tests/docs/check-docs.py` must pass. Docs-only commits start with `docs:`.
