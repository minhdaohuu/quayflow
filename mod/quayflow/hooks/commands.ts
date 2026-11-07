// `/quay <subcommand>`: keyboard-first and headless-safe control (T42, build
// plan §3.4). Text in, text out; a question is asked only in an interactive
// session, and an approval that needs one is refused in a headless run unless
// typed explicitly (`--approve`, `--accept-disclosure`).
import type { EngineInterface } from 'claude-code'

import type { Mode, StateView } from '../types'
import { MOD_VERSION, Q, errorText, setView } from './context'
import { awaitLink, establish, fetchState, hasCode } from './engine'
import {
  account,
  adoptionWords,
  findAccount,
  findGroup,
  healthText,
  isTerminal,
  label,
  linkOffText,
  myEnv,
  myGroup,
  persistentWarnings,
  sessionName,
  transitionWords,
  usageText,
  when,
} from './format'
import { liveAccountHash } from './identity'
import { DORG_ON_TEXT } from './words'
import { actReportPreview, actCanary, actMigratePoc, actPoll, actAccept, actAssign, actEngine, actGroupCreate, actGroupDelete, actHandoff, actIgnoreUnknown, actLaneCheck, actLaneCreate, actLaneDelete, actLanes, actMembers, actPreferred, actRank, actRegister, actReport, actSetDefault, actSetMode, actSwitchTo, actTuning, actUnassign, actUnpin, handoffOutcome, switchOutcome } from './actions'
import { ensureWatcher, paintStatus, refresh } from './watcher'

export const HELP = [
  '/quay                         open the Quayflow pane (interactive) or print status',
  '/quay status                  accounts, usage, recommendation and running sessions',
  '/quay accept                  accept the current suggestion',
  '/quay use <account>           switch this environment to an account (pins it)',
  '/quay unpin                   end a manual pin',
  '/quay mode <manual|suggest|automatic> [group] [--approve]',
  '/quay group [list] | create <G> | delete <G> | add <G> <acct..> | remove <G> <acct..>',
  '            | preferred <G> <acct|none> | rank <G> <acct> <preferred|normal|backup>',
  '            | default <G> | tuning <G> [threshold] [margin] | assign <G> | unassign',
  '/quay pause | resume          pause or resume automatic switching and recovery',
  '/quay poll                    refresh usage of every account now',
  '/quay register [label] [group] [--accept-disclosure]  register the current login',
  '/quay add                     add another account (native sign-in)',
  '/quay lanes [create|delete|check <name>]',
  '/quay replace <lane>          move this session into a lane (same terminal, PowerShell hook)',
  '/quay open <lane>             open this session in a lane in a new window',
  '/quay report [--save]         preview the local support report; --save saves it after review',
  '/quay whoami                  which account this session uses now',
  '/quay canary                  live version canary status',
  '/quay migrate-poc [group]     import the PoC vault (once)',
  '/quay start                   start the engine (after an intentional stop)',
  '/quay help',
].join('\n')

