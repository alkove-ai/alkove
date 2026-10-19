import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { beforeAll, describe, expect, it } from 'vitest'

import type { ChainEvent, ChainOptions } from '../src/core/chain'
import { SPEECH_DEFAULTS } from '../src/core/cue'
import { WasmEngine } from '../src/core/engine'
import { Voiceprint } from '../src/core/timbre'
import { cos, exp, ln, log10, pow, sin } from '../src/dsp/math'
import { Resampler } from '../src/dsp/resample'
import { Alk } from '../src/engine/alk'
import { HF_ORG, MODELS } from '../src/manifest'
import { coreOptions, Processor, type ProcessOptions, type ProcessPart } from '../src/process'
import { f32, loadHarness, modelBytes, noise, wav, workspace } from './helpers'

// the Rust core against the TypeScript it replaced: side by side where the TypeScript remains, else against digests of what it gave

interface Harness {
  alk_h_math(kind: number, x: number, y: number): number
  alk_h_out(): number
  alk_h_resampler(inRate: number, outRate: number, aligned: number): number
  alk_h_resampler_push(r: number, p: number, len: number): number
  alk_h_resampler_free(r: number): void
  alk_h_resample_all(p: number, len: number, inRate: number, outRate: number): number
}

const goldenPath = join(import.meta.dirname, 'fixtures/core.json')

let alk: Alk
let h: Harness
let engine: WasmEngine
beforeAll(async () => {
  alk = await loadHarness()
  h = alk.x as unknown as Harness
  engine = new WasmEngine(alk, modelBytes)
}, 300_000)

const rng = (seed: number) => {
  let s = seed >>> 0
  return () => (s = (s * 1664525 + 1013904223) >>> 0) / 2 ** 32
}

// floats into the wasm for the length of `fn`
function inWasm<T>(x: Float32Array | Float64Array, fn: (p: number) => T): T {
  const bytes = new Uint8Array(x.buffer, x.byteOffset, x.byteLength)
  return alk.with(bytes, (p) => fn(p))
}

const floats = (p: number, n: number) => new Float32Array(alk.heap, p, n).slice()
const bits = (x: Float32Array) => Buffer.from(x.buffer, x.byteOffset, x.byteLength)

// voices at any level, silences, noise: what a mic gives over a few minutes
function stream(seed: number, seconds: number, dense = false): Float32Array {
  const r = rng(seed)
  const voices = [wav('speech.wav'), wav('mix.wav'), wav('enroll.wav')]
  const out = new Float32Array(Math.round(seconds * 16000))
  let o = 0
  while (o < out.length) {
    const kind = dense ? 0 : r()
    const len = Math.min(out.length - o, Math.floor((0.3 + r() * 4) * 16000))
    if (kind < 0.5) {
      const v = voices[Math.floor(r() * voices.length)]!
      const from = Math.floor(r() * v.length)
      const gain = 0.05 + r() * 2
      for (let i = 0; i < len; i++) out[o + i] = v[(from + i) % v.length]! * gain
    } else if (kind < 0.75) {
      const n = r() < 0.5 ? null : noise(len, seed + o, 1e-6)
      if (n) out.set(n, o)
    } else out.set(noise(len, seed + o, 0.002 + r() * 0.05), o)
    o += len
  }
  return out
}

const print = () => f32('enroll.timbre.f32').slice(0, 192)
// long synchronous loops starve vitest's own messages
const breathe = () => new Promise((r) => setImmediate(r))
const WHY = ['silence', 'forced', 'wake']

type Tap = { type: 'focus'; t: number; hop: Buffer } | { type: 'cue'; speech: number; t: number }

interface Control {
  print?: Float32Array | null
  strength?: number
  enabled?: boolean
}

// a chain driven hop by hop, as the real-time worker does
interface Driven {
  step(hop: Float32Array): { out: Buffer; events: (ChainEvent | Tap)[] }
  control(c: Control): void
  state(): number[]
}

