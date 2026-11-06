// Flows of the /quay pane that are not top tabs: engine off and
// incompatible states, first use, the switch flow, group conflict and trust views.
import type { EngineInterface } from 'claude-code'

import type { EnvView, SigninStatus } from '../types'
import { Q, flash, patchSigninStatus, pause, setDisclosure, setSignin, setView } from './context'
import { establish, hasCode } from './engine'
import { actCancelTransition, actDorg, actIgnoreUnknown, actOnboarding, actRegister, actSigninCancel, actSigninConfirm, actSigninStart, actSigninStatus, actSwitchTo, switchOutcome } from './actions'
import {
  account,
  adoptionWords,
  label,
  linkOffText,
  myEnv,
  myGroup,
  orgName,
  kindText,
  failureWords,
  sessionName,
  transitionWords,
  usageText,
  when,
} from './format'
import { refresh } from './watcher'
import { Btn, Field, Line, Pick, Row, Section, Toggle, clearForm, go, run, setForm } from './ui'
import type { Ctx } from './ui'

// ------------------------------------------------------------ engine off

export function OffView($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const reason = c.link.reason
  if (reason === 'no_consent') return Welcome($, c)
  const lines = [Line(c, `Quayflow is off: ${linkOffText(c.link)}.`, { tone: reason === 'incompatible' ? 'warn' : undefined })]
  lines.push(Line(c, 'Switching, recovery and holds are off; Claude behaves exactly as without Quayflow.', { dim: true }))
  if (reason === 'incompatible') {
    lines.push(
      Section(
        c,
        'Mod and engine are incompatible',
        Line(c, c.link.message ?? 'The engine does not accept this Mod\'s protocol.'),
        Line(c, 'A different version number alone is not an incompatibility: the engine accepts the current and the previous protocol.', { dim: true }),
        Line(c, 'Update both as a matched pair, in a terminal:  quayflow update'),
        Line(c, 'The update is hash-verified and never replaces components during a transition.', { dim: true }),
      ),
    )
  }
  if (reason === 'not_installed') {
    lines.push(Line(c, 'Install the engine with the Quayflow install script, then open /quay again.'))
  }
  const buttons = []
  if (reason === 'stopped_by_user') {
    buttons.push(Btn(c, 'start', 'Start engine', () => void startEngine($, c), true))
  } else if (reason === 'unreachable' || reason === 'start_failed') {
    buttons.push(Btn(c, 'retry', 'Try again', () => void startEngine($, c), true))
  }
  return (
    <Box flexDirection="column">
      {lines}
      {buttons.length > 0 && Row(c, ...buttons)}
    </Box>
  )
}

async function startEngine($: EngineInterface, c: Ctx): Promise<void> {
  await run($, 'starting the engine', async () => {
    await establish($, { allowStart: true, explicit: true })
    await refresh($)
    return Q.link.on
  }, on => (on ? 'Engine running.' : `Engine not available: ${linkOffText(Q.link)}.`))
}

// ------------------------------------------------------------ first use

const CONSENTS = [
  { key: 'c:engineStart', label: 'Start the Quayflow engine on demand when a Claude session finds none running (required to continue)' },
  { key: 'c:signinStartup', label: 'Also start it at Windows sign-in (optional)' },
  { key: 'c:credentialStorage', label: 'Store reusable copies of logins I register in Quayflow\'s protected vault (needed to register accounts)' },
  { key: 'c:terms', label: 'I am responsible for complying with the provider\'s terms and my organization\'s rules (required)' },
]

/** A consent the engine already holds (from the last state read); false when unknown. */
function given(c: Ctx, key: string): boolean {
  const consents = (c.s?.consents ?? {}) as Record<string, unknown>
  return !!consents[key.replace(/^c:/, '')]
}

/** A Welcome toggle: what the person set ('1' / '0'), else the consent already given. */
function consentOn(c: Ctx, key: string): boolean {
  const v = c.form[key]
  if (v === '1') return true
  if (v === '0') return false
  return given(c, key)
}