export function statusReport(s: StateView, now: number): string {
  const env = myEnv(s)
  const lines: string[] = []
  const g = myGroup(s)
  lines.push(`Quayflow ${MOD_VERSION} · engine ${s.engine.version} (protocol ${s.engine.protocol}) · ${s.engine.integrity}`)
  if (!env) return [...lines, 'No credential environment is known yet.'].join('\n')
  const verified = s.session?.verifiedAccount
  lines.push(
    `This session: ${verified ? `last request verified on "${label(s, verified)}"${s.session?.verifiedAt ? ` at ${when(s.session.verifiedAt)}` : ''}` : 'no verified request yet'} · environment ${env.id} · group ${g} (${s.session?.groupSource === 'assigned' ? 'project assignment' : 'default'}) · mode ${env.mode}${env.automationPaused ? ' (automation paused)' : ''}`,
  )
  const live = env.live
  lines.push(`Stored login of ${env.id}: ${live.signedOut ? 'signed out' : live.known ? `"${label(s, live.account)}"` : 'unknown login (not registered)'}`)
  lines.push('Accounts:')
  for (const a of s.accounts) {
    const mark = a.liveIn.includes(env.id) ? '>' : ' '
    const elig = a.eligibility ? (a.eligibility.eligible ? 'eligible' : `not eligible: ${a.eligibility.reason}`) : ''
    const health = a.health === 'ok' ? '' : ` · ${healthText(a.health)}`
    lines.push(`  ${mark} ${a.label.padEnd(14)} ${usageText(a, true).padEnd(34)} ${a.groups.join(', ')}${elig ? ` · ${elig}` : ''}${health}`)
  }
  if (!s.accounts.length) lines.push('  (none registered: /quay register)')
  lines.push(`Decision: ${env.decision}`)
  if (env.suggestion) lines.push(`Recommendation: switch to "${label(s, env.suggestion.to)}" (${env.suggestion.reason}) — /quay accept`)
  if (env.pin) lines.push(`Pinned: "${label(s, env.pin.account)}" until ${when(env.pin.until)} — /quay unpin`)
  const t = env.transition
  if (t && !isTerminal(t)) lines.push(`Transition to "${label(s, t.to)}": ${transitionWords(t.state)}`)
  if (env.adoption.length) {
    lines.push('Adoption per client (stored destination vs last request identity):')
    for (const c of env.adoption) lines.push(`  pid ${c.pid}: ${adoptionWords(c.state)}${c.lastRequestAccount ? ` (last request: ${label(s, c.lastRequestAccount)})` : ''}`)
  }
  lines.push(`Running sessions of ${env.id} (other clients, such as editor extensions, may not be listed):`)
  for (const x of env.sessions) {
    lines.push(`  ${sessionName(x)}${x.sessionId === Q.sessionId ? ' [this session]' : ''} · ${x.status ?? '?'} · group ${x.group ?? '?'}${x.version ? ` · Claude ${x.version}` : ''}`)
  }
  if (!env.sessions.length) lines.push('  none listed')
  const warnings = persistentWarnings(s, env)
  if (warnings.length) lines.push(`Warnings: ${warnings.join(' · ')}`)
  for (const n of env.notices) lines.push(`Notice (${n.level}): ${n.text}`)
  if (s.dOrg.enabled) lines.push(`${DORG_ON_TEXT} Turn it off in /quay > Settings.`)
  void now
  return lines.join('\n')
}

function parseArgs(args: string): string[] {
  const out: string[] = []
  const re = /"([^"]*)"|'([^']*)'|(\S+)/g
  let m: RegExpExecArray | null
  while ((m = re.exec(args))) out.push(m[1] ?? m[2] ?? m[3] ?? '')
  return out
}

function resolveAccount(s: StateView, ref: string): string {
  return findAccount(s, ref)?.id ?? ref
}

function automaticQuestion(group: string): string {
  return `Approve Automatic for group "${group}"? Quayflow will then switch accounts of this group without asking, also in unattended headless jobs, within the group's policy.`
}

function notApproved(group: string): string {
  return Q.headless ? `Automatic needs explicit approval: /quay mode automatic ${group} --approve` : 'Automatic was not approved; the mode is unchanged.'
}

async function confirm($: EngineInterface, question: string, yes: string): Promise<boolean> {
  if (Q.headless) return false
  try {
    const answer = await $.ui.ask(question, [yes, 'Cancel'])
    return answer === yes
  } catch {
    return false
  }
}

async function needState($: EngineInterface): Promise<StateView> {
  return Q.state ?? (await fetchState($))
}

/** Runs one `/quay` subcommand and returns its text. Pane opening is handled by the caller. */
const KNOWN_FLAGS = ['--approve', '--accept-disclosure', '--idle-wait', '--save']

