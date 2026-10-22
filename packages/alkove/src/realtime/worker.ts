import type { ChainOptions } from '../core/chain'
import { WasmEngine } from '../core/engine'
import type { NativeCoach, NativePipeline } from '../core/native'
import { type Context, type WordJSON, wordOf } from '../core/spot/spot'
import { Alk } from '../engine/alk'
import { type ModelName, RATE } from '../manifest'
import { makeCalls } from '../runtime/calls'
import { HOP, RAW_HOP } from './core'
import type { OutputOptions } from './packer'

export type { OutputOptions }

declare const self: DedicatedWorkerGlobalScope

const models = new Map<ModelName, Uint8Array>()
let engine: WasmEngine | null = null
const post = (m: unknown, transfer: Transferable[] = []) => self.postMessage(m, transfer)

async function boot(module: WebAssembly.Module): Promise<void> {
  engine = new WasmEngine(await Alk.create(module), (name) => {
    const b = models.get(name)
    if (!b) throw new Error(`model ${name} not sent`)
    return b
  })
}

// ---- real time: every hop the worklet sends, through the input's pipeline in the core; events batched to the main thread

const EVENT_MS = 100
const TIMING_HOPS = 125

export interface RealtimeOptions {
  pipeline: 'call' | 'agent' | 'wake' | 'enroll'
  enroll?: { word: WordJSON; takes: number }
  verify?: boolean
  chain?: ChainOptions & { levels?: boolean; near?: boolean | { gapDb?: number } }
  words?: WordJSON[]
  context?: Context
  learn?: boolean
  keepRecent?: number
  trace?: boolean
  stats?: boolean
  output?: OutputOptions & { audio: boolean; utterance: boolean }
}

interface Realtime {
  id: number
  port: MessagePort | null
  closed: boolean
  timer: ReturnType<typeof setInterval>
  opts: RealtimeOptions
  p: NativePipeline
  events: unknown[]
  shift: number
  timing: { n: number; sum: number; max: number; slow: number }
  bg: MessagePort | null
  out: MessagePort | null
  recordId: number | null
}

const inputs = new Map<number, Realtime>()

function emitTo(r: Realtime, e: Record<string, unknown>): void {
  const s = r.shift
  if (typeof e.start === 'number') e.start += s
  if (typeof e.end === 'number') e.end += s
  if (typeof e.at === 'number') e.at += s
  const d = e.data as Record<string, unknown> | undefined
  if (e.type === 'diag' && typeof d?.start === 'number' && typeof d.end === 'number') {
    d.start += s
    d.end += s
  }
  r.events.push(e)
}

const words = (list: WordJSON[]) => list.map((w) => wordOf(w).toJSON())
const enrolling = (spec: { word: WordJSON; takes: number } | null | undefined) => (spec ? { word: wordOf(spec.word).toJSON(), takes: spec.takes } : null)

function realtime(id: number, opts: RealtimeOptions, bg: MessagePort | null, out: MessagePort | null): Realtime {
  const chained = opts.pipeline !== 'wake' && opts.pipeline !== 'enroll'
  const p = engine!.pipeline({
    chain: chained ? (opts.chain ?? {}) : null,
    verify: !!(opts.verify && bg),
    learn: !!(opts.learn && bg),
    keepRecent: opts.keepRecent ?? 0,
    trace: !!opts.trace,
    stats: !!opts.stats,
    output: opts.output ?? null,
    words: words(opts.words ?? []),
    context: opts.context ?? 'anywhere',
    enroll: enrolling(opts.enroll),
  })
  const r: Realtime = { id, port: null, closed: false, timer: setInterval(() => flush(r), EVENT_MS), opts, p, events: [], shift: 0, timing: { n: 0, sum: 0, max: 0, slow: 0 }, bg, out, recordId: null }
  if (bg && opts.verify)
    bg.addEventListener('message', (ev: MessageEvent) => {
      if (ev.data.type !== 'verified' || r.closed) return
      r.p.verified(ev.data.key, ev.data.same, ev.data.score)
      drain(r)
    })
  inputs.set(id, r)
  return r
}