export function Welcome($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const on = (k: string): boolean => consentOn(c, k)
  const ready = on('c:engineStart') && on('c:terms')
  return (
    <Box flexDirection="column">
      {Line(c, 'Welcome to Quayflow. You keep typing claude; nothing about your workflow changes.')}
      {Section(
        c,
        'What it is',
        Line(c, '- This Mod shows account usage and suggestions inside Claude and verifies which account each session really uses.'),
        Line(c, '- A per-user engine (quayflow.exe, no window, no admin) watches usage of the accounts you register and changes the login when you or your group policy say so.'),
        Line(c, '- Credentials stay on this machine, in a protected vault (Windows DPAPI). Nothing is uploaded.'),
        Line(c, '- A switch changes the login of a credential environment: every Claude session using it moves at its next request. Requests in progress finish on their account.'),
        Line(c, '- Modes: Suggest (default) shows a candidate and switches on /quay accept. Automatic is opt-in per group. Unattended headless jobs (claude -p) only switch or recover in Automatic mode.'),
      )}
      {Section(c, 'Consent (each is separate; only consents you already gave are checked)', ...CONSENTS.map(k => Toggle(c, k.key, k.label, on(k.key), v => setForm($, k.key, v ? '1' : '0'))))}
      {Row(
        c,
        Btn(c, 'welcome-continue', ready ? 'Continue' : 'Continue (needs the two required consents)', () => {
          if (!ready) {
            void flash($, 'warn', 'Engine start and the terms acknowledgment are required to continue.')
            return
          }
          void finishWelcome($, c)
        }, true),
        Btn(c, 'welcome-not-now', 'Not now', () => void $.ui.close({ id: 'quay' }).catch(() => {})),
      )}
      {Line(c, 'Not now closes this without starting the engine, storing credentials or switching anything.', { dim: true })}
    </Box>
  )
}

async function finishWelcome($: EngineInterface, c: Ctx): Promise<void> {
  const ok = await run($, 'starting the engine and recording your consent', async () => {
    if (!Q.link.on) await establish($, { allowStart: true, explicit: true })
    if (!Q.link.on) throw new Error(`the engine did not start: ${linkOffText(Q.link)}`)
    // The engine reads an explicit false as a revocation: an optional consent is sent only
    // when the person changed it here (ticked it, or unticked one already given).
    const consents: Record<string, boolean> = { engineStart: true, terms: true }
    for (const key of ['c:signinStartup', 'c:credentialStorage']) {
      const v = c.form[key]
      const had = given(c, key)
      if (v === '1' && !had) consents[key.slice(2)] = true
      else if (v === '0' && had) consents[key.slice(2)] = false
    }
    const r = await actOnboarding($, consents, true)
    if (!r.ok) throw new Error(r.message ?? 'the engine did not record the consent')
    return true
  })
  if (ok) go($, { tab: 'onboarding', sub: 'readiness' })
}

export function Onboarding($: EngineInterface, c: Ctx): JSX.Element {
  const sub = c.view.sub ?? (c.s?.onboarded ? 'readiness' : 'welcome')
  if (sub === 'welcome') return Welcome($, c)
  if (sub === 'register') return Register($, c)
  if (sub === 'add') return AddAccount($, c)
  if (sub === 'done') return Ready($, c)
  return Readiness($, c)
}

function Readiness($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const s = c.s
  if (!s) return Line(c, 'Reading the engine state...')
  const env = myEnv(s)
  const compat = s.compat.find(r => r.version === c.claudeVersion)
  const checks = [
    Line(c, `Engine: running, version ${s.engine.version}, protocol ${s.engine.protocol}.`, { tone: 'ok' }),
    compat
      ? Line(
          c,
          `Claude ${compat.version}: static check ${compat.staticPass ? 'passed' : 'failed'}${compat.livePass === false ? '; a live adoption check failed (in-place switching off)' : ''}.`,
          { tone: compat.staticPass && compat.livePass !== false ? 'ok' : 'warn' },
        )
      : Line(c, `Claude ${c.claudeVersion || '(version unknown)'}: not checked yet; the engine runs a static check before any in-place switch.`, { tone: 'warn' }),
    env
      ? Line(
          c,
          env.live.signedOut
            ? 'Live login: signed out. Sign in with Claude first; a signed-out environment cannot be registered.'
            : env.live.known
              ? `Live login: registered as "${label(s, env.live.account)}".`
              : `Live login: account #${env.live.account ?? '?'}${env.live.kind ? ` (${kindText(env.live.kind, env.live.subscription)})` : ''} is not registered yet.`,
          { tone: env.live.known ? 'ok' : 'warn' },
        )
      : Line(c, 'Environment: not detected yet.', { tone: 'warn' }),
    env && (env.managed.forceOrg?.length || env.managed.forceMethod)
      ? Line(c, `Organization-managed login enforcement detected (${(env.managed.sources ?? []).join(', ')}): accounts outside it are not eligible here.`, { tone: 'warn' })
      : Line(c, 'No organization-managed login enforcement detected.', { dim: true }),
  ]
  const buttons = []
  if (env && !env.live.known && !env.live.signedOut) buttons.push(Btn(c, 'ready-register', 'Register current login', () => go($, { tab: 'onboarding', sub: 'register' }), true))
  buttons.push(Btn(c, 'ready-add', 'Add another account', () => go($, { tab: 'onboarding', sub: 'add' })))
  buttons.push(Btn(c, 'ready-version', 'Version details', () => go($, { tab: 'settings' })))
  buttons.push(Btn(c, 'ready-done', 'Done', () => go($, { tab: 'onboarding', sub: 'done' })))
  return (
    <Box flexDirection="column">
      {Section(c, 'Readiness (independent checks; none switches an account)', ...checks)}
      {Row(c, ...buttons)}
    </Box>
  )
}

