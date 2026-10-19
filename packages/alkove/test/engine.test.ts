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

  it('lex: the encoder once, then the decoder phoneme by phoneme, its keys and values following each beam', () => {
    expect(model('lex').plans.map((p) => p.name)).toEqual(['encoder', 'decoder'])
    expect(() => model('lex').session('nothing')).toThrow(EngineError)
    const enc = model('lex').session('encoder')
    const dec = model('lex').session('decoder')
    const word = new Int32Array(LEX.word)
    ;[...'camille'].forEach((c, i) => (word[i] = c.codePointAt(0)!))
    const { cross, pad } = enc.call({ word: { data: word, dims: [1, LEX.word] }, lang: { data: Int32Array.of(0), dims: [1] } })
    const B = LEX.beams
    const step = (s: typeof dec, tokens: number[], parents: number[], pos: number) =>
      s.call({ token: { data: Int32Array.from(tokens), dims: [B] }, parent: { data: Int32Array.from(parents), dims: [B] }, pos: { data: Int32Array.of(pos), dims: [1] }, cross: cross!, pad: pad! }).logp!
    const first = step(dec, [LEX.bos, LEX.bos, LEX.bos], [0, 0, 0], 0)
    expect(first.dims).toEqual([B, LEX.vocab])
    const row = first.data.subarray(0, LEX.vocab)
    expect(row.every(Number.isFinite)).toBe(true)
    expect(Math.abs(Array.from(row, Math.exp).reduce((a, v) => a + v, 0) - 1)).toBeLessThan(1e-4)
    // the same beams in other rows: through parent, each keeps its keys and values, to the bit
    const run = (second: number[], parents: number[]) => {
      dec.reset()
      step(dec, [LEX.bos, LEX.bos, LEX.bos], [0, 0, 0], 0)
      step(dec, second, [0, 0, 0], 1)
      return step(dec, [5, 5, 5], parents, 2).data
    }
    const straight = run([10, 20, 30], [0, 1, 2])
    expect(maxDiff(run([10, 30, 20], [0, 2, 1]), straight)).toBe(0)
    expect(maxDiff(run([10, 30, 20], [0, 1, 2]), straight)).toBeGreaterThan(0)
    expect(() => dec.call({ token: { data: Int32Array.of(1, 2, 3), dims: [B] } })).toThrow(EngineError)
    enc.release()
    dec.release()
  })

  it('a session too big for the wasm memory is an error, not a crash', () => {
    const str = (t: string) => {
      const b = new Uint8Array(4 * Math.ceil(t.length / 4))
      b.set(new TextEncoder().encode(t))
      return [t.length, ...new Uint32Array(b.buffer)]
    }
    // one plan, no weight, no instruction, the largest arena a file may ask for (8 GB)
    const words = [...str('t'), 0, 1, ...str('main'), 2 ** 31 - 1, 0, 0, 0, 0]
    const file = new Uint8Array(16 + 4 * words.length + 12)
    const head = new Uint32Array(file.buffer, 0, 4 + words.length)
    head.set([0x004b4c41, 1, 4 * words.length, file.length, ...words])
    const huge = alk.model(file)
    expect(() => huge.session()).toThrow(EngineError)
    huge.release()
    expect(model('cue').session().plan.name).toBe('main')
  })

  it('a file that is not a plan is refused, not run', () => {
    expect(() => alk.model(new Uint8Array(64))).toThrow(EngineError)
    expect(() => alk.model(modelBytes('cue').subarray(0, 4096))).toThrow(EngineError)
  })
})
