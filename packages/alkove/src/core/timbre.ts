import { ln } from '../dsp/math'
import { RATE, TIMBRE } from '../manifest'

export type Source = 'raw' | 'focus'

export interface Center {
  embedding: Float32Array
  id: Float32Array
  // seconds of voice it was made from: a short print needs other thresholds
  seconds: number
}

export interface VoiceprintJSON {
  alkove: 'voiceprint'
  model: string
  source: Source
  centers: { embedding: number[]; id: number[]; seconds: number }[]
  windows?: string
}

export const dot = (a: ArrayLike<number>, b: ArrayLike<number>) => {
  let s = 0
  for (let i = 0; i < a.length; i++) s += a[i]! * b[i]!
  return s
}

// one or more centers (near / far, phone / headset); a voice is as close as its closest center
export class Voiceprint {
  constructor(
    readonly centers: Center[],
    readonly source: Source = 'focus',
    readonly model: string = TIMBRE.model,
    readonly windows: Center[] = [],
  ) {
    if (!centers.length) throw new TypeError('a voiceprint needs a center')
  }

  get embedding(): Float32Array {
    return this.centers[0]!.embedding
  }

  similarity(other: Voiceprint | Center): number {
    return this.closest(other).score
  }

  // the best pair of centers, and how much voice this side's center was made from
  closest(other: Voiceprint | Center): { score: number; mine: number; theirs: number } {
    const theirs = other instanceof Voiceprint ? other.centers : [other]
    let best = { score: -1, mine: 0, theirs: 0 }
    for (const a of this.centers) for (const b of theirs) {
      const score = dot(a.id, b.id)
      if (score > best.score) best = { score, mine: a.seconds, theirs: b.seconds }
    }
    return best
  }

  toArray(): Float32Array {
    return this.centers[0]!.id.slice()
  }

  toJSON(): VoiceprintJSON {
    const round = (v: Float32Array) => Array.from(v, (x) => +x.toFixed(5))
    return {
      alkove: 'voiceprint',
      model: this.model,
      source: this.source,
      centers: this.centers.map((c) => ({ embedding: round(c.embedding), id: round(c.id), seconds: +c.seconds.toFixed(1) })),
      ...(this.windows.length ? { windows: packWindows(this.windows) } : {}),
    }
  }

  // null for a print of another Timbre: it no longer means anything, learn again
  static fromJSON(json: VoiceprintJSON | string): Voiceprint | null {
    const j: VoiceprintJSON = typeof json === 'string' ? JSON.parse(json) : json
    if (j.alkove !== 'voiceprint') throw new TypeError('not an Alkove voiceprint')
    if (j.model !== TIMBRE.model) return null
    const centers = j.centers.map((c) => ({ embedding: Float32Array.from(c.embedding), id: Float32Array.from(c.id), seconds: c.seconds }))
    return new Voiceprint(centers, j.source, j.model, j.windows ? unpackWindows(j.windows) : [])
  }

  static merge(prints: Voiceprint[]): Voiceprint {
    return new Voiceprint(
      prints.flatMap((p) => p.centers),
      prints.every((p) => p.source === 'raw') ? 'raw' : 'focus',
    )
  }
}

// each vector in int8 with its own scale: 2 x 193 bytes a window
function packWindows(ws: Center[]): string {
  const bytes = new Uint8Array(ws.length * 2 * (TIMBRE.dim + 4))
  const view = new DataView(bytes.buffer)
  let o = 0
  for (const w of ws)
    for (const v of [w.embedding, w.id]) {
      let max = 1e-9
      for (const x of v) max = Math.max(max, Math.abs(x))
      view.setFloat32(o, max / 127, true)
      o += 4
      for (const x of v) view.setInt8(o++, Math.round((x / max) * 127))
    }
  let s = ''
  for (const b of bytes) s += String.fromCharCode(b)
  return btoa(s)
}

function unpackWindows(text: string): Center[] {
  const raw = atob(text)
  const bytes = Uint8Array.from(raw, (c) => c.charCodeAt(0))
  const view = new DataView(bytes.buffer)
  const out: Center[] = []
  let o = 0
  while (o + 2 * (TIMBRE.dim + 4) <= bytes.length) {
    const pair: Float32Array[] = []
    for (let k = 0; k < 2; k++) {
      const scale = view.getFloat32(o, true)
      o += 4
      const v = new Float32Array(TIMBRE.dim)
      for (let i = 0; i < v.length; i++) v[i] = view.getInt8(o++) * scale
      pair.push(v)
    }
    out.push({ embedding: pair[0]!, id: pair[1]!, seconds: TIMBRE.window / RATE })
  }
  return out
}

export type Level = 'lenient' | 'balanced' | 'strict'

// cosine of `id` over which `seconds` of voice match a print made from `printSeconds` (Common Voice tests: 1 % false rejects, EER, 1 % false accepts): bilinear in the log of both, bounded to the table
export function threshold(level: Level, seconds: number, printSeconds: number): number {
  const at = (xs: number[], x: number): [number, number, number] => {
    const v = Math.min(Math.max(x, xs[0]!), xs[xs.length - 1]!)
    const i = Math.max(1, xs.findIndex((y) => y >= v))
    return [i - 1, i, ln(v / xs[i - 1]!) / ln(xs[i]! / xs[i - 1]!)]
  }
  const [p0, p1, wp] = at(LEVELS.print, printSeconds)
  const [h0, h1, wh] = at(LEVELS.heard, seconds)
  const t = LEVELS[level]
  const row = (p: number) => t[p]![h0]! + (t[p]![h1]! - t[p]![h0]!) * wh
  return row(p0) + (row(p1) - row(p0)) * wp
}

const LEVELS: { print: number[]; heard: number[] } & Record<Level, number[][]> = {
  print: [1.5, 3, 6, 12, 24],
  heard: [0.6, 0.9, 1.5, 3],
  lenient: [
    [-0.07, -0.065, -0.05, -0.0325],
    [0.0025, 0.0275, 0.0825, 0.1075],
    [0.045, 0.105, 0.135, 0.17],
    [0.07, 0.1125, 0.2025, 0.255],
    [0.0875, 0.15, 0.23, 0.2825],
  ],
  balanced: [
    [0.1675, 0.195, 0.2225, 0.245],
    [0.2, 0.2325, 0.2675, 0.2925],
    [0.23, 0.27, 0.3125, 0.335],
    [0.2475, 0.295, 0.34, 0.37],
    [0.26, 0.3125, 0.36, 0.3925],
  ],
  strict: [
    [0.335, 0.35, 0.365, 0.38],
    [0.3475, 0.365, 0.385, 0.405],
    [0.3625, 0.3825, 0.4075, 0.4275],
    [0.3725, 0.395, 0.4225, 0.445],
    [0.38, 0.405, 0.4325, 0.455],
  ],
}

// what the main thread knows of Timbre without running it: whether two prints are the same voice (the model runs in the core)
export class Timbre {
  // a: the known print, b: the voice heard; each knows how much voice it holds
  static verify(a: Voiceprint, b: Voiceprint, { level = 'balanced' }: { level?: Level } = {}): { same: boolean; score: number } {
    const { score, mine, theirs } = a.closest(b)
    return { same: score >= threshold(level, theirs, mine), score }
  }

  static identify(voice: Voiceprint, known: Record<string, Voiceprint>, opts: { level?: Level } = {}): { who: string; score: number } | null {
    let best: { who: string; score: number } | null = null
    for (const [who, p] of Object.entries(known)) {
      const { same, score } = Timbre.verify(p, voice, opts)
      if (same && (!best || score > best.score)) best = { who, score }
    }
    return best
  }
}