export async function runCommand($: EngineInterface, argsText: string): Promise<string> {
  const argv = parseArgs(argsText)
  const flags = new Set(argv.filter(a => a.startsWith('--')))
  const words = argv.filter(a => !a.startsWith('--'))
  const verb = (words[0] ?? 'status').toLowerCase()
  const rest = words.slice(1)

  if (verb === 'help' || verb === '?') return HELP
  // A mistyped option is refused, never ignored: "--aprove" must not read as no approval.
  const unknown = [...flags].filter(f => !KNOWN_FLAGS.includes(f))
  if (unknown.length) {
    const near = (f: string) => KNOWN_FLAGS.find(k => k[2] === f[2] && Math.abs(k.length - f.length) <= 2)
    return unknown.map(f => `Quayflow: unknown option ${f}${near(f) ? ` (did you mean ${near(f)}?)` : ''}.`).join('\n')
  }

  if (verb === 'start') {
    await establish($, { allowStart: true, explicit: true })
    await refresh($)
    return Q.link.on ? `Quayflow engine is running (${Q.link.engineVersion ?? '?'}).` : `Quayflow: ${linkOffText(Q.link)}.`
  }

  // A handshake under way (or one this module instance never started) is waited for
  // before the session is reported as off.
  if (!Q.link.on) await reconnect($)
  if (!Q.link.on) {
    await paintStatus($)
    return `Quayflow is off: ${linkOffText(Q.link)}${Q.link.message && Q.link.reason !== 'start_failed' ? ` (${Q.link.message})` : ''}. Claude works as usual.`
  }

  try {
    const s = await needState($)
    switch (verb) {
      case 'status': {
        const fresh = (await refresh($)) ?? s
        return statusReport(fresh, Date.now())
      }
      case 'accept': {
        const env = myEnv(s)
        if (!env?.suggestion) return 'Quayflow: there is no suggestion to accept right now.'
        const r = await actAccept($)
        return switchOutcome(Q.state, r, false)
      }
      case 'use':
      case 'switch': {
        if (!rest[0]) return 'usage: /quay use <account> [--idle-wait]'
        const r = await actSwitchTo($, resolveAccount(s, rest.join(' ')), flags.has('--idle-wait'))
        return switchOutcome(Q.state, r, true)
      }
      case 'unpin': {
        const r = await actUnpin($)
        return r.ok ? 'Quayflow: pin cleared; suggestions and automatic switching resume under the group policy.' : `Quayflow: ${r.message ?? 'nothing pinned'}.`
      }
      case 'mode': {
        const mode = (rest[0] ?? '').toLowerCase().replace(/^auto$/, 'automatic') as Mode
        if (!['manual', 'suggest', 'automatic'].includes(mode)) return 'usage: /quay mode <manual|suggest|automatic> [group] [--approve]'
        const g = findGroup(s, rest[1] ?? myGroup(s))
        if (!g) return `Quayflow: no group named "${rest[1] ?? myGroup(s)}".`
        // Automatic is approved once per group; the engine refuses it unapproved (409 needs_approval).
        let approve = false
        if (mode === 'automatic' && !g.automaticApprovedAt) {
          approve = flags.has('--approve') || (await confirm($, automaticQuestion(g.name), 'Approve Automatic'))
          if (!approve) return notApproved(g.name)
        }
        try {
          const r = await actSetMode($, g.name, mode, approve)
          return r.ok ? `Quayflow: group "${g.name}" is now in ${mode} mode.` : `Quayflow: ${r.message ?? 'refused'}.`
        } catch (err) {
          if (!hasCode(err, 'needs_approval')) throw err
          if (!(flags.has('--approve') || (await confirm($, automaticQuestion(g.name), 'Approve Automatic')))) return notApproved(g.name)
          await actSetMode($, g.name, mode, true)
          return `Quayflow: group "${g.name}" is now in automatic mode.`
        }
      }
      case 'group':
      case 'groups':
        return await groupCommand($, s, rest, flags)
      case 'pause':
      case 'resume': {
        const r = await actEngine($, verb)
        if (!r.ok) return `Quayflow: ${r.message ?? 'refused'}.`
        if (verb === 'pause') return 'Quayflow: automatic switching and recovery are paused. Manual switches still work. /quay resume to resume.'
        // Resume ends a pause by you or a hold; another cause keeps automation paused.
        const after = Q.state ? myEnv(Q.state) : undefined
        return after?.automationPaused ? `Quayflow: still paused: ${after.pausedReason ?? 'another cause'}. /quay shows its remedy.` : 'Quayflow: automation resumed.'
      }
      case 'register': {
        if (!rest.length && !Q.headless) return 'pane:register'
        const labelText = rest[0]
        const group = findGroup(s, rest[1] ?? s.defaultGroup)?.name
        if (!labelText) return 'usage: /quay register <label> [group] [--accept-disclosure]'
        if (!group) return `Quayflow: no group named "${rest[1]}". Create it first: /quay group create ${rest[1]}`
        if (!s.consents.credentialStorage) {
          return "Registering stores a reusable copy of this login in Quayflow's protected vault, which needs your consent first: open /quay (interactive) or run quayflow consent credential-storage on."
        }
        const saved = (a: { label: string; id: string; kind: string; subscription?: string | null; note?: string }): string =>
          `Saved "${a.label}" (account #${a.id}, ${a.kind}${a.subscription ? `, ${a.subscription}` : ''}) to the vault in group ${group}. Nothing was switched.${a.note ? ` ${a.note}` : ''}`
        try {
          return saved(await actRegister($, labelText, group, flags.has('--accept-disclosure')))
        } catch (err) {
          if (!hasCode(err, 'needs_disclosure')) throw err
          const accepted = await confirm($, `${err.message}\nApprove this group's account set and register?`, 'Approve group')
          if (!accepted) {
            return Q.headless
              ? `${err.message}\nNot saved. To accept: /quay register ${labelText} ${group} --accept-disclosure`
              : 'Not saved: the disclosure was not accepted.'
          }
          return saved(await actRegister($, labelText, group, true))
        }
      }
      case 'add':
        if (!Q.headless) return 'pane:add'
        return 'Adding an account needs a native sign-in: run /quay add in an interactive session, or quayflow add --label <L> --group <G>.'
      case 'lanes':
      case 'lane':
        return await laneCommand($, rest)
      case 'replace':
      case 'open': {
        const lane = rest[0]
        if (!lane) return `usage: /quay ${verb} <lane>`
        const what =
          verb === 'open'
            ? `Open this session in lane "${lane}" with its full history, in a new window? It continues there once this session ends; keep only one of the two working on the task.`
            : `Replace this session with one in lane "${lane}" with its full history? After /exit, the PowerShell prompt hook resumes it in this terminal; other shells get the exact command.`
        if (!Q.headless && !(await confirm($, what, verb === 'open' ? 'Open with full history' : 'Replace with full history'))) return 'Not moved: nothing was prepared. /quay > Session actions shows the full review.'
        const r = await actHandoff($, verb, lane)
        return handoffOutcome(verb, lane, r)
      }
      case 'report': {
        // Saved only when asked after a review: the preview itself writes nothing.
        if (flags.has('--save')) {
          const r = await actReport($)
          return `Support report saved locally (nothing is uploaded): ${r.path}\n\n${r.preview}`
        }
        const p = await actReportPreview($)
        return `Support report preview (not saved; nothing is uploaded). Review it, then save with /quay report --save or in /quay > Activity:\n\n${p.text}`
      }
      case 'whoami': {
        const actual = await liveAccountHash($).catch(() => undefined)
        const a = account(s, actual)
        const env = myEnv(s)
        return `This session's credential: ${actual ? `account #${actual}` : 'unknown (no first-party login)'} = ${a ? `"${a.label}"` : 'not registered'}. Stored login of ${env?.id ?? '?'}: ${env?.live.known ? `"${label(s, env.live.account)}"` : 'unknown'}.`
      }
      case 'poll': {
        const r = await actPoll($)
        const fresh = Q.state ?? s
        return r.ok ? `Usage refreshed.\n${statusReport(fresh, Date.now())}` : `Quayflow: ${r.message ?? 'refused'}.`
      }
      case 'migrate-poc': {
        const group = findGroup(s, rest[0] ?? s.defaultGroup)?.name
        if (!group) return `Quayflow: no group named "${rest[0]}".`
        try {
          const r = await actMigratePoc($, group, flags.has('--accept-disclosure'))
          return r.message ?? (r.ok ? 'PoC accounts imported.' : 'Nothing imported.')
        } catch (err) {
          if (!hasCode(err, 'needs_disclosure')) throw err
          if (!(await confirm($, `${err.message}\nApprove this group's account set and import?`, 'Approve group'))) {
            return `${err.message}\nNot imported. To accept: /quay migrate-poc ${group} --accept-disclosure`
          }
          const r = await actMigratePoc($, group, true)
          return r.message ?? 'PoC accounts imported.'
        }
      }
      case 'canary': {
        const c = await actCanary($)
        if (!c) return 'The live canary is not recording. Start it in a terminal: quayflow canary --live (needs the live-canary consent).'
        return `Live canary recording since ${when(c.since)}: ${c.steps.length} request(s) verified so far. Each request reports the account it used.`
      }
      case 'ignore': {
        const r = await actIgnoreUnknown($)
        return r.ok ? 'Quayflow: noted. Automation stays paused while this unknown login is live; register it to resume.' : `Quayflow: ${r.message ?? 'refused'}.`
      }
      default:
        return `Unknown subcommand "${verb}".\n${HELP}`
    }
  } catch (err) {
    return `Quayflow: ${errorText(err)}`
  }
}

