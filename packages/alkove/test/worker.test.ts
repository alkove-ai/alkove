import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { beforeAll, describe, expect, it } from 'vitest'

import type { WordJSON } from '../src/core/spot/spot'
import { MODELS, type ModelName } from '../src/manifest'
import { modelBytes, noise, wav } from './helpers'

// the built worker fed messages on a simulated clock: every channel gives what the worker before the core gave (compared side by side, then kept as digests)

const root = join(import.meta.dirname, '..')
const HOP = 256
const WIRE = HOP + 2
const RAW_HOP = -1

const GOLDEN: Record<string, string> = JSON.parse(readFileSync(join(import.meta.dirname, 'fixtures/core.json'), 'utf8'))
const canon = (v: unknown): unknown =>
  v instanceof ArrayBuffer
    ? Array.from(new Uint8Array(v))
    : ArrayBuffer.isView(v)
      ? Array.from(v as Uint8Array)
      : Array.isArray(v)
        ? v.map(canon)
        : v && typeof v === 'object'
          ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, canon((v as Record<string, unknown>)[k])]))
          : v
const digest = (v: unknown) => createHash('sha256').update(JSON.stringify(canon(v))).digest('hex')

let module: WebAssembly.Module
beforeAll(async () => {
  module = await WebAssembly.compile(readFileSync(join(root, 'dist/alkove-engine.wasm')))
})

// a message port that delivers when the test says, as the browser does on its next task
class Port {
  other!: Port
  onmessage: ((e: { data: unknown }) => void) | null = null
  private listeners: ((e: { data: unknown }) => void)[] = []
  inbox: unknown[] = []
  sent: unknown[] = []
  postMessage(m: unknown): void {
    this.sent.push(m)
    this.other.inbox.push(m)
  }
  addEventListener(_: string, fn: (e: { data: unknown }) => void): void {
    this.listeners.push(fn)
  }
  close(): void {}
  deliver(): number {
    const n = this.inbox.length
    for (const m of this.inbox.splice(0)) {
      this.onmessage?.({ data: m })
      for (const fn of this.listeners) fn({ data: m })
    }
    return n
  }
}

function channel(): [Port, Port] {
  const a = new Port()
  const b = new Port()
  a.other = b
  b.other = a
  return [a, b]
}

// a worker bundle in this realm: its globals given, its posts kept
function worker(file: string, clock: { now: number }) {
  const posted: unknown[] = []
  const timers: (() => void)[] = []
  const self = { postMessage: (m: unknown) => posted.push(m), onmessage: null as ((e: { data: unknown }) => Promise<void>) | null }
  const run = new Function('self', 'performance', 'setInterval', 'clearInterval', readFileSync(file, 'utf8'))
  run(self, { now: () => clock.now }, (fn: () => void) => timers.push(fn), () => {})
  return { posted, tick: () => timers.forEach((t) => t()), send: (data: unknown) => self.onmessage!({ data }) }
}

const bytes = (names: ModelName[]) => Object.fromEntries(names.map((n) => [n, modelBytes(n).slice().buffer]))

interface Scenario {
  name: string
  opts: Record<string, unknown>
  models: ModelName[]
  learn?: boolean
  reference?: unknown
  audio: Float32Array
  // hop index -> a 'set' message, a record request, raw hops
  sets?: Record<number, Record<string, unknown>>
  record?: number
  raw?: [number, number]
}

async function run(file: string, s: Scenario) {
  const clock = { now: 0 }
  const rt = worker(file, clock)
  const bg = worker(file, clock)
  await rt.send({ type: 'boot', module })
  await bg.send({ type: 'boot', module })
  await rt.send({ type: 'models', id: 1, models: bytes([...s.models, 'spot']) })
  await bg.send({ type: 'models', id: 1, models: bytes(['timbre']) })
  const [rtSide, bgSide] = channel()
  const [appOut, workerOut] = channel()
  const [worklet, workerHops] = channel()
  await bg.send({ type: 'rt-port', input: 1, port: bgSide, ref: s.reference ?? null })
  if (s.learn) await bg.send({ type: 'call', id: 2, method: 'learner', args: [1, null] })
  await rt.send({ type: 'realtime', id: 3, input: 1, opts: s.opts, bg: rtSide, out: workerOut })
  await rt.send({ type: 'worklet', input: 1, port: workerHops })
  const hops = Math.floor(s.audio.length / HOP)
  for (let k = 0; k < hops; k++) {
    clock.now = (k + 1) * 16
    if (s.sets?.[k]) await rt.send({ type: 'set', input: 1, ...s.sets[k] })
    if (s.record === k) await rt.send({ type: 'call', id: 9, method: 'record', args: [1, 3] })
    const wire = new Float32Array(WIRE)
    wire.set(s.audio.subarray(k * HOP, (k + 1) * HOP))
    wire[HOP] = k > 200 ? 3000 : 0
    wire[HOP + 1] = s.raw && k >= s.raw[0] && k < s.raw[1] ? RAW_HOP : 1
    worklet.postMessage(wire.buffer)
    workerHops.deliver()
    // the background answers before the next hop, as it would within a few ms
    while (bgSide.deliver() + rtSide.deliver()) {}
    if (k % 6 === 5) {
      rt.tick()
      bg.tick()
    }
  }
  rt.tick()
  await rt.send({ type: 'call', id: 10, method: 'stats', args: [1] })
  await rt.send({ type: 'call', id: 11, method: 'recent', args: [1] })
  const strip = (m: unknown) => {
    const x = m as Record<string, unknown>
    if (x.type === 'window') delete x.t
    return x
  }
  return {
    rt: rt.posted.filter((m) => (m as { type: string }).type !== 'events'),
    events: rt.posted.filter((m) => (m as { type: string }).type === 'events').flatMap((m) => (m as { list: unknown[] }).list),
    bg: bg.posted,
    hops: worklet.inbox,
    toBackground: rtSide.sent.map(strip),
    fromBackground: bgSide.sent,
    audio: workerOut.sent.filter((m) => (m as { type: string }).type === 'audio'),
    utterances: workerOut.sent.filter((m) => (m as { type: string }).type === 'utterance'),
  }
}

