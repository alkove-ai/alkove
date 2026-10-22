import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { beforeAll, describe, expect, it } from 'vitest'

import { WasmEngine } from '../src/core/engine'
import type { NativePipeline } from '../src/core/native'
import { Alk } from '../src/engine/alk'
import { modelBytes, wav } from './helpers'

// the agent pipeline on the models, with each phrase's level, then with the voices far under the session's kept out
const HOP = 256
const OUTPUT = { format: 'f32', rate: 16000, chunkMs: 32, audio: false, utterance: true }

let engine: WasmEngine
beforeAll(async () => {
  engine = new WasmEngine(await Alk.create(await WebAssembly.compile(readFileSync(join(import.meta.dirname, '../dist/alkove-engine.wasm')))), modelBytes)
})

const silence = (seconds: number) => new Float32Array(Math.round(seconds * 16000))
const scaled = (x: Float32Array, gain: number) => x.map((v) => v * gain)
const concat = (...parts: Float32Array[]) => {
  const out = new Float32Array(parts.reduce((n, p) => n + p.length, 0))
  let o = 0
  for (const p of parts) {
    out.set(p, o)
    o += p.length
  }
  return out
}

type Event = Record<string, unknown>
type Level = { voiceDb: number; noiseDb: number; refDb: number | null }

function feed(p: NativePipeline, audio: Float32Array) {
  const events: Event[] = []
  const utterances: { start: number; end: number; level?: Level }[] = []
  const out = new Float32Array(Math.floor(audio.length / HOP) * HOP)
  for (let k = 0; (k + 1) * HOP <= audio.length; k++) {
    out.set(p.step(audio.subarray(k * HOP, (k + 1) * HOP), 0), k * HOP)
    for (const o of p.outs()) {
      if (o[0] === 'event') events.push(o[1])
      if (o[0] === 'utterance') utterances.push({ start: o[2], end: o[3], level: o[4] as Level | undefined })
    }
  }
  return { events, utterances, out }
}

// speech.wav loud twice, 20 dB under, then loud: where each copy starts and ends in the stream
const speech = wav('speech.wav')
const GAP = 1.5
const copies = [1, 0.1, 1].map((g, i) => ({ gain: g, from: 1 + (i + 1) * (speech.length / 16000 + GAP) }))
const audio = concat(silence(1), speech, silence(GAP), ...copies.flatMap((c) => [scaled(speech, c.gain), silence(GAP)]))
const within = (t: number, c: { from: number }) => t >= c.from - 0.3 && t <= c.from + speech.length / 16000 + 0.3

describe('the level of each phrase', () => {
  it('comes with speechend and the utterance, Focus before the leveling, and changes nothing else', () => {
    const plain = feed(engine.pipeline({ chain: {}, output: OUTPUT }), audio)
    const levels = feed(engine.pipeline({ chain: { levels: true }, output: OUTPUT }), audio)
    expect(levels.out).toEqual(plain.out)
    expect(levels.utterances.map(({ start, end }) => ({ start, end }))).toEqual(plain.utterances)
    expect(levels.utterances.length).toBeGreaterThanOrEqual(4)
    for (const u of levels.utterances) expect(Object.keys(u.level ?? {})).toEqual(['voiceDb', 'noiseDb', 'refDb'])
    const ends = levels.events.filter((e) => e.type === 'speechend')
    expect(ends.map((e) => e.voiceDb)).toEqual(levels.utterances.map((u) => u.level!.voiceDb))
    const mean = (c: (typeof copies)[number]) => {
      const l = levels.utterances.filter((u) => within(u.start, c)).map((u) => u.level!.voiceDb)
      return l.reduce((a, b) => a + b, 0) / l.length
    }
    expect(mean(copies[0]!) - mean(copies[1]!)).toBeGreaterThan(15)
    expect(Math.abs(mean(copies[0]!) - mean(copies[2]!))).toBeLessThan(1)
    // the same numbers in Python (test_levels_and_near_give_the_webs_events)
    expect(levels.utterances.map((u) => u.level!.voiceDb)).toEqual([-19.2, -18.3, -40.2, -18.4])
    expect(levels.utterances.map((u) => u.level!.refDb)).toEqual([null, null, -18.7, -18.7])
  })
})

describe('near', () => {
  it('keeps a voice 20 dB under the session’s out of the phrases, and tells it as speech.far', () => {
    const { events, utterances } = feed(engine.pipeline({ chain: { near: true }, output: OUTPUT }), audio)
    const starts = events.filter((e) => e.type === 'speechstart').map((e) => e.start as number)
    const far = events.filter((e) => e.type === 'diag' && e.code === 'speech.far').map((e) => e.data as Event)
    expect(starts.some((t) => within(t, copies[1]!))).toBe(false)
    expect(far.length).toBeGreaterThan(0)
    for (const f of far) {
      expect(within(f.start as number, copies[1]!)).toBe(true)
      expect(f.refDb as number).toBeGreaterThan((f.voiceDb as number) + 8)
    }
    expect(utterances.some((u) => within(u.start, copies[0]!))).toBe(true)
    expect(utterances.some((u) => within(u.start, copies[2]!))).toBe(true)
    expect(utterances.every((u) => !within(u.start, copies[1]!))).toBe(true)
  })

  it('lets it through with a wider gap', () => {
    const { utterances } = feed(engine.pipeline({ chain: { near: { gapDb: 30 } }, output: OUTPUT }), audio)
    expect(utterances.some((u) => within(u.start, copies[1]!))).toBe(true)
  })
})

describe('near on a call', () => {
  it('keeps the gate shut on a far voice', () => {
    const rms = (x: Float32Array, c: { from: number }) => {
      const y = x.subarray(Math.round((c.from + 1.4) * 16000), Math.round((c.from + 3.8) * 16000))
      return 10 * Math.log10(y.reduce((s, v) => s + v * v, 0) / y.length + 1e-12)
    }
    const open = feed(engine.pipeline({ chain: { gate: true }, output: null }), audio).out
    const shut = feed(engine.pipeline({ chain: { gate: true, near: true }, output: null }), audio).out
    expect(rms(open, copies[1]!)).toBeGreaterThan(-45)
    expect(rms(shut, copies[1]!)).toBeLessThan(-80)
    expect(rms(shut, copies[2]!)).toBeGreaterThan(-45)
  })
})

describe('a quiet microphone', () => {
  it('is brought up before Focus: its phrases come as at a normal level, measured as the mic gave them', () => {
    const loud = concat(silence(1), speech, speech, speech, speech, silence(1))
    const quiet = scaled(loud, 10 ** (-36 / 20))
    const ends = (x: Float32Array) => feed(engine.pipeline({ chain: { levels: true }, output: OUTPUT }), x).events.filter((e) => e.type === 'speechend')
    const atNormal = ends(loud)
    const atQuiet = ends(quiet)
    const last = (e: Event[]) => e.filter((x) => (x.start as number) > 1 + 2 * (speech.length / 16000))
    expect(last(atQuiet).length).toBe(last(atNormal).length)
    for (const [q, n] of last(atQuiet).map((e, i) => [e, last(atNormal)[i]!] as const)) expect(Math.abs((q.voiceDb as number) - ((n.voiceDb as number) - 36))).toBeLessThan(1.5)
  })
})
