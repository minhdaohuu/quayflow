// The group editor: mode, threshold and margin, home, ranks and members, edited as a draft.
// Nothing is sent until Save; a live what-if line says what the draft would do with the
// accounts' current usage, and Revert drops it. Save applies in a safe order (members, which
// may need the mixed-identity disclosure; tuning; home and ranks; mode last, Automatic through
// its one-time approval). The draft lives in `$.state` form keys `gd:<group>:...` and is
// compared with the engine's state, so a part already saved drops out of it by itself.
import type { EngineInterface } from 'claude-code'

import type { AccountView, GroupView, Mode, StateView, Window } from '../types'
import { clearFormKeys, flash, setDisclosure } from './context'
import { actGroupDelete, actMembers, actPreferred, actRank, actSetDefault, actSetMode, actTuning } from './actions'
import { label, myEnv, myGroup } from './format'
import { hasCode } from './engine'
import { Badge, Btn, Card, Field, Gauge, Hint, Line, Row, Section, go, run, setForm } from './ui'
import type { Ctx } from './ui'

const MODES: { mode: Mode; name: string; words: string }[] = [
  { mode: 'manual', name: 'Manual', words: 'Only the switches you ask for. Headless jobs: no switching.' },
  { mode: 'suggest', name: 'Suggest', words: 'Suggests a switch; /quay accept makes it. Headless jobs: a recorded suggestion only.' },
  { mode: 'automatic', name: 'Automatic', words: 'Switches by itself, returns home and recovers usage limits, in headless jobs too.' },
]

export const PRESETS: { key: string; name: string; threshold: number; margin: number; words: string }[] = [
  { key: 'early', name: 'Early', threshold: 75, margin: 10, words: 'switch at 75%; targets need 65% or less' },
  { key: 'balanced', name: 'Balanced', threshold: 85, margin: 10, words: 'switch at 85%; targets need 75% or less' },
  { key: 'late', name: 'Late', threshold: 95, margin: 5, words: 'switch at 95%; targets need 90% or less' },
]

const RANKS = ['preferred', 'normal', 'backup'] as const
type Rank = (typeof RANKS)[number]

/** The draft of one group: only what differs from the engine's state. */
export type Draft = {
  mode?: Mode
  threshold?: number
  margin?: number
  /** A new home: an account id, or null to clear it. */
  home?: string | null
  ranks: Record<string, Rank>
  add: string[]
  remove: string[]
  /** Why the draft cannot be saved as it is (a bad number). */
  problems: string[]
  /** One phrase per change, for the unsaved bar. */
  changes: string[]
}

const key = (g: string, k: string): string => `gd:${g}:${k}`

/** Every form key a group's draft may use: cleared together on Save and Revert. */
function draftKeys(c: Ctx, g: GroupView): string[] {
  const p = `gd:${g.name}:`
  return Object.keys(c.form).filter(k => k.startsWith(p))
}

function rankOf(g: GroupView, id: string): Rank {
  return ((g.ranks as Record<string, string> | undefined)?.[id] as Rank | undefined) ?? 'normal'
}

const cap = (m: string): string => m[0]!.toUpperCase() + m.slice(1)

export function draftOf(c: Ctx, s: StateView, g: GroupView): Draft {
  const f = c.form
  const d: Draft = { ranks: {}, add: [], remove: [], problems: [], changes: [] }
  const name = (id: string): string => label(s, id)
  const mode = f[key(g.name, 'mode')] as Mode | undefined
  if (mode && mode !== g.mode) {
    d.mode = mode
    d.changes.push(`mode ${cap(g.mode)} → ${cap(mode)}`)
  }
  const num = (k: 'thr' | 'mar'): number | undefined => {
    const raw = f[key(g.name, k)]
    if (raw === undefined || raw.trim() === '') return undefined
    const n = Number(raw.trim())
    return Number.isFinite(n) ? n : NaN
  }
  const t = num('thr')
  const m = num('mar')
  const thr = t ?? g.threshold
  const mar = m ?? g.margin
  if (Number.isNaN(thr) || !(thr > 0 && thr <= 100)) d.problems.push('Threshold is a percentage, 1-100.')
  else if (Number.isNaN(mar) || !(mar >= 0 && mar < thr)) d.problems.push(`Margin is 0 or more and below the threshold (${thr}).`)
  if (t !== undefined && t !== g.threshold) {
    d.threshold = t
    d.changes.push(`threshold ${g.threshold}% → ${Number.isNaN(t) ? '?' : `${t}%`}`)
  }
  if (m !== undefined && m !== g.margin) {
    d.margin = m
    d.changes.push(`margin ${g.margin} → ${Number.isNaN(m) ? '?' : m}`)
  }
  const home = f[key(g.name, 'home')]
  if (home !== undefined) {
    const next = home === 'none' ? null : home
    if (next !== (g.preferred ?? null)) {
      d.home = next
      d.changes.push(next ? `home ${name(next)}` : 'no home')
    }
  }
  for (const id of g.accounts) {
    const r = f[key(g.name, `rank:${id}`)] as Rank | undefined
    if (r && r !== rankOf(g, id)) {
      d.ranks[id] = r
      d.changes.push(`${name(id)} ${r}`)
    }
    if (f[key(g.name, `rm:${id}`)] === '1') {
      d.remove.push(id)
      d.changes.push(`remove ${name(id)}`)
    }
  }
  for (const a of s.accounts) {
    if (!g.accounts.includes(a.id) && f[key(g.name, `add:${a.id}`)] === '1') {
      d.add.push(a.id)
      d.changes.push(`add ${a.label}`)
    }
  }
  return d
}