// audio and utterances go to the app's own port when it gave one, else to the main thread
function send(r: Realtime, m: Record<string, unknown>, transfer: Transferable[]): void {
  if (r.out) r.out.postMessage(m, transfer)
  else post({ ...m, kind: m.type, type: 'pcm', input: r.id }, transfer)
}

// what the pipeline gave: events to batch, windows and wakes to the background, audio out
function drain(r: Realtime): void {
  for (const o of r.p.outs())
    switch (o[0]) {
      case 'event':
        emitTo(r, o[1])
        break
      case 'window':
        r.bg?.postMessage({ type: 'window', pcm: o[1], farQuietMs: o[2], t: o[3] }, [o[1].buffer])
        break
      case 'verify':
        r.bg?.postMessage({ type: 'verify', key: o[1], pcm: o[2] }, [o[2].buffer])
        break
      case 'audio':
        send(r, { type: 'audio', pcm: o[1].buffer }, [o[1].buffer])
        break
      case 'utterance':
        send(r, { type: 'utterance', pcm: o[1].buffer, start: o[2] + r.shift, end: o[3] + r.shift, ...(o[4] && { level: o[4] }) }, [o[1].buffer])
        break
      case 'record':
        post({ type: 'reply', id: r.recordId, result: o[1] }, [o[1].buffer])
        r.recordId = null
    }
}

// loading Spot blocks this worker for hundreds of ms: the worklet goes raw at the same delay meanwhile, instead of starving into an overload
function withSpot<T>(r: Realtime, make: () => T): T {
  const pause = !engine!.has('spot') && r.port
  if (pause) r.port!.postMessage({ pause: true })
  try {
    return make()
  } finally {
    if (pause) r.port!.postMessage({ pause: false })
  }
}

function onHop(r: Realtime, buf: ArrayBuffer, port: MessagePort): void {
  const f = new Float32Array(buf)
  const hop = f.subarray(0, HOP)
  if (f[HOP + 1] === RAW_HOP) {
    r.p.raw(hop)
    drain(r)
    port.postMessage(buf, [buf])
    return
  }
  const t0 = performance.now()
  const out = r.p.step(hop, f[HOP]!)
  const ms = performance.now() - t0
  f.set(out)
  port.postMessage(buf, [buf])
  drain(r)
  timing(r, ms)
}

function timing(r: Realtime, ms: number): void {
  const tm = r.timing
  tm.n++
  tm.sum += ms
  tm.max = Math.max(tm.max, ms)
  if (ms > (HOP / RATE) * 1000) tm.slow++
  if (tm.n < TIMING_HOPS) return
  const chained = r.p.chained
  emitTo(r, { type: 'diag', code: 'timing', level: tm.slow > tm.n / 10 ? 'warn' : 'info', data: { meanMs: +(tm.sum / tm.n).toFixed(2), maxMs: +tm.max.toFixed(1), slow: tm.slow, asleep: chained && r.p.sleeping, gainDb: chained ? +r.p.gainDb.toFixed(1) : 0 } })
  r.timing = { n: 0, sum: 0, max: 0, slow: 0 }
}

function flush(r: Realtime): void {
  if (!r.events.length) return
  post({ type: 'events', input: r.id, list: r.events })
  r.events = []
}

// ---- background: Timbre, Lex, the learners (one per input), process()

interface Learning {
  coach: NativeCoach
  port: MessagePort
}

const calls = makeCalls(() => engine!)
const learning = new Map<number, Learning>()

const event = (input: number, e: Record<string, unknown>) => post({ type: 'event', input, event: e })

// a window of the user's speech at sample `t` of the input: the print learns from it when the rules allow
function onWindow(input: number, pcm: Float32Array, farQuietMs: number, t: number): void {
  const l = learning.get(input)
  if (!l) return
  const t0 = performance.now()
  const outs = l.coach.window(engine!.timbre(), pcm, farQuietMs, t)
  const ms = Math.round(performance.now() - t0)
  for (const o of outs) {
    if (o[0] === 'print') l.port.postMessage({ type: 'print', embedding: o[1] })
    else {
      if (o[1].type === 'diag') (o[1].data as Record<string, unknown>).ms = ms
      event(input, o[1])
    }
  }
}

