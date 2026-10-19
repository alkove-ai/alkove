import { beforeAll, describe, expect, it } from 'vitest'

import { WasmEngine } from '../src/core/engine'
import type { WordJSON } from '../src/core/spot/spot'
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
    const typed = engine.compile('amendement', ['fr']).word
    const speech = wav('speech.wav')
    const quiet = new Float32Array(16000)
    const enroller = engine.enroller(typed, 3, 'raw')
    const { takes, done } = enroller.push(join(speech, quiet, speech, quiet, speech, quiet))
    const ok = takes.filter((t) => t.ok) as { score: number }[]
    expect(ok.length).toBe(3)
    expect(done).toBe(true)
    const word = enroller.result().word as unknown as WordJSON
    enroller.release()
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