/** The draft's effective values: the draft where it changes something, else the group's. */
function effective(g: GroupView, d: Draft) {
  const ok = (n: number | undefined, fallback: number): number => (n === undefined || Number.isNaN(n) ? fallback : n)
  const members = [...g.accounts.filter(id => !d.remove.includes(id)), ...d.add]
  const home = d.home === undefined ? (g.preferred ?? null) : d.home
  return {
    mode: d.mode ?? g.mode,
    threshold: ok(d.threshold, g.threshold),
    margin: ok(d.margin, g.margin),
    home: home && members.includes(home) ? home : null,
    members,
  }
}

function fresh5And7(a: AccountView | undefined): { five?: Window; seven?: Window; binding?: number } {
  if (!a?.usage?.fresh) return {}
  const ws = a.usage.windows ?? []
  return {
    five: ws.find(w => w.key === '5h' || w.key === 'five_hour'),
    seven: ws.find(w => w.key === '7d' || w.key === 'seven_day'),
    binding: a.usage.binding?.pct,
  }
}

/**
 * What the draft would do now, from current readings: whether the live account would be moved,
 * and to which account. A preview only: the engine decides with its full rules (cooldown, pins,
 * weekly balancing, plan and group checks).
 */
export function whatIf(s: StateView, g: GroupView, d: Draft): { text: string; tone?: 'warn' } {
  const e = effective(g, d)
  const env = myEnv(s)
  const liveId = env?.live.account ?? undefined
  const acct = (id: string) => s.accounts.find(a => a.id === id)
  const gate = e.threshold - e.margin
  const targets = e.members
    .filter(id => id !== liveId)
    .map(id => ({ id, a: acct(id), u: fresh5And7(acct(id)) }))
    // As the engine: only a rejected sign-in or an unreadable vault copy rules an account out.
    .filter(x => x.a && x.a.health !== 'refresh_rejected' && x.a.health !== 'vault_error' && x.u.binding !== undefined && x.u.binding <= gate && x.u.binding < s.tuning.hardLimitPct)
    .sort((x, y) => (y.a!.usage?.weeklyPerDay ?? -1) - (x.a!.usage?.weeklyPerDay ?? -1) || x.u.binding! - y.u.binding!)
  const names = targets.map(x => `${x.a!.label} (${Math.round(x.u.binding!)}%)`)
  const targetsText = names.length ? `targets now: ${names.join(', ')}` : `no account is at ${gate}% or less now`
  if (myGroup(s) !== g.name || !liveId) return { text: `Targets need ${gate}% or less; ${targetsText}.` }
  const live = acct(liveId)
  const lu = fresh5And7(live)
  if (lu.binding === undefined) return { text: `${live?.label ?? 'The live account'}: usage unknown, nothing is decided on it; ${targetsText}.` }
  const at = `${live!.label} at ${Math.round(lu.binding)}%`
  const act = (to: string): string =>
    e.mode === 'automatic' ? `would switch now to ${to}` : e.mode === 'suggest' ? `would suggest ${to}` : `Manual: nothing moves by itself (best: ${to})`
  if (e.home && e.home !== liveId) {
    const h = targets.find(x => x.id === e.home)
    const home = h ? `home ${h.a!.label} (${Math.round(h.u.binding!)}%)` : ''
    if (h) return { text: `${at}: ${e.mode === 'manual' ? act(home) : e.mode === 'automatic' ? `would return to ${home}` : `would suggest returning to ${home}`}.`, tone: 'warn' }
  }
  if (lu.binding < e.threshold) return { text: `${at}, below ${e.threshold}%: stays (weekly balancing may still move it); ${targetsText}.` }
  if (!targets.length) return { text: `${at} ≥ ${e.threshold}%, but ${targetsText}: stays.`, tone: 'warn' }
  return { text: `${at} ≥ ${e.threshold}%: ${act(`${targets[0]!.a!.label} (${Math.round(targets[0]!.u.binding!)}%)`)}.`, tone: 'warn' }
}

