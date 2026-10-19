import { WasmEngine } from '../src/core/engine'
import { Lex } from '../src/core/lex'
import { compileWord } from '../src/core/spot/spot'
import { processPcm } from '../src/process'
import { loadEngine, modelBytes, wav } from '../test/helpers'

const engine = new WasmEngine(await loadEngine(), modelBytes)
const mix = wav('mix.wav')
const minute = new Float32Array(60 * 16000)
for (let i = 0; i < minute.length; i++) minute[i] = mix[i % mix.length]!
const word = compileWord('amendement', { lang: 'fr' }, () => new Lex(engine.net('lex', 'encoder'), engine.net('lex', 'decoder')))
const cases = {
  'focus + cue (call)': { focus: true, cue: true, normalize: true },
  'focus + cue + spot': { focus: true, cue: true, spot: { words: [word] } },
  'spot (wake)': { spot: { words: [word] } },
  'cue alone': { cue: true },
} as const
for (const [name, opts] of Object.entries(cases)) {
  const runs: number[] = []
  for (let k = 0; k < 3; k++) {
    const t = performance.now()
    processPcm(engine, minute, opts)
    runs.push((performance.now() - t) / 60)
  }
  console.log(`${name}: ${runs.map((r) => r.toFixed(1)).join(' / ')} ms per s of audio`)
}
