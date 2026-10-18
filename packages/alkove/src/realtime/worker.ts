import factory from 'alkove:ort-glue'

import { Chain, type ChainOptions } from '../core/chain'
import { Cue } from '../core/cue'
import { WasmEngine } from '../core/engine'
import { Focus } from '../core/focus'
import { LEARN, rms, VoiceprintLearner } from '../core/learner'
import { ENROLL, Enroller } from '../core/spot/enroll'
import { type Context, Spot, Spotter, type WordJSON, wordOf } from '../core/spot/spot'
import { dot, Timbre as TimbreCore, Voiceprint, type VoiceprintJSON } from '../core/timbre'
import { Resampler } from '../dsp/resample'
import { encodeWav } from '../dsp/wav'
import { Ort } from '../engine/ort'
import { FOCUS, type ModelName, RATE, TIMBRE } from '../manifest'
import { makeCalls } from '../runtime/calls'
import { HOP } from './core'

declare const self: DedicatedWorkerGlobalScope

const models = new Map<ModelName, Uint8Array>()
let engine: WasmEngine | null = null
const post = (m: unknown, transfer: Transferable[] = []) => self.postMessage(m, transfer)

async function boot(module: WebAssembly.Module): Promise<void> {
  const ort = await Ort.create(factory, module)
  engine = new WasmEngine(ort, (name) => {
    const b = models.get(name)
    if (!b) throw new Error(`model ${name} not sent`)
    return b
  })
}

// ---- real time: every hop the worklet sends, through the input's pipeline; events batched to the main thread

const EVENT_MS = 100
const TIMING_HOPS = 125
const TRACE_EVERY = 6
// learning windows: 1.5 s of Focus's output while the user talks, one every 0.5 s; cleared after 1 s without speech
const WINDOW_HOP = 0.5 * RATE
const WINDOW_RESET = 1 * RATE
const PREROLL = 0.3

export interface OutputOptions {
  format: 'f32' | 's16'
  rate: number
  chunkMs: number
}

export interface RealtimeOptions {
  pipeline: 'call' | 'agent' | 'wake' | 'enroll'
  enroll?: { word: WordJSON; takes: number }
  verify?: boolean
  chain?: ChainOptions
  words?: WordJSON[]
  context?: Context
  learn?: boolean
  keepRecent?: number
  trace?: boolean
  stats?: boolean
  output?: OutputOptions & { audio: boolean; utterance: boolean }
}

class Packer {
  private resampler: Resampler
  private buf: Float32Array
  private fill = 0

  constructor(
    readonly opts: OutputOptions,
    private send: (chunk: ArrayBuffer) => void,
  ) {
    this.resampler = new Resampler(RATE, opts.rate)
    this.buf = new Float32Array(Math.round((opts.rate * opts.chunkMs) / 1000))
  }

  push(pcm: Float32Array): void {
    this.resampler.push(pcm, (x) => {
      this.buf[this.fill++] = x
      if (this.fill === this.buf.length) {
        this.send(encode(this.buf, this.opts.format))
        this.fill = 0
      }
    })
  }
}

function encode(pcm: Float32Array, format: 'f32' | 's16'): ArrayBuffer {
  if (format === 'f32') return pcm.slice().buffer
  const out = new Int16Array(pcm.length)
  for (let i = 0; i < pcm.length; i++) {
    const x = Math.max(-1, Math.min(1, pcm[i]!))
    out[i] = x < 0 ? x * 32768 : x * 32767
  }
  return out.buffer
}

interface Realtime {
  id: number
  timer: ReturnType<typeof setInterval>
  opts: RealtimeOptions
  chain: Chain | null
  spot: { net: Spot; spotter: Spotter; shift: number } | null
  events: unknown[]
  shift: number
  timing: { n: number; sum: number; max: number; slow: number }
  learn: { buf: Float32Array; fill: number; fresh: number; silent: number } | null
  bg: MessagePort | null
  out: MessagePort | null
  farQuietMs: number
  record: { left: Float32Array; right: Float32Array; at: number; id: number } | null
  recent: { raw: Float32Array; out: Float32Array; pos: number; full: boolean } | null
  stats: { speech: number; hops: number; rawQuiet: number; outQuiet: number } | null
  audio: Packer | null
  history: { buf: Float32Array; pos: number; total: number } | null
  utterance: { parts: Float32Array[]; start: number } | null
  hops: number
  enroll: { net: Spot; enroller: Enroller; audio: Float32Array[]; sent: boolean } | null
  tape: Float32Array | null
  written: number
  held: { e: Record<string, unknown>; ready: number; key: number }[]
}

