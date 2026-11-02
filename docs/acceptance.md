# Acceptance status (Quayflow v0.1)

The [release conditions](requirements.md#release-conditions-for-v01) require recorded results on Windows against the **product build**. This table keeps three kinds of evidence apart. They are not interchangeable.

- **Logic:** Rust unit and scenario tests on a fake Claude environment (`cargo test`; scenarios in `crates/qf-engine/tests/engine.rs`). They prove decisions and file handling. They cannot prove client adoption, isolation or refresh behavior.
- **Smoke:** `tests/e2e/smoke-fake-env.ps1`, which runs the real `quayflow.exe` process against a fake environment (21 checks), and `tests/e2e/cli-fake-env.ps1`, which covers CLI argument validation and error paths the same way (70 checks).
- **Simulation:** `tests/e2e/sim-usage.ps1` scripts usage over time from a loopback fake of the usage and profile API (`tests/sim/fake_api.py`) and checks observation, suggestion, Automatic switching, return home, stale data, backoff, rejected tokens, hard-limit recovery, concurrent failures and the all-exhausted case (25 checks). It proves Quayflow's logic against these fakes, not that the real service behaves the same.
- **Real:** owner-run scripts against real logins (`tests/e2e/real-switching.ps1` and the manual steps below). Only this kind of evidence can pass a scenario.


**Status on 2026-11-03:** Logic, Smoke and Simulation pass in CI. The first real-login run (`real-switching.ps1`, cases H, S, R1, R2) passed 13/13 on 2026-11-03 with two real accounts (one organization, one personal) on Claude Code 2.1.316. That run used a **local build of commit `0e657ca`** (GNU toolchain), not a published release, so the rows below say "Passed (local build)"; they must be repeated on the release build before v0.1 ships.

An earlier real run the same day found a defect, fixed in `0e657ca`: a request that was already in flight when a switch was written finished on the previous account (as designed) and was taken as proof that Claude does not adopt a switched login, which turned in-place switching off. Requests are now classified by when they started.

| ID | Scenario | Logic / Smoke evidence | Real (product build) | How to run |
|---|---|---|---|---|
| AC-01 | Ordinary interactive launch | Smoke: engine start, single instance, no window | Not run | Install, run `claude`, check status line |
| AC-02 | Ordinary headless launch | — | Passed (local build, 2026-11-03): cases H, S, R1 each exited 0 with one JSON result | `real-switching.ps1 -Case H` (one JSON result) |
| AC-03 | Register existing login | Logic `register_requires_consent_and_group`; Smoke | Not run | `/quay register` or `quayflow register` |
| AC-04 | Add or cancel another login | — (needs sign-in window) | Not run | `quayflow add`, cancel once, complete once |
| AC-05 | Reauthenticate, duplicate, remove, dead refresh | Logic: duplicate, `model_not_found`/auth fallback | Not run | `quayflow reauth`, `remove`; dead token needs a revoked login |
| AC-06 | Unassigned project uses Default | Logic `project_resolution_longest_match` | Not run | Start Claude in an unassigned folder; `/quay status` |
| AC-07 | Explicit project / hostile repository | Logic (assignments only from the engine's config) | Not run | Assign a folder; add `.claude/` content in the repo; check the group |
| AC-08 | Suggest in headless | Logic `recovery_refuses_unconfirmed_limits_and_suggest_mode_only_records` | Passed (local build, 2026-11-03): case R2, no switch, live account unchanged | `real-switching.ps1 -Case R2` |
| AC-09 | Threshold and stale usage | Policy tests `stale_is_never_zero`, `threshold_switches_to_lowest_fresh_with_margin` | Not run | Watch `quayflow status` over a session |
| AC-10 | Account ranking | Policy `ranks_break_close_choices_only`, exclusions explained in status | Not run | `quayflow status` eligibility column |
| AC-11 | Group conflict | Logic `group_conflict_names_the_blocking_session` | Not run | Two sessions in different groups; `/quay use` |
| AC-12 | Lanes | Logic `create_check_delete_keeps_shared_setup`, `lanes_switch_independently` | Not run (release condition) | `quayflow lanes create work`; run Claude with plugins in the lane |
| AC-13 | Mid-job switch | Logic `manual_switch_pins_and_tracks_adoption`, `request_in_flight_at_the_switch_is_not_adoption_evidence` | Passed (local build, 2026-11-03): case H, 2 requests before the swap on the first account, 3 after on the second | `real-switching.ps1 -Case H` |
| AC-14 | Concurrent requests and subagents | Logic: concurrent recovery shares one switch | Passed (local build, 2026-11-03): case S, two parallel subagents, 4 requests before and 4 after the swap on the expected accounts | `real-switching.ps1 -Case S` |
| AC-15 | Local tools and user input | — | Passed (local build, 2026-11-03): case H, sequential tools continued across the swap in the same process | Case H (tools continue; no replay) |
| AC-16 | Lock held before write | Logic `held_lock_leaves_login_unchanged`; Smoke | Not run | Covered by Smoke on the real binary; repeat with real login optional |
| AC-17 | Post-write uncertainty | Logic: uncertain path reconciled from file state | Not run | Kill the engine during a switch (manual) |
| AC-18 | Account adoption | Logic adoption tracking | Passed (local build, 2026-11-03): cases H and S, per-request identity matched the switch | Cases H and S (per-request identity) |
| AC-19 | Synthetic pre-request verification | Not built: out-of-group switches are refused | n/a | — |
| AC-20 | Refresh race | Logic `native_refresh_is_captured_into_the_vault` | Not run | Long session across a token refresh; vault stays valid |
| AC-21 | Hard-limit recovery | Logic `recovery_automatic_switches_once_and_retries_once`; Simulation | Simulated limit passed (local build, 2026-11-03): case R1, one recovery decision with a switch and one retry; **a real usage limit is still a release condition** | `real-switching.ps1 -Case R1` (simulated), then a real limit |
| AC-22 | Unknown side effect | Logic: one retry of the failed request only | Not run | Review journal after R1 |
| AC-23 | All accounts exhausted | Logic `all_exhausted_switches_to_soonest_without_retry` | Not run (**release condition**: Claude's wait adopts a switch) | Needs all accounts at a limit |
| AC-24 | Deadline, cancellation, exited process | Logic: aborted requests never retried (Mod tests) | Not run | Ctrl+C during a held job |
| AC-25 | Headless output | — | Passed (local build, 2026-11-03): cases H, S, R1, one JSON result each | Cases H, S, R1 check one JSON result |
| AC-26 | Open or replace with full history | Logic history copy tests | Not run | `/quay replace <lane>` in PowerShell 7 and 5.1 |
| AC-27 | Failed handoff | Logic `refuses_diverged_destination` | Not run | Diverge a lane copy; retry handoff |
| AC-28 | Engine stop, crash, restart | Logic `crash_reconciliation_never_replays`; Smoke stop respected | Not run (release condition) | Kill the engine mid-switch; restart; read journal |
| AC-29 | Vault inaccessible | Logic `protect_failure_writes_nothing` | Not run | Copy the vault to another Windows user and try to load |
| AC-30 | Diagnostics and cleanup | Logic `journal_and_report_hold_no_secrets`; Smoke | Not run | `quayflow report`; `quayflow uninstall --delete-data` |
| AC-31 | Unknown version | Logic: static check per version | Not run | Install a new Claude version; check `quayflow canary` |
| AC-32 | Denied capability | No bypass code exists | n/a | — |
| AC-33 | Update or mixed runtimes | Logic: oldest running version governs | Not run | Run two Claude versions; update |
| AC-34 | Long-duration and fault testing | — | Not run | 24 h run with `quayflow status` sampling |
| AC-35 | Mixed-identity group | Logic and Smoke: disclosure once | Not run | Register org + personal in Default |
| AC-36 | Manual pin | Policy `pin_holds_until_release_level`; Smoke | Not run | `/quay use`, observe no suggestions |
| AC-37 | Version check | Logic `adoption_mismatch_disables_in_place_for_the_version` | Not run | `quayflow canary --live` |
| AC-38 | Sign-in environment retirement | Logic `fresh_environment_and_retire` | Not run | `quayflow add`; later switch to that account |
| AC-39 | Unknown live login | Logic `unknown_login_pauses_and_manual_switch_backs_it_up`; Smoke | Not run | `/login` to an unregistered account |
| AC-40 | Managed login enforcement | Logic `evaluates_sources`, `managed_login_and_health_exclude` | Not run (needs a managed machine) | Machine with `forceLoginOrgUUID` |
| AC-41 | Return home | Policy `return_home_when_preferred_has_room` | Not run | Set Preferred; wait for its reset |
| AC-42 | Mod/engine version mismatch | Logic `handshake_rejects_incompatible_protocol`; Smoke | Not run | Old Mod with new engine |
| AC-43 | Plan compatibility | Logic `model_not_found_after_switch_switches_back`, `plan_incompatibility_excludes` | Not run | Pro account + Opus-only model |

## Release conditions

| Condition | Status |
|---|---|
| All scenarios pass against the product build | Open |
| Real usage-limit failure observed end to end; Claude's own retry does not compete | Open |
| Claude's own usage-limit wait adopts a login switched during the wait | Open |
| Claude reads and writes correctly through lane links | Open (logic passes; real lane session pending) |
| Crash reconciliation, version gating, dead-refresh-token handling built and passing | Built; logic passes; real runs open |
| Release artifacts hashed, attested, reproducible; installer and updater refuse a mismatch | Built in the release workflow and updater; first tagged release pending |
| Disclaimer published | Done (README, installer, onboarding) |
| Managed-login override documented with its risk | Done (`docs/security.md`); override not implemented in this build |
