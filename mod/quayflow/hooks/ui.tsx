// Shared pieces of the /quay pane: the render context, navigation, form
// fields, the action runner and the design kit. Every handler writes `$.state`
// (never a value captured at draw time) and reports its outcome in one flash line.
//
// The pane reads as a harbor board: facts first, drawn as gauges, badges and
// cards; explanations are hints, shown only when the person turns them on (the
// "?" in the header). Color always repeats what the words already say.
import type { Elements, EngineInterface, RenderSurface } from 'claude-code'

import type { QuayCacheEntry, QuayDisclosure, QuayFlash, QuayForm, QuayLink, QuayReport, QuaySignin, QuayView, StateView } from '../types'
import { clearFlash, clearFormKeys, errorText, flash, setCacheEntry, setFormValue, setView } from './context'
import { gauge, track } from './chart'
import type { Canvas } from './chart'

export type Els = Elements['terminal']

export type Ctx = {
  els: Els
  surface: RenderSurface
  /** Input and Select exist on this surface (every surface but mobile). */
  canType: boolean
  cols: number
  now: number
  link: QuayLink
  s: StateView | null
  view: QuayView
  form: QuayForm
  flash: QuayFlash | null
  signin: QuaySignin | null
  disclosure: QuayDisclosure | null
  report: QuayReport | null
  cache: Record<string, QuayCacheEntry>
  claudeVersion: string
  /** Raster keys drawn so far in this render (each must be unique in the tree). */
  ids: { n: number }
}

export function go($: EngineInterface, view: QuayView): void {
  void clearFlash($)
  void setView($, view)
}

export function setForm($: EngineInterface, key: string, value: string): void {
  void setFormValue($, key, value)
}

export function clearForm($: EngineInterface, ...keys: string[]): Promise<void> {
  return clearFormKeys($, keys)
}

/**
 * Runs one operation from a press. Shows "Working: ..." first (an actual
 * bounded call, no fake progress), then the operation's own words, or the
 * engine's error. `handle` may take an error over (a refusal that opens a
 * review, such as `needs_disclosure`) by answering true. Returns the
 * operation's value, or undefined on error.
 */
export async function run<T>(
  $: EngineInterface,
  doing: string,
  op: () => Promise<T>,
  describe?: (v: T) => string | undefined,
  handle?: (err: unknown) => Promise<boolean>,
): Promise<T | undefined> {
  await flash($, 'info', `Working: ${doing}`)
  try {
    const v = await op()
    const text = describe?.(v)
    if (text) await flash($, 'info', text)
    else await clearFlash($)
    return v
  } catch (err) {
    if (handle && (await handle(err).catch(() => false))) return undefined
    await flash($, 'error', errorText(err))
    return undefined
  }
}

// ------------------------------------------------------------ design kit

export type Tone = 'info' | 'warn' | 'error' | 'ok'

export function toneColor(tone: Tone | undefined): string | undefined {
  return tone === 'error' ? 'error' : tone === 'warn' ? 'warning' : tone === 'ok' ? 'success' : undefined
}

/** Explanations are shown when the person turns them on. */
export function hintsOn(c: Ctx): boolean {
  return c.form['ui:hints'] === 'on'
}

/** An explanatory sentence: hidden until hints are on. */
export function Hint(c: Ctx, text: string): JSX.Element | null {
  if (!hintsOn(c)) return null
  const { Text } = c.els
  return (
    <Text dimColor italic>
      {`ⓘ ${text}`}
    </Text>
  )
}

/** A titled block: an accent mark, then the title. */
export function Section(c: Ctx, title: string, ...children: unknown[]): JSX.Element {
  const { Box, Text } = c.els
  return (
    <Box flexDirection="column" marginTop={1}>
      <Box flexDirection="row">
        <Text color="claude">{'▍'}</Text>
        <Text bold>{title}</Text>
      </Box>
      <Box flexDirection="column" paddingLeft={1}>
        {children as JSX.Element[]}
      </Box>
    </Box>
  )
}