const inputs = new Map<number, Realtime>()

function emitTo(r: Realtime, e: Record<string, unknown>): void {
  const s = r.shift
  if (typeof e.start === 'number') e.start += s
  if (typeof e.end === 'number') e.end += s
  if (typeof e.at === 'number') e.at += s
  r.events.push(e)
}

function realtime(id: number, opts: RealtimeOptions, bg: MessagePort | null, out: MessagePort | null): Realtime {
  const e = engine!
  const chain = opts.pipeline === 'wake' || opts.pipeline === 'enroll' ? null : new Chain(new Focus(e.stream('focus')), new Cue(e.stream('cue')), opts.chain)
  const o = opts.output
  const r: Realtime = {
    id,
    timer: setInterval(() => flush(r), EVENT_MS),
    opts,
    chain,
    spot: null,
    events: [],
    shift: 0,
    timing: { n: 0, sum: 0, max: 0, slow: 0 },
    learn: opts.learn && chain ? { buf: new Float32Array(TIMBRE.window), fill: 0, fresh: 0, silent: 0 } : null,
    bg,
    out,
    farQuietMs: Infinity,
    record: null,
    recent: opts.keepRecent ? { raw: new Float32Array(opts.keepRecent * RATE), out: new Float32Array(opts.keepRecent * RATE), pos: 0, full: false } : null,
    stats: opts.stats ? { speech: 0, hops: 0, rawQuiet: 0, outQuiet: 0 } : null,
    audio: null,
    history: o?.utterance ? { buf: new Float32Array(4 * RATE), pos: 0, total: 0 } : null,
    utterance: null,
    hops: 0,
    enroll: null,
    tape: chain ? null : new Float32Array(TAPE),
    written: 0,
    held: [],
  }
  if (opts.enroll) startEnroll(r, opts.enroll)
  if (bg && opts.verify)
    bg.addEventListener('message', (ev: MessageEvent) => {
      if (ev.data.type !== 'verified') return
      const i = r.held.findIndex((h) => h.key === ev.data.key)
      if (i < 0) return
      const [h] = r.held.splice(i, 1)
      if (ev.data.same) emitTo(r, { ...h!.e, voice: ev.data.score })
      else emitTo(r, { type: 'diag', code: 'wake.other-voice', level: 'info', data: { word: h!.e.word, voice: +ev.data.score.toFixed(3) } })
    })
  if (o?.audio) r.audio = new Packer(o, (chunk) => send(r, { type: 'audio', pcm: chunk }, [chunk]))
  if (opts.words?.length) setWords(r, opts.words, opts.context ?? 'anywhere')
  if (chain)
    chain.onFocus = (hop, t) => {
      feedSpot(r, hop)
      if (r.learn) tapLearn(r, hop)
      if (r.enroll) {
        r.tape!.set(hop, r.written % TAPE)
        r.written += HOP
        feedEnroll(r, hop)
      }
      if (opts.trace && (t / HOP) % TRACE_EVERY === 0) {
        let s = 0
        for (const x of hop) s += x * x
        emitTo(r, { type: 'trace', primitive: 'focus', t: t / RATE - FOCUS.lag / RATE, db: +(10 * Math.log10(s / HOP + 1e-12)).toFixed(1) })
      }
    }
  inputs.set(id, r)
  return r
}

// audio and utterances go to the app's own port when it gave one, else to the main thread
function send(r: Realtime, m: Record<string, unknown>, transfer: Transferable[]): void {
  if (r.out) r.out.postMessage(m, transfer)
  else post({ ...m, kind: m.type, type: 'pcm', input: r.id }, transfer)
}

function setWords(r: Realtime, words: WordJSON[], context: Context): void {
  const list = words.map(wordOf)
  if (!r.spot) r.spot = { net: new Spot(engine!.stream('spot')), spotter: new Spotter(list, context), shift: r.chain ? FOCUS.lag / RATE : 0 }
  else r.spot.spotter = new Spotter(list, context)
}

