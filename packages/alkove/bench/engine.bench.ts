import * as ort from 'onnxruntime-web'

import { FOCUS, STREAMS } from '../src/manifest'
import { loadOrt, modelBytes, noise } from '../test/helpers'

ort.env.wasm.numThreads = 1
const engine = await loadOrt()
const seconds = 10

for (const name of ['focus', 'cue', 'spot'] as const) {
  const spec = STREAMS[name]
  const chunk = spec.shapes.audio[1]!
  const steps = Math.round((seconds * 16000) / chunk)
  const audio = noise(chunk)
  const ref = await ort.InferenceSession.create(modelBytes(name), { executionProviders: ['wasm'] })
  const binding = engine.session(modelBytes(name)).bind({ ...spec.shapes }, spec.loops)
  for (let round = 0; round < 3; round++) {
    let state = new Float32Array(spec.shapes.state[0]!)
    let t = performance.now()
    for (let i = 0; i < steps; i++) {
      const feeds: Record<string, ort.Tensor> = { audio: new ort.Tensor('float32', audio, [1, chunk]), state: new ort.Tensor('float32', state, [state.length]) }
      if (name === 'focus') feeds.print = new ort.Tensor('float32', new Float32Array(FOCUS.print), [1, FOCUS.print])
      state = (await ref.run(feeds)).next_state!.data as Float32Array
    }
    const web = (performance.now() - t) / seconds
    t = performance.now()
    for (let i = 0; i < steps; i++) {
      binding.f32('audio').set(audio)
      binding.run()
    }
    const mine = (performance.now() - t) / seconds
    console.log(`${name}: onnxruntime-web ${web.toFixed(1)} ms per s of audio, alkove ${mine.toFixed(1)} (${((mine / web - 1) * 100).toFixed(0)} %)`)
  }
}
