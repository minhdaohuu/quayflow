# Plan

Work agreed but not started, in order. Each item moves to `CHANGELOG.md` when it ships.

## 1. Weekly balancing

Status: done in `[Unreleased]` (CHANGELOG, Changed). Not done: the `sim-usage.ps1` scenario with `seven_day_reset_s`.

### Problem

A proactive switch has one trigger: the live account's binding window (the fuller of 5h, 7d and the per-model weekly window) reaches the group threshold (85%) (`proactive()` in `crates/qf-policy/src/lib.rs`). Below that, nothing moves. Typical use stays under 85% of 5h, so one account carries all the work. Its 7d window climbs toward 100% while the other accounts stay unused. Two things make this worse:

- Return home switches back to the Preferred account as soon as its 5h window has room. That drains the home account's weekly allowance first.
- Each account's 7d window resets at its own time. Weekly allowance an idle account has not used by its reset is lost. The busy account then hits its weekly limit and is blocked for days.

### Decisions (agreed 2026-11-25)

Balancing is part of account switching, not a separate feature. There is no on/off setting: the group's mode decides, as for every proactive switch (Automatic switches, Suggest suggests, Manual ignores).

1. **Weekly score.** `score = (100 - pct7d) / max(hours_to_7d_reset, 5)`: the weekly room that would be lost soonest scores highest. The 5-hour floor caps the score of an account close to its reset at its room over 5 hours; it can still be the best target, since that room is lost at the reset (review correction: the floor limits the score, it does not prevent the switch). Only the general 7d window is scored; per-model weekly windows still act through the binding-window gate.
2. **5h is a gate, not a score.** An account is a target only if its binding window (fullest of 5h, 7d and per-model weekly) is at or below threshold minus margin, as today.
3. **Pick.** Every proactive switch, in every group, picks the eligible account with the highest weekly score; scores within 5%/day are a tie, which goes to the account with more five-hour room, then rank (review fix). Recovery after a real limit picks the same way among accounts with room to gain, otherwise the most room below the limit, then soonest available (review fix).
4. **Balancing trigger.** Below the threshold, switch when the best eligible candidate's score is at least 1.5 times the live account's and at least 5%/day higher. The same gap is needed to switch back, which prevents switching back and forth.
5. **Timing and cooldown.** Same as any proactive switch: no idle wait, the existing 5-minute proactive cooldown (`Tuning::cooldown_s`), restored from the journal after an engine restart (review fix).
6. **Home wins.** With a Preferred account in the governing groups (`effective_home`), return home and the threshold trigger work as today and the balancing trigger does not apply. The threshold switch's pick still uses the score.
7. **Unscored accounts.** A fresh reading with no 7d window or no 7d reset time gets no score. Such an account stays eligible but ranks after every scored account (among themselves by today's rule: lowest binding, then rank). It never triggers a balancing switch, and an unscored live account never balances away (the threshold still applies). Stale or missing readings are never targets, as today.
8. **Lanes: out of scope.** An account live in another environment is treated as today.

Behavior change for existing configurations: an Automatic group now also switches on weekly balance, and the threshold switch may pick a different account. Modes are not changed. CHANGELOG under Changed (minor version).

### Work

- `qf-policy`: weekly score; `rank_candidates` by score with unscored last; balancing trigger in `proactive()` after return home and before the threshold check; reason texts for the switch and for `Proactive::None` (why no balancing switch). Add to `check()` in `invariants.rs`: no balancing switch on stale, missing or unscored data; none with a home account; none inside the cooldown; destination allowed by every governing group; strictest mode.
- `qf-engine`: no new state (the existing cooldown timestamp is used); a journal reason that says the switch was for weekly balance; decision text in the state view. Engine `tick()` test with two accounts and different 7d resets; Suggest and Automatic both.
- CLI and Mod: show the reason; nothing new to configure.
- `tests/sim`: `sim-usage.ps1` scenario using `seven_day_reset_s`.
- Docs: user guide (Modes, defaults per group, how a target is picked), tech spec (ranking), requirements (new FR), CHANGELOG.

## 2. `quayflow status`: full usage for every account

Do this after item 1, so the status view can show the weekly score.

### Problem

The Accounts section of `quayflow status` (`print::status` in `crates/quayflow/src/print.rs`) prints one figure per account: its binding window (`45% 5h (2 min old)`). It does not show:

- the 5h and 7d windows separately, or the per-model weekly window;
- when each window resets;
- the account's weekly score and whether it is the recommended target;
- which environment an account is live in, when there are several (only a `▶` mark).

### Work

- Print every window per account with its reset countdown, for example `5h 45% ↻ 2 h 10 min · 7d 62% ↻ 3 d 4 h · Opus 7d 70% ↻ 3 d 4 h`. A stale reading stays `usage unknown (last read … ago)`; no figure is printed as current.
- Show the weekly score and mark the recommended account (★), as the Mod's fleet chart does.
- Show the weekly score in the "Why this account?" review too (`explain_all`, `inspect.rs`), so the reason one eligible account is picked over another is visible.
- Name the environment for the live mark.
- Check the API v1 state view already carries every window and reset time; add them if not.
- `--json` stays as it is or only gains fields.
- Update the user guide's `quayflow status` description; CHANGELOG under Changed.
