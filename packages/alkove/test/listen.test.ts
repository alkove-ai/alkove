import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { beforeAll, describe, expect, it } from 'vitest'

import { WasmEngine } from '../src/core/engine'
import type { NativePipeline } from '../src/core/native'
import { Alk } from '../src/engine/alk'
import { modelBytes, wav } from './helpers'

// who makes the agent's phrases: Cue, nobody (listen off, the phrase under way dropped), or the app between hold and release
const HOP = 256
const OUTPUT = { format: 'f32', rate: 16000, chunkMs: 32, audio: false, utterance: true }

let engine: WasmEngine
beforeAll(async () => {
  engine = new WasmEngine(await Alk.create(await WebAssembly.compile(readFileSync(join(import.meta.dirname, '../dist/alkove-engine.wasm')))), modelBytes)
})

const silence = (seconds: number) => new Float32Array(Math.round(seconds * 16000))
const concat = (...parts: Float32Array[]) => {
  const out = new Float32Array(parts.reduce((n, p) => n + p.length, 0))
  let o = 0
  for (const p of parts) {
    out.set(p, o)
    o += p.length
  }
  return out
}

// three copies of speech.wav, one phrase each for Cue
const speech = wav('speech.wav')
const audio = concat(silence(1), speech, silence(1.5), speech, silence(1.5), speech, silence(1.5))
const hop = (t: number) => Math.round((t * 16000) / HOP)

type Event = Record<string, unknown>

function feed(p: NativePipeline, at: Record<number, Event> = {}) {
  const events: Event[] = []
  const utterances: { start: number; end: number; samples: number }[] = []
  for (let k = 0; (k + 1) * HOP <= audio.length; k++) {
    if (at[k]) p.set(at[k]!)
    p.step(audio.subarray(k * HOP, (k + 1) * HOP), 0)
    for (const o of p.outs()) {
      if (o[0] === 'event' && String(o[1].type).startsWith('speech')) events.push(o[1])
      if (o[0] === 'utterance') utterances.push({ start: o[2], end: o[3], samples: o[1].byteLength / 4 })
    }
  }
  return { events, utterances }
}

const agent = (chain = {}) => engine.pipeline({ chain, output: OUTPUT })
const kinds = (events: Event[]) => events.map((e) => `${e.type} ${e.start}${e.end === undefined ? '' : ` ${e.end}`}`)
// Cue's phrases, alone: [start, end] of each copy's
let cue: [number, number][]
beforeAll(() => {
  const ends = feed(agent()).events.filter((e) => e.type === 'speechend')
  cue = ends.map((e) => [e.start as number, e.end as number])
})
const phrase = (i: number) => [`speechstart ${cue[i]![0]}`, `speechend ${cue[i]![0]} ${cue[i]![1]}`]

describe('listen', () => {
  it('Cue alone gives one phrase per copy', () => {
    expect(cue).toHaveLength(3)
    expect(cue[0]![0]).toBeLessThan(3)
    expect(cue[1]![0]).toBeGreaterThan(9)
    expect(cue[2]![1]).toBeLessThan(20)
  })

  it('off drops the phrase under way, back on mid-speech skips that speech, the next phrase comes', () => {
    const { events, utterances } = feed(agent(), { [hop(3)]: { listen: false }, [hop(10.5)]: { listen: true } })
    expect(kinds(events)).toEqual([phrase(0)[0], `speechcancel ${cue[0]![0]}`, ...phrase(2)])
    expect(utterances.map((u) => u.start)).toEqual([cue[2]![0] - 0.3])
  })
})