/** A rounded card with an optional title; the border carries the tone. */
export function Card(c: Ctx, title: string | null, tone: Tone | 'accent' | undefined, ...children: unknown[]): JSX.Element {
  const { Box, Text } = c.els
  const color = tone === 'accent' ? 'claude' : toneColor(tone)
  return (
    <Box flexDirection="column" borderStyle="round" borderColor={color} borderDimColor={!color} paddingX={1} marginTop={1}>
      {title ? (
        <Text bold color={color}>
          {title}
        </Text>
      ) : null}
      {children as JSX.Element[]}
    </Box>
  )
}

export function Line(c: Ctx, text: string, opts: { dim?: boolean; tone?: Tone } = {}): JSX.Element {
  const { Text } = c.els
  return (
    <Text dimColor={opts.dim} color={toneColor(opts.tone)}>
      {text}
    </Text>
  )
}

/** A short word on a colored ground: LIVE, HOME, PINNED, SIGN IN. */
export function Badge(c: Ctx, text: string, tone: Tone | 'accent' | 'muted' = 'muted'): JSX.Element {
  const { Text } = c.els
  const bg = tone === 'accent' ? 'claude' : tone === 'muted' ? 'inactive' : toneColor(tone)
  return (
    <Text backgroundColor={bg} color="inverseText" bold>
      {` ${text} `}
    </Text>
  )
}

const EIGHTHS = ['', '▏', '▎', '▍', '▌', '▋', '▊', '▉']

/** Gauge tone: at or past the threshold red, within the margin of it amber, below green. */
export function gaugeTone(p: number, threshold = 85, margin = 10): Tone {
  return p >= threshold ? 'error' : p >= threshold - margin ? 'warn' : 'ok'
}

/**
 * A usage gauge: filled in eighths of a cell, the rest a shaded track with the
 * threshold drawn as a quay line where it is not yet reached. Unknown usage is
 * a dashed track, never an empty gauge.
 */
/** True-color cell graphics exist on this surface (the terminal's Raster). */
export function canPaint(c: Ctx): boolean {
  return c.surface === 'terminal' && !!(c.els as { Raster?: unknown }).Raster
}

/** A canvas as a Raster element, with a key unique in this render. */
export function Paint(c: Ctx, name: string, cv: Canvas): JSX.Element {
  const { Raster } = c.els as unknown as { Raster: (p: { key: string; columns: number; rows: number; cells: string }) => JSX.Element }
  const e = cv.encode()
  return <Raster key={`${name}:${c.ids.n++}`} columns={e.columns} rows={e.rows} cells={e.cells} />
}

export function Gauge(c: Ctx, p: number | null | undefined, width: number, opts: { threshold?: number; margin?: number; stale?: boolean } = {}): JSX.Element {
  const { Box, Text } = c.els
  if (canPaint(c)) return Paint(c, 'gauge', gauge(opts.stale ? undefined : p, width, opts.threshold, opts.margin))
  if (p == null || !Number.isFinite(p)) return <Text dimColor>{'╌'.repeat(width)}</Text>
  const v = Math.max(0, Math.min(100, p))
  const cells = (v / 100) * width
  const full = Math.min(width, Math.floor(cells))
  const part = full < width ? (EIGHTHS[Math.floor((cells - full) * 8)] ?? '') : ''
  const used = full + (part ? 1 : 0)
  const mark = opts.threshold != null ? Math.min(width - 1, Math.floor((opts.threshold / 100) * width)) : -1
  let rest = ''
  for (let i = used; i < width; i++) rest += i === mark ? '│' : '░'
  return (
    <Box flexDirection="row">
      <Text color={opts.stale ? undefined : toneColor(gaugeTone(v, opts.threshold, opts.margin))} dimColor={opts.stale}>
        {'█'.repeat(full) + part}
      </Text>
      <Text dimColor>{rest}</Text>
    </Box>
  )
}

export function Btn(c: Ctx, key: string, labelText: string, onPress: () => void, primary = false): JSX.Element {
  const { Button } = c.els
  return <Button key={key} label={labelText} variant={primary ? 'primary' : undefined} onPress={onPress} />
}

