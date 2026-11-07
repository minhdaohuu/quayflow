// Cell graphics for the terminal's Raster element: true-color usage gauges, the fleet
// chart of every account's windows, and the step track of a switch. Pure functions:
// each returns the cells (code point, foreground, background) and their size; the
// pane wraps them in a Raster on the terminal and draws text gauges elsewhere.
//
// Color only repeats what the text beside each graphic already says (the figure, the
// account name, the step title), so nothing is told by color or shape alone.

/** The terminal's own default color (foreground or background). */
export const DEFAULT = 0x01000000
export const GREEN = 0x3fb950
export const AMBER = 0xd29922
export const RED = 0xf85149
export const ACCENT = 0xd97757
export const TRACK = 0x6e7681
export const MUTED = 0x8b949e

export type Cells = { columns: number; rows: number; cells: string }

/** A grid of cells, row-major, every cell a width-1 character with two colors. */
export class Canvas {
  readonly words: Uint32Array
  constructor(
    readonly columns: number,
    readonly rows: number,
  ) {
    this.words = new Uint32Array(columns * rows * 3)
    for (let i = 0; i < columns * rows; i++) this.words.set([0x20, DEFAULT, DEFAULT], i * 3)
  }
  put(x: number, y: number, ch: string, fg = DEFAULT, bg = DEFAULT): void {
    if (x < 0 || y < 0 || x >= this.columns || y >= this.rows) return
    const cp = ch.codePointAt(0) ?? 0x20
    this.words.set([cp > 0xffff || cp < 0x20 ? 0x20 : cp, fg, bg], (y * this.columns + x) * 3)
  }
  text(x: number, y: number, s: string, fg = DEFAULT): void {
    let i = 0
    for (const ch of s) this.put(x + i++, y, ch, fg)
  }
  get(x: number, y: number): string {
    return String.fromCodePoint(this.words[(y * this.columns + x) * 3] ?? 0x20)
  }
  /** One text line per row: what a reader without color sees (tests, previews). */
  lines(): string[] {
    const out: string[] = []
    for (let y = 0; y < this.rows; y++) {
      let l = ''
      for (let x = 0; x < this.columns; x++) l += this.get(x, y)
      out.push(l)
    }
    return out
  }
  encode(): Cells {
    return { columns: this.columns, rows: this.rows, cells: base64(new Uint8Array(this.words.buffer)) }
  }
}

const B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'

/** Standard padded base64 (little-endian words, as the host reads them). */
export function base64(bytes: Uint8Array): string {
  let out = ''
  for (let i = 0; i < bytes.length; i += 3) {
    const a = bytes[i]!
    const b = bytes[i + 1]
    const c = bytes[i + 2]
    const n = (a << 16) | ((b ?? 0) << 8) | (c ?? 0)
    out += B64[(n >> 18) & 63]! + B64[(n >> 12) & 63]! + (b === undefined ? '=' : B64[(n >> 6) & 63]!) + (c === undefined ? '=' : B64[n & 63]!)
  }
  return out
}

