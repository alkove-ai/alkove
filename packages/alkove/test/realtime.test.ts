import { describe, expect, it } from 'vitest'

import { type CoreMessage, HOP, RAW_HOP, WIRE, WorkletCore } from '../src/realtime/core'

const RATE = 48000
const Q = 128
const quantumMs = (Q / RATE) * 1000

// a context rate worklet, a worker that answers each hop after `lagMs` (or never), a fake clock moved one quantum at a time
function rig({ lagMs = 4, delay16 = 0, answer = (h: Float32Array) => h.slice(0, WIRE) } = {}) {
  let now = 0
  const messages: CoreMessage[] = []
  const core = new WorkletCore(RATE, delay16, () => now, (m) => messages.push(m))
  const pending: { at: number; hop: Float32Array }[] = []
  let alive = true
  core.connect((hop) => {
    if (alive) pending.push({ at: now + lagMs, hop: answer(hop) })
  })
  const out: number[] = []
  const tone = (i: number) => Math.sin((2 * Math.PI * 440 * i) / RATE) * 0.5
  let t = 0
  const step = (n: number) => {
    for (let k = 0; k < n; k++) {
      now += quantumMs
      while (pending.length && pending[0]!.at <= now) core.receive(pending.shift()!.hop)
      const input = Float32Array.from({ length: Q }, (_, i) => tone(t + i))
      const output = new Float32Array(Q)
      core.process(input, null, output)
      out.push(...output)
      t += Q
    }
  }
  return { core, messages, out, step, tone, kill: () => (alive = false), revive: () => (alive = true), now: () => now }
}

const quanta = (ms: number) => Math.ceil(ms / quantumMs)

// in place, each sample `samples` later: the chain's delay as a worker adds it
function delay(samples: number) {
  const buf = new Float32Array(samples)
  let pos = 0
  return (x: Float32Array) => {
    for (let i = 0; i < x.length; i++) {
      const out = buf[pos]!
      buf[pos] = x[i]!
      x[i] = out
      pos = (pos + 1) % samples
    }
  }
}

describe('worklet core with a fake clock', () => {
  it('a worker that gives the hop back: the mic comes out `latency` samples late', () => {
    const r = rig()
    r.step(quanta(1000))
    const L = r.core.latency
    let err = 0
    for (let i = RATE / 2; i < RATE; i++) err = Math.max(err, Math.abs(r.out[i]! - r.tone(i - L)))
    expect(err).toBeLessThan(0.02)
    expect(r.core.mode).toBe('active')
    expect(r.messages.filter((m) => m.type === 'overload')).toEqual([])
  })

  it('a worker whose chain is 1856 samples late (call): the announced latency is the real one', () => {
    const chain = delay(1856)
    const r = rig({
      delay16: 1856,
      answer: (h) => {
        const w = h.slice(0, WIRE)
        chain(w.subarray(0, HOP))
        return w
      },
    })
    r.step(quanta(1000))
    const L = r.core.latency
    let err = 0
    for (let i = RATE / 2; i < RATE; i++) err = Math.max(err, Math.abs(r.out[i]! - r.tone(i - L)))
    expect(err).toBeLessThan(0.02)
    expect(Math.round(r.core.latencyMs)).toBe(149)
  })

  it('raw output keeps the same delay: a switch moves nothing in time', () => {
    const r = rig()
    r.kill()
    r.step(quanta(600))
    expect(r.core.mode).toBe('raw')
    const L = r.core.latency
    let err = 0
    for (let i = r.out.length - RATE / 10; i < r.out.length; i++) err = Math.max(err, Math.abs(r.out[i]! - r.tone(i - L)))
    expect(err).toBeLessThan(1e-6)
  })

  it('a worker that hangs: raw after 250 ms, the models again 10 s later', () => {
    const r = rig()
    r.step(quanta(500))
    r.kill()
    r.step(quanta(300))
    const overloads = r.messages.filter((m) => m.type === 'overload')
    expect(overloads.length).toBe(1)
    expect(r.core.mode).toBe('raw')
    r.revive()
    r.step(quanta(9000))
    expect(r.messages.some((m) => m.type === 'retry')).toBe(false)
    r.step(quanta(1500))
    expect(r.messages.some((m) => m.type === 'retry')).toBe(true)
    expect(r.core.mode).toBe('active')
  })

  it('a slow worker (17 ms a hop): starvation budget spent, raw, then twice longer before the next try', () => {
    let last = 0
    let now = 0
    const r = rig({ lagMs: 0 })
    // worker answering one hop every 17 ms whatever comes: slower than the 16 ms real time
    const core = new WorkletCore(RATE, 256, () => now, (m) => r.messages.push(m))
    const queue: Float32Array[] = []
    core.connect((h) => queue.push(h.slice(0, WIRE)))
    const out = new Float32Array(Q)
    let retries = 0
    for (let k = 0; k < quanta(60_000); k++) {
      now += quantumMs
      if (now - last >= 17 && queue.length) {
        core.receive(queue.shift()!)
        last = now
      }
      core.process(new Float32Array(Q).fill(0.1), null, out)
    }
    const overloads = r.messages.filter((m) => m.type === 'overload').length
    retries = r.messages.filter((m) => m.type === 'retry').length
    expect(overloads).toBeGreaterThanOrEqual(2)
    expect(retries).toBe(overloads - 1)
  })

  it('agent input in raw mode: 16 kHz hops to the main thread before the worker, to the worker (marked raw) during an overload', () => {
    let now = 0
    const messages: CoreMessage[] = []
    const core = new WorkletCore(RATE, 0, () => now, (m) => messages.push(m), true)
    const out = new Float32Array(Q)
    const run = (ms: number) => {
      for (let k = 0; k < quanta(ms); k++) {
        now += quantumMs
        core.process(new Float32Array(Q).fill(0.1), null, out)
      }
    }
    run(1000)
    const raw = messages.filter((m) => m.type === 'raw')
    expect(raw.length).toBeGreaterThanOrEqual(61)
    expect(raw.every((m) => m.type === 'raw' && m.pcm.length === HOP)).toBe(true)
    const sent: Float32Array[] = []
    core.connect((h) => sent.push(h.slice(0, WIRE)))
    run(300)
    expect(core.mode).toBe('raw')
    const before = sent.length
    run(1000)
    const tapped = sent.slice(before)
    expect(tapped.length).toBeGreaterThanOrEqual(61)
    expect(tapped.every((h) => h[HOP + 1] === RAW_HOP)).toBe(true)
    expect(messages.filter((m) => m.type === 'raw').length).toBe(raw.length)
  })

  it('engine crashed (stop): raw, and a new connect brings the models back', () => {
    const r = rig()
    r.step(quanta(200))
    r.core.stop()
    r.step(quanta(200))
    expect(r.core.mode).toBe('raw')
    r.core.connect(() => {})
    expect(r.core.mode).toBe('active')
  })

  it('far end: quiet time counted, reset when the app says its voice plays', () => {
    const r = rig()
    r.step(quanta(500))
    expect(r.core.farQuietMs).toBeGreaterThan(400)
    r.core.setFarActive(true)
    r.step(1)
    expect(r.core.farQuietMs).toBe(0)
  })
})