export function Row(c: Ctx, ...children: unknown[]): JSX.Element {
  const { Box } = c.els
  return (
    <Box flexDirection="row" flexWrap="wrap" columnGap={1}>
      {children as JSX.Element[]}
    </Box>
  )
}

/** A text field; falls back to a note on a surface without Input. */
export function Field($: EngineInterface, c: Ctx, key: string, labelText: string, opts: { placeholder?: string; submitLabel?: string; autoFocus?: boolean; initial?: string; onSubmit: (value: string) => void }): JSX.Element {
  if (!c.canType) return Line(c, `${labelText}: type this in a terminal or desktop session (/quay).`, { dim: true })
  const { Input } = c.els
  return (
    <Input
      key={key}
      label={`${labelText}: `}
      placeholder={opts.placeholder}
      value={c.form[key] ?? opts.initial ?? ''}
      submitLabel={opts.submitLabel}
      autoFocus={opts.autoFocus ? true : undefined}
      onInput={(value: string) => setForm($, key, value)}
      onSubmit={(value: string) => {
        setForm($, key, value)
        opts.onSubmit(value)
      }}
    />
  )
}

/** A pick list; falls back to one button per option on a surface without Select. */
export function Pick(c: Ctx, key: string, labelText: string, options: { value: string; label: string }[], value: string | undefined, onSelect: (value: string) => void): JSX.Element {
  if (!options.length) return Line(c, `${labelText}: nothing to choose`, { dim: true })
  if (!c.canType) {
    return Row(c, Line(c, `${labelText}:`), ...options.map(o => Btn(c, `${key}:${o.value}`, o.value === value ? `* ${o.label}` : o.label, () => onSelect(o.value))))
  }
  const { Select } = c.els
  return <Select key={key} label={`${labelText}: `} options={options} value={value ?? options[0]!.value} onSelect={(v: string) => onSelect(v)} />
}

/** A checkbox drawn as a button whose label carries its state. */
export function Toggle(c: Ctx, key: string, labelText: string, on: boolean, onChange: (on: boolean) => void): JSX.Element {
  const { Button } = c.els
  return <Button key={key} plain label={`[${on ? 'x' : ' '}] ${labelText}`} onPress={() => onChange(!on)} />
}

/** A label and its value on one line: the label dim and aligned (design `line(label, value)`). */
export function KV(c: Ctx, labelText: string, value: string, opts: { dim?: boolean; tone?: Tone } = {}): JSX.Element {
  const { Text } = c.els
  return (
    <Text dimColor={opts.dim} color={toneColor(opts.tone)}>
      <Text dimColor>{`${labelText.padEnd(c.cols < 60 ? 14 : 20)} `}</Text>
      {value}
    </Text>
  )
}

/** A notice card: the title carries its tone in words ("Warning:"), the border in color. */
/** `note` is an info notice always shown in full: a rule the person must see before committing. */
export function Alert(c: Ctx, title: string, body: string, tone: Tone | 'note' = 'info'): JSX.Element {
  const { Text } = c.els
  // An info notice explains; with hints off it is its title alone.
  if (tone === 'info' && !hintsOn(c)) return <Text dimColor>{`ⓘ ${title}`}</Text>
  const lead = tone === 'error' ? '✖ Error: ' : tone === 'warn' ? '⚠ Warning: ' : tone === 'ok' ? '✔ ' : ''
  return Card(c, `${lead}${title}`, tone === 'info' || tone === 'note' ? undefined : tone, <Text>{body}</Text>)
}

export type StepMark = 'done' | 'current' | 'todo' | 'failed'

