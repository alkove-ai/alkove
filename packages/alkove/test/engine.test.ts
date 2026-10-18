import * as ort from 'onnxruntime-web'
import { beforeAll, describe, expect, it } from 'vitest'

import type { Ort } from '../src/engine/ort'
import { FOCUS, LEX, STREAMS, TIMBRE } from '../src/manifest'
import { loadOrt, maxDiff, modelBytes, noise } from './helpers'

let engine: Ort
beforeAll(async () => {
  engine = await loadOrt()
  ort.env.wasm.numThreads = 1
})

const reference = (name: 'focus' | 'cue' | 'spot' | 'timbre' | 'lex') => ort.InferenceSession.create(modelBytes(name), { executionProviders: ['wasm'] })

describe('engine against onnxruntime-web 1.30.0', () => {
  for (const name of ['focus', 'cue', 'spot'] as const) {
    it(`${name}: 15 streamed steps, same outputs`, async () => {
      const spec = STREAMS[name]
      const ref = await reference(name)
      const binding = engine.session(modelBytes(name)).bind({ ...spec.shapes }, spec.loops)
      const chunk = spec.shapes.audio[1]!
      let state = new Float32Array(spec.shapes.state[0]!)
      let worst = 0
      for (let step = 0; step < 15; step++) {
        const audio = noise(chunk, step + 1)
        const feeds: Record<string, ort.Tensor> = {
          audio: new ort.Tensor('float32', audio, [1, chunk]),
          state: new ort.Tensor('float32', state, [state.length]),
        }
        if (name === 'focus') feeds.print = new ort.Tensor('float32', new Float32Array(FOCUS.print), [1, FOCUS.print])
        const out = await ref.run(feeds)
        binding.f32('audio').set(audio)
        binding.run()
        for (const o of ref.outputNames.filter((n) => n !== 'next_state')) worst = Math.max(worst, maxDiff(binding.f32(o), out[o]!.data as Float32Array))
        state = out.next_state!.data as Float32Array
        worst = Math.max(worst, maxDiff(binding.f32('state'), state))
      }
      expect(worst).toBe(0)
    })
  }

  it('timbre: windows of any length', async () => {
    const ref = await reference('timbre')
    const s = engine.session(modelBytes('timbre'))
    for (const n of [TIMBRE.min, 16000, TIMBRE.window]) {
      const audio = noise(n, n)
      const out = await ref.run({ audio: new ort.Tensor('float32', audio, [1, n]) })
      const mine = s.run({ audio: { data: audio, dims: [1, n] } })
      expect(maxDiff(mine.embedding!.data, out.embedding!.data as Float32Array)).toBe(0)
      expect(maxDiff(mine.id!.data, out.id!.data as Float32Array)).toBe(0)
    }
  })

  it('lex: int32 inputs', async () => {
    const ref = await reference('lex')
    const s = engine.session(modelBytes('lex'))
    const word = new Int32Array(LEX.word)
    ;[...'camille'].forEach((c, i) => (word[i] = c.codePointAt(0)!))
    const prefix = new Int32Array(LEX.beams * LEX.prefix)
    for (let b = 0; b < LEX.beams; b++) prefix[b * LEX.prefix] = LEX.bos
    const feeds = { word, lang: Int32Array.of(0), prefix, pos: Int32Array.of(0) }
    const dims = { word: [1, LEX.word], lang: [1], prefix: [LEX.beams, LEX.prefix], pos: [1] }
    const out = await ref.run(Object.fromEntries(Object.entries(feeds).map(([k, v]) => [k, new ort.Tensor('int32', v, dims[k as keyof typeof dims])])))
    const mine = s.run(Object.fromEntries(Object.entries(feeds).map(([k, v]) => [k, { data: v, dims: dims[k as keyof typeof dims] }])))
    expect(mine.logp!.dims).toEqual([LEX.beams, LEX.vocab])
    expect(maxDiff(mine.logp!.data, out.logp!.data as Float32Array)).toBe(0)
  })
})
