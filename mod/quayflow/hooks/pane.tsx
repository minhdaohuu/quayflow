// The /quay pane (T43-T48): tabs Overview, Accounts, Groups, Activity and
// Settings drawn from host elements, plus the flows in flows.tsx. Tabs are a
// row of buttons with hotkeys 1-5; Tab and the arrows walk the controls.
import type { Elements, EngineInterface, RenderSurface } from 'claude-code'

import type { Mode, QuayView } from '../types'
import { Q, flash, readPaneState, setDisclosure, setReport } from './context'
import { actAccept, actRegister, actSigninConfirm, actAssign, actCompatCheck, actEngine, actGroupCreate, actGroupDelete, actHandoff, actLaneCheck, actLaneCreate, actLaneDelete, actMembers, actPreferred, actRank, actRelabel, actRemove, actReport, actSetDefault, actSetMode, actTuning, actUnassign, actUnpin, handoffOutcome } from './actions'
import {
  account,
  age,
  healthText,
  isTerminal,
  label,
  myEnv,
  myGroup,
  orgName,
  persistentWarnings,
  sessionName,
  transitionWords,
  usageText,
  when,
  windowLine,
} from './format'
import { hasCode } from './engine'
import { Conflict, OffView, Onboarding, SwitchProgress, SwitchReview, Trust, afterConfirm, afterRegister } from './flows'
import { Btn, Field, Line, Pick, Row, Section, clearForm, go, run, setForm } from './ui'
import type { Ctx } from './ui'

const TABS = [
  { id: 'overview', label: 'Overview' },
  { id: 'accounts', label: 'Accounts' },
  { id: 'groups', label: 'Groups' },
  { id: 'activity', label: 'Activity' },
  { id: 'settings', label: 'Settings' },
]

export async function renderPane(
  $: EngineInterface,
  e: { surface: RenderSurface; props: { bodyColumns?: number } },
  els: Elements['terminal'],
  claudeVersion: string,
): Promise<JSX.Element> {
  const c: Ctx = {
    els,
    surface: e.surface,
    canType: e.surface !== 'mobile',
    cols: e.props.bodyColumns ?? 80,
    now: Date.now(),
    ...(await readPaneState($)),
    claudeVersion,
  }
  const { Box, Text } = els
  return (
    <Box flexDirection="column">
      {Header($, c)}
      {c.flash && (
        <Text color={c.flash.tone === 'error' ? 'error' : c.flash.tone === 'warn' ? 'warning' : undefined}>
          {c.flash.tone === 'error' ? 'Error: ' : c.flash.tone === 'warn' ? 'Note: ' : ''}
          {c.flash.text}
        </Text>
      )}
      <Box flexDirection="column" marginTop={1}>
        {Body($, c)}
      </Box>
    </Box>
  )
}

function Header($: EngineInterface, c: Ctx): JSX.Element {
  const { Box, Button, Text } = c.els
  const s = c.s
  const env = s ? myEnv(s) : undefined
  const tab = c.view.tab
  const inTabs = TABS.some(t => t.id === tab)
  let context = 'Quayflow'
  if (c.link.on && s && env) {
    const v = s.session?.verifiedAccount
    context = `This session: ${v ? `last request on "${label(s, v)}"` : 'no verified request yet'} · environment ${env.id} · group ${myGroup(s)} · ${env.mode}${env.automationPaused ? ' (paused)' : ''}`
  }
  return (
    <Box flexDirection="column">
      <Box flexDirection="row" flexWrap="wrap" columnGap={1}>
        {TABS.map((t, i) => (
          <Button
            key={`tab:${t.id}`}
            label={t.id === tab || (!inTabs && i === 0 && false) ? `> ${t.label}` : t.label}
            hotkey={String(i + 1)}
            variant={t.id === tab ? 'primary' : undefined}
            onPress={() => go($, { tab: t.id })}
          />
        ))}
        <Button key="close" label="Close" role="dismiss" onPress={() => void $.ui.close({ id: 'quay' }).catch(() => {})} />
      </Box>
      <Text dimColor>{context}</Text>
    </Box>
  )
}

function Body($: EngineInterface, c: Ctx): JSX.Element {
  if (!c.link.on) return OffView($, c)
  const s = c.s
  if (!s) return Line(c, 'Reading the engine state...')
  if (!s.onboarded || c.view.tab === 'onboarding') return Onboarding($, c)
  switch (c.view.tab) {
    case 'accounts':
      return Accounts($, c)
    case 'groups':
      return Groups($, c)
    case 'activity':
      return Activity($, c)
    case 'settings':
      return Settings($, c)
    case 'switch':
      return c.view.sub === 'review' ? SwitchReview($, c) : c.view.sub === 'conflict' ? Conflict($, c) : SwitchProgress($, c)
    case 'trust':
      return Trust($, c)
    default:
      return Overview($, c)
  }
}

// ------------------------------------------------------------ Overview