// the SDK's chain, on the core
function sdkChain(opts: ChainOptions): Driven {
  const chain = engine.chain(opts)
  let events: (ChainEvent | Tap)[] = []
  chain.onFocus = (hop, t) => events.push({ type: 'focus', t, hop: bits(hop.slice()) })
  chain.onCue = (speech, t) => events.push({ type: 'cue', speech, t })
  return {
    step(hop) {
      events = []
      const out = bits(chain.step(hop, (e) => events.push(e)).slice())
      return { out, events }
    },
    control(c) {
      if ('print' in c) chain.setPrint(c.print ?? null)
      if (c.strength !== undefined) chain.setStrength(c.strength)
      if (c.enabled !== undefined) chain.setEnabled(c.enabled)
    },
    state: () => [+chain.speaking, +chain.sleeping, chain.time, chain.gainDb, chain.delay],
  }
}

describe('the manifest every SDK reads', () => {
  it('src/models.generated.ts holds crates/core/models.json (pnpm build writes it)', () => {
    const { org, models } = JSON.parse(readFileSync(join(workspace, 'crates/core/models.json'), 'utf8'))
    expect({ org: HF_ORG, models: MODELS }).toEqual({ org, models })
  })
})

describe('math, as the TypeScript computed it', () => {
  it('exp, ln, log10, pow, sin, cos: the same doubles', () => {
    const r = rng(7)
    const fns = [(x: number) => exp(x), ln, log10, (x: number, y: number) => pow(x, y), sin, cos]
    for (let i = 0; i < 200_000; i++) {
      const k = i % 6
      const x = k === 0 ? (r() - 0.5) * 1500 : k === 4 || k === 5 ? (r() - 0.5) * 2e5 : k === 3 ? 10 ** ((r() - 0.5) * 8) : 10 ** ((r() - 0.5) * 640)
      const y = (r() - 0.5) * 20
      const ts = fns[k]!(x, y)
      const rs = h.alk_h_math(k, x, y)
      if (!Object.is(ts, rs)) expect([k, x, y, rs]).toEqual([k, x, y, ts])
    }
  })

  it('Math.round, toFixed, Math.max and Math.min', () => {
    const r = rng(11)
    const special = [0.5, -0.5, 1.5, -1.5, 2.5, -2.5, 0.49999999999999994, -0, 0, 0.25, 0.015625, 1.005, 2.675, -0.04, 1e21, 9.99e20, 123456789012.34567, -9.87654321e17, 5e-324, 1e-7, NaN, Infinity, -Infinity]
    const xs = [...special, ...Array.from({ length: 60_000 }, (_, i) => (i % 3 === 0 ? (r() - 0.5) * 100 : i % 3 === 1 ? Math.round((r() - 0.5) * 1e6) / 2 ** Math.floor(r() * 12) : (r() - 0.5) * 10 ** (r() * 21)))]
    for (const x of xs) {
      expect(Object.is(h.alk_h_math(6, x, 0), Math.round(x)), `round ${x}`).toBe(true)
      for (const d of [0, 1, 2, 3, 4, 5]) expect(Object.is(h.alk_h_math(7, x, d), +x.toFixed(d)), `toFixed ${x} ${d}`).toBe(true)
      for (const y of [0, -0, NaN, 1, -1, x / 2]) {
        expect(Object.is(h.alk_h_math(8, x, y), Math.max(x, y)), `max ${x} ${y}`).toBe(true)
        expect(Object.is(h.alk_h_math(9, x, y), Math.min(x, y)), `min ${x} ${y}`).toBe(true)
      }
    }
  })
})

describe('resampler, as the TypeScript computed it', () => {
  const pairs = [
    [48000, 16000],
    [44100, 16000],
    [16000, 48000],
    [16000, 44100],
    [22050, 16000],
    [8000, 16000],
    [96000, 16000],
  ] as const

  it('in chunks of any size, aligned or not', () => {
    const pcm = stream(3, 6)
    for (const [a, b] of pairs)
      for (const aligned of [false, true]) {
        const r = rng(a + b + +aligned)
        const ts = new Resampler(a, b, aligned)
        const core = h.alk_h_resampler(a, b, +aligned)
        const want: number[] = []
        const got: number[] = []
        for (let i = 0; i < pcm.length; ) {
          const n = Math.min(pcm.length - i, 1 + Math.floor(r() * 3000))
          const chunk = pcm.subarray(i, i + n)
          ts.push(chunk, (x) => want.push(x))
          const k = inWasm(chunk, (p) => h.alk_h_resampler_push(core, p, n))
          got.push(...floats(h.alk_h_out(), k))
          i += n
        }
        h.alk_h_resampler_free(core)
        expect(bits(Float32Array.from(got)).equals(bits(Float32Array.from(want))), `${a} -> ${b} ${aligned}`).toBe(true)
      }
  })

  it('a whole signal', () => {
    const pcm = stream(4, 3)
    for (const [a, b] of pairs) {
      const want = Resampler.all(pcm, a, b)
      const n = inWasm(pcm, (p) => h.alk_h_resample_all(p, pcm.length, a, b))
      expect(bits(floats(h.alk_h_out(), n)).equals(bits(want)), `${a} -> ${b}`).toBe(true)
    }
  })
})

