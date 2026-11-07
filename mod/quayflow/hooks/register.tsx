// Quayflow Mod: the in-Claude face of the per-user Quayflow engine.
//
// It never reads or writes a credential. It reports each request to the
// engine, verifies which account a request used when the engine asks,
// recovers a request that failed at a usage limit (one retry at most), and
// shows usage, suggestions and controls through the status line, /quay and
// its pane. In a headless run (`claude -p`, the SDK) only the hooks run: no
// pane, prompt, toast, status or log line. With no compatible engine,
// everything Quayflow does is off and Claude behaves as without it.
import type { Elements, EngineInterface, Register } from 'claude-code'

import { Q, errorText, pause, readHintText, setLink, setSnapshot, shownLink, uiLog } from './context'
import type { UsageDisplay } from './context'
import { establish, stuckConnecting } from './engine'
import { onStep } from './step'
import { openPane, reconnect, runCommand, HELP } from './commands'
import { renderPane } from './pane'
import { paintStatus, refresh, startWatcher } from './watcher'

export const register: Register = (on, options) => {
  Q.usageDisplay = usageDisplayOption(options?.statusLineUsage)
  on('session.start', async ($, e, next) => {
    const started = await next(e)
    Q.startedAt = await $.clock.now().catch(() => Date.now())
    Q.cwd = e.cwd
    Q.headless = !e.isInteractive || e.surface === null
    Q.sessionId = await $.session.id().catch(() => '')
    await $.command.register({
      name: 'quay',
      description: 'Quayflow: accounts, usage and switching (/quay help)',
      argumentHint: '[status|accept|use <account>|unpin|mode|group|pause|resume|register|add|lanes|report|whoami|help]',
    })
    // Off the startup path: the session never waits for the engine.
    let settle: () => void = () => {}
    Q.ready = new Promise<void>(resolve => (settle = resolve))
    // Counts as a handshake under way from here, so the pane does not start another.
    Q.linkAttempts += 1
    void (async () => {
      try {
        Q.claudeVersion = (await $.session.version().catch(() => undefined))?.version ?? ''
        // A reload keeps the pane's last link: one that was on is not painted "connecting"
        // over while this instance shakes hands again (it is replaced by the outcome).
        if (!(await shownLink($))?.on) await setLink($, { on: false, reason: 'connecting' })
        await establish($, { allowStart: true })
      } catch {
        // establish fails open; nothing to do
      } finally {
        Q.linkAttempts -= 1
        settle()
      }
      if (Q.headless) return
      try {
        await refresh($)
        await paintStatus($)
      } catch (err) {
        uiLog($, `Quayflow: could not draw its status (${errorText(err)}); it will retry.`)
      }
      // The watcher keeps the status current and reconnects; it must start whatever happened above.
      startWatcher($)
    })().catch(() => {})
    return started
  })

  // A resume or /clear goes on in this process under another session id, and no
  // session.start fires for it: the pane's values (held per session) start over as
  // "connecting" while this module is still linked. Carry the link over to the new session.
  on('session.end', async ($, e, next) => {
    const ended = await next(e)
    if (e.reason === 'clear' || e.reason === 'resume') void pause($, 300).then(() => resync($)).catch(() => {})
    return ended
  })

  on('turn.step', async function* ($, e, next) {
    return yield* onStep($, e, next)
  })

  on('command.run', { command: 'quay' }, async ($, e) => {
    const args = e.args.trim()
    // The status line is budgeted to the terminal's width, as the last command saw it.
    const columns = e.presentation?.columns
    if (typeof columns === 'number' && columns > 0 && columns !== Q.columns) {
      Q.columns = columns
      if (!Q.headless) void paintStatus($).catch(() => {})
    }
    if (!args && !Q.headless) {
      await openPane($)
      return { text: 'Quayflow pane opened (keys 1-5 switch tabs; Esc returns to the prompt).' }
    }
    if (!args && Q.headless) return { text: await runCommand($, 'status') }
    const text = await runCommand($, args)
    if (text.startsWith('pane:')) {
      const sub = text.slice(5)
      await openPane($, 'onboarding', sub)
      return { text: `Quayflow pane opened: ${sub === 'add' ? 'add an account' : 'register the current login'}.` }
    }
    return { text }
  }).catch(() => ({ text: `Quayflow could not run that command.\n${HELP}` }))

  // The everyday status line, dim after the prompt hint (the terminal draws `tail`).
  on('ui.render', { component: 'PromptHint' }, async ($, e, next) => {
    if (Q.headless || e.surface !== 'terminal') return next(e)
    if (!Q.footer) {
      Q.footer = true
      // Not from inside this draw: drawing is pure, so the repaint runs as its own event.
      Q.repaint?.()
    }
    const hint = await readHintText($)
    return hint ? next({ ...e, props: { ...e.props, tail: `  ${hint}` } }) : next(e)
  })
  on('ui.render', { component: 'Pane', requestId: 'quay' }, async ($, e) => {
    // A pane left on "connecting" with no handshake under way starts one; a pane that
    // shows "off" while this module is linked (another session's values) is repainted.
    if (stuckConnecting()) void reconnect($).catch(() => {})
    else if (Q.link.on && !(await shownLink($))?.on) void resync($).catch(() => {})
    const els = $.ui.resolve(e) as unknown as Elements['terminal']
    return renderPane($, e, els, Q.claudeVersion)
  })
}

let resyncing = false

/**
 * Brings this session's pane values and the engine's record up to date with the
 * module: after a resume or /clear the session id changes, so the engine hears a
 * hello under the new id and the snapshot is read again for it.
 */
async function resync($: EngineInterface): Promise<void> {
  if (resyncing || Q.headless) return
  resyncing = true
  try {
    const id = await $.session.id().catch(() => '')
    const moved = !!id && id !== Q.sessionId
    if (moved) {
      Q.sessionId = id
      Q.startedAt = await $.clock.now().catch(() => Date.now())
    }
    await setLink($, Q.link)
    await setSnapshot($, Q.state ?? null)
    if (!Q.link.on) await reconnect($)
    else if (moved) {
      await establish($, { allowStart: false })
      await refresh($)
    }
    await paintStatus($)
  } finally {
    resyncing = false
  }
}

/** The `statusLineUsage` option (plugin.json `userConfig`): `auto` unless set to `show` or `hide`. */
function usageDisplayOption(v: unknown): UsageDisplay {
  return v === 'show' || v === 'hide' ? v : 'auto'
}
