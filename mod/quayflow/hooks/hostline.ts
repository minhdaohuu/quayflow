// Whether Claude Code's own status line already shows the usage limits, so
// the Quayflow line does not repeat them. Read-only and cheap: the merged
// `statusLine` setting (user, project, local, flag and managed settings, in
// Claude Code's precedence) and, when its command names a script file, that
// file's text. The command is never run. The answer is cached for a few
// minutes and checked again when the setting's command changes.
import type { EngineInterface } from 'claude-code'

import { Q } from './context'

/** Text that marks a status line showing the 5-hour / 7-day limits. */
const USAGE_MARK = /rate_?limits|five_hour|seven_day|percent_?used|used_percentage|\b5h\b|\b7d\b/i
/** A command token that names a script file worth reading. */
const SCRIPT = /\.(sh|bash|zsh|js|mjs|cjs|ts|mts|py|ps1|psm1|cmd|bat|rb|pl|lua)$/i
const RECHECK_MS = 3 * 60_000
const MAX_FILES = 3
const MAX_TEXT = 256 * 1024

let cache: { command: string; shows: boolean; at: number } | undefined

/** Forgets the cached answer (tests, and a module reload starts without one anyway). */
export function resetHostLineCache(): void {
  cache = undefined
}

/** True when the configured status line text, or a script it runs, mentions the usage limits. */
export async function hostLineShowsUsage($: EngineInterface): Promise<boolean> {
  const setting = await $.settings.read({ key: 'statusLine' }).catch(() => undefined)
  const command = commandOf(setting)
  const now = Date.now()
  if (cache && cache.command === command && now - cache.at < RECHECK_MS) return cache.shows
  const shows = command ? await detect($, command) : false
  cache = { command, shows, at: now }
  return shows
}

/** Whether the Quayflow status line should show usage figures, after the `statusLineUsage` option. */
export async function showUsageFigures($: EngineInterface): Promise<boolean> {
  if (Q.usageDisplay === 'show') return true
  if (Q.usageDisplay === 'hide') return false
  return !(await hostLineShowsUsage($).catch(() => false))
}

function commandOf(setting: unknown): string {
  if (!setting || typeof setting !== 'object') return ''
  const c = (setting as { command?: unknown }).command
  return typeof c === 'string' ? c.trim() : ''
}

async function detect($: EngineInterface, command: string): Promise<boolean> {
  if (USAGE_MARK.test(command)) return true
  const paths = await scriptPaths($, command)
  for (const p of paths.slice(0, MAX_FILES)) {
    const text = await $.fs.read(p).catch(() => undefined)
    if (typeof text === 'string' && USAGE_MARK.test(text.slice(0, MAX_TEXT))) return true
  }
  return false
}

/** The script files a status line command names, with `~` and environment variables expanded. */
export async function scriptPaths($: EngineInterface, command: string): Promise<string[]> {
  const tokens = command.match(/"[^"]+"|'[^']+'|[^\s"']+/g) ?? []
  const out: string[] = []
  for (const raw of tokens) {
    const t = raw.replace(/^["']|["']$/g, '')
    if (!SCRIPT.test(t)) continue
    let p = await expand($, t)
    if (!p) continue
    if (!/^([A-Za-z]:[\\/]|[\\/])/.test(p) && Q.cwd) p = `${Q.cwd.replace(/[\\/]+$/, '')}\\${p}`
    out.push(p)
  }
  return [...new Set(out)]
}

/** The few variables a status line path commonly starts from; each read by its literal name. */
async function envVar($: EngineInterface, name: string): Promise<string | undefined> {
  const get = (p: Promise<string | undefined>): Promise<string | undefined> => p.then(v => v || undefined).catch(() => undefined)
  switch (name.toUpperCase()) {
    case 'HOME':
      return (await get($.env.get('HOME'))) ?? (await get($.env.get('USERPROFILE')))
    case 'USERPROFILE':
      return get($.env.get('USERPROFILE'))
    case 'APPDATA':
      return get($.env.get('APPDATA'))
    case 'LOCALAPPDATA':
      return get($.env.get('LOCALAPPDATA'))
    case 'CLAUDE_CONFIG_DIR':
      return get($.env.get('CLAUDE_CONFIG_DIR'))
    default:
      return undefined
  }
}

async function expand($: EngineInterface, path: string): Promise<string | undefined> {
  let p = path
  if (p === '~' || p.startsWith('~/') || p.startsWith('~\\')) {
    const home = await envVar($, 'USERPROFILE') ?? (await envVar($, 'HOME'))
    if (!home) return undefined
    p = home + p.slice(1)
  }
  // $VAR, ${VAR}, $env:VAR and %VAR%; an unknown or unset variable leaves the path unreadable.
  const names = [...p.matchAll(/\$env:(\w+)|\$\{(\w+)\}|\$(\w+)|%(\w+)%/g)]
  for (const m of names) {
    const value = await envVar($, m[1] ?? m[2] ?? m[3] ?? m[4] ?? '')
    if (!value) return undefined
    p = p.replace(m[0], value)
  }
  return p
}