// what a run gave, in one sha256: outputs, events and state at every step, as the TypeScript gave them before the core replaced it
const GOLDEN: Record<string, string> = JSON.parse(readFileSync(goldenPath, 'utf8'))

function record(e: ChainEvent | Tap): number[] {
  switch (e.type) {
    case 'speechstart':
      return [0, e.start, 0, 0]
    case 'speechend':
      return [1, e.start, e.end, 0]
    case 'idle':
      return [2, +e.asleep, e.at, 0]
    case 'diag': {
      const d = e.data as { print?: boolean; after: number; why: string }
      return e.code === 'focus.print' ? [3, +d.print!, d.after, WHY.indexOf(d.why)] : [4, d.after, WHY.indexOf(d.why), 0]
    }
    case 'focus':
      return [5, e.t, 0, 0]
    case 'cue':
      return [6, e.speech, e.t, 0]
  }
}

// two parts as one, as the core joins the tail of a resampled stream with its end
function joined(a: ProcessPart, b: ProcessPart): ProcessPart {
  const audio = new Float32Array(a.audio.length + b.audio.length)
  audio.set(a.audio)
  audio.set(b.audio, a.audio.length)
  const trace = a.trace || b.trace ? { focus: [...(a.trace?.focus ?? []), ...(b.trace?.focus ?? [])], cue: [...(a.trace?.cue ?? []), ...(b.trace?.cue ?? [])], spot: [] } : undefined
  return { start: a.start, audio, speech: [...a.speech, ...b.speech], wakes: [], diag: [...a.diag, ...b.diag], ...(trace ? { trace } : {}) }
}

// a part as numbers, for its digest
function partNumbers(p: ProcessPart): Float64Array {
  const n: number[] = [p.start, p.audio.length, ...p.audio, p.speech.length, ...p.speech.flatMap((s) => [s.start, s.end]), p.diag.length, ...p.diag.flatMap((e) => record(e as ChainEvent))]
  if (!p.trace) n.push(0)
  else {
    n.push(1, p.trace.focus.length, ...p.trace.focus.flatMap((r) => [r.t, r.db!]), p.trace.cue.length, ...p.trace.cue.flatMap((r) => [r.t, r.speech!]))
  }
  return Float64Array.from(n)
}

