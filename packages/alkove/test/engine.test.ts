import { beforeAll, describe, expect, it } from 'vitest'

import { type Alk, EngineError, type Model } from '../src/engine/alk'
import { FOCUS, LEX, type ModelName, TIMBRE } from '../src/manifest'
import { loadEngine, maxDiff, modelBytes, noise } from './helpers'

let alk: Alk
const models = new Map<ModelName, Model>()
const model = (name: ModelName) => models.get(name) ?? models.set(name, alk.model(modelBytes(name))).get(name)!

beforeAll(async () => {
  alk = await loadEngine()
})

const steps = (name: 'focus' | 'cue' | 'spot', n: number, out: string, setup?: (s: ReturnType<Model['session']>) => void) => {
  const s = model(name).session()
  setup?.(s)
  const chunk = s.f32('audio').length
  const got: Float32Array[] = []
  for (let k = 0; k < n; k++) {
    s.f32('audio').set(noise(chunk, k + 1))
    s.run()
    got.push(s.f32(out).slice())
  }
  return { s, got }
}

describe('engine', () => {
  it('one state per stream: a second session of a model starts from zero', () => {
    const a = steps('cue', 6, 'speech')
    const b = steps('cue', 6, 'speech')
    expect(a.got.map((x) => x[0])).toEqual(b.got.map((x) => x[0]))
    expect(a.got[5]![0]).not.toBe(a.got[0]![0])
    a.s.release()
    b.s.release()
  })

  it('reset clears the state and keeps the inputs (Focus keeps its print)', () => {
    const print = (s: ReturnType<Model['session']>) => s.f32('print').set(noise(FOCUS.print, 9))
    const a = steps('focus', 4, 'audio_out', print)
    a.s.reset()
    a.s.f32('audio').set(noise(FOCUS.chunk, 1))
    a.s.run()
    const fresh = steps('focus', 1, 'audio_out', print)
    expect(maxDiff(a.s.f32('audio_out'), fresh.got[0]!)).toBe(0)
    a.s.release()
    fresh.s.release()
  })

  it('timbre: windows of any length, each as from a fresh session', () => {
    const s = model('timbre').session()
    for (const n of [TIMBRE.window, TIMBRE.min, 16000, TIMBRE.window - 1]) {
      const audio = noise(n, n)
      const fresh = model('timbre').session()
      const want = fresh.call({ audio: { data: audio, dims: [1, n] } })
      const got = s.call({ audio: { data: audio, dims: [1, n] } })
      expect(got.id!.dims).toEqual([1, TIMBRE.dim])
      expect(maxDiff(got.id!.data, want.id!.data)).toBe(0)
      expect(maxDiff(got.embedding!.data, want.embedding!.data)).toBe(0)
      fresh.release()
    }
    for (const n of [TIMBRE.min - 1, TIMBRE.window + 1]) expect(() => s.call({ audio: { data: noise(n), dims: [1, n] } })).toThrow(EngineError)
    s.release()
  })

  it('lex: int32 inputs, one row of log-probabilities per beam', () => {
    const s = model('lex').session()
    const word = new Int32Array(LEX.word)
    ;[...'camille'].forEach((c, i) => (word[i] = c.codePointAt(0)!))
    const prefix = new Int32Array(LEX.beams * LEX.prefix)
    for (let b = 0; b < LEX.beams; b++) prefix[b * LEX.prefix] = LEX.bos
    const out = s.call({ word: { data: word, dims: [1, LEX.word] }, lang: { data: Int32Array.of(0), dims: [1] }, prefix: { data: prefix, dims: [LEX.beams, LEX.prefix] }, pos: { data: Int32Array.of(0), dims: [1] } })
    expect(out.logp!.dims).toEqual([LEX.beams, LEX.vocab])
    const row = out.logp!.data.subarray(0, LEX.vocab)
    expect(row.every(Number.isFinite)).toBe(true)
    expect(Math.abs(Array.from(row, Math.exp).reduce((a, v) => a + v, 0) - 1)).toBeLessThan(1e-4)
    expect(() => s.call({ word: { data: word, dims: [1, LEX.word] } })).toThrow(EngineError)
    s.release()
  })

  it('a file that is not a plan is refused, not run', () => {
    expect(() => alk.model(new Uint8Array(64))).toThrow(EngineError)
    expect(() => alk.model(modelBytes('cue').subarray(0, 4096))).toThrow(EngineError)
  })
})