// on the raw mic, a wake carries its audio; with onlyMyVoice it waits 0.5 s more for Timbre in the background worker
const VERIFY_AFTER = 0.5 * RATE
let keys = 0

function onWake(r: Realtime, w: Record<string, unknown> & { start: number; end: number }): void {
  if (!r.tape) return emitTo(r, w)
  if (!r.opts.verify || !r.bg) return emitTo(r, { ...w, audio: cut(r, w.start, w.end) })
  r.held.push({ e: w, ready: Math.round(w.end * RATE) + VERIFY_AFTER, key: ++keys })
}

function cut(r: Realtime, start: number, end: number): Float32Array {
  const now = r.written
  const from = Math.max(now - TAPE + HOP, Math.round(start * RATE))
  const to = Math.min(now, Math.round(end * RATE))
  const out = new Float32Array(Math.max(0, to - from))
  for (let i = 0; i < out.length; i++) out[i] = r.tape![(from + i) % TAPE]!
  return out
}

function release(r: Realtime): void {
  for (const h of r.held) {
    if (h.ready > r.written || h.e.audio) continue
    const e = h.e as { start: number; end: number }
    h.e.audio = cut(r, e.start, e.end)
    const pcm = cut(r, e.start, e.end + VERIFY_AFTER / RATE)
    r.bg!.postMessage({ type: 'verify', key: h.key, pcm }, [pcm.buffer])
  }
}

function feedSpot(r: Realtime, pcm: Float32Array): void {
  const s = r.spot
  if (!s) return
  s.net.push(pcm, (f, step) => {
    s.spotter.frame(f, (w) => onWake(r, { type: 'wake', ...w, start: Math.max(0, w.start - s.shift), end: Math.max(0, w.end - s.shift) }))
    if (r.opts.trace && step % 3 === 0) emitTo(r, { type: 'trace', primitive: 'spot', t: step * 0.03 - s.shift, speech: f.speech ?? 0, scores: s.spotter.scores })
    if (!r.chain && f.speech !== null && step % 10 === 0) emitTo(r, { type: 'heard', speech: f.speech, scores: s.spotter.scores })
  })
}

// the last seconds of the mic, to cut the takes out of for the voiceprint
const TAPE = 8 * RATE

function startEnroll(r: Realtime, spec: { word: WordJSON; takes: number } | null): void {
  r.enroll?.net.release()
  r.enroll = null
  if (!spec) return
  const w = wordOf(spec.word)
  const chain = r.chain ? 'focus' : 'raw'
  r.enroll = { net: new Spot(engine!.stream('spot')), enroller: new Enroller(w.text, w.lang, w.phonemes, chain, { ...ENROLL, takes: spec.takes }), audio: [], sent: false }
  if (!r.tape) {
    r.tape = new Float32Array(TAPE)
    r.written = 0
  }
}

function feedEnroll(r: Realtime, hop: Float32Array): void {
  const en = r.enroll!
  const shift = r.chain ? FOCUS.lag / RATE : 0
  en.net.push(hop, (f) =>
    en.enroller.frame(f, (take) => {
      emitTo(r, { type: 'take', ...take, start: Math.max(0, take.start - shift), end: Math.max(0, take.end - shift) })
      if (take.ok) en.audio.push(cut(r, take.start, take.end))
    }),
  )
  if (en.enroller.done && !en.sent) {
    en.sent = true
    try {
      const { word, takes, warnings } = en.enroller.result()
      const audio = concat(en.audio)
      emitTo(r, { type: 'enrolled', word: word.toJSON(), takes, warnings, audio })
    } catch (err) {
      emitTo(r, { type: 'enrolled', error: String((err as Error).message) })
    }
  }
}

function tapLearn(r: Realtime, hop: Float32Array): void {
  const l = r.learn!
  if (!r.chain!.speaking) {
    l.silent += HOP
    if (l.silent >= WINDOW_RESET) l.fill = l.fresh = 0
    return
  }
  l.silent = 0
  if (l.fill + HOP > l.buf.length) {
    l.buf.copyWithin(0, HOP)
    l.fill -= HOP
  }
  l.buf.set(hop, l.fill)
  l.fill += HOP
  l.fresh += HOP
  if (l.fresh < WINDOW_HOP || l.fill < 0.9 * l.buf.length || !r.bg) return
  l.fresh = 0
  const win = l.buf.slice(0, l.fill)
  r.bg.postMessage({ type: 'window', pcm: win, farQuietMs: r.farQuietMs }, [win.buffer])
}

