import { beforeAll, describe, expect, it } from 'vitest'

import { WasmEngine } from '../src/core/engine'
import { Lex } from '../src/core/lex'
import { Enroller, type Take } from '../src/core/spot/enroll'
import { compileWord, Spot } from '../src/core/spot/spot'
import { processPcm } from '../src/process'
import { loadEngine, modelBytes, wav } from './helpers'

let engine: WasmEngine
beforeAll(async () => {
  engine = new WasmEngine(await loadEngine(), modelBytes)
})

const join = (...parts: Float32Array[]) => {
  const out = new Float32Array(parts.reduce((n, p) => n + p.length, 0))
  let o = 0
  for (const p of parts) {
    out.set(p, o)
    o += p.length
  }
  return out
}

describe('enrolling a word', () => {
  it('three takes set a threshold under them and over the rest, which then finds the word', () => {
    const lex = () => new Lex(engine.net('lex', 'encoder'), engine.net('lex', 'decoder'))
    const typed = compileWord('amendement', { lang: 'fr' }, lex)
    const speech = wav('speech.wav')
    const quiet = new Float32Array(16000)
    const audio = join(speech, quiet, speech, quiet, speech, quiet)
    const enroller = new Enroller(typed.text, typed.lang, typed.phonemes, 'raw')
    const takes: Take[] = []
    const spot = new Spot(engine.stream('spot'))
    spot.push(audio, (f) => enroller.frame(f, (t) => takes.push(t)))
    const ok = takes.filter((t) => t.ok)
    expect(ok.length).toBe(3)
    expect(enroller.done).toBe(true)
    const { word } = enroller.result()
    expect(word.calibrated).toBe(true)
    expect(word.thresholds.anywhere).toBeLessThanOrEqual(Math.min(...ok.map((t) => t.score)))
    expect(word.thresholds.anywhere).toBeGreaterThanOrEqual(-2.2)
    expect(word.thresholds.pause).toBeLessThan(word.thresholds.anywhere)
    const r = processPcm(engine, speech, { spot: { words: [word] } })
    expect(r.wakes.map((w) => w.word)).toEqual(['amendement'])
    const other = processPcm(engine, wav('enroll.wav'), { spot: { words: [word] } })
    expect(other.wakes).toEqual([])
  })
})