function groupOptions(c: Ctx): { value: string; label: string }[] {
  return (c.s?.groups ?? []).map(g => ({ value: g.name, label: `${g.name}${g.isDefault ? ' (default)' : ''}` }))
}

function Register($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const s = c.s
  if (!s) return Line(c, 'Reading the engine state...')
  const env = myEnv(s)
  const group = c.form['reg:group'] ?? s.defaultGroup
  const needsConsent = !s.consents.credentialStorage
  const consent = c.form['reg:consent'] === '1'
  return (
    <Box flexDirection="column">
      {Section(
        c,
        'Register the current login',
        Line(c, `Identity (verified by a profile lookup, not by its name): account #${env?.live.account ?? '?'}${env?.live.kind ? `, ${kindText(env.live.kind, env.live.subscription)}` : ''}, in environment ${env?.id ?? '?'}.`),
        Line(c, 'Registering stores a reusable copy of this login in the protected vault. It does not switch, change ongoing work, or enable Automatic.', { dim: true }),
      )}
      {Field($, c, 'reg:label', 'Label', { placeholder: 'work, personal, ...', submitLabel: 'set' , onSubmit: () => {} })}
      {Pick(c, 'reg:group', 'Group (required)', groupOptions(c), group, v => setForm($, 'reg:group', v))}
      {Line(c, 'Adding an account to the default group makes it eligible for unfamiliar and future projects that have no assignment.', { dim: true })}
      {needsConsent && Toggle(c, 'reg:consent', 'Store a reusable copy of this login in Quayflow\'s protected vault', consent, v => setForm($, 'reg:consent', v ? '1' : ''))}
      {Row(
        c,
        Btn(c, 'reg-save', 'Save account', () => void saveRegister($, c), true),
        Btn(c, 'reg-cancel', 'Cancel', () => go($, { tab: c.s?.onboarded ? 'accounts' : 'onboarding', sub: c.s?.onboarded ? undefined : 'readiness' })),
      )}
    </Box>
  )
}

async function saveRegister($: EngineInterface, c: Ctx): Promise<void> {
  const s = c.s
  if (!s) return
  const labelText = (c.form['reg:label'] ?? '').trim()
  const group = c.form['reg:group'] ?? s.defaultGroup
  if (!labelText) return void flash($, 'warn', 'Type a label first (Enter in the Label field).')
  if (!s.consents.credentialStorage && c.form['reg:consent'] !== '1') {
    return void flash($, 'warn', 'Storage consent is needed to register: tick the vault box.')
  }
  const a = await run(
    $,
    'saving to the vault',
    async () => {
      if (!s.consents.credentialStorage) await actOnboarding($, { credentialStorage: true }, false)
      return actRegister($, labelText, group)
    },
    undefined,
    err => reviewDisclosure($, err, { then: 'register', group, label: labelText }),
  )
  if (a) await afterRegister($, c, a.label, group, a.note)
}

/** A 409 `needs_disclosure` opens the one-time disclosure; the change is resent only after acceptance. */
async function reviewDisclosure(
  $: EngineInterface,
  err: unknown,
  d: { then: 'register' | 'confirm'; group: string; label: string; signinId?: string },
): Promise<boolean> {
  if (!hasCode(err, 'needs_disclosure')) return false
  await setDisclosure($, { ...d, text: err.message })
  await setView($, { tab: 'groups', sub: 'disclosure' })
  return true
}

export async function afterRegister($: EngineInterface, c: Ctx, labelText: string, group: string, note?: string): Promise<void> {
  await clearForm($, 'reg:label', 'reg:consent')
  go($, c.s?.onboarded && c.view.tab !== 'onboarding' ? { tab: 'accounts' } : { tab: 'onboarding', sub: 'done' })
  await flash($, 'info', `Saved "${labelText}" to the vault in group ${group}. Nothing was switched.${note ? ` ${note}` : ''}`)
}

// Native sign-in for another account.

let signinPoll = 0