function onHop(r: Realtime, buf: ArrayBuffer, port: MessagePort): void {
  const f = new Float32Array(buf)
  const hop = f.subarray(0, HOP)
  r.farQuietMs = f[HOP]!
  const raw = r.record || r.recent || r.stats ? hop.slice() : null
  const t0 = performance.now()
  let out: Float32Array
  if (r.chain) out = r.chain.step(hop, (e) => emitTo(r, e))
  else {
    r.tape!.set(hop, r.written % TAPE)
    r.written += HOP
    feedSpot(r, hop)
    if (r.enroll) feedEnroll(r, hop)
    if (r.held.length) release(r)
    out = hop
  }
  const ms = performance.now() - t0
  f.set(out)
  port.postMessage(buf, [buf])
  r.hops++
  if (r.audio) r.audio.push(out)
  if (r.history) keepHistory(r, out)
  timing(r, ms)
  if (raw) extras(r, raw, out)
}

function timing(r: Realtime, ms: number): void {
  const tm = r.timing
  tm.n++
  tm.sum += ms
  tm.max = Math.max(tm.max, ms)
  if (ms > (HOP / RATE) * 1000) tm.slow++
  if (tm.n < TIMING_HOPS) return
  emitTo(r, { type: 'diag', code: 'timing', level: tm.slow > tm.n / 10 ? 'warn' : 'info', data: { meanMs: +(tm.sum / tm.n).toFixed(2), maxMs: +tm.max.toFixed(1), slow: tm.slow, asleep: r.chain?.sleeping ?? false, gainDb: r.chain ? +r.chain.gainDb.toFixed(1) : 0 } })
  r.timing = { n: 0, sum: 0, max: 0, slow: 0 }
}

// the output's last seconds, for the pre-roll of an utterance
function keepHistory(r: Realtime, out: Float32Array): void {
  const h = r.history!
  for (let i = 0; i < out.length; i++) {
    h.buf[h.pos] = out[i]!
    h.pos = (h.pos + 1) % h.buf.length
  }
  h.total += out.length
  if (r.utterance) r.utterance.parts.push(out.slice())
}

// output sample index of an input time: the chain's delay, the worklet clock shift
const outIndex = (r: Realtime, t: number) => Math.round((t - r.shift) * RATE) + (r.chain?.delay ?? 0)

function onSpeech(r: Realtime, e: { type: string; start: number; end?: number }): void {
  if (!r.history) return
  const h = r.history
  if (e.type === 'speechstart') {
    const at = outIndex(r, e.start)
    const from = Math.max(h.total - h.buf.length, at - Math.round(PREROLL * RATE))
    const head = new Float32Array(Math.max(0, h.total - from))
    for (let i = 0; i < head.length; i++) head[i] = h.buf[(h.pos - head.length + i + h.buf.length) % h.buf.length]!
    r.utterance = { parts: [head], start: e.start - (at - from) / RATE }
  } else if (e.type === 'speechend' && r.utterance) {
    const all = concat(r.utterance.parts)
    const keep = Math.max(0, all.length - Math.max(0, h.total - outIndex(r, e.end!)))
    const pcm = all.subarray(0, keep)
    const o = r.opts.output!
    const res = Resampler.all(pcm, RATE, o.rate)
    const data = encode(res, o.format)
    send(r, { type: 'utterance', pcm: data, start: r.utterance.start, end: e.end }, [data])
    r.utterance = null
  }
}

function concat(parts: Float32Array[]): Float32Array {
  const out = new Float32Array(parts.reduce((n, p) => n + p.length, 0))
  let o = 0
  for (const p of parts) {
    out.set(p, o)
    o += p.length
  }
  return out
}

