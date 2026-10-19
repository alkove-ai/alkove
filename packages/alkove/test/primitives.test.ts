import { beforeAll, describe, expect, it } from 'vitest'

import { Cue } from '../src/core/cue'
import { WasmEngine } from '../src/core/engine'
import { Focus } from '../src/core/focus'
import { Lex } from '../src/core/lex'
import { compileWord, Spot } from '../src/core/spot/spot'
import { dot, Timbre } from '../src/core/timbre'
import { processPcm } from '../src/process'
import { f32, loadEngine, maxDiff, modelBytes, wav } from './helpers'

let engine: WasmEngine
beforeAll(async () => {
  engine = new WasmEngine(await loadEngine(), modelBytes)
})

// the references come from the engine's native build: its wasm must give the same bits
describe('primitives against the engine natively (lab)', () => {
  it('focus', () => {
    const out = new Focus(engine.stream('focus')).push(wav('mix.wav'))
    const ref = f32('mix.focus.f32')
    expect(out.length).toBe(ref.length)
    expect(maxDiff(out, ref)).toBe(0)
  })

  it('cue: one probability per 30 ms, one step late', () => {
    const ref = f32('speech.cue.f32')
    const got: number[] = []
    new Cue(engine.stream('cue')).push(wav('speech.wav'), (p, k) => (got[k] = p))
    expect(got.length).toBe(ref.length - 1)
    expect(maxDiff(got, ref.subarray(1))).toBe(0)
  })

  it('spot', () => {
    const logp = f32('speech.spot.logp.f32')
    const speech = f32('speech.spot.speech.f32')
    let worst = 0
    new Spot(engine.stream('spot')).push(wav('speech.wav'), (f, s) => {
      for (let k = 0; k < 59; k++) worst = Math.max(worst, Math.abs(f.logp[k]! - logp[59 * s + k]!))
      if (f.speech !== null) worst = Math.max(worst, Math.abs(f.speech - speech[s]!))
    })
    expect(worst).toBe(0)
  })

  it('timbre', () => {
    const ref = f32('enroll.timbre.f32')
    const w = new Timbre(engine.net('timbre')).window(wav('enroll.wav').subarray(0, 24000))
    expect(maxDiff(w.embedding, ref.subarray(0, 192))).toBe(0)
    expect(maxDiff(w.id, ref.subarray(192))).toBe(0)
  })
})

describe('what the primitives say', () => {
  it('lex writes « amendement » as French does', () => {
    const p = new Lex(engine.net('lex')).phonemize('amendement', 'fr')
    expect(p[0]!.ipa).toBe('amɑ̃dmɑ̃')
    expect(p[0]!.p).toBeGreaterThan(0.5)
  })

  it('timbre knows the same voice from another clip', () => {
    const timbre = new Timbre(engine.net('timbre'))
    const me = timbre.embed(wav('enroll.wav'))
    const speech = wav('speech.wav').subarray(8000, 8000 + 64000)
    const other = wav('mix.wav')
    expect(me.similarity(timbre.embed(speech))).toBeGreaterThan(0.4)
    expect(dot(me.centers[0]!.id, timbre.embed(speech).centers[0]!.id)).toBeGreaterThan(me.similarity(timbre.embed(other)))
  })

  it('process: speech found, the word spotted', () => {
    const word = compileWord('amendement', { lang: 'fr' }, () => new Lex(engine.net('lex')))
    const r = processPcm(engine, wav('speech.wav'), { cue: true, spot: { words: [word] } })
    expect(r.speech.length).toBe(1)
    // 0.5 s of silence, then the clip's own 0.7 s before the voice
    expect(r.speech[0]!.start).toBeGreaterThan(1.0)
    expect(r.speech[0]!.start).toBeLessThan(1.4)
    expect(r.speech[0]!.end).toBeGreaterThan(3.5)
    expect(r.speech[0]!.end).toBeLessThan(4.5)
    expect(r.wakes.map((w) => w.word)).toEqual(['amendement'])
    expect(r.wakes[0]!.start).toBeGreaterThan(1.2)
    expect(r.wakes[0]!.end).toBeLessThan(2.5)
  })

  it('process with focus: same length as the input, aligned with it', () => {
    const mix = wav('mix.wav')
    const r = processPcm(engine, mix, { focus: true, cue: true })
    expect(r.audio.length).toBe(mix.length)
    const ref = f32('mix.focus.f32')
    expect(maxDiff(r.audio.subarray(0, ref.length - 256), ref.subarray(256))).toBe(0)
    expect(r.speech.length).toBeGreaterThan(0)
  })
})
