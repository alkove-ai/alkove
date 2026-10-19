import { WasmEngine } from '../src/core/engine'
import { FOCUS, LEX, TIMBRE } from '../src/manifest'
import { loadEngine, modelBytes, noise } from '../test/helpers'

const engine = new WasmEngine(await loadEngine(), modelBytes)
const seconds = 10

for (const name of ['focus', 'cue', 'spot'] as const) {
  const net = engine.stream(name)
  const chunk = net.f32('audio').length
  const steps = Math.round((seconds * 16000) / chunk)
  const audio = noise(chunk)
  if (name === 'focus') net.f32('print').set(noise(FOCUS.print))
  const runs: number[] = []
  for (let round = 0; round < 3; round++) {
    const t = performance.now()
    for (let i = 0; i < steps; i++) {
      net.f32('audio').set(audio)
      net.run()
    }
    runs.push((performance.now() - t) / seconds)
  }
  console.log(`${name}: ${runs.map((r) => r.toFixed(1)).join(', ')} ms per s of audio`)
  net.release()
}

const timbre = engine.net('timbre')
const window = noise(TIMBRE.window)
let t = performance.now()
for (let i = 0; i < 20; i++) timbre.run({ audio: { data: window, dims: [1, TIMBRE.window] } })
console.log(`timbre: ${((performance.now() - t) / 20).toFixed(1)} ms per 1.5 s window`)

const encoder = engine.net('lex', 'encoder')
const decoder = engine.net('lex', 'decoder')
const word = { word: { data: new Int32Array(LEX.word).fill(97), dims: [1, LEX.word] }, lang: { data: Int32Array.of(0), dims: [1] } }
t = performance.now()
for (let i = 0; i < 20; i++) encoder.run(word)
console.log(`lex encoder: ${((performance.now() - t) / 20).toFixed(2)} ms per word`)
const { cross, pad } = encoder.run(word)
const beams = Int32Array.of(LEX.bos, LEX.bos, LEX.bos)
t = performance.now()
for (let i = 0; i < 200; i++) decoder.run({ token: { data: beams, dims: [LEX.beams] }, parent: { data: new Int32Array(LEX.beams), dims: [LEX.beams] }, pos: { data: Int32Array.of(i % LEX.steps), dims: [1] }, cross: cross!, pad: pad! })
console.log(`lex decoder: ${((performance.now() - t) / 200).toFixed(2)} ms per phoneme`)
