## What and why

<!-- One or two sentences. Link the issue. -->

## How it was tested

- [ ] `cargo fmt`, `cargo clippy -D warnings`, `cargo test` pass
- [ ] Mod: `claude plugin validate` and `claude plugin test` pass (if the Mod changed)
- [ ] `tests/e2e/smoke-fake-env.ps1` passes (if engine or CLI behavior changed)
- [ ] Real-login scenarios run by a maintainer (if switching behavior changed): ______

## Checklist

- [ ] No secrets, emails or conversation content in code, tests, logs or fixtures
- [ ] Does not weaken consent, group permissions, managed-login enforcement or other restrictions
- [ ] `docs/tech-spec.md` and `CHANGELOG.md` updated where relevant