/** An operation timeline: done, current, to do, stopped. No percentages. */
export function Steps(c: Ctx, items: { mark: StepMark; title: string; desc?: string }[]): JSX.Element {
  const { Box, Text } = c.els
  const glyph = (m: StepMark): string => (m === 'done' ? '✔' : m === 'current' ? '▸' : m === 'failed' ? '✖' : '·')
  const color = (m: StepMark): string | undefined => (m === 'done' ? 'success' : m === 'current' ? 'claude' : m === 'failed' ? 'warning' : undefined)
  return (
    <Box flexDirection="column">
      {canPaint(c) && items.length > 1 ? <Box marginBottom={1}>{Paint(c, 'track', track(items.map(i => i.mark), Math.max(3, Math.min(10, Math.floor((c.cols - 4) / Math.max(1, items.length - 1))))))}</Box> : null}
      {items.map((i, n) => (
        <Box key={`step:${n}`} flexDirection="row" columnGap={1}>
          <Text color={color(i.mark)} dimColor={i.mark === 'todo'}>
            {glyph(i.mark)}
          </Text>
          <Text bold={i.mark === 'current'} dimColor={i.mark === 'todo'} color={i.mark === 'failed' ? 'warning' : undefined}>
            {`${i.title}${i.desc ? ` — ${i.desc}` : ''}`}
          </Text>
        </Box>
      ))}
    </Box>
  )
}

/**
 * A table of text cells. Columns share the pane's width; on a narrow pane each row
 * becomes "header: value" lines so no cell is cut.
 */
export function Table(c: Ctx, key: string, headers: string[], rows: string[][]): JSX.Element {
  const { Box, Text } = c.els
  if (c.cols < 60) {
    return (
      <Box flexDirection="column">
        {rows.map((r, i) => (
          <Box key={`${key}:${i}`} flexDirection="column" marginTop={i ? 1 : 0}>
            {r.map((cell, j) => Line(c, `${headers[j] ?? ''}: ${cell}`, { dim: j > 0 }))}
          </Box>
        ))}
      </Box>
    )
  }
  const n = headers.length
  const widths = headers.map((head, j) => Math.max(head.length, ...rows.map(r => (r[j] ?? '').length)))
  const total = Math.max(20, c.cols - n)
  const first = Math.min(widths[0] ?? 10, Math.floor(total * 0.4))
  const rest = n > 1 ? Math.floor((total - first) / (n - 1)) : 0
  const w = (j: number): number => (j === 0 ? first : Math.min(rest, Math.max(widths[j] ?? 1, 1)) || rest)
  return (
    <Box flexDirection="column">
      <Box flexDirection="row" columnGap={1}>
        {headers.map((head, j) => (
          <Box key={`${key}:h${j}`} width={j === n - 1 ? undefined : w(j)}>
            <Text dimColor bold>
              {head}
            </Text>
          </Box>
        ))}
      </Box>
      {rows.map((r, i) => (
        <Box key={`${key}:${i}`} flexDirection="row" columnGap={1}>
          {r.map((cell, j) => (
            <Box key={`${key}:${i}:${j}`} width={j === n - 1 ? undefined : w(j)} flexShrink={j === n - 1 ? 1 : 0}>
              <Text>{cell}</Text>
            </Box>
          ))}
        </Box>
      ))}
    </Box>
  )
}

const loading = new Set<string>()

/**
 * An engine read for a review screen. Drawing asks for it once per `key` (a new key,
 * such as a newer state sequence, reads again); the answer lands in the pane's cache
 * and redraws. Until then the previous answer of the slot is shown when `keepStale`.
 */
export function useRead<T>($: EngineInterface, c: Ctx, slot: string, key: string, load: () => Promise<T>, keepStale = false): { value?: T; error?: string; pending: boolean } {
  const entry = c.cache[slot]
  const current = entry?.key === key
  if (!current && !loading.has(`${slot}\n${key}`)) {
    const id = `${slot}\n${key}`
    loading.add(id)
    void load()
      .then(
        value => setCacheEntry($, slot, { key, at: Date.now(), value }),
        // A read cancelled because the pane was drawn again is not an answer: nothing is
        // cached, so the next draw asks again.
        err => (/superseded/i.test(errorText(err)) ? undefined : setCacheEntry($, slot, { key, at: Date.now(), error: errorText(err) })),
      )
      .finally(() => loading.delete(id))
  }
  if (current) return { value: entry.value as T | undefined, error: entry.error, pending: false }
  return { value: keepStale ? (entry?.value as T | undefined) : undefined, pending: true }
}

/** Forgets a cached read so the next draw asks again. */
export function forget($: EngineInterface, slot: string): Promise<void> {
  return setCacheEntry($, slot, null)
}
