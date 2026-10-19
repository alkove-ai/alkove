import { beforeAll, describe, expect, it } from 'vitest'

import { WasmEngine } from '../src/core/engine'
import { dot, Voiceprint, type VoiceprintJSON } from '../src/core/timbre'
import type { WordJSON } from '../src/core/spot/spot'
import { processPcm } from '../src/process'
import { f32, loadEngine, maxDiff, modelBytes, wav } from './helpers'

let engine: WasmEngine
beforeAll(async () => {
  engine = new WasmEngine(await loadEngine(), modelBytes)
})

// the references come from the engine's native build: its wasm must give the same bits
describe('primitives against the engine natively', () => {
  it('focus', () => {
    const net = engine.stream('focus')
    const pcm = wav('mix.wav')
    const out = new Float32Array(Math.floor(pcm.length / 256) * 256)
    for (let i = 0; i < out.length; i += 256) {
      net.f32('audio').set(pcm.subarray(i, i + 256))
      net.run()
      out.set(net.f32('audio_out'), i)
    }
    const ref = f32('mix.focus.f32')
    expect(out.length).toBe(ref.length)
    expect(maxDiff(out, ref)).toBe(0)
  })

  it('cue: one probability per 30 ms, one step late', () => {
    const ref = f32('speech.cue.f32')
    const net = engine.stream('cue')
    const pcm = wav('speech.wav')
    const got: number[] = []
    for (let i = 0; i + 480 <= pcm.length; i += 480) {
      net.f32('audio').set(pcm.subarray(i, i + 480))
      net.run()
      got.push(net.f32('speech')[0]!)
    }
    expect(got.length).toBe(ref.length)
    expect(maxDiff(got, ref)).toBe(0)
  })

  it('spot', () => {
    const logp = f32('speech.spot.logp.f32')
    const speech = f32('speech.spot.speech.f32')
    const net = engine.stream('spot')
    const pcm = wav('speech.wav')
    let worst = 0
    for (let s = 0; 480 * (s + 1) <= pcm.length; s++) {
      net.f32('audio').set(pcm.subarray(480 * s, 480 * (s + 1)))
      net.run()
      const out = net.f32('logp')
      for (let k = 0; k < 59; k++) worst = Math.max(worst, Math.abs(out[k]! - logp[59 * s + k]!))
      worst = Math.max(worst, Math.abs(net.f32('speech')[0]! - speech[s]!))
    }
    expect(worst).toBe(0)
  })

  it('timbre', () => {
    const ref = f32('enroll.timbre.f32')
    const [w] = engine.timbre().windows(wav('enroll.wav').subarray(0, 24000))
    expect(maxDiff(w!.embedding, ref.subarray(0, 192))).toBe(0)
    expect(maxDiff(w!.id, ref.subarray(192))).toBe(0)
  })
})

describe('what the primitives say', () => {
  it('lex writes « amendement » as French does', () => {
    const [p] = engine.lex().phonemize(['amendement'], 'fr')
    expect(p![0]!.ipa).toBe('amɑ̃dmɑ̃')
    expect(p![0]!.p).toBeGreaterThan(0.5)
  })

  it('timbre knows the same voice from another clip', () => {
    const embed = (pcm: Float32Array) => Voiceprint.fromJSON(engine.timbre().embed(pcm, 'raw') as unknown as VoiceprintJSON)!
    const me = embed(wav('enroll.wav'))
    const speech = wav('speech.wav').subarray(8000, 8000 + 64000)
    const other = wav('mix.wav')
    expect(me.similarity(embed(speech))).toBeGreaterThan(0.4)
    expect(dot(me.centers[0]!.id, embed(speech).centers[0]!.id)).toBeGreaterThan(me.similarity(embed(other)))
  })

  it('process: speech found, the word spotted', () => {
    const word = engine.compile('amendement', ['fr']).word as unknown as WordJSON
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
