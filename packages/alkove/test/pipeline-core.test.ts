import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { beforeAll, describe, expect, it } from 'vitest'

import { WasmEngine } from '../src/core/engine'
import type { NativePipeline } from '../src/core/native'
import { Alk } from '../src/engine/alk'
import { modelBytes, wav } from './helpers'

// the real-time pipeline's clocks when Spot starts, or starts again, after the stream did; what it refuses
const HOP = 256

let engine: WasmEngine
let word: Record<string, unknown>
beforeAll(async () => {
  engine = new WasmEngine(await Alk.create(await WebAssembly.compile(readFileSync(join(import.meta.dirname, '../dist/alkove-engine.wasm')))), modelBytes)
  word = engine.compile('amendement', ['fr']).word as Record<string, unknown>
})

const silence = (seconds: number) => new Float32Array(Math.round(seconds * 16000))
const concat = (...parts: Float32Array[]) => {
  const out = new Float32Array(parts.reduce((n, p) => n + p.length, 0))
  let o = 0
  for (const p of parts) {
    out.set(p, o)
    o += p.length
  }
  return out
}

// hops fed until `at` (a hop index) calls `then`; the events that came out
function feed(p: NativePipeline, audio: Float32Array, at = -1, then = () => {}): Record<string, unknown>[] {
  const events: Record<string, unknown>[] = []
  for (let k = 0; (k + 1) * HOP <= audio.length; k++) {
    if (k === at) then()
    p.step(audio.subarray(k * HOP, (k + 1) * HOP), 0)
    for (const o of p.outs()) if (o[0] === 'event') events.push(o[1])
  }
  return events
}

const wakes = (events: Record<string, unknown>[]) => events.filter((e) => e.type === 'wake').map((e) => ({ start: e.start as number, end: e.end as number, audio: (e.audio as Float32Array).length }))

describe('Spot on the stream clock', () => {
  it('a listener started on a running input dates its wakes on the stream', () => {
    // 300 hops: a whole number of Spot's 30 ms frames, so both listeners cut the audio alike
    const audio = concat(silence(4.8), wav('speech.wav'), silence(1))
    const early = wakes(feed(engine.pipeline({ chain: null, words: [word], context: 'anywhere' }), audio))
    const p = engine.pipeline({ chain: null })
    const late = wakes(
      feed(p, audio, 300, () => {
        p.spot(engine.model('spot'))
        p.set({ words: [word], context: 'anywhere' })
      }),
    )
    expect(early).toHaveLength(1)
    expect(late).toHaveLength(1)
    expect(late[0]!.start).toBeGreaterThan(4.8)
    expect(Math.abs(late[0]!.start - early[0]!.start)).toBeLessThan(0.1)
    expect(Math.abs(late[0]!.end - early[0]!.end)).toBeLessThan(0.1)
    expect(late[0]!.audio).toBeGreaterThan(0)
  })

  it('words replaced on the way keep the clock', () => {
    const s = wav('speech.wav')
    const audio = concat(s, silence(3), s, silence(1))
    const kept = wakes(feed(engine.pipeline({ chain: null, words: [word], context: 'anywhere' }), audio))
    const p = engine.pipeline({ chain: null, words: [word], context: 'anywhere' })
    const replaced = wakes(feed(p, audio, Math.floor((s.length + 16000) / HOP), () => p.set({ words: [word], context: 'anywhere' })))
    expect(kept).toHaveLength(2)
    expect(replaced).toEqual(kept)
  })

  it('a second enrollment gives the audio of its takes, dated on the stream', () => {
    const s = wav('speech.wav')
    const takes = concat(s, silence(1), s, silence(1), s, silence(1))
    const p = engine.pipeline({ chain: null, enroll: { word, takes: 2 } })
    const first = feed(p, takes)
    p.set({ enroll: { word, takes: 2 } })
    const again = feed(p, takes)
    const enrolled = (events: Record<string, unknown>[]) => events.find((e) => e.type === 'enrolled')!
    expect((enrolled(first).audio as Float32Array).length).toBeGreaterThan(0)
    expect((enrolled(again).audio as Float32Array).length).toBe((enrolled(first).audio as Float32Array).length)
    const startsOf = (events: Record<string, unknown>[]) => events.filter((e) => e.type === 'take').map((e) => e.start as number)
    const duration = Math.floor(takes.length / HOP) * HOP / 16000
    expect(startsOf(again).map((t) => +(t - duration).toFixed(3))).toEqual(startsOf(first).map((t) => +t.toFixed(3)))
  })
})

describe('what the pipeline refuses', () => {
  it('sizes past memory, rates the resampler cannot follow, a word without phonemes, a released pipeline', () => {
    expect(() => engine.pipeline({ chain: null, keepRecent: Infinity })).toThrow(/keepRecent/)
    expect(() => engine.pipeline({ chain: null, output: { format: 'f32', rate: 0, chunkMs: 20, audio: true, utterance: false } })).toThrow(/audio.rate/)
    const p = engine.pipeline({ chain: null })
    expect(() => p.record(Infinity)).toThrow(RangeError)
    p.spot(engine.model('spot'))
    expect(() => p.set({ words: [{ ...word, phonemes: [] }] })).toThrow(TypeError)
    p.release()
    expect(() => p.outs()).toThrow('used after release')
  })
})