async function followSignin($: EngineInterface, id: string): Promise<void> {
  const mine = ++signinPoll
  for (let i = 0; i < 600 && mine === signinPoll; i++) {
    await pause($, 2_000)
    if (mine !== signinPoll) return
    let st: SigninStatus
    try {
      st = await actSigninStatus($, id)
    } catch {
      continue
    }
    await patchSigninStatus($, st)
    if (st.state !== 'waiting') return
  }
}

function AddAccount($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const s = c.s
  if (!s) return Line(c, 'Reading the engine state...')
  const si = c.signin
  if (!si || si.status.state === 'cancelled' || si.status.state === 'saved') {
    const group = c.form['add:group'] ?? s.defaultGroup
    return (
      <Box flexDirection="column">
        {Section(
          c,
          c.view.arg ? `Sign in again for "${label(s, c.view.arg)}"` : 'Add another account',
          Line(c, 'Claude\'s own sign-in runs in a separate, temporary environment: your current login is not touched, and nothing is activated.'),
          si?.status.state === 'cancelled' ? Line(c, 'The last sign-in was cancelled; the existing login is intact.', { dim: true }) : null,
        )}
        {!c.view.arg && Field($, c, 'add:label', 'Label', { placeholder: 'personal, team, ...', submitLabel: 'set', onSubmit: () => {} })}
        {!c.view.arg && Pick(c, 'add:group', 'Group (required)', groupOptions(c), group, v => setForm($, 'add:group', v))}
        {Row(
          c,
          Btn(c, 'add-continue', 'Continue with native sign-in', () => void startSignin($, c), true),
          Btn(c, 'add-cancel', 'Cancel', () => go($, { tab: s.onboarded ? 'accounts' : 'onboarding', sub: s.onboarded ? undefined : 'readiness' })),
        )}
      </Box>
    )
  }
  const st = si.status
  if (st.state === 'waiting') {
    return (
      <Box flexDirection="column">
        {Section(
          c,
          'Native sign-in in progress',
          Line(c, 'A new window runs Claude\'s own sign-in. Finish it there; this view follows its real state.'),
          Line(c, `If no window opened, run this in a terminal: ${st.command}`),
        )}
        {Row(c, Btn(c, 'signin-cancel', 'Cancel sign-in', () => void cancelSignin($, c, st.id)))}
      </Box>
    )
  }
  if (st.state === 'failed') {
    return (
      <Box flexDirection="column">
        {Line(c, `Sign-in did not complete: ${st.message ?? 'unknown reason'}. Your existing login is unchanged.`, { tone: 'warn' })}
        {Row(c, Btn(c, 'signin-again', 'Try again', () => void setSignin($, null)), Btn(c, 'signin-close', 'Close', () => void closeSignin($, c)))}
      </Box>
    )
  }
  // ready: identity review before saving
  const id = st.identity
  const dup = id?.duplicateOf
  const unexpected = id?.unexpected
  return (
    <Box flexDirection="column">
      {Section(
        c,
        'Review the new identity',
        Line(c, `Account #${id?.account ?? '?'} · ${id?.kind ?? '?'}${id?.subscription ? ` · ${id.subscription}` : ''}`),
        id?.display ? Line(c, `Shown once for review, not stored: ${id.display}`, { dim: true }) : null,
        Line(c, si.reauth ? 'Reauthentication for an existing account.' : `Label "${si.label}" · group ${si.group}`),
        dup ? Line(c, `This identity is already registered as "${dup}". It is the same capacity, not an additional account.`, { tone: 'warn' }) : null,
        unexpected ? Line(c, 'This is a different identity from the account being reauthenticated. It will not replace it.', { tone: 'error' }) : null,
        Line(c, 'Saving stores the credential in the protected vault, then retires the temporary copy without signing out (signing out could revoke the stored refresh token).', { dim: true }),
      )}
      {Row(
        c,
        !unexpected && Btn(c, 'signin-save', dup ? 'Update existing registration' : 'Save account', () => void confirmSignin($, c), true),
        Btn(c, 'signin-discard', 'Cancel', () => void cancelSignin($, c, st.id)),
      )}
    </Box>
  )
}

async function startSignin($: EngineInterface, c: Ctx): Promise<void> {
  const s = c.s
  if (!s) return
  const reauth = c.view.arg
  const labelText = (c.form['add:label'] ?? '').trim()
  const group = c.form['add:group'] ?? s.defaultGroup
  if (!reauth && !labelText) return void flash($, 'warn', 'Type a label first (Enter in the Label field).')
  if (!s.consents.credentialStorage) {
    return void flash($, 'warn', 'Credential storage consent is needed first: register the current login or review consents in Settings.')
  }
  const st = await run($, 'opening a native sign-in', () => actSigninStart($, reauth))
  if (!st) return
  await setSignin($, { status: st, label: labelText || label(s, reauth), group, reauth })
  void followSignin($, st.id)
}