function Overview($: EngineInterface, c: Ctx): JSX.Element {
  const { Box, Button, Text } = c.els
  const s = c.s!
  const env = myEnv(s)
  if (!env) return Line(c, 'No credential environment is known yet.')
  const warnings = persistentWarnings(s, env)
  const unknown = !env.live.known && !env.live.signedOut
  const switcher = env.notices.some(n => n.kind === 'other_switcher')
  const managed = (env.managed.forceOrg?.length ?? 0) > 0 || !!env.managed.forceMethod
  const t = env.transition
  const sg = env.suggestion
  const assigned = s.session?.groupSource === 'assigned'
  return (
    <Box flexDirection="column">
      {Line(c, `Environment ${env.id} stores: ${env.live.signedOut ? 'signed out' : env.live.known ? `"${label(s, env.live.account)}"` : `unknown login #${env.live.account ?? '?'}`}`)}
      {Line(c, `Project group: ${myGroup(s)} (${assigned ? 'explicit project assignment' : 'default group; no assignment'}) · mode ${env.mode}`, { dim: true })}
      {warnings.length > 0 &&
        Section(
          c,
          'Warnings',
          ...warnings.map(w => Line(c, `! ${w}`, { tone: 'warn' })),
          Row(
            c,
            unknown && Btn(c, 'ov-unknown', 'Review unknown login', () => go($, { tab: 'trust', sub: 'unknown' })),
            switcher && Btn(c, 'ov-switcher', 'Another switcher', () => go($, { tab: 'trust', sub: 'switcher' })),
            managed && Btn(c, 'ov-managed', 'Managed login', () => go($, { tab: 'trust', sub: 'managed' })),
            s.dOrg.enabled && Btn(c, 'ov-dorg', 'Managed-login exception', () => go($, { tab: 'trust', sub: 'dorg' })),
            env.automationPaused && Btn(c, 'ov-resume', 'Resume automation', () => void run($, 'resuming automation', () => actEngine($, 'resume'), r => (r.ok ? 'Automation resumed.' : (r.message ?? 'Refused.')))),
          ),
        )}
      {t && !isTerminal(t) &&
        Section(
          c,
          'Switch in progress',
          Line(c, `To "${label(s, t.to)}": ${transitionWords(t.state)}`),
          Row(c, Btn(c, 'ov-progress', 'View progress', () => go($, { tab: 'switch', sub: 'progress' }))),
        )}
      {Section(
        c,
        'Recommendation',
        sg ? Line(c, `Switch to "${label(s, sg.to)}": ${sg.reason}`) : Line(c, env.decision || 'Stay on the current account.'),
        env.pin ? Line(c, `Pinned "${label(s, env.pin.account)}" until ${when(env.pin.until)}; no suggestion or automatic switch away from it until then.`, { dim: true }) : null,
        Row(
          c,
          sg && Btn(c, 'ov-accept', 'Accept recommendation', () => void acceptSuggestion($, c), true),
          sg && Btn(c, 'ov-why', 'Why this account?', () => go($, { tab: 'accounts', sub: 'detail', arg: sg.to })),
          env.pin && Btn(c, 'ov-unpin', 'Unpin', () => void run($, 'unpinning', () => actUnpin($), r => (r.ok ? 'Pin cleared.' : (r.message ?? 'Nothing pinned.')))),
        ),
      )}
      {Section(
        c,
        'Accounts (binding window · observation age)',
        ...s.accounts.map(a => (
          <Button key={`ov-acct:${a.id}`} plain onPress={() => go($, { tab: 'accounts', sub: 'detail', arg: a.id })}>
            {a.liveIn.includes(env.id) ? '> ' : '  '}
            {a.label} <Text dimColor>{usageText(a, true)}{a.eligibility && !a.eligibility.eligible ? ` · not eligible: ${a.eligibility.reason}` : ''}{a.health !== 'ok' ? ` · ${a.health.replace('_', ' ')}` : ''}</Text>
          </Button>
        )),
        s.accounts.length === 0 ? Line(c, 'No accounts registered yet.', { dim: true }) : null,
      )}
      {Section(
        c,
        `Running sessions of ${env.id}`,
        ...env.sessions.map(x => Line(c, `${sessionName(x)}${x.sessionId === Q.sessionId ? ' [this session]' : ''} · ${x.status ?? '?'} · group ${x.group ?? '?'}${x.version ? ` · Claude ${x.version}` : ''}`)),
        env.sessions.length === 0 ? Line(c, 'None listed.', { dim: true }) : null,
        Line(c, 'Other clients of this environment (for example editor extensions) may not be listed.', { dim: true }),
      )}
    </Box>
  )
}

async function acceptSuggestion($: EngineInterface, c: Ctx): Promise<void> {
  const r = await run($, 'accepting the recommendation', () => actAccept($))
  if (!r) return
  if (!r.ok && r.blockedBy) {
    setForm($, 'conflict', JSON.stringify(r.blockedBy))
    go($, { tab: 'switch', sub: 'conflict', arg: myEnv(c.s!)?.suggestion?.to ?? undefined })
    return
  }
  if (r.ok) go($, { tab: 'switch', sub: 'progress' })
  else await flash($, 'warn', r.message ?? 'Not switched.')
}

// ------------------------------------------------------------ Accounts

function Accounts($: EngineInterface, c: Ctx): JSX.Element {
  const { Box, Button, Text } = c.els
  const s = c.s!
  const env = myEnv(s)
  const sub = c.view.sub
  if (sub === 'detail' && c.view.arg) return AccountDetail($, c, c.view.arg)
  if (sub === 'remove' && c.view.arg) return RemoveAccount($, c, c.view.arg)
  return (
    <Box flexDirection="column">
      {s.accounts.map(a => (
        <Button key={`acct:${a.id}`} plain onPress={() => go($, { tab: 'accounts', sub: 'detail', arg: a.id })}>
          {env && a.liveIn.includes(env.id) ? '> ' : '  '}
          {a.label} <Text dimColor>#{a.id} · {a.kind}{a.subscription ? ` ${a.subscription}` : ''} · {usageText(a)} · {a.health === 'ok' ? 'healthy' : a.health.replace('_', ' ')}</Text>
        </Button>
      ))}
      {s.accounts.length === 0 && Line(c, 'No accounts registered yet.', { dim: true })}
      {Row(
        c,
        env && !env.live.known && !env.live.signedOut && Btn(c, 'ac-register', 'Register current login', () => go($, { tab: 'onboarding', sub: 'register' }), true),
        Btn(c, 'ac-add', 'Add account', () => go($, { tab: 'onboarding', sub: 'add' })),
      )}
    </Box>
  )
}

function AccountDetail($: EngineInterface, c: Ctx, id: string): JSX.Element {
  const { Box } = c.els
  const s = c.s!
  const a = account(s, id)
  if (!a) return Line(c, 'That account is no longer registered.')
  const env = myEnv(s)
  const isLive = !!env && a.liveIn.includes(env.id)
  const u = a.usage
  return (
    <Box flexDirection="column">
      {Section(
        c,
        `${a.label}  (account #${a.id})`,
        Line(c, `${a.kind}${a.subscription ? ` · ${a.subscription}` : ''} · groups: ${a.groups.join(', ') || 'none'}${a.rank ? ` · rank ${a.rank}` : ''}`),
        Line(c, isLive ? `Stored login of ${env!.id}.` : a.liveIn.length ? `Stored login of ${a.liveIn.join(', ')}.` : 'Not the stored login of any environment.', { dim: true }),
        Line(c, healthText(a.health), { tone: a.health === 'ok' ? undefined : a.health === 'stale_access' ? undefined : 'warn' }),
      )}
      {Section(
        c,
        'Usage',
        ...(u ? u.windows.map(w => Line(c, `${windowLine(w)}${u.binding?.key === w.key ? '  (binding)' : ''}`)) : []),
        u ? Line(c, `Observed ${age(u.ageS)} ago (${u.source})${u.fresh ? '' : '; stale: not used for an automatic decision'}. An idle account's local snapshot does not count use on other machines.`, { dim: true }) : Line(c, a.usageNote ?? 'Not observed yet; missing usage is never treated as 0%.', { dim: true }),
        a.eligibility ? Line(c, `${a.eligibility.eligible ? 'Eligible' : 'Not eligible'}: ${a.eligibility.reason}`, { tone: a.eligibility.eligible ? undefined : 'warn' }) : null,
        (a.unavailableModels?.length ?? 0) > 0 ? Line(c, `Models learned unavailable on this account: ${a.unavailableModels!.join(', ')}`, { dim: true }) : null,
      )}
      {Field($, c, `rename:${a.id}`, 'Rename', { placeholder: a.label, submitLabel: 'rename', onSubmit: v => void renameAccount($, c, a.id, v) })}
      {Row(
        c,
        !isLive && Btn(c, 'ad-switch', 'Switch and pin', () => go($, { tab: 'switch', sub: 'review', arg: a.id }), true),
        Btn(c, 'ad-reauth', 'Reauthenticate', () => go($, { tab: 'onboarding', sub: 'add', arg: a.id })),
        Btn(c, 'ad-remove', 'Remove account', () => go($, { tab: 'accounts', sub: 'remove', arg: a.id })),
        Btn(c, 'ad-back', 'Back', () => go($, { tab: 'accounts' })),
      )}
    </Box>
  )
}

async function renameAccount($: EngineInterface, c: Ctx, id: string, value: string): Promise<void> {
  const v = value.trim()
  if (!v) return
  await run($, 'renaming', () => actRelabel($, id, v), r => (r.ok ? `Renamed to "${v}".` : (r.message ?? 'Refused.')))
  await clearForm($, `rename:${id}`)
}

function RemoveAccount($: EngineInterface, c: Ctx, id: string): JSX.Element {
  const { Box } = c.els
  const s = c.s!
  const a = account(s, id)
  if (!a) return Line(c, 'That account is no longer registered.')
  if (a.liveIn.length) {
    return (
      <Box flexDirection="column">
        {Line(c, `"${a.label}" is the stored login of ${a.liveIn.join(', ')} and cannot be removed while in use.`, { tone: 'warn' })}
        {Line(c, 'Switch that environment to another account first.', { dim: true })}
        {Row(c, Btn(c, 'rm-overview', 'View active environments', () => go($, { tab: 'overview' })), Btn(c, 'rm-cancel', 'Cancel', () => go($, { tab: 'accounts', sub: 'detail', arg: id })))}
      </Box>
    )
  }
  return (
    <Box flexDirection="column">
      {Section(
        c,
        `Remove "${a.label}"?`,
        Line(c, 'This removes Quayflow\'s entry and its protected credential copy, and withdraws the account from its groups for future switches.'),
        Line(c, 'It does not sign out, revoke provider authorization, or delete conversation history.', { dim: true }),
      )}
      {Row(
        c,
        Btn(c, 'rm-cancel', 'Cancel', () => go($, { tab: 'accounts', sub: 'detail', arg: id }), true),
        Btn(c, 'rm-confirm', 'Remove', () => void run($, 'removing', () => actRemove($, id), r => (r.ok ? `Removed "${a.label}" from Quayflow.` : (r.message ?? 'Refused.'))).then(r => r?.ok && go($, { tab: 'accounts' }))),
      )}
    </Box>
  )
}

// ------------------------------------------------------------ Groups

function Groups($: EngineInterface, c: Ctx): JSX.Element {
  const sub = c.view.sub
  if (sub === 'group' && c.view.arg) return GroupDetail($, c, c.view.arg)
  if (sub === 'automatic' && c.view.arg) return ApproveAutomatic($, c, c.view.arg)
  if (sub === 'disclosure') return Disclosure($, c)
  if (sub === 'lanes') return Lanes($, c)
  const { Box, Button, Text } = c.els
  const s = c.s!
  // The engine resolves the assignment; the path shown is the longest one that covers this folder.
  const assigned =
    s.session?.groupSource === 'assigned'
      ? [...s.projects].sort((a, b) => b.path.length - a.path.length).find(p => Q.cwd.toLowerCase().startsWith(p.path.toLowerCase()))
      : undefined
  const groupOpts = s.groups.map(g => ({ value: g.name, label: g.name }))
  return (
    <Box flexDirection="column">
      {Line(c, 'A group is a data approval: work in projects using it may be processed under any of its accounts.', { dim: true })}
      {s.groups.map(g => (
        <Button key={`grp:${g.name}`} plain onPress={() => go($, { tab: 'groups', sub: 'group', arg: g.name })}>
          {g.isDefault ? '* ' : '  '}
          {g.name} <Text dimColor>{g.mode}{g.mode === 'automatic' && !g.automaticApprovedAt ? ' (not approved)' : ''} · {g.accounts.length} account(s){g.preferred ? ` · home ${label(s, g.preferred)}` : ''}{g.mixed ? ' · mixed identities' : ''}</Text>
        </Button>
      ))}
      {Line(c, '* default group: used by projects without an assignment (a fallback, not a union of all accounts).', { dim: true })}
      {Section(
        c,
        'This project',
        Line(c, `${Q.cwd || '?'} -> ${myGroup(s)} (${assigned ? `explicit assignment for ${assigned.path}` : 'default group'})`),
        Pick(c, 'assign-group', 'Assign to', groupOpts, c.form['assign-group'] ?? myGroup(s), v => setForm($, 'assign-group', v)),
        Row(
          c,
          Btn(c, 'gr-assign', 'Save local assignment', () => void run($, 'assigning', () => actAssign($, Q.cwd, c.form['assign-group'] ?? myGroup(s)), r => (r.ok ? 'Assignment saved. Running jobs keep the destinations they started with.' : (r.message ?? 'Refused.')))),
          assigned && Btn(c, 'gr-unassign', 'Use default group', () => void run($, 'removing the assignment', () => actUnassign($, assigned.path), r => (r.ok ? `This project now uses the default group "${s.defaultGroup}".` : (r.message ?? 'Refused.')))),
        ),
        Line(c, 'Repository content can suggest settings but never grants account access.', { dim: true }),
      )}
      {Field($, c, 'new-group', 'New group', { placeholder: 'name', submitLabel: 'create', onSubmit: v => void createGroup($, c, v) })}
      {Row(c, Btn(c, 'gr-lanes', 'Lanes', () => go($, { tab: 'groups', sub: 'lanes' })))}
    </Box>
  )
}

async function createGroup($: EngineInterface, c: Ctx, name: string): Promise<void> {
  const n = name.trim()
  if (!n) return
  const r = await run($, 'creating the group', () => actGroupCreate($, n), r => (r.ok ? `Group "${n}" created (Suggest mode, no accounts).` : (r.message ?? 'Refused.')))
  if (r?.ok) {
    await clearForm($, 'new-group')
    go($, { tab: 'groups', sub: 'group', arg: n })
  }
}

function GroupDetail($: EngineInterface, c: Ctx, name: string): JSX.Element {
  const { Box } = c.els
  const s = c.s!
  const g = s.groups.find(x => x.name === name)
  if (!g) return Line(c, `No group "${name}".`)
  const nonMembers = s.accounts.filter(a => !g.accounts.includes(a.id))
  const modeOpts = (['manual', 'suggest', 'automatic'] as const).map(m => ({ value: m, label: m === 'automatic' && !g.automaticApprovedAt ? 'automatic (needs approval)' : m }))
  const rankOpts = ['preferred', 'normal', 'backup'].map(r => ({ value: r, label: r === 'preferred' ? 'preferred (home)' : r }))
  return (
    <Box flexDirection="column">
      {Section(
        c,
        `Group ${g.name}${g.isDefault ? ' (default)' : ''}`,
        g.mixed ? Line(c, `Mixed identities${g.mixedCovered ? ': disclosure accepted for this account set' : ': needs the one-time disclosure'}.`, { tone: g.mixedCovered ? undefined : 'warn' }) : null,
      )}
      {Pick(c, `mode:${g.name}`, 'Mode', modeOpts, g.mode, v => void changeMode($, c, g.name, v as Mode))}
      {Field($, c, `thr:${g.name}`, `Threshold % (now ${g.threshold})`, { placeholder: String(g.threshold), submitLabel: 'save', onSubmit: v => void saveTuning($, c, g.name, v, undefined) })}
      {Field($, c, `mar:${g.name}`, `Margin points (now ${g.margin})`, { placeholder: String(g.margin), submitLabel: 'save', onSubmit: v => void saveTuning($, c, g.name, undefined, v) })}
      {Section(
        c,
        'Members (rank breaks close choices; preferred = home)',
        ...g.accounts.map(id =>
          Row(
            c,
            Line(c, `${label(s, id)}`),
            Pick(c, `rank:${g.name}:${id}`, 'rank', rankOpts, g.preferred === id ? 'preferred' : (g.ranks?.[id] ?? 'normal'), v => void changeRank($, c, g.name, id, v)),
            Btn(c, `rm:${g.name}:${id}`, 'Remove from group', () => void changeMembers($, c, g.name, [], [id])),
          ),
        ),
        g.accounts.length === 0 ? Line(c, 'No accounts yet.', { dim: true }) : null,
      )}
      {nonMembers.length > 0 &&
        Pick(c, `add:${g.name}`, 'Add account', [{ value: '', label: '(choose)' }, ...nonMembers.map(a => ({ value: a.id, label: a.label }))], '', v => v && void changeMembers($, c, g.name, [v], []))}
      {Row(
        c,
        !g.isDefault && Btn(c, 'gd-default', 'Make default', () => void run($, 'setting the default', () => actSetDefault($, g.name), r => (r.ok ? `"${g.name}" is the default group now.` : (r.message ?? 'Refused.')))),
        g.preferred && Btn(c, 'gd-nohome', 'Clear home', () => void run($, 'clearing home', () => actPreferred($, g.name, null), r => (r.ok ? 'No home account: Quayflow stays on the current account until a trigger.' : (r.message ?? 'Refused.')))),
        !g.isDefault && Btn(c, 'gd-delete', 'Delete group', () => void run($, 'deleting the group', () => actGroupDelete($, g.name), r => (r.ok ? `Group "${g.name}" deleted.` : (r.message ?? 'Refused.'))).then(r => r?.ok && go($, { tab: 'groups' }))),
        Btn(c, 'gd-back', 'Back', () => go($, { tab: 'groups' })),
      )}
    </Box>
  )
}

async function changeMode($: EngineInterface, c: Ctx, group: string, mode: Mode): Promise<void> {
  const approved = !!c.s?.groups.find(g => g.name === group)?.automaticApprovedAt
  if (mode === 'automatic' && !approved) return go($, { tab: 'groups', sub: 'automatic', arg: group })
  await run(
    $,
    'saving the mode',
    () => actSetMode($, group, mode, false),
    r => (r.ok ? `Group "${group}" is in ${mode} mode.` : (r.message ?? 'Refused.')),
    async err => {
      if (!hasCode(err, 'needs_approval')) return false
      go($, { tab: 'groups', sub: 'automatic', arg: group })
      return true
    },
  )
}

async function saveTuning($: EngineInterface, c: Ctx, group: string, threshold: string | undefined, margin: string | undefined): Promise<void> {
  const t = threshold !== undefined ? Number(threshold) : undefined
  const m = margin !== undefined ? Number(margin) : undefined
  if ((t !== undefined && !(t > 0 && t <= 100)) || (m !== undefined && !(m >= 0 && m < 100))) {
    return void flash($, 'warn', 'Threshold is a percentage (1-100); margin is points below it (0-99).')
  }
  await run($, 'saving tuning', () => actTuning($, group, t, m), r => (r.ok ? 'Tuning saved.' : (r.message ?? 'Refused.')))
  await clearForm($, `thr:${group}`, `mar:${group}`)
}

async function changeRank($: EngineInterface, c: Ctx, group: string, id: string, rank: string): Promise<void> {
  if (rank === 'preferred') {
    await run($, 'setting home', () => actPreferred($, group, id), r => (r.ok ? `"${label(c.s ?? undefined, id)}" is the home of ${group}; it replaces any earlier home.` : (r.message ?? 'Refused.')))
    return
  }
  await run($, 'saving the rank', () => actRank($, group, id, rank), r => (r.ok ? 'Rank saved.' : (r.message ?? 'Refused.')))
}

async function changeMembers($: EngineInterface, c: Ctx, group: string, add: string[], remove: string[]): Promise<void> {
  const r = await run($, 'saving membership', () => actMembers($, group, add, remove, false))
  if (!r) return
  if (r.needsDisclosure) {
    await setDisclosure($, { then: 'members', group, add, text: r.disclosure ?? '' })
    go($, { tab: 'groups', sub: 'disclosure' })
    return
  }
  await flash($, r.ok ? 'info' : 'warn', r.ok ? (remove.length ? 'Removed from the group: its approval for future switches is withdrawn. Running work is not moved.' : 'Added to the group.') : (r.message ?? 'Refused.'))
}

function Disclosure($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const d = c.disclosure
  const s = c.s!
  if (!d) return Line(c, 'No pending disclosure.')
  const members = s.groups.find(x => x.name === d.group)?.accounts ?? []
  const joining =
    d.then === 'members' ? (d.add ?? []) : d.then === 'register' ? [myEnv(s)?.live.account ?? ''] : [c.signin?.status.identity?.account ?? '']
  const names = [
    ...members.map(id => `${label(s, id)} (${account(s, id)?.kind ?? '?'})`),
    ...joining.filter(id => id && !members.includes(id)).map(id => `${account(s, id)?.label ?? d.label ?? `#${id}`} (joining)`),
  ]
  const back: QuayView = d.then === 'members' ? { tab: 'groups' } : { tab: 'onboarding', sub: d.then === 'register' ? 'register' : 'add' }
  return (
    <Box flexDirection="column">
      {Section(
        c,
        `One-time disclosure for group ${d.group}`,
        Line(c, d.text || `Group "${d.group}" would then mix identities.`),
        Line(c, `Accounts: ${names.join(', ')}`),
        Line(c, "Work in projects using this group may be sent to any of these accounts. Your organization's rules on where its work may be processed remain your responsibility."),
        Line(c, 'This does not broaden the destinations of jobs already running.', { dim: true }),
      )}
      {Row(
        c,
        Btn(c, 'dc-approve', 'Approve group and save', () => void approveDisclosure($, c)),
        Btn(c, 'dc-other', 'Choose a different group', () => void dropDisclosure($, back)),
        Btn(c, 'dc-cancel', 'Cancel', () => void dropDisclosure($, d.then === 'members' ? { tab: 'groups', sub: 'group', arg: d.group } : { tab: 'accounts' }), true),
      )}
    </Box>
  )
}

/** Resends the change that needed the disclosure, now with `acceptDisclosure: true`. */
async function approveDisclosure($: EngineInterface, c: Ctx): Promise<void> {
  const d = c.disclosure
  if (!d) return
  const doing = 'saving with the accepted disclosure'
  if (d.then === 'register') {
    const a = await run($, doing, () => actRegister($, d.label ?? '', d.group, true))
    if (!a) return
    await setDisclosure($, null)
    await afterRegister($, c, a.label, d.group, a.note)
    return
  }
  if (d.then === 'confirm') {
    const r = await run($, doing, () => actSigninConfirm($, d.signinId ?? '', d.label ?? '', d.group, true))
    if (!r) return
    await setDisclosure($, null)
    await afterConfirm($, c, d.label ?? '', d.group, r.message ?? undefined)
    return
  }
  const r = await run($, doing, () => actMembers($, d.group, d.add ?? [], [], true), r =>
    r.ok ? `Group "${d.group}" saved; the disclosure acceptance is recorded with it.` : (r.message ?? 'Refused.'),
  )
  if (r?.ok) await dropDisclosure($, { tab: 'groups', sub: 'group', arg: d.group })
}

async function dropDisclosure($: EngineInterface, view: QuayView): Promise<void> {
  await setDisclosure($, null)
  go($, view)
}

function ApproveAutomatic($: EngineInterface, c: Ctx, group: string): JSX.Element {
  const { Box } = c.els
  return (
    <Box flexDirection="column">
      {Section(
        c,
        `Approve Automatic for group ${group}?`,
        Line(c, 'Quayflow will then, without asking each time: switch proactively at the threshold, return to the home account after a reset, and recover from confirmed usage limits by switching and retrying the failed request once.'),
        Line(c, 'This applies to interactive sessions and to unattended headless jobs (claude -p) of projects using this group.'),
        Line(c, 'Still enforced: group permissions, managed login, pins, cooldown, plan compatibility, version checks and adoption verification. A session is never moved into a lane automatically.', { dim: true }),
      )}
      {Row(
        c,
        Btn(c, 'au-approve', 'Approve Automatic', () => void run($, 'saving Automatic', () => actSetMode($, group, 'automatic', true), r => (r.ok ? `Group "${group}" is in Automatic mode.` : (r.message ?? 'Refused.'))).then(() => go($, { tab: 'groups', sub: 'group', arg: group }))),
        Btn(c, 'au-cancel', 'Keep current mode', () => go($, { tab: 'groups', sub: 'group', arg: group }), true),
      )}
    </Box>
  )
}

function Lanes($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const s = c.s!
  return (
    <Box flexDirection="column">
      {Line(c, 'A lane is a named credential environment with its own login, .claude.json, session folder and policy files; your settings, CLAUDE.md, plugins, skills, agents, commands and memory are linked.', { dim: true })}
      {s.lanes.map(l =>
        Section(
          c,
          `Lane ${l.name}`,
          Line(c, `${l.live ? `login "${label(s, l.live)}"` : 'no login yet'} · ${l.sessions} session(s) listed · ${l.configDir}`),
          Line(c, `Links ${l.linksOk ? 'ok' : `need repair: ${l.linkDetail.join('; ')}`} · in-place switching ${l.canaryPassed ? 'enabled (canary passed)' : 'off until its canary passes'}`, { tone: l.linksOk ? undefined : 'warn' }),
          Row(
            c,
            Btn(c, `ln-replace:${l.name}`, 'Move this session here (same terminal)', () => void handoff($, c, 'replace', l.name)),
            Btn(c, `ln-open:${l.name}`, 'Open in a new window', () => void handoff($, c, 'open', l.name)),
            Btn(c, `ln-check:${l.name}`, 'Check links', () => void run($, 'checking the lane', () => actLaneCheck($, l.name), r => r.message ?? (r.ok ? 'Lane checked.' : 'Check failed.'))),
            Btn(c, `ln-delete:${l.name}`, 'Delete lane', () => void run($, 'deleting the lane', () => actLaneDelete($, l.name), r => (r.ok ? `Lane "${l.name}" deleted.` : (r.message ?? 'Refused.')))),
          ),
        ),
      )}
      {s.lanes.length === 0 && Line(c, 'No lanes yet.', { dim: true })}
      {Field($, c, 'new-lane', 'New lane', { placeholder: 'name', submitLabel: 'create', onSubmit: v => void createLane($, c, v) })}
      {Line(c, 'Same-terminal replacement needs the PowerShell prompt hook (quayflow shell install); other shells get a new window or the exact command.', { dim: true })}
      {Row(c, Btn(c, 'ln-back', 'Back', () => go($, { tab: 'groups' })))}
    </Box>
  )
}

async function createLane($: EngineInterface, c: Ctx, name: string): Promise<void> {
  const n = name.trim()
  if (!n) return
  await run($, 'creating the lane and linking your setup', () => actLaneCreate($, n), l => `Lane "${l.name}" created; links ${l.linksOk ? 'ok' : 'need repair'}. In-place switching there waits for its canary.`)
  await clearForm($, 'new-lane')
}

async function handoff($: EngineInterface, c: Ctx, how: 'open' | 'replace', lane: string): Promise<void> {
  await run($, how === 'replace' ? 'preparing the full-history replacement' : 'opening a full-history session', () => actHandoff($, how, lane), r => handoffOutcome(how, lane, r))
}

// ------------------------------------------------------------ Activity

function Activity($: EngineInterface, c: Ctx): JSX.Element {
  const { Box, Code } = c.els
  const s = c.s!
  const items = [...s.activity].sort((a, b) => b.ts - a.ts).slice(0, 40)
  return (
    <Box flexDirection="column">
      {items.map(i => Line(c, `${when(i.ts)}  ${i.kind.padEnd(12)} ${i.text}`))}
      {items.length === 0 && Line(c, 'No activity recorded yet.', { dim: true })}
      {Line(c, `Routine logs are sanitized and kept ${s.tuning.logRetentionDays} days (${s.tuning.logMaxMb} MB per day); unresolved journal records are kept until resolved.`, { dim: true })}
      {Row(c, Btn(c, 'act-report', 'Build support report', () => void buildReport($, c)))}
      {c.report &&
        Section(
          c,
          'Support report (saved locally; nothing is uploaded)',
          Line(c, c.report.path),
          <Code key="report-preview" source={c.report.preview.split('\n').slice(0, 60).join('\n')} />,
        )}
    </Box>
  )
}

async function buildReport($: EngineInterface, c: Ctx): Promise<void> {
  const r = await run($, 'building the support report', () => actReport($), r => `Report saved for your review: ${r.path}`)
  if (r) await setReport($, r)
}

// ------------------------------------------------------------ Settings

function Settings($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const s = c.s!
  const env = myEnv(s)
  const sub = c.view.sub
  const e = s.engine
  if (sub === 'stop') {
    return (
      <Box flexDirection="column">
        {Section(
          c,
          'Stop the engine?',
          Line(c, 'Claude keeps running unchanged. Quayflow stays off, and is not restarted on demand, until you start it again (/quay start or quayflow start).'),
        )}
        {Row(
          c,
          Btn(c, 'st-cancel', 'Keep running', () => go($, { tab: 'settings' }), true),
          Btn(c, 'st-stop', 'Stop engine', () => void run($, 'stopping the engine', () => actEngine($, 'stop'), r => (r.ok ? 'Engine stopped.' : (r.message ?? 'Refused.')))),
        )}
      </Box>
    )
  }
  const consents = s.consents
  const consentLine = (name: string, at: number | null | undefined): JSX.Element => Line(c, `${at ? '[x]' : '[ ]'} ${name}${at ? ` (since ${when(at)})` : ''}`)
  const managed = s.envs.filter(x => (x.managed.forceOrg?.length ?? 0) > 0 || x.managed.forceMethod)
  return (
    <Box flexDirection="column">
      {Section(
        c,
        'Engine',
        Line(c, `quayflow ${e.version} · protocol ${e.protocol} · pid ${e.pid} · started ${when(e.startedAt)} · data ${e.dataDir}`),
        Line(c, `Integrity: ${e.integrity} · version ${e.version} · SHA-256 ${e.exeSha256}`),
        Line(c, `Mod ${Q.link.engineVersion ? '' : ''}0.1.0 · Claude ${c.claudeVersion || '?'}`, { dim: true }),
        Row(
          c,
          env?.automationPaused
            ? Btn(c, 'se-resume', 'Resume automation', () => void run($, 'resuming', () => actEngine($, 'resume'), r => (r.ok ? 'Automation resumed.' : (r.message ?? 'Refused.'))))
            : Btn(c, 'se-pause', 'Pause automation', () => void run($, 'pausing', () => actEngine($, 'pause'), r => (r.ok ? 'Automation paused; manual switches still work.' : (r.message ?? 'Refused.')))),
          Btn(c, 'se-stop', 'Stop engine', () => go($, { tab: 'settings', sub: 'stop' })),
        ),
      )}
      {Section(
        c,
        'Version compatibility',
        ...s.compat.map(r =>
          Line(c, `Claude ${r.version}: static ${r.staticPass ? 'pass' : 'fail'}${r.livePass === true ? ', live pass' : r.livePass === false ? `, live FAIL${r.liveDetail ? ` (${r.liveDetail})` : ''}` : ', live not run'} · checked ${when(r.checkedAt)}`, {
            tone: r.staticPass && r.livePass !== false ? undefined : 'warn',
          }),
        ),
        s.compat.length === 0 ? Line(c, 'No version checked yet.', { dim: true }) : null,
        env ? Line(c, `This environment: in-place switching ${env.inPlace.allowed ? 'allowed' : 'off'} (${env.inPlace.reason})${env.inPlace.governingVersion ? `; oldest running Claude ${env.inPlace.governingVersion}` : ''}.`, { dim: true }) : null,
        Row(c, Btn(c, 'se-compat', 'Run static check', () => void run($, 'checking this Claude version', () => actCompatCheck($), () => 'Static check done; see the list above.'))),
      )}
      {Section(
        c,
        'Organization-managed login',
        ...(managed.length
          ? managed.map(x => Line(c, `${x.id}: ${(x.managed.forceOrg ?? []).map(o => orgName(s, o)).join(', ') || 'no organization'}${x.managed.forceMethod ? ` · method ${x.managed.forceMethod}` : ''} · ${(x.managed.sources ?? []).join(', ')}`))
          : [Line(c, 'No enforcement detected.', { dim: true })]),
        Line(c, `Managed-login exception: ${s.dOrg.enabled ? 'ON (Quayflow may switch to accounts the policy would refuse)' : 'off (enforcement respected)'}`, { tone: s.dOrg.enabled ? 'warn' : undefined }),
        Row(c, Btn(c, 'se-dorg', s.dOrg.enabled ? 'Review exception' : 'Review owner-specified exception', () => go($, { tab: 'trust', sub: 'dorg' }))),
      )}
      {Section(
        c,
        'Consents',
        consentLine('Start the engine on demand', consents.engineStart),
        consentLine('Start at Windows sign-in', consents.signinStartup),
        consentLine('Store credentials in the protected vault', consents.credentialStorage),
        consentLine('Terms and organization-rules acknowledgment', consents.terms),
        consentLine('PowerShell prompt hook (same-terminal replacement)', consents.promptHook),
        consentLine('Live version canary', consents.liveCanary),
        Line(c, 'Change one in a terminal: quayflow consent <engine-start|signin-startup|credential-storage|terms|live-canary> <on|off>', { dim: true }),
      )}
      {Section(
        c,
        'Privacy and retention',
        Line(c, 'Credentials live only in the per-user vault (Windows DPAPI); no cloud vault, export or sync. Protected-storage failure never falls back to plaintext.'),
        Line(c, 'Logs and reports hold opaque ids, labels and sanitized failure shapes: never prompts, responses, tool content, tokens, source code or e-mail addresses.'),
        Line(c, 'Before a manual switch away from an unknown login, a protected backup of it is stored.', { dim: true }),
      )}
      {Section(
        c,
        'Update and uninstall',
        Line(c, 'Update the engine and this Mod together (hash-verified; never during a transition):  quayflow update'),
        Line(c, 'Uninstall:  quayflow uninstall   (removes the Mod, startup entry, prompt hook and engine; your current login and history stay)'),
        Line(c, 'Add --delete-data to also delete the vault and logs; unresolved recovery records are reported first.', { dim: true }),
      )}
    </Box>
  )
}