async function groupCommand($: EngineInterface, s: StateView, rest: string[], flags: Set<string>): Promise<string> {
  const sub = (rest[0] ?? 'list').toLowerCase()
  const name = rest[1]
  const need = (n: string | undefined): string | undefined => (n && findGroup(s, n) ? findGroup(s, n)!.name : undefined)
  switch (sub) {
    case 'list': {
      const lines = s.groups.map(g => {
        const members = g.accounts.map(id => `${label(s, id)}${g.preferred === id ? ' (home)' : ''}`).join(', ') || 'no accounts'
        return `${g.isDefault ? '*' : ' '} ${g.name}: ${g.mode}${g.mode === 'automatic' && !g.automaticApprovedAt ? ' (not approved)' : ''} · threshold ${g.threshold}% margin ${g.margin} · ${members}${g.mixed ? ` · mixed identities${g.mixedCovered ? ' (disclosure accepted)' : ' (disclosure needed)'}` : ''}`
      })
      const projects = s.projects.map(p => `  ${p.path} -> ${p.group}`)
      return [
        'Groups (* = default for projects without an assignment):',
        ...lines,
        `This project resolves to: ${myGroup(s)}`,
        ...(projects.length ? ['Project assignments:', ...projects] : []),
      ].join('\n')
    }
    case 'create':
    case 'delete': {
      if (!name) return `usage: /quay group ${sub} <name>`
      const r = sub === 'create' ? await actGroupCreate($, name) : await actGroupDelete($, need(name) ?? name)
      return r.ok ? `Quayflow: group "${name}" ${sub === 'create' ? 'created' : 'deleted'}.` : `Quayflow: ${r.message ?? 'refused'}.`
    }
    case 'add':
    case 'remove': {
      const g = need(name)
      const accounts = rest.slice(2).map(r => resolveAccount(s, r))
      if (!g || !accounts.length) return `usage: /quay group ${sub} <group> <account..>`
      const add = sub === 'add' ? accounts : []
      const remove = sub === 'remove' ? accounts : []
      let r = await actMembers($, g, add, remove, false)
      if (r.needsDisclosure) {
        const text = r.disclosure ?? `Group "${g}" would then mix identities: work in projects using this group may be sent to any of its accounts. Your organization's rules on where its work may be processed remain your responsibility.`
        const accepted =
          flags.has('--accept-disclosure') || (await confirm($, `${text}\nApprove this group's account set?`, 'Approve group'))
        if (!accepted) {
          return Q.headless ? `${text}\nNot saved. To accept: /quay group add ${g} ${rest.slice(2).join(' ')} --accept-disclosure` : 'Not saved: the disclosure was not accepted.'
        }
        r = await actMembers($, g, add, remove, true)
      }
      return r.ok ? `Quayflow: group "${g}" membership updated.` : `Quayflow: ${r.message ?? 'refused'}.`
    }
    case 'preferred':
    case 'home': {
      const g = need(name)
      const who = rest[2]
      if (!g || !who) return 'usage: /quay group preferred <group> <account|none>'
      const r = await actPreferred($, g, who.toLowerCase() === 'none' ? null : resolveAccount(s, who))
      return r.ok ? `Quayflow: home of "${g}" set to ${who.toLowerCase() === 'none' ? 'none' : `"${label(s, resolveAccount(s, who))}"`}.` : `Quayflow: ${r.message ?? 'refused'}.`
    }
    case 'rank': {
      const g = need(name)
      const who = rest[2]
      const rank = (rest[3] ?? '').toLowerCase()
      if (!g || !who || !['preferred', 'normal', 'backup'].includes(rank)) return 'usage: /quay group rank <group> <account> <preferred|normal|backup>'
      const r = await actRank($, g, resolveAccount(s, who), rank)
      return r.ok ? `Quayflow: "${label(s, resolveAccount(s, who))}" ranked ${rank} in "${g}".` : `Quayflow: ${r.message ?? 'refused'}.`
    }
    case 'default': {
      const g = need(name)
      if (!g) return 'usage: /quay group default <group>'
      const r = await actSetDefault($, g)
      return r.ok ? `Quayflow: "${g}" is now the default group for projects without an assignment.` : `Quayflow: ${r.message ?? 'refused'}.`
    }
    case 'tuning': {
      const g = need(name)
      const threshold = rest[2] !== undefined ? Number(rest[2]) : undefined
      const margin = rest[3] !== undefined ? Number(rest[3]) : undefined
      if (!g || (threshold !== undefined && !Number.isFinite(threshold)) || (margin !== undefined && !Number.isFinite(margin))) {
        return 'usage: /quay group tuning <group> [threshold%] [margin]'
      }
      const r = await actTuning($, g, threshold, margin)
      return r.ok ? `Quayflow: "${g}" tuning saved.` : `Quayflow: ${r.message ?? 'refused'}.`
    }
    case 'assign': {
      const g = need(name)
      if (!g) return 'usage: /quay group assign <group>  (assigns this project folder)'
      const r = await actAssign($, Q.cwd, g)
      return r.ok ? `Quayflow: ${Q.cwd} now uses group "${g}". Running jobs keep the destinations they started with.` : `Quayflow: ${r.message ?? 'refused'}.`
    }
    case 'unassign': {
      const r = await actUnassign($, Q.cwd)
      return r.ok ? `Quayflow: ${Q.cwd} has no assignment now and uses the default group "${s.defaultGroup}".` : `Quayflow: ${r.message ?? 'refused'}.`
    }
    default:
      return `Unknown group subcommand "${sub}".\n${HELP}`
  }
}