async function cancelSignin($: EngineInterface, c: Ctx, id: string): Promise<void> {
  signinPoll++
  await run($, 'cancelling the sign-in', () => actSigninCancel($, id), () => 'Sign-in cancelled. Your existing login is unchanged.')
  await setSignin($, null)
}

async function closeSignin($: EngineInterface, c: Ctx): Promise<void> {
  signinPoll++
  await setSignin($, null)
  go($, { tab: c.s?.onboarded ? 'accounts' : 'onboarding', sub: c.s?.onboarded ? undefined : 'readiness' })
}

async function confirmSignin($: EngineInterface, c: Ctx): Promise<void> {
  const si = c.signin
  if (!si) return
  const r = await run(
    $,
    'saving to the vault',
    () => actSigninConfirm($, si.status.id, si.label, si.group),
    undefined,
    err => reviewDisclosure($, err, { then: 'confirm', group: si.group, label: si.label, signinId: si.status.id }),
  )
  if (r) await afterConfirm($, c, si.label, si.group, r.message ?? undefined)
}

export async function afterConfirm($: EngineInterface, c: Ctx, labelText: string, group: string, message?: string): Promise<void> {
  signinPoll++
  await setSignin($, null)
  await clearForm($, 'add:label')
  await setView($, c.s?.onboarded ? { tab: 'accounts' } : { tab: 'onboarding', sub: 'done' })
  await flash($, 'info', message ?? `Saved "${labelText}" to the vault in group ${group}. The temporary sign-in copy is retired; nothing was switched.`)
}

function Ready($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const s = c.s
  const env = s ? myEnv(s) : undefined
  return (
    <Box flexDirection="column">
      {Section(
        c,
        `Ready in ${env?.mode ?? 'Suggest'} mode`,
        Line(c, `Effective mode here: ${env?.mode ?? 'suggest'} (group ${s ? myGroup(s) : '?'}). In Suggest, Quayflow shows a candidate in the status line and switches on /quay accept.`),
        (s?.accounts.length ?? 0) < 2 ? Line(c, 'One registered account is valid; there is simply no alternative to switch to until you add another.', { dim: true }) : null,
        Line(c, 'Unattended headless jobs (claude -p) only switch and recover in Automatic mode, which you approve per group.'),
      )}
      {Row(
        c,
        Btn(c, 'done-overview', 'Open overview', () => go($, { tab: 'overview' }), true),
        Btn(c, 'done-automatic', 'Review Automatic', () => go($, { tab: 'groups', sub: 'group', arg: s ? myGroup(s) : undefined })),
        Btn(c, 'done-add', 'Add another account', () => go($, { tab: 'onboarding', sub: 'add' })),
      )}
    </Box>
  )
}

// ------------------------------------------------------------ switch flow

export function SwitchReview($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const s = c.s
  if (!s) return Line(c, 'Reading the engine state...')
  const env = myEnv(s)
  const target = account(s, c.view.arg)
  if (!env || !target) return Line(c, 'That account is not known.')
  const idle = c.form['sw:idle'] === '1'
  return (
    <Box flexDirection="column">
      {Section(
        c,
        `Switch environment ${env.id} to "${target.label}"`,
        Line(c, `Usage: ${usageText(target, true)}${target.eligibility && !target.eligibility.eligible ? ` · not eligible: ${target.eligibility.reason}` : ''}`),
        Line(c, `Every session of ${env.id} moves at its next request (${env.sessions.length} listed; other clients, such as editor extensions, may not be listed). Requests in progress finish on the current account.`),
        Line(c, `A manual choice is pinned for ${s.tuning.pinMinutes} min (or until you unpin, its window resets, or it reaches ${s.tuning.pinReleasePct}%).`),
        Line(c, 'The prompt cache is not shared between accounts: the first request after a switch processes the full context again.', { dim: true }),
        env.inPlace.allowed ? null : Line(c, `In-place switching is off here: ${env.inPlace.reason}.`, { tone: 'warn' }),
      )}
      {Toggle(c, 'sw:idle', `Prefer a moment with no listed session busy (waits up to ${s.tuning.idleWaitS} s)`, idle, v => setForm($, 'sw:idle', v ? '1' : ''))}
      {Row(
        c,
        Btn(c, 'sw-go', 'Switch and pin', () => void doSwitch($, c, target.id, idle), true),
        Btn(c, 'sw-cancel', 'Cancel', () => go($, { tab: 'accounts', sub: 'detail', arg: target.id })),
      )}
    </Box>
  )
}

