// page side of the browser tests: the built package, a file played as a microphone, a recorder, events kept with their time
import * as sdk from '/dist/index.js'

export { sdk }

// models from the test origin; the wasm from its default place, next to the package's files
export const CONFIG = { modelBaseUrl: '/models/' }

export function setup(extra = {}) {
  sdk.Alkove.configure({ ...CONFIG, ...extra })
}

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

export async function until(fn, ms = 20000, what = 'condition') {
  const t0 = performance.now()
  while (!(await fn())) {
    if (performance.now() - t0 > ms) throw new Error(`timeout after ${ms} ms: ${what}`)
    await sleep(20)
  }
}

export async function bytes(path) {
  const r = await fetch(path)
  if (!r.ok) throw new Error(`HTTP ${r.status}: ${path}`)
  return new Uint8Array(await r.arrayBuffer())
}

// the fixtures are 16-bit PCM mono WAV
export function wavPcm(u8) {
  const v = new DataView(u8.buffer, u8.byteOffset, u8.byteLength)
  let o = 12
  let rate = 16000
  while (o + 8 <= u8.length) {
    const id = String.fromCharCode(...u8.subarray(o, o + 4))
    const size = v.getUint32(o + 4, true)
    if (id === 'fmt ') rate = v.getUint32(o + 12, true)
    if (id === 'data') {
      const pcm = new Float32Array(size >> 1)
      for (let i = 0; i < pcm.length; i++) pcm[i] = v.getInt16(o + 8 + 2 * i, true) / 32768
      return { pcm, rate }
    }
    o += 8 + size + (size & 1)
  }
  throw new Error('no data chunk')
}

export async function fixture(name) {
  return wavPcm(await bytes(`/fixtures/${name}`))
}

// a file played into a MediaStream, the way a microphone delivers it
export class Player {
  constructor(ctx = new AudioContext()) {
    this.ctx = ctx
    this.dest = ctx.createMediaStreamDestination()
    this.stream = this.dest.stream
    this.sources = []
  }

  async load(name) {
    const { pcm, rate } = await fixture(name)
    const buf = this.ctx.createBuffer(1, pcm.length, rate)
    buf.getChannelData(0).set(pcm)
    return buf
  }

  // `times` plays `gap` s apart, the first `delay` s from now, at `gain`; the context time of the first sample
  play(buffer, { delay = 0.3, times = 1, gap = 0, gain = 1 } = {}) {
    let when = this.ctx.currentTime + delay
    const first = when
    const g = this.ctx.createGain()
    g.gain.value = gain
    g.connect(this.dest)
    for (let i = 0; i < times; i++) {
      const s = this.ctx.createBufferSource()
      s.buffer = buffer
      s.connect(g)
      s.start(when)
      this.sources.push(s)
      when += buffer.duration + gap
    }
    this.end = when
    return first
  }

  stop() {
    for (const s of this.sources) s.stop()
    this.sources = []
  }
}

const recorderLoaded = new WeakMap()

// a track recorded in `ctx` from now on; startTime is the context time of its first sample
export async function record(ctx, track) {
  if (!recorderLoaded.has(ctx)) recorderLoaded.set(ctx, ctx.audioWorklet.addModule('/browser/recorder.js'))
  await recorderLoaded.get(ctx)
  const src = ctx.createMediaStreamSource(new MediaStream([track]))
  const node = new AudioWorkletNode(ctx, 'alkove-test-recorder', { numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [1] })
  const mute = ctx.createGain()
  mute.gain.value = 0
  src.connect(node).connect(mute).connect(ctx.destination)
  const chunks = []
  let frame = null
  node.port.onmessage = (e) => {
    if (frame === null) frame = e.data.frame
    chunks.push(e.data.pcm)
  }
  return {
    rate: ctx.sampleRate,
    get startTime() {
      return frame === null ? null : frame / ctx.sampleRate
    },
    stop() {
      src.disconnect()
      node.disconnect()
      return concat(chunks)
    },
  }
}

export function concat(parts) {
  const out = new Float32Array(parts.reduce((n, p) => n + p.length, 0))
  let o = 0
  for (const p of parts) {
    out.set(p, o)
    o += p.length
  }
  return out
}

// what crosses back to Node: no typed arrays, no class instances
export function plain(v) {
  if (v instanceof Float32Array) return { samples: v.length }
  if (v instanceof ArrayBuffer) return { bytes: v.byteLength }
  if (v && typeof v.toJSON === 'function') return { json: v.toJSON() }
  if (v && typeof v === 'object') return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, plain(x)]))
  return v
}

// every event of an input, with the input's clock when it arrived
export function watch(input, types = ['mode', 'status', 'speechstart', 'speechend', 'wake', 'diag', 'learn', 'voiceprint', 'take', 'enrolled']) {
  const log = []
  for (const type of types) input.on(type, (v) => log.push({ type, now: input.now(), v: plain(v) }))
  return log
}

export function b64(f32) {
  const u8 = new Uint8Array(f32.buffer, f32.byteOffset, f32.byteLength)
  let s = ''
  for (let i = 0; i < u8.length; i += 0x8000) s += String.fromCharCode.apply(null, u8.subarray(i, i + 0x8000))
  return btoa(s)
}

export function rms(x, from = 0, to = x.length) {
  let s = 0
  for (let i = from; i < to; i++) s += x[i] * x[i]
  return Math.sqrt(s / Math.max(1, to - from))
}

// input.record()'s stereo WAV: left the mic, right the output, both 16 kHz
export async function stereo(blob) {
  const { pcm, rate } = wavPcm(new Uint8Array(await blob.arrayBuffer()))
  const n = pcm.length >> 1
  const left = new Float32Array(n)
  const right = new Float32Array(n)
  for (let i = 0; i < n; i++) {
    left[i] = pcm[2 * i]
    right[i] = pcm[2 * i + 1]
  }
  return { left, right, rate }
}