async function laneCommand($: EngineInterface, rest: string[]): Promise<string> {
  const sub = (rest[0] ?? 'list').toLowerCase()
  const name = rest[1]
  if (sub === 'list') {
    const lanes = await actLanes($)
    if (!lanes.length) return 'No lanes. A lane is a separate credential environment for sessions that need a different account: /quay lanes create <name>.'
    return [
      'Lanes (each has its own login and session folder, and links your shared Claude setup):',
      ...lanes.map(
        l =>
          `  ${l.name}: ${l.live ? `login "${label(Q.state, l.live)}"` : 'no login yet'} · ${l.sessions} session(s) listed · links ${l.linksOk ? 'ok' : `need repair (${l.linkDetail.join('; ')})`} · in-place switching ${l.canaryPassed ? 'enabled (canary passed)' : 'off until its canary passes'}`,
      ),
    ].join('\n')
  }
  if (!name) return `usage: /quay lanes ${sub} <name>`
  if (sub === 'create') {
    const l = await actLaneCreate($, name)
    return `Lane "${l.name}" created at ${l.configDir}; links ${l.linksOk ? 'ok' : `need repair (${l.linkDetail.join('; ')})`}. Move a session into it with /quay replace ${l.name} or /quay open ${l.name}.`
  }
  if (sub === 'delete' || sub === 'check') {
    const r = sub === 'delete' ? await actLaneDelete($, name) : await actLaneCheck($, name)
    return r.ok ? `Lane "${name}": ${r.message ?? (sub === 'delete' ? 'deleted' : 'checked')}.` : `Quayflow: ${r.message ?? 'refused'}.`
  }
  return `Unknown lanes subcommand "${sub}".`
}

/** `/quay` with no arguments: interactive opens the pane, headless prints status. */
/** Brings a "connecting" link up (never starts the engine), then draws and follows it. */
export async function reconnect($: EngineInterface): Promise<void> {
  if (!(await awaitLink($).catch(() => false))) return
  await refresh($).catch(() => undefined)
  ensureWatcher($)
}

export async function openPane($: EngineInterface, tab = 'overview', sub?: string): Promise<boolean> {
  if (Q.headless) return false
  await setView($, sub ? { tab, sub } : { tab })
  if (!Q.link.on) void reconnect($)
  const r = await $.ui.open({ id: 'quay', title: 'Quayflow', focus: true }).catch(() => ({ isPlaced: false }))
  return r.isPlaced
}