/** A row of choices, the chosen one filled: a segmented control drawn with Buttons. */
function Segments(c: Ctx, k: string, items: { value: string; name: string }[], chosen: string | undefined, onPick: (v: string) => void): JSX.Element {
  const { Box, Button, Text } = c.els
  return (
    <Box flexDirection="row" flexWrap="wrap" columnGap={1}>
      {items.map(it => (
        <Button key={`${k}:${it.value}`} label={it.name} onPress={() => onPick(it.value)}>
          {it.value === chosen ? (
            <Text backgroundColor="claude" color="inverseText" bold>{` ${it.name} `}</Text>
          ) : (
            <Text>{` ${it.name} `}</Text>
          )}
        </Button>
      ))}
    </Box>
  )
}

export function GroupDetail($: EngineInterface, c: Ctx, name: string): JSX.Element {
  const { Box, Button, Text } = c.els
  const s = c.s!
  const g = s.groups.find(x => x.name === name)
  if (!g) return Line(c, `No group "${name}".`)
  const d = draftOf(c, s, g)
  const e = effective(g, d)
  const set = (k: string, v: string) => setForm($, key(g.name, k), v)
  const preset = PRESETS.find(p => p.threshold === e.threshold && p.margin === e.margin)
  const nonMembers = s.accounts.filter(a => !g.accounts.includes(a.id))
  const unsaved = d.changes.length
  const preview = whatIf(s, g, d)
  const modeNow = MODES.find(m => m.mode === e.mode)!
  const needsApproval = e.mode === 'automatic' && !g.automaticApprovedAt

  const member = (id: string): JSX.Element => {
    const a = s.accounts.find(x => x.id === id)
    const u = fresh5And7(a)
    const removing = d.remove.includes(id)
    const isHome = e.home === id
    const rank = d.ranks[id] ?? rankOf(g, id)
    const liveHere = s.envs.some(x => x.live?.account === id)
    return (
      <Box key={`gm:${id}`} flexDirection="column" marginTop={1}>
        <Box flexDirection="row" columnGap={1}>
          <Text color={isHome ? 'claude' : undefined}>{isHome ? '⌂' : ' '}</Text>
          <Text bold strikethrough={removing} dimColor={removing}>
            {label(s, id)}
          </Text>
          {isHome ? Badge(c, 'HOME', 'accent') : null}
          {liveHere ? Badge(c, 'LIVE', 'ok') : null}
          {removing ? Badge(c, 'REMOVING', 'warn') : null}
          <Text dimColor>5h</Text>
          {Gauge(c, u.five?.pct, 6, { threshold: e.threshold, margin: e.margin, stale: !a?.usage?.fresh })}
          <Text dimColor>7d</Text>
          {Gauge(c, u.seven?.pct, 6, { threshold: e.threshold, margin: e.margin, stale: !a?.usage?.fresh })}
          {a?.usage?.weeklyPerDay != null ? <Text dimColor>{`${Math.round(a.usage.weeklyPerDay)}%/day`}</Text> : null}
        </Box>
        {removing ? (
          <Box flexDirection="row" columnGap={1} paddingLeft={2}>
            {Btn(c, `gm-keep:${id}`, 'Keep', () => set(`rm:${id}`, ''))}
          </Box>
        ) : (
          <Box flexDirection="row" flexWrap="wrap" columnGap={1} paddingLeft={2}>
            {isHome ? (
              <Text dimColor>ranks first as home</Text>
            ) : (
              Segments(c, `gm-rank:${id}`, RANKS.map(r => ({ value: r, name: cap(r) })), rank, v => set(`rank:${id}`, v))
            )}
            {isHome ? Btn(c, `gm-nohome:${id}`, 'No home', () => set('home', 'none')) : Btn(c, `gm-home:${id}`, 'Make home', () => set('home', id))}
            {Btn(c, `gm-rm:${id}`, 'Remove', () => set(`rm:${id}`, '1'))}
          </Box>
        )}
      </Box>
    )
  }

  return (
    <Box flexDirection="column">
      <Box flexDirection="row" columnGap={1}>
        <Text color="claude">{'▍'}</Text>
        <Text bold>{`Group ${g.name}`}</Text>
        {g.isDefault ? Badge(c, 'DEFAULT', 'muted') : null}
        {g.mixed ? Badge(c, 'MIXED', g.mixedCovered ? 'muted' : 'warn') : null}
        {unsaved ? <Text color="warning">{`● ${unsaved} unsaved`}</Text> : <Text dimColor>saved</Text>}
      </Box>
      {g.mixed && !g.mixedCovered ? Line(c, 'Mixed identities: needs the one-time disclosure.', { tone: 'warn' }) : null}

      {Section(
        c,
        'Mode',
        Segments(c, `gd-mode`, MODES.map(m => ({ value: m.mode, name: m.name })), e.mode, v => set('mode', v)),
        Line(c, modeNow.words, { dim: true }),
        needsApproval ? Line(c, 'Automatic needs its one-time approval: Save opens it.', { tone: 'warn' }) : null,
      )}

      {Section(
        c,
        'Switch point',
        Segments(c, 'gd-preset', PRESETS.map(p => ({ value: p.key, name: `${p.name} ${p.threshold}/${p.margin}` })), preset?.key, v => {
          const p = PRESETS.find(x => x.key === v)!
          set('thr', String(p.threshold))
          set('mar', String(p.margin))
        }),
        Line(c, preset ? `${preset.name}: ${preset.words}.` : `Custom: switch at ${e.threshold}%; targets need ${e.threshold - e.margin}% or less.`, { dim: true }),
        Row(
          c,
          Field($, c, key(g.name, 'thr'), 'Threshold %', { placeholder: String(g.threshold), submitLabel: 'set', onSubmit: () => {} }),
          Field($, c, key(g.name, 'mar'), 'Margin', { placeholder: String(g.margin), submitLabel: 'set', onSubmit: () => {} }),
        ),
        Hint(c, 'The threshold is the usage of the live account\'s fullest window (5 h, 7 d or per-model) at which Quayflow moves; a target account must be at least the margin below it. Weekly balancing can also move earlier when another account\'s week would be lost sooner.'),
      )}

      {Section(
        c,
        'Members',
        ...g.accounts.map(member),
        ...d.add.map(id => (
          <Box key={`gm-new:${id}`} flexDirection="row" columnGap={1} marginTop={1}>
            <Text color="success">+</Text>
            <Text bold>{label(s, id)}</Text>
            {Badge(c, 'ADDING', 'ok')}
            {Btn(c, `gm-undo:${id}`, 'Undo', () => set(`add:${id}`, ''))}
          </Box>
        )),
        g.accounts.length === 0 && d.add.length === 0 ? Line(c, 'No accounts yet.', { dim: true }) : null,
        nonMembers.filter(a => !d.add.includes(a.id)).length > 0
          ? Row(c, <Text dimColor>+ Add</Text>, ...nonMembers.filter(a => !d.add.includes(a.id)).map(a => Btn(c, `gm-add:${a.id}`, a.label, () => set(`add:${a.id}`, '1'))))
          : null,
        Hint(c, 'Membership is the data approval: work in projects using this group may be processed under any of its accounts. Home: Quayflow returns to it when it has room again. Ranks only break close choices.'),
      )}

      {Card(
        c,
        unsaved ? `${unsaved} unsaved change${unsaved > 1 ? 's' : ''}` : 'What these settings do now',
        unsaved ? 'accent' : undefined,
        unsaved ? Line(c, d.changes.join(' · ')) : null,
        Line(c, `What if: ${preview.text}`, { tone: preview.tone }),
        ...d.problems.map(p => Line(c, p, { tone: 'error' })),
        Hint(c, 'A preview from current readings; the engine decides with its full rules (cooldown, pins, weekly balancing, plan and group checks).'),
        unsaved ? (
          <Box flexDirection="row" columnGap={1}>
            <Button key="gd-save" label={`Save ${unsaved} change${unsaved > 1 ? 's' : ''}`} hotkey="s" variant="primary" onPress={() => void saveDraft($, c, g.name)} />
            <Button key="gd-revert" label="Revert" hotkey="r" onPress={() => void clearFormKeys($, draftKeys(c, g))} />
          </Box>
        ) : null,
      )}

      {Row(
        c,
        !g.isDefault && Btn(c, 'gd-default', 'Make default', () => void run($, 'setting the default', () => actSetDefault($, g.name), r => (r.ok ? `"${g.name}" is the default group now.` : (r.message ?? 'Refused.')))),
        g.preferred && !s.envs.some(x => x.live?.account === g.preferred) && Btn(c, 'gd-home', 'Switch home', () => go($, { tab: 'switch', sub: 'review', arg: g.preferred! })),
        !g.isDefault && Btn(c, 'gd-delete', 'Delete group', () => void run($, 'deleting the group', () => actGroupDelete($, g.name), r => (r.ok ? `Group "${g.name}" deleted.` : (r.message ?? 'Refused.'))).then(r => r?.ok && go($, { tab: 'groups' }))),
        Btn(c, 'gd-back', 'Back', () => go($, { tab: 'groups' })),
      )}
    </Box>
  )
}