describe('the chain, hop by hop as in real time', () => {
  // name, seed, seconds, options, speech without pauses, settings changed (per hop)
  const cases: [string, number, number, ChainOptions, boolean, number][] = [
    ['defaults, left alone past the 30 s reset', 1, 40, {}, false, 0],
    ['gate, a print, other speech settings, settings changed often', 2, 40, { gate: true, print: print(), speech: { confidence: 0.6, stopSecs: 0.3 } }, false, 0.008],
    ['no normalize, no sleep, half strength', 3, 30, { normalize: false, sleep: false, strength: 0.5 }, false, 0.002],
    ['speech that never stops, past the forced reset', 4, 75, { speech: { stopSecs: 5 } }, true, 0],
  ]
  const seen = new Set<string>()
  for (const [name, seed, seconds, opts, dense, rate] of cases)
    it(name, async () => {
      const pcm = stream(seed, seconds, dense)
      const chain = sdkChain(opts)
      const digest = createHash('sha256')
      const r = rng(seed * 31)
      for (let i = 0; i + 256 <= pcm.length; i += 256) {
        const c: Control = {}
        const x = r() / rate
        if (x < 0.375) c.print = r() < 0.5 ? print().map((v, j) => v * (1 + 0.1 * Math.sin(i + j))) : null
        else if (x < 0.75) c.strength = r() * 1.4 - 0.2
        else if (x < 1) c.enabled = r() < 0.5
        if (Object.keys(c).length) chain.control(c)
        const a = chain.step(pcm.subarray(i, i + 256))
        digest.update(a.out)
        for (const e of a.events) {
          digest.update(bits(new Float32Array(Float64Array.from(record(e)).buffer)))
          if (e.type === 'focus') digest.update(e.hop)
          seen.add(e.type === 'idle' ? `idle ${e.asleep}` : e.type === 'diag' ? `${e.code} ${(e.data as { why: string }).why}` : e.type)
        }
        digest.update(bits(new Float32Array(Float64Array.from(chain.state()).buffer)))
        if (i % (2000 * 256) === 0) await breathe()
      }
      expect(digest.digest('hex')).toBe(GOLDEN[`chain: ${name}`])
    }, 120_000)

  it('went through every event', () => {
    expect([...seen].sort()).toEqual(['cue', 'focus', 'focus.print silence', 'focus.print wake', 'focus.reset forced', 'focus.reset silence', 'idle false', 'idle true', 'speechend', 'speechstart'])
  })
})

describe('process, in blocks of any size', () => {
  const voiceprint = () => new Voiceprint([{ embedding: print(), id: print(), seconds: 3 }])
  // name, seed, options, rate of the blocks (resampled to 16 kHz as Alkove.stream does)
  const cases: [string, number, ProcessOptions, number][] = [
    ['focus and cue', 5, { focus: true, cue: true }, 16000],
    ['focus with a print, normalize, trace, other speech settings', 6, { focus: { voiceprint: voiceprint(), strength: 0.7 }, cue: { confidence: 0.4, startSecs: 0.1 }, normalize: true, trace: true }, 16000],
    ['focus without speech', 7, { focus: true, cue: false }, 16000],
    ['cue alone, traced', 8, { cue: { stopSecs: 1 }, trace: true }, 16000],
    ['nothing chosen', 9, {}, 16000],
    ['focus and cue, blocks at 44.1 kHz', 10, { focus: true, cue: true, trace: true }, 44100],
  ]
  for (const [name, seed, opts, rate] of cases)
    it(name, async () => {
      const pcm = rate === 16000 ? stream(seed, 25) : Resampler.all(stream(seed, 25), 16000, rate)
      const ts = new Processor(engine, opts)
      const resampler = new Resampler(rate, 16000, true)
      let given = 0
      let taken = 0
      const core = engine.processor(coreOptions(opts, rate))
      const digest = createHash('sha256')
      const r = rng(seed * 17)
      const compare = (a: ProcessPart, b: Record<string, unknown>) => {
        expect({ ...b, audio: bits(b.audio as Float32Array) }).toEqual({ ...a, audio: bits(a.audio) })
        digest.update(bits(new Float32Array(partNumbers(a).buffer)))
      }
      const empty = (b: Record<string, unknown>) => expect({ ...b, start: 0, audio: (b.audio as Float32Array).length }).toEqual({ start: 0, audio: 0, speech: [], wakes: [], diag: [] })
      for (let i = 0; i < pcm.length; ) {
        const n = Math.min(pcm.length - i, 1 + Math.floor(r() * 5000))
        const block = pcm.subarray(i, i + n)
        const out: number[] = []
        given += n
        resampler.push(block, (x) => out.push(x))
        taken += out.length
        const got = core.push(block)
        if (out.length) compare(ts.push(Float32Array.from(out)), got)
        else empty(got)
        i += n
        await breathe()
      }
      // Alkove.stream's tail: the last half window, up to the length the input had
      const tail: number[] = []
      const want = Math.round((given * 16000) / rate) - taken
      if (want > 0) resampler.push(new Float32Array(Resampler.HALF * 2 + 8), (x) => tail.length < want && tail.push(x))
      const first = tail.length ? ts.push(Float32Array.from(tail)) : null
      const last = ts.end()
      compare(first ? joined(first, last) : last, core.end())
      core.release()
      expect(digest.digest('hex')).toBe(GOLDEN[`process: ${name}`])
    }, 120_000)
})