async function doSwitch($: EngineInterface, c: Ctx, to: string, idle: boolean): Promise<void> {
  const r = await run($, 'asking the engine to switch', () => actSwitchTo($, to, idle))
  if (!r) return
  if (!r.ok && r.blockedBy) {
    setForm($, 'conflict', JSON.stringify(r.blockedBy))
    await setView($, { tab: 'switch', sub: 'conflict', arg: to })
    return
  }
  await flash($, r.ok ? 'info' : 'warn', switchOutcome(Q.state, r, true))
  if (r.ok) await setView($, { tab: 'switch', sub: 'progress' })
}

export function SwitchProgress($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const s = c.s
  if (!s) return Line(c, 'Reading the engine state...')
  const env = myEnv(s) as EnvView
  const t = env?.transition ?? env?.lastSwitch
  if (!t) return (
    <Box flexDirection="column">
      {Line(c, 'No transition for this environment.')}
      {Row(c, Btn(c, 'sp-back', 'Back to overview', () => go($, { tab: 'overview' })))}
    </Box>
  )
  const beforeWrite = t.state === 'requested' || t.state === 'preparing' || t.state === 'idle_wait'
  const failure = failureWords(t)
  const mine = env.adoption.filter(a => a.transition === t.id)
  const mismatch = mine.some(a => a.state === 'mismatch')
  const allAdopted = mine.length > 0 && mine.every(a => a.state === 'adopted')
  const head =
    t.state === 'completed' && allAdopted
      ? `Observed clients adopted "${label(s, t.to)}".`
      : failure
        ? `Switch to "${label(s, t.to)}": ${failure}.`
        : `Switch to "${label(s, t.to)}": ${transitionWords(t.state)}.`
  return (
    <Box flexDirection="column">
      {Section(
        c,
        `Environment ${t.env}`,
        Line(c, head, { tone: t.state === 'failed' || t.state === 'uncertain' || mismatch ? 'warn' : undefined }),
        Line(c, `${t.how.replace('_', ' ')}: ${t.reason} · requested ${when(t.requestedAt)}${t.writtenAt ? ` · stored ${when(t.writtenAt)}` : ''}`, { dim: true }),
        t.detail ? Line(c, t.detail, { dim: true }) : null,
      )}
      {mine.length > 0 &&
        Section(
          c,
          'Per client (stored destination vs last request identity)',
          ...mine.map(a => {
            const x = env.sessions.find(z => z.pid === a.pid)
            return Line(
              c,
              `${x ? sessionName(x) : `pid ${a.pid}`}${a.sessionId === Q.sessionId ? ' [this session]' : ''}: ${adoptionWords(a.state)} · stored "${label(s, a.destination)}"${a.lastRequestAccount ? ` · last request "${label(s, a.lastRequestAccount)}"` : ''}${a.version ? ` · Claude ${a.version}` : ''}`,
              { tone: a.state === 'mismatch' ? 'error' : undefined },
            )
          }),
          Line(c, 'Idle clients adopt at their own next request; nothing is sent just to turn this list green.', { dim: true }),
        )}
      {mismatch &&
        Section(
          c,
          'Adoption mismatch',
          Line(c, 'A client\'s request did not use the stored account. In-place switching is now disabled for that Claude version; recovery is under way.', { tone: 'warn' }),
          Line(c, 'Fallback: continue the session in a lane with full history (Groups > Lanes), or restart that Claude session.'),
        )}
      {Row(
        c,
        beforeWrite && Btn(c, 'sp-cancel', 'Cancel switch', () => void cancelTransition($, c, t.id)),
        Btn(c, 'sp-overview', 'Back to /quay', () => go($, { tab: 'overview' })),
        Btn(c, 'sp-activity', 'View activity', () => go($, { tab: 'activity' })),
        mismatch && Btn(c, 'sp-lanes', 'Lanes', () => go($, { tab: 'groups', sub: 'lanes' })),
      )}
      {beforeWrite && Line(c, 'Closing this view does not cancel; Cancel switch asks the engine and waits for its answer.', { dim: true })}
    </Box>
  )
}

async function cancelTransition($: EngineInterface, c: Ctx, id: string): Promise<void> {
  await flash($, 'info', 'Cancellation requested.')
  try {
    const r = await actCancelTransition($, id)
    await flash(
      $,
      r.started ? 'warn' : 'info',
      r.started ? 'Transition already started; checking state.' : r.ok ? 'Cancelled before a write; the login is unchanged.' : (r.message ?? 'The engine did not cancel.'),
    )
  } catch (err) {
    await flash($, 'error', `Cancel not confirmed: ${(err as Error).message}. Checking state.`)
  }
}

