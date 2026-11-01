// Quayflow Mod: the in-Claude face of the per-user Quayflow engine.
//
// It never reads or writes a credential. It reports each request to the
// engine, verifies which account a request used when the engine asks,
// recovers a request that failed at a usage limit (one retry at most), and
// shows usage, suggestions and controls through the status line, /quay and
// its pane. In a headless run (`claude -p`, the SDK) only the hooks run: no
// pane, prompt, toast, status or log line. With no compatible engine,
// everything Quayflow does is off and Claude behaves as without it.
import type { Elements, Register } from 'claude-code'

import { Q, setLink } from './context'
import { establish } from './engine'
import { onStep } from './step'
import { openPane, runCommand, HELP } from './commands'
import { renderPane } from './pane'
import { paintStatus, refresh, startWatcher } from './watcher'

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    const started = await next(e)
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
    void (async () => {
      try {
        Q.claudeVersion = (await $.session.version().catch(() => undefined))?.version ?? ''
        await setLink($, { on: false, reason: 'connecting' })
        await establish($, { allowStart: true })
      } catch {
        // establish fails open; nothing to do
      } finally {
        settle()
      }
      if (Q.headless) return
      await refresh($).catch(() => undefined)
      await paintStatus($)
      startWatcher($)
    })()
    return started
  })

  on('turn.step', async function* ($, e, next) {
    return yield* onStep($, e, next)
  })

  on('command.run', { command: 'quay' }, async ($, e) => {
    const args = e.args.trim()
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

  on('ui.render', { component: 'Pane', requestId: 'quay' }, async ($, e) => {
    const els = $.ui.resolve(e) as unknown as Elements['terminal']
    return renderPane($, e, els, Q.claudeVersion)
  })
}