function mix(a: number, b: number, t: number): number {
  const k = Math.max(0, Math.min(1, t))
  const ch = (s: number): number => Math.round(((a >> s) & 255) * (1 - k) + ((b >> s) & 255) * k)
  return (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

/** Usage color: green, warming to amber at the margin below the threshold, red from it. */
export function heat(p: number, threshold = 85, margin = 10): number {
  const calm = Math.max(1, threshold - margin)
  if (p >= threshold) return RED
  if (p < calm) return mix(GREEN, AMBER, Math.pow(p / calm, 2))
  return mix(AMBER, RED, (p - calm) / Math.max(1, threshold - calm))
}

const LEFT = ['', '▏', '▎', '▍', '▌', '▋', '▊', '▉']
const LOWER = ['', '▁', '▂', '▃', '▄', '▅', '▆', '▇']

/**
 * A one-row gauge: each filled cell takes the color of the usage it stands for, so the
 * bar warms along its length; the threshold is a bright tick where not yet reached.
 * Unknown usage is a dashed track, never an empty gauge.
 */
export function gauge(p: number | null | undefined, width: number, threshold = 85, margin = 10): Canvas {
  const cv = new Canvas(Math.max(1, width), 1)
  if (p == null || !Number.isFinite(p)) {
    for (let x = 0; x < cv.columns; x++) cv.put(x, 0, '╌', TRACK)
    return cv
  }
  const v = Math.max(0, Math.min(100, p))
  const cells = (v / 100) * cv.columns
  const full = Math.floor(cells)
  const part = Math.floor((cells - full) * 8)
  const mark = Math.min(cv.columns - 1, Math.floor((threshold / 100) * cv.columns))
  for (let x = 0; x < cv.columns; x++) {
    const at = ((x + 0.5) / cv.columns) * 100
    if (x < full) cv.put(x, 0, '█', heat(Math.min(at, v), threshold, margin))
    else if (x === full && part > 0) cv.put(x, 0, LEFT[part]!, heat(v, threshold, margin))
    else if (x === mark) cv.put(x, 0, '┃', ACCENT)
    else cv.put(x, 0, '─', TRACK)
  }
  return cv
}

export type FleetColumn = {
  /** Current 5-hour and 7-day figures; undefined when not current. */
  five?: number
  seven?: number
  live: boolean
  recommended: boolean
}

/** Width of one account's group in the fleet chart: two bars, their gap and the space around. */
export const GROUP = 12
const BAR_ROWS = 7

/**
 * The fleet chart: per account a 5-hour and a 7-day column, filled from the bottom in
 * eighths with the warmth of their level, figures on top, the threshold as a dashed
 * line across, ▼ over the live account and ★ over the recommended one. The account
 * names are drawn under it by the caller (as buttons), GROUP columns each.
 */
export function fleet(cols: FleetColumn[], threshold = 85, margin = 10): Canvas {
  const rows = BAR_ROWS + 3
  const cv = new Canvas(Math.max(1, cols.length * GROUP), rows)
  const top = 2
  const thrRow = top + BAR_ROWS - 1 - Math.min(BAR_ROWS - 1, Math.floor((threshold / 100) * BAR_ROWS))
  for (let x = 0; x < cv.columns; x++) cv.put(x, thrRow, '╌', ACCENT)
  cols.forEach((col, i) => {
    const x0 = i * GROUP + 2
    const mid = i * GROUP + Math.floor(GROUP / 2) - 1
    if (col.live) cv.text(mid, 0, '▼', GREEN)
    if (col.recommended) cv.text(col.live ? mid + 1 : mid, 0, '★', ACCENT)
    const bar = (bx: number, p: number | undefined, name: string): void => {
      cv.text(bx, top + BAR_ROWS, name, MUTED)
      if (p == null) {
        cv.text(bx, 1, '--', TRACK)
        for (let y = 0; y < BAR_ROWS; y++) for (let k = 0; k < 2; k++) cv.put(bx + k, top + y, '╎', TRACK)
        return
      }
      const v = Math.max(0, Math.min(100, p))
      const label = String(Math.round(v))
      cv.text(bx + (label.length >= 3 ? -1 : 2 - label.length), 1, label, heat(v, threshold, margin))
      const eighths = Math.round((v / 100) * BAR_ROWS * 8)
      for (let y = 0; y < BAR_ROWS; y++) {
        const level = BAR_ROWS - 1 - y // 0 at the bottom
        const filled = eighths - level * 8
        const at = ((level + 0.5) / BAR_ROWS) * 100
        for (let k = 0; k < 2; k++) {
          if (filled >= 8) cv.put(bx + k, top + y, '█', heat(Math.min(at, v), threshold, margin))
          else if (filled > 0) cv.put(bx + k, top + y, LOWER[filled]!, heat(v, threshold, margin))
          else if (top + y !== thrRow) cv.put(bx + k, top + y, '░', TRACK)
        }
      }
    }
    bar(x0 + 1, col.five, '5h')
    bar(x0 + 5, col.seven, '7d')
  })
  return cv
}

export type TrackMark = 'done' | 'current' | 'todo' | 'failed'

/** A step track: ● done, ◉ current, ○ to do, ✖ stopped, joined by lines that fill as steps complete. */
export function track(marks: TrackMark[], span = 6): Canvas {
  const n = Math.max(1, marks.length)
  const cv = new Canvas((n - 1) * span + 1, 1)
  marks.forEach((m, i) => {
    const x = i * span
    const color = m === 'done' ? GREEN : m === 'current' ? ACCENT : m === 'failed' ? RED : TRACK
    cv.put(x, 0, m === 'done' ? '●' : m === 'current' ? '◉' : m === 'failed' ? '✖' : '○', color)
    if (i < n - 1) {
      const next = marks[i + 1]
      const solid = m === 'done' && next !== 'todo'
      for (let k = 1; k < span; k++) cv.put(x + k, 0, solid ? '━' : '─', solid ? GREEN : TRACK)
    }
  })
  return cv
}