export function Conflict($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  let blocked: { pid?: number; name?: string; cwd?: string; group?: string; sessionId?: string } = {}
  try {
    blocked = JSON.parse(c.form['conflict'] ?? '{}')
  } catch {}
  const name = blocked.name ?? `${(blocked.cwd ?? '?').split(/[\\/]/).filter(Boolean).pop()} (pid ${blocked.pid ?? '?'})`
  return (
    <Box flexDirection="column">
      {Section(
        c,
        'A group conflict blocks the shared switch',
        Line(c, `Session "${name}" (project ${blocked.cwd ?? '?'}, group ${blocked.group ?? '?'}) does not permit "${label(c.s ?? undefined, c.view.arg)}".`),
        Line(c, 'A shared switch would move that session too, so nothing was switched. Its group is a data approval; it is not widened here.'),
        Line(c, 'Remedy: move that session into a lane (its own credential environment) with full history, from that session: /quay replace <lane> or /quay open <lane>. Then switch the rest.'),
        Line(c, 'Automatic mode never moves a session into a lane by itself.', { dim: true }),
      )}
      {Row(
        c,
        Btn(c, 'cf-lanes', 'Review move to lane', () => go($, { tab: 'groups', sub: 'lanes' }), true),
        Btn(c, 'cf-keep', 'Keep shared login', () => go($, { tab: 'overview' })),
        Btn(c, 'cf-policy', 'View project policy', () => go($, { tab: 'groups' })),
      )}
    </Box>
  )
}

// ------------------------------------------------------------ trust views

export function Trust($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const s = c.s
  if (!s) return Line(c, 'Reading the engine state...')
  const env = myEnv(s)
  if (!env) return Line(c, 'No environment.')
  const sub = c.view.sub
  if (sub === 'unknown') {
    return (
      <Box flexDirection="column">
        {Section(
          c,
          'An unknown login is live',
          Line(c, `Environment ${env.id} uses account #${env.live.account ?? '?'}${env.live.kind ? ` (${kindText(env.live.kind, env.live.subscription)})` : ''}, which Quayflow has not registered (for example after /login, a sign-out, or another tool).`),
          Line(c, `Automatic switching and recovery are paused for ${env.id}.`, { tone: 'warn' }),
          Line(c, 'Register it to resume, or ignore the notice: automation then stays paused while this login is live.'),
          Line(c, 'A manual switch stays possible; Quayflow first stores a protected backup of this login, and refuses the switch if it cannot.', { dim: true }),
        )}
        {Row(
          c,
          !env.live.signedOut && Btn(c, 'tu-register', 'Register current login', () => go($, { tab: 'onboarding', sub: 'register' }), true),
          Btn(c, 'tu-ignore', 'Ignore for now', () => void run($, 'recording', () => actIgnoreUnknown($), r => (r.ok ? 'Noted. Automation stays paused while this login is live.' : (r.message ?? 'Refused.')))),
          Btn(c, 'tu-back', 'Back', () => go($, { tab: 'overview' })),
        )}
      </Box>
    )
  }
  if (sub === 'switcher') {
    const n = env.notices.find(x => x.kind === 'other_switcher')
    return (
      <Box flexDirection="column">
        {Section(
          c,
          'Another switcher may be active',
          Line(c, n?.text ?? 'The live login changed repeatedly without Quayflow.'),
          Line(c, 'Automation is paused for this environment. Running two switchers over one environment is not supported; stop the other tool, then resume.', { tone: 'warn' }),
        )}
        {Row(c, Btn(c, 'sw-activity', 'Inspect activity', () => go($, { tab: 'activity' })), Btn(c, 'sw-back', 'Close', () => go($, { tab: 'overview' })))}
      </Box>
    )
  }
  if (sub === 'dorg') return DOrg($, c)
  // managed enforcement
  const m = env.managed
  const excluded = s.accounts.filter(a => a.eligibility && !a.eligibility.eligible && /organi[sz]ation|managed|policy|enforc/i.test(a.eligibility.reason))
  return (
    <Box flexDirection="column">
      {Section(
        c,
        'Organization-managed login enforcement',
        (m.forceOrg?.length ?? 0) > 0 ? Line(c, `Enforced organization: ${(m.forceOrg ?? []).map(o => orgName(s, o)).join(', ')}`) : null,
        m.forceMethod ? Line(c, `Enforced login method: ${m.forceMethod}`) : null,
        Line(c, `Sources: ${(m.sources ?? []).join(', ') || 'none'}`, { dim: true }),
        Line(c, 'Accounts that do not match are not eligible here; your organization\'s administrator controls this. Group membership does not override it.'),
        ...excluded.map(a => Line(c, `- ${a.label}: ${a.eligibility?.reason}`, { dim: true })),
      )}
      {Row(
        c,
        Btn(c, 'mg-accounts', 'Choose a matching account', () => go($, { tab: 'accounts' })),
        Btn(c, 'mg-dorg', 'Review owner-specified exception', () => go($, { tab: 'trust', sub: 'dorg' })),
        Btn(c, 'mg-close', 'Close', () => go($, { tab: 'overview' })),
      )}
    </Box>
  )
}