function extras(r: Realtime, raw: Float32Array, out: Float32Array): void {
  if (r.stats) {
    r.stats.hops++
    if (r.chain?.speaking) r.stats.speech++
    else {
      r.stats.rawQuiet += energy(raw)
      r.stats.outQuiet += energy(out)
    }
  }
  if (r.record) {
    const rec = r.record
    const n = Math.min(HOP, rec.left.length - rec.at)
    rec.left.set(raw.subarray(0, n), rec.at)
    rec.right.set(out.subarray(0, n), rec.at)
    rec.at += n
    if (rec.at >= rec.left.length) {
      const wav = encodeWav([rec.left, rec.right], RATE)
      post({ type: 'reply', id: rec.id, result: wav }, [wav.buffer])
      r.record = null
    }
  }
  if (r.recent) {
    const k = r.recent
    for (let i = 0; i < HOP; i++) {
      k.raw[k.pos] = raw[i]!
      k.out[k.pos] = out[i]!
      k.pos = (k.pos + 1) % k.raw.length
      if (k.pos === 0) k.full = true
    }
  }
}

const energy = (x: Float32Array) => {
  let s = 0
  for (let i = 0; i < x.length; i++) s += x[i]! * x[i]!
  return s
}

function flush(r: Realtime): void {
  if (!r.events.length) return
  for (const e of r.events as { type: string; start: number; end?: number }[]) if (e.type === 'speechstart' || e.type === 'speechend') onSpeech(r, e)
  post({ type: 'events', input: r.id, list: r.events })
  r.events = []
}

function recentWav(r: Realtime | undefined): Uint8Array | null {
  const k = r?.recent
  if (!k) return null
  const order = (x: Float32Array) => (k.full ? concat([x.subarray(k.pos), x.subarray(0, k.pos)]) : x.slice(0, k.pos))
  return encodeWav([order(k.raw), order(k.out)], RATE)
}

// ---- background: Timbre, Lex, the learners (one per input), process()

interface Learning {
  ref: Voiceprint | null
  learner: VoiceprintLearner | null
  sent: Float32Array | null
  at: number
  port: MessagePort
}

const calls = makeCalls(() => engine!)
const learning = new Map<number, Learning>()
// one window every 1.5 s until the print holds 40, then one every 10 s; only while the far end and the agent's voice were quiet 2 s
const LEARN_GAP_MS = [1500, 10_000]
const LEARN_SLOW = 40
const LEARN_QUIET_MS = 2000

const { getTimbre } = calls
const event = (input: number, e: Record<string, unknown>) => post({ type: 'event', input, event: e })

function onWindow(input: number, pcm: Float32Array, farQuietMs: number): void {
  const l = learning.get(input)
  if (!l?.learner) return
  const now = performance.now()
  const count = l.learner.voiceprint?.windows.length ?? 0
  const gap = LEARN_GAP_MS[count >= LEARN_SLOW ? 1 : 0]!
  if (now - l.at < gap || farQuietMs < LEARN_QUIET_MS || rms(pcm) < LEARN.level) return
  l.at = now
  const t0 = performance.now()
  const r = l.learner.learn(getTimbre().window(pcm))
  const ms = Math.round(performance.now() - t0)
  event(input, { type: 'diag', code: r.accepted ? 'print.learn' : r.restarted ? 'print.restart' : 'print.reject', level: 'info', data: { score: +r.score.toFixed(3), progress: l.learner.progress, ms } })
  if (!r.accepted) return
  event(input, { type: 'learn', progress: l.learner.progress })
  const print = l.learner.voiceprint
  if (!print || (l.sent && dot(print.embedding, l.sent) >= LEARN.resend)) return
  l.sent = print.embedding
  l.port.postMessage({ type: 'print', embedding: print.embedding })
  event(input, { type: 'voiceprint', json: print.toJSON() })
}

type Rpc = { id: number; method: string; args: any[] }