type Rpc = { id: number; method: string; args: any[]; progress?: boolean }

function call({ id, method, args, progress }: Rpc): unknown {
  switch (method) {
    case 'process':
      return calls.process(args[0], args[1], progress ? (p) => post({ type: 'progress', id, p }) : undefined)
    case 'embed':
    case 'learnerOpen':
    case 'learnerOffer':
    case 'learnerForget':
    case 'learnerClose':
    case 'phonemize':
    case 'compile':
    case 'streamOpen':
    case 'streamPush':
    case 'streamEnd':
      return (calls[method] as (...a: unknown[]) => unknown)(...args)
    case 'learner': {
      const l = learning.get(args[0])
      if (!l) throw new Error('no learning port for this input')
      return { progress: l.coach.learner(args[1] ?? null) }
    }
    case 'forget':
      learning.get(args[0])?.coach.forget()
      return null
    case 'record': {
      const r = inputs.get(args[0])
      if (!r) throw new Error('no input')
      r.p.record(args[1] as number)
      r.recordId = id
      return undefined
    }
    case 'recent':
      return inputs.get(args[0])?.p.recent() ?? null
    case 'stats':
      return inputs.get(args[0])?.p.stats() ?? null
    default:
      throw new Error(`unknown method ${method}`)
  }
}

function dispose(input: number): void {
  const r = inputs.get(input)
  if (r) {
    r.closed = true
    r.port?.close()
    r.bg?.close()
    clearInterval(r.timer)
    r.p.release()
    inputs.delete(input)
  }
  learning.get(input)?.port.close()
  learning.get(input)?.coach.release()
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
        dispose(m.input)
        const r = realtime(m.input, m.opts, m.bg ?? null, m.out ?? null)
        if (m.bg)
          (m.bg as MessagePort).onmessage = (ev: MessageEvent) => {
            if (ev.data.type === 'print' && !r.closed) r.p.set({ print: ev.data.embedding })
          }
        post({ type: 'reply', id: m.id, result: true })
        break
      }
      case 'worklet': {
        const r = inputs.get(m.input)
        const port = m.port as MessagePort
        if (!r) break
        r.port = port
        port.onmessage = (ev: MessageEvent) => {
          // hops still on their way when the input was closed land on released sessions
          if (r.closed) return
          if (ev.data instanceof ArrayBuffer) onHop(r, ev.data, port)
          else if (typeof ev.data?.clock === 'number') r.shift = ev.data.clock / RATE - r.p.time
        }
        break
      }
      case 'rt-port': {
        dispose(m.input)
        const l: Learning = { coach: engine!.coach(m.ref ?? null), port: m.port }
        learning.set(m.input, l)
        l.port.onmessage = (ev: MessageEvent) => {
          if (ev.data.type === 'window') onWindow(m.input, ev.data.pcm, ev.data.farQuietMs, ev.data.t)
          else if (ev.data.type === 'verify') l.port.postMessage({ type: 'verified', key: ev.data.key, ...l.coach.verify(engine!.timbre(), ev.data.pcm) })
        }
        break
      }
      case 'set': {
        const r = inputs.get(m.input)
        if (!r) break
        const change: Record<string, unknown> = {}
        if ('strength' in m) change.strength = m.strength
        if ('enabled' in m) change.enabled = m.enabled
        if ('print' in m) change.print = m.print
        if ('words' in m) Object.assign(change, { words: words(m.words), context: m.context ?? 'anywhere' })
        if ('enroll' in m) change.enroll = enrolling(m.enroll)
        if ('listen' in m) change.listen = m.listen
        if ('hold' in m) change.hold = m.hold
        if ('words' in m || ('enroll' in m && m.enroll))
          withSpot(r, () => {
            r.p.spot(engine!.model('spot'))
            r.p.set(change)
          })
        else r.p.set(change)
        drain(r)
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
    const message = String((err as Error)?.message ?? err)
    if (m.id !== undefined) post({ type: 'reply', id: m.id, error: { message, code: (err as { code?: string })?.code } })
    else post({ type: 'events', input: m.input, list: [{ type: 'diag', code: 'worker.error', level: 'warn', data: { message, on: m.type } }] })
  }
}