function DOrg($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const s = c.s!
  const env = myEnv(s)
  const orgs = [...new Set(s.envs.flatMap(e => e.managed.forceOrg ?? []))]
  if (s.dOrg.enabled) {
    return (
      <Box flexDirection="column">
        {Section(
          c,
          'Managed-login exception is ON',
          Line(c, `Quayflow may switch to accounts that organization #${s.dOrg.confirmedOrg ?? '?'}'s login policy would refuse (confirmed ${when(s.dOrg.confirmedAt)}).`, { tone: 'warn' }),
          Line(c, 'Every switch it allows that the policy would have refused is recorded in the journal. This works around your organization\'s access control; responsibility is yours.'),
          Line(c, 'If the policy source or the organization changes, a new acknowledgment is required before it is used again.', { dim: true }),
        )}
        {Row(
          c,
          Btn(c, 'dorg-disable', 'Disable exception', () => void run($, 'turning the exception off', () => actDorg($, false, s.dOrg.confirmedOrg ?? '', ''), r => (r.ok ? 'Managed-login exception is off; enforcement is respected again.' : (r.message ?? 'Refused.'))), true),
          Btn(c, 'dorg-activity', 'Review affected switches', () => go($, { tab: 'activity' })),
          Btn(c, 'dorg-back', 'Back to /quay', () => go($, { tab: 'overview' })),
        )}
      </Box>
    )
  }
  if (!orgs.length) {
    return (
      <Box flexDirection="column">
        {Line(c, 'No organization-managed login enforcement is detected, so there is nothing to make an exception for.')}
        {Row(c, Btn(c, 'dorg-back', 'Back', () => go($, { tab: 'settings' })))}
      </Box>
    )
  }
  const org = c.form['dorg:org'] ?? orgs[0]!
  const orgText = orgName(s, org)
  const phrase = `I accept the risk for ${orgText}`
  const typed = (c.form['dorg:ack'] ?? '').trim()
  return (
    <Box flexDirection="column">
      {Section(
        c,
        'Managed-login exception: explicit risk review',
        Line(c, `${orgText[0]!.toUpperCase()}${orgText.slice(1)} (#${org}) enforces which accounts may sign in to Claude in ${env?.id ?? 'this environment'} (${(env?.managed.sources ?? []).join(', ') || 'managed settings'}).`),
        Line(c, 'Turning this on lets Quayflow switch to accounts that policy would refuse at sign-in.', { tone: 'warn' }),
        Line(c, 'Risk: this works around your organization\'s access control. It can violate your employment or organization policy and expose company work to non-organization accounts. Responsibility is yours.', { tone: 'warn' }),
        Line(c, 'Group approval, version checks and the credential-selector restriction still apply. Each switch the policy would have refused is recorded.', { dim: true }),
      )}
      {orgs.length > 1 && Pick(c, 'dorg:org', 'Enforcing organization', orgs.map(o => ({ value: o, label: orgName(s, o) })), org, v => setForm($, 'dorg:org', v))}
      {Line(c, `To enable, type exactly: ${phrase}`)}
      {Field($, c, 'dorg:ack', 'Acknowledgment', { placeholder: phrase, submitLabel: 'check', onSubmit: () => {} })}
      {Row(
        c,
        Btn(c, 'dorg-keep', 'Keep enforcement enabled', () => go($, { tab: 'settings' }), true),
        Btn(c, 'dorg-enable', 'Enable exception', () => {
          if (typed !== phrase) return void flash($, 'warn', 'The acknowledgment does not match; the exception stays off.')
          void run($, 'recording the exception', () => actDorg($, true, org, typed), r =>
            r.ok ? 'Managed-login exception recorded as ON. It is shown in the status line and here until you disable it.' : (r.message ?? 'Refused.'),
          ).then(() => clearForm($, 'dorg:ack'))
        }),
      )}
    </Box>
  )
}

