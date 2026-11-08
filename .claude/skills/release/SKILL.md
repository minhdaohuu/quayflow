---
name: release
description: Ship a Quayflow release (vX.Y.Z): checks, version bump in every place, CHANGELOG, docs, the tag (pushed by the user), then verification of the published assets and attestations. Use when the user asks to release, cut or publish a version.
---

# Releasing Quayflow

1. **Ready?** `git status` clean on `main`, fetched and up to date. The last CI run on `main` passed (`gh run list --branch main --limit 1`). Run the `review` skill if it has not run since the last release.
2. **Version.** Pick X.Y.Z (0.x: minor for features, patch for fixes). Set it in every place CLAUDE.md lists: `Cargo.toml`, `Cargo.lock` (`cargo update -w --offline`), `mod/quayflow/.claude-plugin/plugin.json`, `hooks/context.ts` `MOD_VERSION`, `mod/quayflow/README.md` (two places), `tests/quayflow.test.ts` `modVersion`.
3. **CHANGELOG.** Move `[Unreleased]` into `## [X.Y.Z] - <date>` with a one-line summary; leave `[Unreleased]` empty; update the compare links at the bottom.
4. **Docs.** "currently vX.Y.Z" (`SECURITY.md`, `docs/known-limits.md`), "Quayflow vX.Y.Z" (`docs/user-guide.md`), the scope line (`docs/tech-spec.md`), example versions in `docs/verify-release.md` (`$v`, `v=`, `git checkout`), and an older version where an example means "not the latest". A new platform or asset: the `docs-sync` skill. Run `python3 tests/docs/check-docs.py`.
5. **Checks.** fmt, clippy, `QUAYFLOW_KEYCHAIN=off cargo test --workspace --locked`, the smoke and installer tests for this OS, `claude plugin test mod/quayflow`. Commit `release: vX.Y.Z, <summary>`, push (ask first), wait for CI to pass.
6. **Release workflow changed?** Dry run first: Actions > Release > Run workflow on `main` (builds twice and compares, publishes nothing).
7. **Tag.** Pushing a tag publishes a public release: give the user the command, do not run it.
   `git tag -a vX.Y.Z -m vX.Y.Z <commit> && git push origin vX.Y.Z`
8. **After publishing.** Watch the Release run (`gh run list --workflow release.yml`). Then, in a new folder in the scratchpad: `gh release view vX.Y.Z` (every asset present), download this OS's assets, check `SHA256SUMS` (`shasum -a 256 -c --ignore-missing SHA256SUMS` or `sha256sum -c`), `SOURCE_COMMIT` equals the tagged commit, and for each program and installer `gh attestation verify <file> --repo minhdaohuu/quayflow --signer-workflow minhdaohuu/quayflow/.github/workflows/release.yml --source-ref refs/tags/vX.Y.Z`. Run the program's `version`.
9. **Report** what was verified, and what is still untested on each platform (`docs/known-limits.md`).
