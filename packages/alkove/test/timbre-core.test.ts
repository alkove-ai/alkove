import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { beforeAll, describe, expect, it } from 'vitest'

import { WasmEngine } from '../src/core/engine'
import { type Level, threshold, Timbre, Voiceprint, type VoiceprintJSON } from '../src/core/timbre'
import type { Alk } from '../src/engine/alk'
import { loadHarness, modelBytes, noise, wav } from './helpers'

// Timbre and the learner in the core: against the main thread's prints where they remain, else against digests of what the TypeScript gave

let alk: Alk
let engine: WasmEngine
beforeAll(async () => {
  alk = await loadHarness()
  engine = new WasmEngine(alk, modelBytes)
}, 300_000)

const goldenPath = join(import.meta.dirname, 'fixtures/core.json')
const GOLDEN: Record<string, string> = JSON.parse(readFileSync(goldenPath, 'utf8'))
const canon = (v: unknown): unknown =>
  ArrayBuffer.isView(v) ? Array.from(v as Float32Array) : Array.isArray(v) ? v.map(canon) : v && typeof v === 'object' ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, canon((v as Record<string, unknown>)[k])])) : v
function digest(key: string, outputs: unknown[]): void {
  expect(createHash('sha256').update(JSON.stringify(canon(outputs))).digest('hex'), key).toBe(GOLDEN[key])
}

const rng = (seed: number) => {
  let s = seed >>> 0
  return () => (s = (s * 1664525 + 1013904223) >>> 0) / 2 ** 32
}

const clips = () => [wav('speech.wav'), wav('mix.wav'), wav('enroll.wav')]

describe('Timbre, as the TypeScript ran it', () => {
  it('windows of any length, and the print of a clip', () => {
    const r = rng(1)
    const all: unknown[] = []
    for (const c of clips())
      for (let k = 0; k < 6; k++) {
        const from = Math.floor(r() * c.length * 0.5)
        const len = [280, 4000, 24000, 24001, 36000, c.length - from][k]!
        const pcm = c.subarray(from, from + len)
        all.push(engine.timbre().windows(pcm))
        for (const source of ['raw', 'focus'] as const) all.push(engine.timbre().embed(pcm, source))
      }
    digest('timbre: windows and prints', all)
    expect(() => engine.timbre().windows(new Float32Array(279))).toThrow('audio.too-short')
  })
})

describe('prints, as the TypeScript read, wrote and compared them', () => {
  it('JSON with and without windows, merged prints', () => {
    const learner = engine.learner('focus', null)
    const c = wav('enroll.wav')
    for (let i = 0; i + 24000 <= c.length; i += 12000) learner.learn(engine.timbre(), c.subarray(i, i + 24000))
    const learned = learner.voiceprint() as unknown as VoiceprintJSON
    const embedded = engine.timbre().embed(wav('speech.wav'), 'raw') as unknown as VoiceprintJSON
    for (const j of [learned, embedded]) {
      expect(engine.voiceprint({ op: 'read', print: j })).toEqual(Voiceprint.fromJSON(j)!.toJSON())
      expect(engine.voiceprint({ op: 'read', print: { ...j, model: 'alkove-timbre-1-preview' } })).toBeNull()
    }
    const merged = Voiceprint.merge([Voiceprint.fromJSON(learned)!, Voiceprint.fromJSON(embedded)!]).toJSON()
    expect(engine.voiceprint({ op: 'merge', prints: [learned, embedded] })).toEqual(merged)
  })

  it('the thresholds, verify and identify', () => {
    const levels: Level[] = ['lenient', 'balanced', 'strict']
    const r = rng(2)
    const prints: VoiceprintJSON[] = []
    for (const c of clips())
      for (let k = 0; k < 4; k++) {
        const len = Math.floor(4000 + r() * (c.length - 4000))
        const from = Math.floor(r() * (c.length - len))
        prints.push(engine.timbre().embed(c.subarray(from, from + len), 'raw') as unknown as VoiceprintJSON)
      }
    for (const a of prints)
      for (const b of prints)
        for (const level of levels) {
          const want = Timbre.verify(Voiceprint.fromJSON(a)!, Voiceprint.fromJSON(b)!, { level })
          expect(engine.voiceprint({ op: 'verify', a, b, level })).toEqual(want)
        }
    for (const level of levels) {
      const known = prints.slice(0, 6).map((p, i) => [`p${i}`, p] as const)
      for (const v of prints) {
        const want = Timbre.identify(Voiceprint.fromJSON(v)!, Object.fromEntries(known.map(([w, p]) => [w, Voiceprint.fromJSON(p)!])), { level })
        expect(engine.voiceprint({ op: 'identify', voice: v, known, level })).toEqual(want)
      }
    }
    // verify takes the print's own seconds: a grid of them through the table's edges
    for (let k = 0; k < 400; k++) {
      const a = { ...prints[0]!, centers: [{ ...prints[0]!.centers[0]!, seconds: +(0.5 + r() * 30).toFixed(1) }] }
      const b = { ...prints[3]!, centers: [{ ...prints[3]!.centers[0]!, seconds: +(0.2 + r() * 4).toFixed(1) }] }
      const level = levels[k % 3]!
      expect(engine.voiceprint({ op: 'verify', a, b, level })).toEqual(Timbre.verify(Voiceprint.fromJSON(a)!, Voiceprint.fromJSON(b)!, { level }))
    }
    expect(threshold('balanced', 1.5, 6)).toBeGreaterThan(0)
  })
})

describe('learning a print, as the TypeScript learned it', () => {
  it('window by window: voices that change, rejects, restarts, a print that holds', () => {
    const r = rng(3)
    const cs = clips()
    const all: unknown[] = []
    for (const start of [null, 'embedded', 'learned'] as const) {
      const prior = engine.learner('focus', null)
      for (let i = 0; i + 24000 <= cs[2]!.length; i += 12000) prior.learn(engine.timbre(), cs[2]!.subarray(i, i + 24000))
      const json = start === 'embedded' ? engine.timbre().embed(cs[2]!, 'focus') : start === 'learned' ? prior.voiceprint() : null
      const native = engine.learner('focus', json)
      for (let k = 0; k < 60; k++) {
        // mostly one voice, sometimes another, sometimes noise
        const x = r()
        const c = x < 0.7 ? cs[2]! : x < 0.9 ? cs[Math.floor(r() * 2)]! : noise(30000, k, 0.05)
        const len = 20000 + Math.floor(r() * 4000)
        const from = Math.floor(r() * (c.length - len))
        const { windows: _, ...got } = native.learn(engine.timbre(), c.subarray(from, from + len))
        all.push(got, native.voiceprint())
      }
      native.release()
      prior.release()
    }
    digest('timbre: learning window by window', all)
  })

  it('offered speech of any length, then forgotten', () => {
    const c = wav('enroll.wav')
    const native = engine.learner('raw', null)
    const quiet = new Float32Array(30000)
    const all: unknown[] = []
    for (const pcm of [c, quiet, c.subarray(5000), c.subarray(0, 21000), c.subarray(0, 21700)]) {
      const { windows: _, ...got } = native.offer(engine.timbre(), pcm)
      all.push(got, native.voiceprint(), native.voiceprint(true))
    }
    digest('timbre: learning offered speech', all)
    native.forget()
    expect(native.voiceprint()).toBeNull()
  })
})