function call({ id, method, args }: Rpc): unknown {
  switch (method) {
    case 'embed':
    case 'windows':
    case 'phonemize':
    case 'compile':
    case 'process':
    case 'streamOpen':
    case 'streamPush':
    case 'streamEnd':
      return (calls[method] as (...a: unknown[]) => unknown)(...args)
    case 'learner': {
      const l = learning.get(args[0])
      if (!l) throw new Error('no learning port for this input')
      const start = args[1] ? Voiceprint.fromJSON(args[1] as VoiceprintJSON) : null
      l.learner = new VoiceprintLearner((pcm) => getTimbre().window(pcm), 'focus', start)
      l.sent = start?.embedding ?? null
      return { progress: l.learner.progress }
    }
    case 'forget': {
      const l = learning.get(args[0])
      l?.learner?.forget()
      if (l) l.sent = null
      return null
    }
    case 'record': {
      const r = inputs.get(args[0])
      if (!r) throw new Error('no input')
      const n = Math.round((args[1] as number) * RATE)
      r.record = { left: new Float32Array(n), right: new Float32Array(n), at: 0, id }
      return undefined
    }
    case 'recent':
      return recentWav(inputs.get(args[0]))
    case 'stats': {
      const s = inputs.get(args[0])?.stats
      if (!s) return null
      return { speechSeconds: (s.speech * HOP) / RATE, seconds: (s.hops * HOP) / RATE, noiseRemovedDb: s.outQuiet > 0 ? +(10 * Math.log10(s.rawQuiet / s.outQuiet)).toFixed(1) : null }
    }
    default:
      throw new Error(`unknown method ${method}`)
  }
}

function dispose(input: number): void {
  const r = inputs.get(input)
  if (r) {
    clearInterval(r.timer)
    r.spot?.net.release()
    inputs.delete(input)
  }
  learning.get(input)?.port.close()
  learning.delete(input)
}

self.onmessage = async (e: MessageEvent) => {
  const m = e.data
  try {
    switch (m.type) {
      case 'boot':
        await boot(m.module)
        post({ type: 'booted' })
        break
      case 'models':
        for (const [name, bytes] of Object.entries(m.models as Record<ModelName, ArrayBuffer>)) models.set(name as ModelName, new Uint8Array(bytes))
        post({ type: 'reply', id: m.id, result: true })
        break
      case 'realtime': {
        const r = realtime(m.input, m.opts, m.bg ?? null, m.out ?? null)
        if (m.bg)
          (m.bg as MessagePort).onmessage = (ev: MessageEvent) => {
            if (ev.data.type === 'print') r.chain?.setPrint(ev.data.embedding)
          }
        post({ type: 'reply', id: m.id, result: true })
        break
      }
      case 'worklet': {
        const r = inputs.get(m.input)
        const port = m.port as MessagePort
        if (!r) break
        port.onmessage = (ev: MessageEvent) => {
          if (ev.data instanceof ArrayBuffer) onHop(r, ev.data, port)
          else if (typeof ev.data?.clock === 'number') r.shift = ev.data.clock / RATE - (r.chain?.time ?? (r.hops * HOP) / RATE)
        }
        break
      }
      case 'rt-port': {
        const l: Learning = { ref: m.ref ? Voiceprint.fromJSON(m.ref) : null, learner: null, sent: null, at: -Infinity, port: m.port }
        learning.set(m.input, l)
        l.port.onmessage = (ev: MessageEvent) => {
          if (ev.data.type === 'window') onWindow(m.input, ev.data.pcm, ev.data.farQuietMs)
          else if (ev.data.type === 'verify') {
            const pcm = ev.data.pcm as Float32Array
            const voice = pcm.length >= TIMBRE.min && l.ref ? TimbreCore.verify(l.ref, new Voiceprint([getTimbre().window(pcm)]), { seconds: pcm.length / RATE }) : { same: true, score: 1 }
            l.port.postMessage({ type: 'verified', key: ev.data.key, ...voice })
          }
        }
        break
      }
      case 'set': {
        const r = inputs.get(m.input)
        if (!r) break
        if ('strength' in m) r.chain?.setStrength(m.strength)
        if ('enabled' in m) r.chain?.setEnabled(m.enabled)
        if ('print' in m) r.chain?.setPrint(m.print)
        if ('words' in m) setWords(r, m.words, m.context ?? 'anywhere')
        if ('enroll' in m) startEnroll(r, m.enroll)
        break
      }
      case 'dispose':
        dispose(m.input)
        break
      case 'call': {
        const result = call(m as Rpc)
        if (result !== undefined) post({ type: 'reply', id: m.id, result })
        break
      }
    }
  } catch (err) {
    post({ type: 'reply', id: m.id, error: { message: String((err as Error)?.message ?? err), code: (err as { code?: string })?.code } })
  }
}
