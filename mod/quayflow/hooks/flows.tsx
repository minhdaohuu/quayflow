// Flows of the /quay pane that are not top tabs: engine off and
// incompatible states, first use, the switch flow, group conflict and trust views.
import type { EngineInterface } from 'claude-code'

import type { EnvView, IdentityView, SigninIdentity, SigninStatus } from '../types'
import { Q, flash, patchSigninStatus, pause, setDisclosure, setSignin, setView } from './context'
import { establish, hasCode } from './engine'
import { actAccept, actCancelTransition, actDorg, actGroupCreate, actIdentity, actIgnoreUnknown, actOnboarding, actRegister, actSigninCancel, actSigninConfirm, actSigninStart, actSigninStatus, actSwitchTo, switchOutcome } from './actions'
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
import { Alert, Badge, Btn, Card, Field, Gauge, Hint, KV, Line, Pick, Row, Section, Steps, Table, Toggle, clearForm, go, run, setForm, useRead } from './ui'
import { minutes, phases, switchHeading } from './words'
import type { Ctx } from './ui'

// ------------------------------------------------------------ engine off

export function OffView($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const reason = c.link.reason
  if (reason === 'no_consent') return Welcome($, c)
  // Connecting is a moment, not "off": the handshake or the engine start is under way.
  const head = reason === 'connecting' ? `◌ ${c.link.message ? c.link.message[0]!.toUpperCase() + c.link.message.slice(1) : 'Connecting to the engine'}…` : `Quayflow is off: ${linkOffText(c.link)}.`
  const lines = [Line(c, head, { tone: reason === 'incompatible' ? 'warn' : undefined })]
  { const h = Hint(c, 'Switching, recovery and holds are off; Claude behaves exactly as without Quayflow.'); if (h) lines.push(h) }
  if (reason === 'incompatible') {
    lines.push(
      Section(
        c,
        'Incompatible',
        Line(c, c.link.message ?? 'The engine does not accept this Mod\'s protocol.'),
        Hint(c, 'A different version number alone is not an incompatibility: the engine accepts the current and the previous protocol.'),
        Line(c, 'Update both as a matched pair, in a terminal:  quayflow update'),
        Hint(c, 'The update is hash-verified and never replaces components during a transition.'),
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
  }, on => (on ? 'Engine running.' : `The engine did not start: ${linkOffText(Q.link)}.`))
}

// ------------------------------------------------------------ first use

const CONSENTS = [
  { key: 'c:engineStart', label: 'Start the engine when Claude needs it (required)' },
  { key: 'c:signinStartup', label: 'Also start it when you sign in' },
  { key: 'c:credentialStorage', label: 'Keep registered logins in the protected vault' },
  { key: 'c:terms', label: 'I follow my provider\'s terms and my organization\'s rules (required)' },
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
      {Section(
        c,
        'Welcome to Quayflow',
        Line(c, 'Watches your accounts\' usage and switches the login before a usage limit.'),
        Line(c, 'You keep typing claude. Logins stay on this machine.', { dim: true }),
        Hint(c, 'A per-user engine (quayflow, no window, no admin) watches usage of the accounts you register. Credentials stay in a vault for your user only (Windows: DPAPI; Linux and macOS: an owner-only file); nothing is uploaded. Suggest proposes and switches on /quay accept; unattended headless jobs (claude -p) only switch or recover in Automatic mode.'),
      )}
      {Alert(c, 'Default group: Automatic', 'Accounts in the Default group switch by themselves, including in headless jobs (claude -p). Groups you create start in Suggest. Change a group\'s mode in Groups or with /quay mode.', 'note')}
      {Alert(c, 'Shared login, shared effect', 'A switch moves every Claude session of this login at its next request.', 'note')}
      {Section(c, 'Consent', ...CONSENTS.map(k => Toggle(c, k.key, k.label, on(k.key), v => setForm($, k.key, v ? '1' : '0'))))}
      {Hint(c, 'Before a login is saved, you review its identity, its group and the credential-storage consent.')}
      {Line(c, 'Quayflow is not affiliated with or endorsed by Anthropic.', { dim: true })}
      {Row(
        c,
        Btn(c, 'welcome-continue', 'Continue', () => {
          if (!ready) {
            void flash($, 'warn', 'Engine start and the terms acknowledgment are required to continue.')
            return
          }
          void finishWelcome($, c)
        }, true),
        Btn(c, 'welcome-not-now', 'Not now', () => void $.ui.close({ id: 'quay' }).catch(() => {})),
      )}
      {Hint(c, 'Not now closes this without starting the engine, storing credentials or switching anything.')}
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
  const compat = s.compat.find(r => r.version === (env?.inPlace.governingVersion ?? c.claudeVersion)) ?? s.compat.find(r => r.version === c.claudeVersion)
  const vaultBroken = s.accounts.some(a => a.health === 'vault_error')
  const managed = !!env && (!!env.managed.forceOrg?.length || !!env.managed.forceMethod)
  const rows: string[][] = [
    ['Engine', `${s.engine.version} \u00b7 compatible`],
    ['Vault', vaultBroken ? 'Error: a protected copy cannot be read' : s.consents.credentialStorage ? 'Ready' : 'Ready (consent asked at registration)'],
    ['Claude', c.claudeVersion || 'unknown'],
    ['Static canary', compat ? (compat.staticPass ? 'Passed' : 'Failed: in-place switching off') : 'Not checked yet'],
    [
      'Current login identity',
      !env ? 'Not detected yet' : env.live.signedOut ? 'Signed out: sign in first' : env.live.known ? `Known: ${label(s, env.live.account)}` : `Not registered (#${env.live.account ?? '?'}${env.live.kind ? `, ${kindText(env.live.kind, env.live.subscription)}` : ''})`,
    ],
    ['Live adoption', compat?.livePass === true ? 'Passed' : compat?.livePass === false ? 'Failed: in-place switching off' : 'Checked at the next real switch'],
    ['Managed login', managed ? 'Enforced: other accounts not eligible' : 'None'],
  ]
  const register = !!env && !env.live.known && !env.live.signedOut
  return (
    <Box flexDirection="column">
      {Section(c, 'Readiness', Table(c, 'readiness', ['Check', 'Result'], rows))}
      {Alert(c, 'No credential change performed', 'Static checks inspect the required behavior and endpoint shapes. They never perform a hidden live switch.')}
      {compat && !compat.staticPass ? Line(c, 'In-place switching is off for this Claude version; use Session actions.', { tone: 'warn' }) : null}
      {Row(
        c,
        register && Btn(c, 'ready-register', 'Register login', () => go($, { tab: 'onboarding', sub: 'register' }), true),
        Btn(c, 'ready-add', 'Add account', () => go($, { tab: 'onboarding', sub: 'add' }), !register && s.accounts.length === 0),
        Btn(c, 'ready-version', 'Version', () => go($, { tab: 'settings', sub: 'version' })),
        Btn(c, 'ready-done', 'Done', () => go($, { tab: 'onboarding', sub: 'done' }), !register && s.accounts.length > 0),
      )}
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
  const live = env && !env.live.signedOut ? env.live.account : undefined
  const idr = useRead<IdentityView>($, c, 'identity', `env:${env?.id ?? '?'}:${live ?? ''}`, () => actIdentity($, { env: env?.id }))
  const id = idr.value && idr.value.account ? idr.value : undefined
  const kind = id ? kindText(id.kind, id.subscription) : env?.live.kind ? kindText(env.live.kind, env.live.subscription) : ''
  const isDefault = group === s.defaultGroup
  const suggestion = suggestName(id ? { account: id.account, kind: id.kind, display: id.display } : undefined, s.accounts)
  const name = (c.form['reg:label'] ?? suggestion).trim()
  const taken = name ? s.accounts.find(a => a.label.toLowerCase() === name.toLowerCase()) : undefined
  const newGroup = c.form['reg:showgroup'] === '1'
  const problem = c.form['reg:problem']
  return (
    <Box flexDirection="column">
      {Section(
        c,
        'Register the account you are signed in to',
        KV(c, 'Signed in as', `${id?.display ?? (idr.pending ? 'checking\u2026' : idr.error ? `not readable (${idr.error})` : 'no e-mail in the login')}${kind ? ` \u00b7 ${kind}` : ''}`),
        KV(c, 'Organization', id?.orgName ?? (id?.kind === 'personal' || env?.live.kind === 'personal' ? 'none (personal)' : 'not named')),
      )}
      {Field($, c, 'reg:label', 'Name', { placeholder: 'work, personal, ...', submitLabel: 'save', autoFocus: true, initial: suggestion, onSubmit: v => void saveRegister($, c, v) })}
      {taken ? Line(c, `\u26a0 "${taken.label}" is already a name here; pick another.`, { tone: 'warn' }) : null}
      {Row(
        c,
        Pick(c, 'reg:group', 'Group', groupOptions(c), group, v => setForm($, 'reg:group', v)),
        newGroup ? null : Btn(c, 'reg-newgroup', '+ New group', () => setForm($, 'reg:showgroup', '1')),
      )}
      {newGroup ? Field($, c, 'reg:newgroup', 'New group', { placeholder: 'name', submitLabel: 'create', autoFocus: true, onSubmit: v => void createGroupInline($, v, 'reg:group').then(() => setForm($, 'reg:showgroup', '')) }) : null}
      {Hint(c, isDefault ? 'The default group applies to all projects without an explicit assignment, including unfamiliar and future ones.' : `Group "${group}" applies only to projects assigned to it.`)}
      {!s.consents.credentialStorage
        ? Toggle(c, 'reg:consent', 'Keep my logins in my local vault (asked once)', c.form['reg:consent'] === '1', v => {
            setForm($, 'reg:consent', v ? '1' : '')
            setForm($, 'reg:problem', '')
          })
        : null}
      {problem ? Line(c, `\u26a0 ${problem}`, { tone: 'warn' }) : null}
      {Hint(c, 'The e-mail address is shown for your review only and never stored. Registering changes nothing that is running.')}
      {Row(
        c,
        Btn(c, 'reg-save', 'Save', () => void saveRegister($, c), true),
        Btn(c, 'reg-cancel', 'Cancel', () => go($, { tab: c.s?.onboarded ? 'accounts' : 'onboarding', sub: c.s?.onboarded ? undefined : 'readiness' })),
      )}
    </Box>
  )
}

async function createGroupInline($: EngineInterface, name: string, formKey: string): Promise<void> {
  const n = name.trim()
  if (!n) return
  const r = await run($, 'creating the group', () => actGroupCreate($, n), r => (r.ok ? `Group "${n}" created (Suggest mode) and selected.` : (r.message ?? 'Refused.')))
  if (r?.ok) {
    setForm($, formKey, n)
    await clearForm($, formKey === 'reg:group' ? 'reg:newgroup' : 'add:newgroup')
  }
}

async function saveRegister($: EngineInterface, c: Ctx, typed?: string): Promise<void> {
  const s = c.s
  if (!s) return
  const idv = c.cache['identity']?.value as IdentityView | undefined
  const suggestion = suggestName(idv && idv.account ? { account: idv.account, kind: idv.kind, display: idv.display } : undefined, s.accounts)
  // `typed`: the value Enter submitted (the form state seen here may predate it).
  const labelText = (typed ?? c.form['reg:label'] ?? suggestion).trim()
  const group = c.form['reg:group'] ?? s.defaultGroup
  setForm($, 'reg:problem', '')
  if (!labelText) return void setForm($, 'reg:problem', 'Type a name for this account.')
  if (s.accounts.some(a => a.label.toLowerCase() === labelText.toLowerCase())) {
    return void setForm($, 'reg:problem', `"${labelText}" is already a name here; pick another.`)
  }
  if (!s.consents.credentialStorage && c.form['reg:consent'] !== '1') {
    return void setForm($, 'reg:problem', 'Tick the vault box first: registering keeps a copy of this login.')
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
  const env = myEnv(s)
  if (!si || si.status.state === 'cancelled' || si.status.state === 'saved') {
    const reauth = c.view.arg
    const group = c.form['add:group'] ?? s.defaultGroup
    const newGroup = c.form['add:showgroup'] === '1'
    // Signing in already keeps a login in a temporary folder, so the one vault consent
    // (asked once, ever) comes before it.
    const needConsent = !s.consents.credentialStorage
    const unregistered = !reauth && !!env && !env.live.known && !env.live.signedOut
    const problem = c.form['add:problem']
    return (
      <Box flexDirection="column">
        {Section(c, reauth ? `Sign in again for "${label(s, reauth)}"` : 'Add an account')}
        {si?.status.state === 'cancelled' ? Line(c, 'The last sign-in was cancelled; nothing changed.', { dim: true }) : null}
        {unregistered
          ? Row(c, Line(c, `You are signed in to an account Quayflow does not know yet (#${env?.live.account ?? '?'}).`, { tone: 'warn' }), Btn(c, 'add-register-current', 'Register it', () => go($, { tab: 'onboarding', sub: 'register' })))
          : null}
        {!reauth && Row(
          c,
          Pick(c, 'add:group', 'Group', groupOptions(c), group, v => setForm($, 'add:group', v)),
          newGroup ? null : Btn(c, 'add-newgroup', '+ New group', () => setForm($, 'add:showgroup', '1')),
        )}
        {!reauth && newGroup ? Field($, c, 'add:newgroup', 'New group', { placeholder: 'name', submitLabel: 'create', autoFocus: true, onSubmit: v => void createGroupInline($, v, 'add:group').then(() => setForm($, 'add:showgroup', '')) }) : null}
        {needConsent ? Toggle(c, 'add:vault', 'Keep the logins I add in my local vault (asked once)', c.form['add:vault'] === '1', v => {
          setForm($, 'add:vault', v ? '1' : '')
          setForm($, 'add:problem', '')
        }) : null}
        {problem ? Line(c, `\u26a0 ${problem}`, { tone: 'warn' }) : null}
        {Hint(c, "Claude's own sign-in opens in a temporary folder. Nothing is saved until you review the account it returns; your live login stays as it is.")}
        {Row(
          c,
          Btn(c, 'add-continue', reauth ? 'Sign in again' : 'Sign in with Claude', () => void startSignin($, c), true),
          Btn(c, 'add-cancel', 'Cancel', () => go($, { tab: s.onboarded ? 'accounts' : 'onboarding', sub: s.onboarded ? undefined : 'readiness' })),
        )}
      </Box>
    )
  }
  const st = si.status
  if (st.state === 'waiting') {
    return (
      <Box flexDirection="column">
        {Section(c, 'Finish the sign-in in the window that opened')}
        {Line(c, 'This screen moves on by itself when you are done there.', { dim: true })}
        {Line(c, `No window? Run this in a terminal: ${st.command}`)}
        {Row(c, Btn(c, 'signin-cancel', 'Cancel', () => void cancelSignin($, c, st.id)))}
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
  const dupAccount = dup ? s.accounts.find(a => a.label === dup) : undefined
  const unexpected = !!id?.unexpected
  const asNew = unexpected && c.form['add:asnew'] === '1'
  const group = c.form['add:group'] ?? si.group
  if (unexpected && !asNew) {
    return (
      <Box flexDirection="column">
        {Alert(c, 'You signed in to a different account', `It does not replace "${label(s, si.reauth)}". Save it as a new account, or cancel.`, 'error')}
        {KV(c, 'Signed in as', `${id?.display ?? `account #${id?.account ?? '?'}`} · ${kindText(id?.kind, id?.subscription)}`)}
        {KV(c, 'Expected', `"${label(s, si.reauth)}"`)}
        {Row(
          c,
          Btn(c, 'signin-as-new', 'Save as new', () => setForm($, 'add:asnew', '1'), true),
          Btn(c, 'signin-review-existing', 'Existing', () => go($, { tab: 'accounts', sub: 'detail', arg: si.reauth })),
          Btn(c, 'signin-discard', 'Cancel', () => void cancelSignin($, c, st.id)),
        )}
      </Box>
    )
  }
  // A new account (or a different identity saved as new) is named here, with a suggestion.
  const naming = !dup && (!si.reauth || asNew)
  const suggestion = naming ? suggestName(id, s.accounts) : ''
  const name = (c.form['add:label'] ?? suggestion).trim()
  const taken = naming && name ? s.accounts.find(a => a.label.toLowerCase() === name.toLowerCase()) : undefined
  const problem = c.form['add:problem']
  return (
    <Box flexDirection="column">
      {Section(
        c,
        dup ? 'This account is already registered' : si.reauth && !asNew ? `Signed in again for "${label(s, si.reauth)}"` : 'Signed in',
        KV(c, 'Signed in as', `${id?.display ?? `account #${id?.account ?? '?'}`} \u00b7 ${kindText(id?.kind, id?.subscription) || '?'}`),
        dup ? KV(c, 'Registered as', `"${dup}" in ${dupAccount?.groups.join(', ') || 'no group'}`) : null,
      )}
      {dup ? Line(c, 'Saving refreshes its saved login; it adds no extra allowance.', { dim: true }) : null}
      {naming && Field($, c, 'add:label', 'Name', { placeholder: 'name', submitLabel: 'save', autoFocus: true, initial: suggestion, onSubmit: v => void confirmSignin($, c, asNew, v) })}
      {taken ? Line(c, `\u26a0 "${taken.label}" is already a name here; pick another.`, { tone: 'warn' }) : null}
      {naming && Pick(c, 'add:group', 'Group', groupOptions(c), group, v => setForm($, 'add:group', v))}
      {problem ? Line(c, `\u26a0 ${problem}`, { tone: 'warn' }) : null}
      {Hint(c, 'The e-mail address is shown for your review only and never stored. Saving retires the temporary sign-in copy without signing out; your live login stays as it is.')}
      {Row(
        c,
        Btn(c, 'signin-save', dup || (si.reauth && !asNew) ? 'Update' : 'Save', () => void confirmSignin($, c, asNew), true),
        Btn(c, 'signin-discard', 'Discard', () => void cancelSignin($, c, st.id)),
      )}
    </Box>
  )
}

/** A name for a newly signed-in account: the e-mail's local part, made unique. */
export function suggestName(id: SigninIdentity | null | undefined, accounts: { label: string }[]): string {
  const local = (id?.display ?? '').split('@')[0] ?? ''
  const base = local.toLowerCase().replace(/[^a-z0-9._-]+/g, '-').replace(/^-+|-+$/g, '') || (id?.kind === 'personal' ? 'personal' : 'account')
  const used = new Set(accounts.map(a => a.label.toLowerCase()))
  if (!used.has(base)) return base
  for (let n = 2; ; n++) if (!used.has(`${base}-${n}`)) return `${base}-${n}`
}

async function startSignin($: EngineInterface, c: Ctx): Promise<void> {
  const s = c.s
  if (!s) return
  const reauth = c.view.arg
  const group = c.form['add:group'] ?? s.defaultGroup
  await setForm($, 'add:problem', '')
  // A "Save as new" choice belongs to the sign-in it was made for, never to the next one.
  await setForm($, 'add:asnew', '')
  if (!s.consents.credentialStorage) {
    if (c.form['add:vault'] !== '1') {
      return void setForm($, 'add:problem', 'Tick the vault box first: the sign-in keeps a login until you save or discard it.')
    }
    const given = await run($, 'recording your vault consent', () => actOnboarding($, { credentialStorage: true }, false))
    if (!given) return
  }
  const st = await run($, 'opening the sign-in window', () => actSigninStart($, reauth))
  if (!st) return
  await clearForm($, 'add:label')
  await setSignin($, { status: st, label: reauth ? label(s, reauth) : '', group, reauth })
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

async function confirmSignin($: EngineInterface, c: Ctx, asNew = false, typed?: string): Promise<void> {
  const si = c.signin
  const s = c.s
  if (!si || !s) return
  const id = si.status.identity
  const dup = id?.duplicateOf ?? undefined
  const naming = !dup && (!si.reauth || asNew)
  // `typed`: the value Enter submitted (the form state seen here may predate it).
  const labelText = dup ?? (naming ? (typed ?? c.form['add:label'] ?? suggestName(id, s.accounts)).trim() : si.label)
  const group = naming ? (c.form['add:group'] ?? si.group) : si.group
  await setForm($, 'add:problem', '')
  if (naming && !labelText) return void setForm($, 'add:problem', 'Type a name for this account.')
  if (naming && s.accounts.some(a => a.label.toLowerCase() === labelText.toLowerCase())) {
    return void setForm($, 'add:problem', `"${labelText}" is already a name here; pick another.`)
  }
  const r = await run(
    $,
    'saving to the vault',
    async () => {
      if (!s.consents.credentialStorage) await actOnboarding($, { credentialStorage: true }, false)
      return actSigninConfirm($, si.status.id, labelText, group, false, asNew)
    },
    undefined,
    err => reviewDisclosure($, err, { then: 'confirm', group, label: labelText, signinId: si.status.id }),
  )
  if (r) await afterConfirm($, c, labelText, group, r.message ?? undefined)
}

export async function afterConfirm($: EngineInterface, c: Ctx, labelText: string, group: string, message?: string): Promise<void> {
  signinPoll++
  await setSignin($, null)
  await clearForm($, 'add:label', 'add:asnew', 'add:problem', 'add:vault')
  await setView($, c.s?.onboarded ? { tab: 'accounts' } : { tab: 'onboarding', sub: 'done' })
  await flash($, 'info', message ?? `Saved "${labelText}" to the vault in group ${group}. The temporary sign-in copy is retired; nothing was switched.`)
}

function Ready($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const s = c.s
  const env = s ? myEnv(s) : undefined
  const mode = env?.mode ?? 'suggest'
  return (
    <Box flexDirection="column">
      {Alert(c, s?.accounts.length ? `${s.accounts.length} account${s.accounts.length === 1 ? '' : 's'} registered` : 'Setup done', 'Your live account has not changed.', 'ok')}
      {Section(
        c,
        `Ready in ${mode[0]!.toUpperCase()}${mode.slice(1)} mode`,
        KV(c, 'Current login', env?.live.known ? label(s ?? undefined, env.live.account) : env?.live.signedOut ? 'signed out' : 'not registered'),
        KV(c, 'Group mode', `${mode} (group ${s ? myGroup(s) : '?'})`),
        KV(c, 'Headless behavior', mode === 'automatic' ? 'Automatic switching and bounded recovery' : 'Recommendations only; no switch'),
        Line(c, 'Unattended headless switching needs Automatic mode on the effective group; you can enable it after reviewing its behavior.'),
        (s?.accounts.length ?? 0) < 2 ? Hint(c, 'One registered account is valid; there is simply no alternative to switch to until you add another.') : null,
      )}
      {Row(
        c,
        Btn(c, 'done-overview', 'Overview', () => go($, { tab: 'overview' }), true),
        Btn(c, 'done-automatic', 'Group mode', () => go($, { tab: 'groups', sub: 'group', arg: s ? myGroup(s) : undefined })),
        Btn(c, 'done-add', 'Add account', () => go($, { tab: 'onboarding', sub: 'add' })),
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
  // `accept`: a recommendation (no pin); `review`: a manual choice (pins).
  const accepting = c.view.sub === 'accept'
  const sg = env?.suggestion
  const target = account(s, accepting ? sg?.to : c.view.arg)
  if (!env || !target) return accepting ? Line(c, 'There is no recommendation to accept right now; it may have changed. Open the overview for the current one.') : Line(c, 'That account is not known.')
  const idle = c.form['sw:idle'] === '1'
  const blockers = env.sessions.filter(x => x.group && !s.groups.find(g => g.name === x.group)?.accounts.includes(target.id))
  const eligible = !target.eligibility || target.eligibility.eligible
  const t = s.tuning
  const busy = env.sessions.filter(x => x.status === 'busy')
  return (
    <Box flexDirection="column">
      {Section(
        c,
        `Switch to ${target.label}`,
        KV(c, 'Moves', `${env.sessions.length} session${env.sessions.length === 1 ? '' : 's'} in ${env.id}`),
        KV(c, 'Usage', usageText(target)),
        accepting ? null : KV(c, 'Pin duration', `${minutes(t.pinMinutes)}`),
        Hint(c, `Scope: environment ${env.id} (shared login): every listed session moves at its next request. Reason: ${accepting ? `accepted suggestion: ${sg?.reason ?? ''}` : 'your choice (manual; pins)'}.`),
        Steps(c, [
          blockers.length
            ? { mark: 'failed', title: 'Groups', desc: `"${blockers.map(x => sessionName(x)).join('", "')}" does not permit ${target.label}` }
            : eligible
              ? { mark: 'done', title: 'Groups and plan' }
              : { mark: 'failed', title: 'Groups and plan', desc: target.eligibility?.reason ?? '?' },
          env.inPlace.allowed ? { mark: 'done', title: 'Claude version' } : { mark: 'failed', title: 'Claude version', desc: env.inPlace.reason },
        ]),
        Hint(c, `Versions: ${env.inPlace.reason}. Applied under Claude's credential locks; in-flight requests keep their original account.`),
      )}
      {Toggle(c, 'sw:idle', `Wait for idle (up to ${t.idleWaitS} s)`, idle, v => setForm($, 'sw:idle', v ? '1' : ''))}
      {idle && env.sessions.length > 0
        ? Table(c, 'idle', ['Participant', 'Current activity'], env.sessions.map(x => [`${x.sessionId === Q.sessionId ? 'This session' : sessionName(x)}`, x.status === 'busy' ? 'working (not held; finishes on its account)' : (x.status ?? 'unknown')]))
        : null}
      {idle && busy.length ? Hint(c, 'Work continues: requests are not held for safety. Each client adopts the new login at its own request boundary.') : null}
      {accepting
        ? Hint(c, 'Accepting a recommendation does not pin. Existing group membership is the data approval; the engine revalidates the recommendation before it switches.')
        : Hint(c, `A manual switch pins ${target.label}: no suggestion, proactive switch or return home away from it until you unpin, its binding window resets, or ${minutes(t.pinMinutes)} pass; at ${t.pinReleasePct}% of a relevant window normal behavior resumes. A confirmed hard-limit recovery ignores the pin and the cooldown (never permissions or safety).`)}
      {Hint(c, 'The prompt cache is not shared between accounts: the first request after a switch processes the full context again.')}
      {Row(
        c,
        accepting
          ? Btn(c, 'sw-accept', 'Switch', () => void doAccept($, c), true)
          : Btn(c, 'sw-go', 'Switch and pin', () => void doSwitch($, c, target.id, idle), true),
        Btn(c, 'sw-details', 'Why?', () => go($, { tab: 'overview', sub: 'why' })),
        Btn(c, 'sw-cancel', 'Cancel', () => go($, accepting ? { tab: 'overview' } : { tab: 'accounts', sub: 'detail', arg: target.id })),
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

async function doAccept($: EngineInterface, c: Ctx): Promise<void> {
  const to = c.s ? myEnv(c.s)?.suggestion?.to : undefined
  const r = await run($, 'accepting the recommendation', () => actAccept($))
  if (!r) return
  if (!r.ok && r.blockedBy) {
    setForm($, 'conflict', JSON.stringify(r.blockedBy))
    go($, { tab: 'switch', sub: 'conflict', arg: to ?? undefined })
    return
  }
  if (r.ok) go($, { tab: 'switch', sub: 'progress' })
  else await flash($, 'warn', r.message ?? 'Not switched.')
}

export function SwitchProgress($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const s = c.s
  if (!s) return Line(c, 'Reading the engine state...')
  const env = myEnv(s) as EnvView
  const t = env?.transition ?? env?.lastSwitch
  if (!t) return (
    <Box flexDirection="column">
      {Line(c, 'No switch recorded for this environment yet.')}
      {Row(c, Btn(c, 'sp-back', 'Back', () => go($, { tab: 'overview' })))}
    </Box>
  )
  const beforeWrite = t.state === 'requested' || t.state === 'preparing' || t.state === 'idle_wait'
  const failure = failureWords(t)
  const ended = t.state === 'failed' || t.state === 'cancelled' || t.state === 'uncertain'
  const mine = env.adoption.filter(a => a.transition === t.id)
  const mismatch = mine.some(a => a.state === 'mismatch')
  const allAdopted = mine.length > 0 && mine.every(a => a.state === 'adopted')
  const head =
    t.state === 'completed' && allAdopted
      ? `Observed clients adopted "${label(s, t.to)}"; your conversation and process are unchanged.`
      : failure
        ? `${failure}.`
        : `${transitionWords(t.state)}.`
  const wrong = mine.find(a => a.state === 'mismatch')
  return (
    <Box flexDirection="column">
      {Section(
        c,
        `${switchHeading(s, t)}${t.env === 'default' ? '' : ` \u00b7 ${t.env}`}`,
        Line(c, head, { tone: t.state === 'failed' || t.state === 'uncertain' || mismatch ? 'warn' : t.state === 'completed' && allAdopted ? 'ok' : undefined }),
        Hint(c, `${t.how.replace('_', ' ')}: ${t.reason} \u00b7 requested ${when(t.requestedAt)}${t.writtenAt ? ` \u00b7 stored ${when(t.writtenAt)}` : ''}`),
        t.detail ? Line(c, t.detail, { dim: true }) : null,
      )}
      {t.state !== 'cancelled' && Steps(c, phases(t, ended))}
      {t.state === 'idle_wait' && env.sessions.length > 0
        ? Section(
            c,
            'Waiting for idle',
            Table(c, 'idle-wait', ['Participant', 'Current activity'], env.sessions.map(x => [x.sessionId === Q.sessionId ? 'This session' : sessionName(x), x.status ?? 'unknown'])),
            Hint(c, `Waits up to ${s.tuning.idleWaitS} s; requests are not held for safety.`),
          )
        : null}
      {t.state === 'applying' ? Hint(c, `A short critical section: each lock wait is bounded (${s.tuning.lockWaitS} s); a young lock is never taken over. No Undo after the write.`) : null}
      {t.how === 'return_home' && t.state !== 'failed'
        ? Hint(c, 'The home account\'s binding window reset and fresh usage is below the threshold minus the margin; pin, cooldown, plan and group checks passed.')
        : null}
      {t.how === 'model_fallback'
        ? Alert(c, 'Model not available on that plan', `The account's plan cannot serve the model; switched back to "${label(s, t.to)}".`, 'warn')
        : null}
      {mine.length > 0 &&
        Section(
          c,
          'Clients',
          Table(
            c,
            'adoption',
            ['Client', 'Adoption', 'Stored', 'Last request'],
            mine.map(a => {
              const x = env.sessions.find(z => z.pid === a.pid)
              return [
                `${a.sessionId === Q.sessionId ? 'This session' : x ? sessionName(x) : `pid ${a.pid}`}${a.version ? ` · Claude ${a.version}` : ''}`,
                adoptionWords(a.state),
                label(s, a.destination),
                a.lastRequestAccount ? label(s, a.lastRequestAccount) : 'not observed yet',
              ]
            }),
          ),
          Hint(c, 'Idle clients adopt at their own next request; nothing is sent just to turn this list green. One client never certifies another, and other consumers may not be listed.'),
        )}
      {mismatch &&
        Section(
          c,
          'Mismatch',
          Alert(c, 'In-place switching disabled for this version', `A next request did not adopt "${label(s, t.to)}"${wrong?.lastRequestAccount ? `: it used "${label(s, wrong.lastRequestAccount)}"` : ''}.`, 'error'),
          Line(c, 'Restart that Claude session, or use a full-history restart.'),
          Hint(c, 'Account state: needs reconciliation. Version evidence: live check failed. Quayflow stopped further in-place changes for this Claude version. A headless job is never restarted silently.'),
          Hint(c, 'Whether requests in flight were paused is not known: nothing here claims all work is safely paused.'),
        )}
      {Row(
        c,
        beforeWrite && Btn(c, 'sp-cancel', 'Cancel', () => void cancelTransition($, c, t.id)),
        mismatch && Btn(c, 'sp-fallback', 'Full history', () => go($, { tab: 'session' }), true),
        mismatch && Btn(c, 'sp-version', 'Version', () => go($, { tab: 'settings', sub: 'version' })),
        t.how === 'model_fallback' && Btn(c, 'sp-plan', 'Why?', () => go($, { tab: 'overview', sub: 'why' })),
        t.how === 'model_fallback' && Btn(c, 'sp-budget', 'Limits', () => go($, { tab: 'overview', sub: 'limits' })),
        Btn(c, 'sp-overview', 'Back', () => go($, { tab: 'overview' }), !mismatch && !beforeWrite),
        Btn(c, 'sp-activity', 'Activity', () => go($, { tab: 'activity' })),
      )}
      {beforeWrite && Hint(c, 'Closing this view does not cancel; Cancel asks the engine and waits for its answer.')}
      {t.state === 'cancelled' && Hint(c, 'A cancelled switch is not a cancelled task: your work continues. The current account may still have its own usage restriction.')}
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
      r.started ? 'Switch already started; checking its state.' : r.ok ? 'Cancelled before a write; the login is unchanged.' : (r.message ?? 'The engine did not cancel.'),
    )
  } catch (err) {
    await flash($, 'error', `Cancel not confirmed: ${(err as Error).message}. Checking state.`)
  }
}

export function Conflict($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const s = c.s
  let blocked: { pid?: number; name?: string; cwd?: string; group?: string; sessionId?: string } = {}
  try {
    blocked = JSON.parse(c.form['conflict'] ?? '{}')
  } catch {}
  const name = blocked.name ?? `${(blocked.cwd ?? '?').split(/[\\/]/).filter(Boolean).pop()} (pid ${blocked.pid ?? '?'})`
  const to = c.view.arg
  const env = s ? myEnv(s) : undefined
  const allowed = (group: string | null | undefined): string => (!s || !to ? '?' : s.groups.find(g => g.name === group)?.accounts.includes(to) ? 'Yes' : 'No')
  return (
    <Box flexDirection="column">
      {Alert(c, `Cannot switch the shared login to "${label(s ?? undefined, to)}"`, `Session "${name}" (project ${blocked.cwd ?? '?'}, group ${blocked.group ?? '?'}) does not permit "${label(s ?? undefined, to)}". No login change was applied.`, 'warn')}
      {env && env.sessions.length > 0
        ? Table(c, 'conflict', ['Participant', 'Effective group', `${label(s ?? undefined, to)} allowed?`], env.sessions.map(x => [`${x.sessionId === Q.sessionId ? 'This session / ' : ''}${sessionName(x)}`, x.group ?? '?', allowed(x.group)]))
        : null}
      {Line(c, 'A shared switch would move that session too. Its group is a data approval; it is not widened here.')}
      {Line(c, 'Move the restricted session into its own lane with full history, or keep the shared login unchanged. Automatic mode never moves a session by itself.')}
      {Row(
        c,
        Btn(c, 'cf-lanes', 'Move to lane', () => go($, { tab: 'switch', sub: 'lanemove', arg: to }), true),
        Btn(c, 'cf-keep', 'Keep shared', () => go($, { tab: 'overview' })),
        Btn(c, 'cf-policy', 'Groups', () => go($, { tab: 'groups' })),
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
        {Alert(c, 'Live login is not registered', `A native /login, a sign-out or another tool changed environment ${env.id}. Automatic switching and recovery are paused here.`, 'warn')}
        {KV(c, 'Environment', `${env.id} (shared login)`)}
        {KV(c, 'Live login', `account #${env.live.account ?? '?'}${env.live.kind ? ` (${kindText(env.live.kind, env.live.subscription)})` : ''}`)}
        {KV(c, 'Current task', 'Continues as usual under Claude')}
        {Line(c, 'Register it (/quay register) to resume, or ignore the notice: automation then stays paused while this login is live.')}
        {Hint(c, 'A manual switch stays possible only after a protected backup of this login is confirmed; Quayflow refuses the switch if it cannot store one. Ignore never authorizes overwriting it in the background.')}
        {Row(
          c,
          !env.live.signedOut && Btn(c, 'tu-register', 'Register', () => go($, { tab: 'onboarding', sub: 'register' }), true),
          Btn(c, 'tu-ignore', 'Ignore', () => void run($, 'recording', () => actIgnoreUnknown($), r => (r.ok ? 'Noted. Automation stays paused while this login is live.' : (r.message ?? 'Refused.')))),
          Btn(c, 'tu-manual', 'Manual switch', () => go($, { tab: 'accounts' })),
          Btn(c, 'tu-back', 'Back', () => go($, { tab: 'overview' })),
        )}
      </Box>
    )
  }
  if (sub === 'switcher') {
    const n = env.notices.find(x => x.kind === 'other_switcher')
    return (
      <Box flexDirection="column">
        {Alert(c, 'Automation paused for this environment', n?.text ?? 'The login changed repeatedly outside Quayflow. Another account manager may be switching the same store.', 'error')}
        {Line(c, 'Stop the competing automation and review the current identity. Quayflow will not keep switching it back.')}
        {KV(c, 'Current task', 'Continues as usual; this warning does not stop it')}
        {KV(c, 'Resumes', 'After you review the live login; never by retrying blindly')}
        {Row(
          c,
          Btn(c, 'sw-live', 'Live login', () => go($, env.live.known ? { tab: 'accounts', sub: 'detail', arg: env.live.account ?? undefined } : { tab: 'trust', sub: 'unknown' }), true),
          Btn(c, 'sw-activity', 'Activity', () => go($, { tab: 'activity' })),
          Btn(c, 'sw-back', 'Close', () => go($, { tab: 'overview' })),
        )}
      </Box>
    )
  }
  if (sub === 'dorg') return DOrg($, c)
  // managed enforcement (S54)
  const m = env.managed
  const excluded = s.accounts.filter(a => a.eligibility && !a.eligibility.eligible && /organi[sz]ation|managed|policy|enforc/i.test(a.eligibility.reason))
  return (
    <Box flexDirection="column">
      {Section(
        c,
        'Managed login',
        Line(c, 'Organization-controlled: the enforced identity and method are respected.', { tone: 'warn' }),
        (m.forceOrg?.length ?? 0) > 0 ? KV(c, 'Enforcing organization', (m.forceOrg ?? []).map(o => orgName(s, o)).join(', ')) : null,
        m.forceMethod ? KV(c, 'Login method', m.forceMethod) : null,
        KV(c, 'Sources', (m.sources ?? []).join(', ') || 'none'),
        ...excluded.map(a => Line(c, `${a.label} is ineligible here: ${a.eligibility?.reason}`)),
        Hint(c, 'Matching accounts can still be used. Quayflow checks every managed-settings source and rechecks after Claude fetches policy; the administrator controls the rule. Group membership does not override it.'),
      )}
      {Row(
        c,
        Btn(c, 'mg-accounts', 'Accounts', () => go($, { tab: 'overview', sub: 'why' }), true),
        Btn(c, 'mg-dorg', 'Exception', () => go($, { tab: 'trust', sub: 'dorg' })),
        Btn(c, 'mg-policy', 'Policy changes', () => go($, { tab: 'trust', sub: 'policy' })),
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
    const org = s.dOrg.confirmedOrg ? orgName(s, s.dOrg.confirmedOrg) : 'unknown organization'
    const notice = env?.notices.find(n => n.kind === 'd_org_enabled')
    return (
      <Box flexDirection="column">
        {Alert(c, 'Managed-login exception setting is ON', `Environment: ${env?.id ?? '?'}. Enforcing organization: ${org} (confirmed ${when(s.dOrg.confirmedAt)}). ${notice?.text ?? 'Recorded only: this build still enforces the policy.'}`, 'error')}
        {KV(c, 'Managed-login enforcement', 'Still enforced in this build')}
        {KV(c, 'Group approval', 'Still required')}
        {KV(c, 'Canary and credential safety', 'Still required')}
        {KV(c, 'Protected selector restriction', 'Never bypassed')}
        {Line(c, 'The setting and its acknowledgment are journaled. If the policy source or the organization changes, a new acknowledgment is required.')}
        {Hint(c, 'Disabling checks is not provider or organization authorization.')}
        {Row(
          c,
          Btn(c, 'dorg-disable', 'Disable', () => void run($, 'turning the exception off', () => actDorg($, false, s.dOrg.confirmedOrg ?? '', ''), r => (r.ok ? 'Managed-login exception setting is off.' : (r.message ?? 'Refused.'))), true),
          Btn(c, 'dorg-activity', 'Affected switches', () => go($, { tab: 'activity' })),
          Btn(c, 'dorg-back', 'Overview', () => go($, { tab: 'overview' })),
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
  const understood = c.form['dorg:understand'] === '1'
  return (
    <Box flexDirection="column">
      {Section(
        c,
        'Managed-login exception',
        Line(c, 'High-risk exception; off by default.', { tone: 'error' }),
        Line(c, `${orgText[0]!.toUpperCase()}${orgText.slice(1)} (#${org}) enforces which accounts may sign in to Claude in ${env?.id ?? 'this environment'} (${(env?.managed.sources ?? []).join(', ') || 'managed settings'}).`),
      )}
      {Alert(c, 'This is about working around organization access control', 'An exception would let a switch select an account the organization refuses at sign-in, so company work could reach a personal or different-organization account. That can violate your employment or organization policy. Responsibility is yours.', 'error')}
      {Alert(c, 'What this build does', 'It records the setting, journals it and shows it everywhere. It still enforces the managed login policy: no switch to an account the policy would refuse is made.', 'warn')}
      {Hint(c, 'Group approval, version checks and the protected-selector restriction always apply. This exception is not a claim of permission.')}
      {orgs.length > 1 && Pick(c, 'dorg:org', 'Enforcing organization', orgs.map(o => ({ value: o, label: orgName(s, o) })), org, v => setForm($, 'dorg:org', v))}
      {Toggle(c, 'dorg:understand', 'I understand this bypass risk and the named organization rule', understood, v => setForm($, 'dorg:understand', v ? '1' : ''))}
      {Line(c, `To record it, type exactly: ${phrase}`)}
      {Field($, c, 'dorg:ack', 'Confirmation phrase', { placeholder: phrase, submitLabel: 'check', onSubmit: () => {} })}
      {Row(
        c,
        Btn(c, 'dorg-keep', 'Keep enforcement', () => go($, { tab: 'settings' }), true),
        Btn(c, 'dorg-enable', 'Record exception', () => {
          if (!understood) return void flash($, 'warn', 'Tick the risk box first; the exception stays off.')
          if (typed !== phrase) return void flash($, 'warn', 'The confirmation phrase does not match; the exception stays off.')
          // The engine's own reply is shown: it says what this build does with the setting.
          void run($, 'recording the exception', () => actDorg($, true, org, typed), r => r.message ?? (r.ok ? 'Setting recorded.' : 'Refused.')).then(() => clearForm($, 'dorg:ack', 'dorg:understand'))
        }),
      )}
    </Box>
  )
}