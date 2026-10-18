import { TIMBRE } from '../manifest'
import type { Net } from './engine'

export type Source = 'raw' | 'focus'

export interface Center {
  embedding: Float32Array
  id: Float32Array
}

export interface VoiceprintJSON {
  alkove: 'voiceprint'
  model: string
  source: Source
  centers: { embedding: number[]; id: number[] }[]
  windows?: string
}

const unit = (v: ArrayLike<number>): Float32Array => {
  let n = 0
  for (let i = 0; i < v.length; i++) n += v[i]! * v[i]!
  n = Math.sqrt(n) || 1
  return Float32Array.from(v, (x) => x / n)
}

export const dot = (a: ArrayLike<number>, b: ArrayLike<number>) => {
  let s = 0
  for (let i = 0; i < a.length; i++) s += a[i]! * b[i]!
  return s
}

const mean = (vs: ArrayLike<number>[]) => {
  const m = new Float32Array(vs[0]!.length)
  for (const v of vs) for (let i = 0; i < m.length; i++) m[i]! += v[i]!
  return unit(m)
}

// mean of the windows without the 10 % least like it
export function trimmedMean(windows: Center[]): Center {
  const first = mean(windows.map((w) => w.id))
  if (windows.length < 10) return { embedding: mean(windows.map((w) => w.embedding)), id: first }
  const scores = windows.map((w) => dot(w.id, first))
  const cut = [...scores].sort((a, b) => a - b)[Math.floor(windows.length / 10)]!
  const kept = windows.filter((_, i) => scores[i]! >= cut)
  return { embedding: mean(kept.map((w) => w.embedding)), id: mean(kept.map((w) => w.id)) }
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
    const theirs = other instanceof Voiceprint ? other.centers : [other]
    let best = -1
    for (const a of this.centers) for (const b of theirs) best = Math.max(best, dot(a.id, b.id))
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
      centers: this.centers.map((c) => ({ embedding: round(c.embedding), id: round(c.id) })),
      ...(this.windows.length ? { windows: packWindows(this.windows) } : {}),
    }
  }

  // null for a print of another Timbre: it no longer means anything, learn again
  static fromJSON(json: VoiceprintJSON | string): Voiceprint | null {
    const j: VoiceprintJSON = typeof json === 'string' ? JSON.parse(json) : json
    if (j.alkove !== 'voiceprint') throw new TypeError('not an Alkove voiceprint')
    if (j.model !== TIMBRE.model) return null
    const centers = j.centers.map((c) => ({ embedding: Float32Array.from(c.embedding), id: Float32Array.from(c.id) }))
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
    out.push({ embedding: pair[0]!, id: pair[1]! })
  }
  return out
}

export type Level = 'lenient' | 'balanced' | 'strict'

// cosine of `id`, a ~12 s print against `seconds` of voice (Common Voice tests): 1 % false rejects, EER, 1 % false accepts; a shorter print rejects more
const LEVELS: Record<Level, [seconds: number, cosine: number][]> = {
  lenient: [[0.6, 0.0775], [0.9, 0.14], [1.5, 0.195], [3, 0.2275]],
  balanced: [[0.6, 0.25], [0.9, 0.29], [1.5, 0.3375], [3, 0.37]],
  strict: [[0.6, 0.37], [0.9, 0.3925], [1.5, 0.415], [3, 0.4425]],
}

// a voice from 16 kHz audio: Timbre on 1.5 s windows, half overlapping
export class Timbre {
  constructor(private net: Net) {}

  window(pcm: Float32Array): Center {
    const out = this.net.run({ audio: { data: pcm, dims: [1, pcm.length] } })
    return { embedding: out.embedding!.data as Float32Array, id: out.id!.data as Float32Array }
  }

  windows(pcm: Float32Array): Center[] {
    if (pcm.length < TIMBRE.min) throw new RangeError(`audio.too-short: ${pcm.length} samples, Timbre needs at least ${TIMBRE.min}`)
    if (pcm.length <= TIMBRE.window) return [this.window(pcm)]
    const hop = TIMBRE.window / 2
    const out: Center[] = []
    for (let i = 0; i + TIMBRE.window <= pcm.length; i += hop) out.push(this.window(pcm.subarray(i, i + TIMBRE.window)))
    return out
  }

  embed(pcm: Float32Array, source: Source = 'raw'): Voiceprint {
    return new Voiceprint([trimmedMean(this.windows(pcm))], source)
  }

  static verify(a: Voiceprint, b: Voiceprint, { level = 'balanced', seconds = 1.5 }: { level?: Level; seconds?: number } = {}): { same: boolean; score: number } {
    const score = a.similarity(b)
    const pts = LEVELS[level]
    const s = Math.min(Math.max(seconds, pts[0]![0]), pts[pts.length - 1]![0])
    const i = Math.max(1, pts.findIndex(([x]) => x >= s))
    const [[s0, c0], [s1, c1]] = [pts[i - 1]!, pts[i]!]
    return { same: score >= c0 + ((c1 - c0) * (s - s0)) / (s1 - s0), score }
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