describe('hold and release', () => {
  it('make a phrase in a silence, its end sent once the output holds it', () => {
    const { events, utterances } = feed(agent(), { [hop(0.5)]: { hold: true }, [hop(1.5)]: { hold: false } })
    const [start, end] = [(hop(0.5) * HOP) / 16000, (hop(1.5) * HOP) / 16000]
    expect(kinds(events).slice(0, 2)).toEqual([`speechstart ${start}`, `speechend ${start} ${end}`])
    expect(utterances[0]!.samples).toBe(Math.round((end - start + 0.3) * 16000))
  })

  it('take over the phrase Cue opened and keep it past Cue’s end, then skip the speech under way', () => {
    const { events, utterances } = feed(agent(), { [hop(3)]: { hold: true }, [hop(6)]: { hold: false }, [hop(9.4)]: { hold: true }, [hop(10.5)]: { hold: false } })
    const t = (s: number) => (hop(s) * HOP) / 16000
    expect(kinds(events)).toEqual([phrase(0)[0], `speechend ${cue[0]![0]} ${t(6)}`, `speechstart ${t(9.4)}`, `speechend ${t(9.4)} ${t(10.5)}`, ...phrase(2)])
    expect(utterances[0]!.samples).toBe(Math.round((t(6) - cue[0]![0] + 0.3) * 16000))
  })

  it('with listening off, only the held phrases', () => {
    const { events } = feed(agent(), { [hop(0.2)]: { listen: false }, [hop(9.4)]: { hold: true }, [hop(10.5)]: { hold: false } })
    const t = (s: number) => (hop(s) * HOP) / 16000
    expect(kinds(events)).toEqual([`speechstart ${t(9.4)}`, `speechend ${t(9.4)} ${t(10.5)}`])
  })

  it('a held phrase carries its level', () => {
    const { events } = feed(agent({ levels: true }), { [hop(0.2)]: { listen: false }, [hop(9.4)]: { hold: true }, [hop(12.5)]: { hold: false } })
    const end = events.find((e) => e.type === 'speechend')!
    expect(end.voiceDb).toBeGreaterThan(-25)
    expect(end.voiceDb).toBeLessThan(-15)
  })
})

describe('hold with near', () => {
  it('silences the far voices the held phrase holds', () => {
    // two loud copies set the session's voice, then a hold over a loud copy and the same 20 dB under
    const held = concat(audio.subarray(0, Math.round(15.5 * 16000)), speech, speech.map((v) => v * 0.1), silence(1))
    const p = engine.pipeline({ chain: { near: true }, output: OUTPUT })
    const events: Event[] = []
    let pcm: Float32Array | null = null
    let start = 0
    for (let k = 0; (k + 1) * HOP <= held.length; k++) {
      if (k === hop(0.2)) p.set({ listen: false })
      if (k === hop(9)) p.set({ listen: true })
      if (k === hop(15.2)) p.set({ hold: true })
      if (k === hop(27)) p.set({ hold: false })
      p.step(held.subarray(k * HOP, (k + 1) * HOP), 0)
      for (const o of p.outs()) {
        if (o[0] === 'event') events.push(o[1])
        if (o[0] === 'utterance' && o[2] > 14) [pcm, start] = [new Float32Array(o[1].buffer.slice(o[1].byteOffset, o[1].byteOffset + o[1].byteLength)), o[2]]
      }
    }
    expect(pcm).not.toBeNull()
    const db = (from: number, to: number) => {
      const x = pcm!.subarray(Math.round((from - start) * 16000), Math.round((to - start) * 16000))
      return 10 * Math.log10(x.reduce((s, v) => s + v * v, 0) / x.length + 1e-12)
    }
    const loud = 15.5 + 1.264
    const quiet = 15.5 + 5.772 + 1.264
    expect(events.some((e) => e.code === 'speech.far')).toBe(true)
    expect(db(loud + 0.2, loud + 2.4)).toBeGreaterThan(-35)
    expect(db(quiet + 0.2, quiet + 2.4)).toBeLessThan(-80)
  })
})