/**
 * Applies a group's draft: members (the mixed-identity disclosure takes over when needed and
 * the rest waits for the next Save), tuning, home and ranks, then the mode (Automatic without
 * its approval opens it). Stops at the first refusal; what was saved drops out of the draft.
 */
export async function saveDraft($: EngineInterface, c: Ctx, group: string): Promise<void> {
  const s = c.s
  const g = s?.groups.find(x => x.name === group)
  if (!s || !g) return
  const d = draftOf(c, s, g)
  if (d.problems.length) return void flash($, 'warn', `${d.problems[0]} Nothing was saved.`)
  const done: string[] = []
  // An array, never a spread next to `$`: the engine reads `$` calls as written.
  const clear = (...ks: string[]) => clearFormKeys($, ks.map(k => key(group, k)))

  if (d.add.length || d.remove.length) {
    const r = await run($, 'saving membership', () => actMembers($, group, d.add, d.remove, false))
    if (!r) return
    if (r.needsDisclosure) {
      // Nothing was saved: the whole draft stays. After the approval the added accounts are
      // members and drop out of it; the rest is saved with the next Save.
      await setDisclosure($, { then: 'members', group, add: d.add, text: r.disclosure ?? '' })
      go($, { tab: 'groups', sub: 'disclosure' })
      return
    }
    if (!r.ok) return void flash($, 'warn', r.message ?? 'Membership refused; nothing else was saved.')
    await clear(...d.add.map(id => `add:${id}`), ...d.remove.map(id => `rm:${id}`))
    done.push('members')
  }
  if (d.threshold !== undefined || d.margin !== undefined) {
    const r = await run($, 'saving the switch point', () => actTuning($, group, d.threshold, d.margin))
    if (!r?.ok) return void (r && flash($, 'warn', r.message ?? 'Threshold and margin refused.'))
    await clear('thr', 'mar')
    done.push('switch point')
  }
  if (d.home !== undefined) {
    const r = await run($, 'saving the home account', () => actPreferred($, group, d.home ?? null))
    if (!r?.ok) return void (r && flash($, 'warn', r.message ?? 'Home refused.'))
    await clear('home')
    done.push(d.home ? `home ${label(s, d.home)}` : 'no home')
  }
  for (const [id, rank] of Object.entries(d.ranks)) {
    const r = await run($, 'saving a rank', () => actRank($, group, id, rank))
    if (!r?.ok) return void (r && flash($, 'warn', r.message ?? 'Rank refused.'))
    await clear(`rank:${id}`)
    done.push(`${label(s, id)} ${rank}`)
  }
  if (d.mode) {
    if (d.mode === 'automatic' && !g.automaticApprovedAt) {
      await clear('mode')
      if (done.length) await flash($, 'info', `Saved: ${done.join(', ')}. Automatic needs its approval.`)
      return go($, { tab: 'groups', sub: 'automatic', arg: group })
    }
    let approval = false
    const r = await run($, 'saving the mode', () => actSetMode($, group, d.mode!, false), undefined, async err => {
      if (!hasCode(err, 'needs_approval')) return false
      approval = true
      return true
    })
    await clear('mode')
    if (approval) return go($, { tab: 'groups', sub: 'automatic', arg: group })
    if (!r?.ok) return void (r && flash($, 'warn', r.message ?? 'Mode refused.'))
    done.push(`${cap(d.mode)} mode`)
  }
  await clearFormKeys($, draftKeys(c, g))
  await flash($, 'info', done.length ? `Saved: ${done.join(', ')}.` : 'Nothing to save.')
}