// a voice that talks, pauses, talks again; another voice; noise
function speech(seconds: number): Float32Array {
  const voices = [wav('enroll.wav'), wav('speech.wav'), wav('mix.wav')]
  const out = new Float32Array(seconds * 16000)
  let o = 0
  let k = 0
  while (o < out.length) {
    const v = voices[k % 5 === 4 ? 1 : 0]!
    const n = Math.min(out.length - o, v.length)
    out.set(v.subarray(0, n), o)
    o += n + 12000
    k++
  }
  out.set(noise(out.length, 7, 0.002).map((x, i) => x + out[i]!))
  return out
}

// the word said three times, with silence between
function takes(): Float32Array {
  const s = wav('speech.wav')
  const out = new Float32Array(3 * (s.length + 16000))
  for (let i = 0; i < 3; i++) out.set(s, i * (s.length + 16000))
  return out
}

describe('the workers, before the core and on it', () => {
  let word: WordJSON
  let print: unknown
  beforeAll(async () => {
    const { WasmEngine } = await import('../src/core/engine')
    const { Alk } = await import('../src/engine/alk')
    const engine = new WasmEngine(await Alk.create(module), modelBytes)
    word = engine.compile('amendement', ['fr']).word as unknown as WordJSON
    print = engine.timbre().embed(wav('enroll.wav'), 'raw')
  })

  const scenarios = (): Scenario[] => [
    {
      name: 'a call that learns the print, traced, kept, with stats',
      models: ['focus', 'cue'],
      learn: true,
      opts: { pipeline: 'call', chain: { gate: true, normalize: true, strength: 1, print: null }, learn: true, keepRecent: 4, trace: true, stats: true },
      audio: speech(40),
      record: 300,
      sets: { 900: { strength: 0.6 }, 1300: { enabled: false }, 1500: { enabled: true } },
    },
    {
      name: 'an agent: audio and utterances at 24 kHz in int16, raw hops on the way',
      models: ['focus', 'cue'],
      opts: { pipeline: 'agent', chain: { gate: false, normalize: true, strength: 1, print: null }, keepRecent: 0, output: { format: 's16', rate: 24000, chunkMs: 32, audio: true, utterance: true } },
      audio: speech(30),
      raw: [100, 160],
    },
    {
      name: 'a wake listener on the raw mic, only for my voice',
      models: [],
      reference: print,
      opts: { pipeline: 'wake', words: [word], context: 'anywhere', trace: true, verify: true },
      audio: speech(40),
    },
    {
      name: 'enrolling a word on the raw mic',
      models: [],
      opts: { pipeline: 'enroll', enroll: { word, takes: 2 } },
      audio: takes(),
    },
  ]

  it('every channel, every scenario', async () => {
    const seen: Record<string, string[]> = {}
    for (const s of scenarios()) {
      const after = await run(join(root, 'dist/alkove-worker.js'), s)
      for (const k of Object.keys(after) as (keyof typeof after)[]) expect(digest(after[k]), `${s.name}: ${k}`).toBe(GOLDEN[`worker: ${s.name}: ${k}`])
      const all = [...after.events, ...after.bg.filter((m) => (m as { type: string }).type === 'event').map((m) => (m as { event: unknown }).event)]
      const kinds = [...new Set(all.map((e) => (e as { type: string; code?: string }).code ?? (e as { type: string }).type))].sort()
      seen[s.name] = kinds
    }
    // what went through: learning, wakes for the right voice, utterances, takes
    expect(seen).toEqual({
      'a call that learns the print, traced, kept, with stats': expect.arrayContaining(['print.learn', 'learn', 'voiceprint', 'speechstart', 'speechend', 'trace', 'timing', 'focus.print']),
      'an agent: audio and utterances at 24 kHz in int16, raw hops on the way': expect.arrayContaining(['speechstart', 'speechend']),
      'a wake listener on the raw mic, only for my voice': expect.arrayContaining(['wake', 'heard', 'trace']),
      'enrolling a word on the raw mic': expect.arrayContaining(['take', 'enrolled']),
    })
  }, 600_000)

  it('a print that comes back after its input closed is dropped', async () => {
    const rt = worker(join(root, 'dist/alkove-worker.js'), { now: 0 })
    await rt.send({ type: 'boot', module })
    await rt.send({ type: 'models', id: 1, models: bytes(['focus', 'cue']) })
    const [rtSide, bgSide] = channel()
    await rt.send({ type: 'realtime', id: 2, input: 1, opts: { pipeline: 'call', chain: { gate: false, normalize: true, strength: 1, print: null }, learn: true }, bg: rtSide, out: null })
    await rt.send({ type: 'dispose', input: 1 })
    bgSide.postMessage({ type: 'print', embedding: new Float32Array(256) })
    expect(() => rtSide.deliver()).not.toThrow()
  })
})

export { MODELS }