describe('the review’s cases', () => {
  const t = (s: number) => (hop(s) * HOP) / 16000

  it('with near, back on during a far voice that never stops: the user near it opens a phrase', () => {
    // a TV 20 dB under, speech.wav's spoken part looped without a pause; the user over it from 6 s into it
    const spoken = speech.subarray(Math.round(1.2 * 16000), Math.round(3.9 * 16000))
    const tv = concat(...Array.from({ length: 5 }, () => spoken.map((v) => v * 0.1)))
    const from = 15.5
    const user = 6
    const mixed = concat(audio.subarray(0, Math.round(from * 16000)), tv, silence(1))
    mixed.set(
      spoken.map((v, i) => v + mixed[Math.round((from + user) * 16000) + i]!),
      Math.round((from + user) * 16000),
    )
    const p = engine.pipeline({ chain: { near: true }, output: OUTPUT })
    const events: Event[] = []
    for (let k = 0; (k + 1) * HOP <= mixed.length; k++) {
      if (k === hop(from - 0.5)) p.set({ listen: false })
      if (k === hop(from + 3)) p.set({ listen: true })
      p.step(mixed.subarray(k * HOP, (k + 1) * HOP), 0)
      for (const o of p.outs()) if (o[0] === 'event' && String(o[1].type).startsWith('speech')) events.push(o[1])
    }
    const late = events.filter((e) => e.type === 'speechstart' && (e.start as number) > from)
    expect(late).toHaveLength(1)
    expect(Math.abs((late[0]!.start as number) - (from + user))).toBeLessThan(0.3)
  })

  it('listen(true) during a hold holds after the release', () => {
    const { events } = feed(agent(), { [hop(0.2)]: { listen: false }, [hop(9)]: { hold: true }, [hop(9.2)]: { listen: true }, [hop(9.4)]: { hold: false } })
    expect(kinds(events)).toEqual([`speechstart ${t(9)}`, `speechend ${t(9)} ${t(9.4)}`, ...phrase(1), ...phrase(2)])
  })

  it('a hold dropped before its start went out sends nothing', () => {
    const p = agent()
    const events: Event[] = []
    for (let k = 0; (k + 1) * HOP <= audio.length; k++) {
      if (k === hop(0.2)) {
        p.set({ hold: true })
        p.set({ listen: false })
      }
      p.step(audio.subarray(k * HOP, (k + 1) * HOP), 0)
      for (const o of p.outs()) if (o[0] === 'event' && String(o[1].type).startsWith('speech')) events.push(o[1])
    }
    expect(events).toEqual([])
  })

  it('a hold in a silence has no voice level', () => {
    const { events } = feed(agent({ levels: true }), { [hop(0.2)]: { hold: true }, [hop(0.6)]: { hold: false } })
    const end = events.find((e) => e.type === 'speechend')!
    expect(end.voiceDb).toBeNull()
    expect(typeof end.noiseDb).toBe('number')
  })

  it('on a call, Cue starting right after a release leaves the held phrase whole', () => {
    // the gate's 1856 samples delay the held end: a Cue start with a short startSecs comes before it
    const p = engine.pipeline({ chain: { gate: true, speech: { startSecs: 0.03 } }, output: OUTPUT })
    const events: Event[] = []
    const rel = cue[0]![0] - 0.03
    for (let k = 0; (k + 1) * HOP <= audio.length; k++) {
      if (k === hop(0.5)) p.set({ hold: true })
      if (k === hop(rel)) p.set({ hold: false })
      p.step(audio.subarray(k * HOP, (k + 1) * HOP), 0)
      for (const o of p.outs()) if (o[0] === 'event' && String(o[1].type).startsWith('speech')) events.push(o[1])
    }
    expect(events[0]).toEqual({ type: 'speechstart', start: t(0.5) })
    expect(events[1]).toEqual({ type: 'speechend', start: t(0.5), end: t(rel) })
    for (const e of events.filter((e) => e.type === 'speechend')) expect(e.end as number).toBeGreaterThanOrEqual(e.start as number)
    expect(events.filter((e) => e.type === 'speechstart')).toHaveLength(events.filter((e) => e.type === 'speechend').length)
  })
})
