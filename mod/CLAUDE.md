# The Mod (mod/quayflow): rules

- **Headless output is never changed.** In `claude -p` or the SDK the Mod prints, asks and toasts nothing, through the `Q.headless` gates in `context.ts`. Every engine failure returns Claude's result untouched.
- **A cancellation is the person's.** Never catch it into a result: rethrow when `next.signal.aborted`.
- **State lives in `$.state` atoms, per session.** Module variables (`Q`, poll timers, `generation`) reset on reload and are shared across sessions; a flow must still work after `/reload-plugins` and after `/clear` or resume.
- **Form state belongs to one flow.** Clear the `add:*` / `reg:*` fields a flow sets when it starts again; a choice made for one sign-in never carries over to the next.
- Every `await` on the host that can reject has a `.catch`; a hook that throws can print on stderr in headless runs.
- Tests: `claude plugin test mod/quayflow`; the harness and conventions are in the root CLAUDE.md, "Mod notes".
