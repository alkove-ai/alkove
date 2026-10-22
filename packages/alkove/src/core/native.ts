import type { Alk, Model } from '../engine/alk'
import { AlkoveError, type ErrorCode } from '../errors'

// a value of the core: what JSON can say, and audio, crossing the wasm as tagged bytes (0 null, 1 false, 2 true, 3 f64, 4 string, 5 array, 6 object, 7 bytes, 8 float32 array)
export type Value = null | boolean | number | string | Value[] | { [key: string]: Value } | Uint8Array | Float32Array

const utf8 = new TextEncoder()
const text = new TextDecoder()

export function encode(v: unknown): Uint8Array {
  const parts: number[] = []
  const bytes: Uint8Array[] = []
  let size = 0
  const flush = () => {
    if (!parts.length) return
    bytes.push(Uint8Array.from(parts))
    size += parts.length
    parts.length = 0
  }
  const raw = (b: Uint8Array) => {
    flush()
    bytes.push(b)
    size += b.length
  }
  const u32 = (n: number) => parts.push(n & 255, (n >>> 8) & 255, (n >>> 16) & 255, n >>> 24)
  const str = (s: string) => {
    const b = utf8.encode(s)
    u32(b.length)
    raw(b)
  }
  const put = (x: unknown): void => {
    if (x === null || x === undefined) parts.push(0)
    else if (typeof x === 'boolean') parts.push(x ? 2 : 1)
    else if (typeof x === 'number') {
      parts.push(3)
      raw(new Uint8Array(Float64Array.of(x).buffer))
    } else if (typeof x === 'string') {
      parts.push(4)
      str(x)
    } else if (x instanceof Uint8Array) {
      parts.push(7)
      u32(x.length)
      raw(x.slice())
    } else if (x instanceof Float32Array) {
      parts.push(8)
      u32(x.length)
      raw(new Uint8Array(x.slice().buffer))
    } else if (Array.isArray(x) || ArrayBuffer.isView(x)) {
      const a = Array.from(x as ArrayLike<unknown>)
      parts.push(5)
      u32(a.length)
      for (const y of a) put(y)
    } else {
      // as JSON: fields left undefined are left out
      const f = Object.entries(x as object).filter(([, y]) => y !== undefined)
      parts.push(6)
      u32(f.length)
      for (const [k, y] of f) {
        str(k)
        put(y)
      }
    }
  }
  put(v)
  flush()
  const out = new Uint8Array(size)
  let o = 0
  for (const b of bytes) {
    out.set(b, o)
    o += b.length
  }
  return out
}

export function decode(b: Uint8Array): Value {
  const v = new DataView(b.buffer, b.byteOffset, b.byteLength)
  let at = 0
  const u32 = () => {
    at += 4
    return v.getUint32(at - 4, true)
  }
  const str = () => {
    const n = u32()
    at += n
    return text.decode(b.subarray(at - n, at))
  }
  const read = (): Value => {
    switch (b[at++]) {
      case 0:
        return null
      case 1:
        return false
      case 2:
        return true
      case 3:
        at += 8
        return v.getFloat64(at - 8, true)
      case 4:
        return str()
      case 5:
        return Array.from({ length: u32() }, read)
      case 6:
        // own properties even for a key like __proto__, as Object.fromEntries made them
        return Object.fromEntries(Array.from({ length: u32() }, () => [str(), read()]))
      case 7: {
        const n = u32()
        at += n
        return b.slice(at - n, at)
      }
      case 8: {
        const n = u32()
        at += 4 * n
        return new Float32Array(b.slice(at - 4 * n, at).buffer)
      }
      default:
        throw new Error('bad value from the core')
    }
  }
  return read()
}

export function send(alk: Alk, v: unknown): void {
  const b = encode(v)
  const p = alk.x.alk_value_in(b.length)
  new Uint8Array(alk.heap, p, b.length).set(b)
}

// the call's output value; a failure comes back as the error the TypeScript threw
export function receive<T = Value>(alk: Alk, n: number): T {
  const out = decode(new Uint8Array(alk.heap, alk.x.alk_value_out(), alk.x.alk_value_len()).slice()) as T
  if (n >= 0) return out
  const e = out as unknown as { kind: string; code: string | null; message: string }
  if (e.kind === 'TypeError') throw new TypeError(e.message)
  if (e.kind === 'RangeError') throw new RangeError(e.message)
  if (e.kind === 'AlkoveError') throw new AlkoveError(e.code as ErrorCode, e.message)
  throw new Error(e.message)
}

// a released handle is 0: a call with it would reach whatever the core keeps at address 0
const live = (ptr: number): number => {
  if (!ptr) throw new Error('used after release')
  return ptr
}

const floats = (alk: Alk, pcm: Float32Array, fn: (p: number, n: number) => number) => alk.with(new Uint8Array(pcm.buffer, pcm.byteOffset, pcm.byteLength), (p) => fn(p, pcm.length))

export interface NativePronunciation {
  ids: number[]
  ipa: string
  p: number
}

// Lex's beam search in the core, on one model holding its encoder and decoder plans
export class NativeLex {
  private ptr: number

  constructor(
    private alk: Alk,
    model: Model,
  ) {
    this.ptr = alk.x.alk_lex(model.ptr)
    if (!this.ptr) alk.fail('lex')
  }

  get handle(): number {
    return this.ptr
  }

  phonemize(texts: string[], lang: string): NativePronunciation[][] {
    send(this.alk, { texts, lang })
    return receive(this.alk, this.alk.x.alk_lex_phonemize(live(this.ptr)))
  }

  release(): void {
    if (this.ptr) this.alk.x.alk_lex_free(this.ptr)
    this.ptr = 0
  }
}

// a typed word (with Lex) or phonemes between slashes (lex may be null): the word's JSON and what to warn about
export function compileNative(alk: Alk, lex: NativeLex | null, text: string, lang: string[], chain?: string): { word: Value; warnings: Value[] } {
  send(alk, { text, lang, chain })
  return receive(alk, alk.x.alk_compile(lex?.handle ?? 0))
}

// the core's processor: { start, speech, wakes, diag, trace? } per part, the audio beside
export class NativeProcessor {
  private ptr: number

  constructor(
    private alk: Alk,
    models: { focus?: Model; cue?: Model; spot?: Model },
    options: Value,
  ) {
    send(alk, options)
    this.ptr = alk.x.alk_processor(models.focus?.ptr ?? 0, models.cue?.ptr ?? 0, models.spot?.ptr ?? 0)
    if (!this.ptr) alk.fail('processor')
  }

  private part(n: number): Record<string, unknown> {
    const { x } = this.alk
    const v = receive<Record<string, unknown>>(this.alk, n)
    const audio = new Float32Array(this.alk.heap, x.alk_processor_audio(live(this.ptr)), x.alk_processor_audio_len(live(this.ptr))).slice()
    const { start, ...rest } = v
    return { start, audio, ...rest }
  }

  push(pcm: Float32Array): Record<string, unknown> {
    return this.part(floats(this.alk, pcm, (p, n) => this.alk.x.alk_processor_push(live(this.ptr), p, n)))
  }

  end(): Record<string, unknown> {
    return this.part(this.alk.x.alk_processor_end(live(this.ptr)))
  }

  release(): void {
    if (this.ptr) this.alk.x.alk_processor_free(this.ptr)
    this.ptr = 0
  }
}

// one of Spot's frames: its step, the speech head (null until it caught up), each word's score, the wakes
export type HeardFrame = [number, number | null, number[], Record<string, unknown>[]]

// Spot and the words it listens for
export class NativeListener {
  private ptr: number

  constructor(
    private alk: Alk,
    spot: Model,
    words: Value[],
    context: string,
  ) {
    send(alk, { words, context })
    this.ptr = alk.x.alk_listener(spot.ptr)
    if (!this.ptr) alk.fail('listener')
  }

  setWords(words: Value[], context: string): void {
    send(this.alk, { words, context })
    receive(this.alk, this.alk.x.alk_listener_words(live(this.ptr)))
  }

  push(pcm: Float32Array): HeardFrame[] {
    return receive(this.alk, floats(this.alk, pcm, (p, n) => this.alk.x.alk_listener_push(live(this.ptr), p, n)))
  }

  release(): void {
    if (this.ptr) this.alk.x.alk_listener_free(this.ptr)
    this.ptr = 0
  }
}

// a word said `takes` times on Spot's own speech head
export class NativeEnroller {
  private ptr: number

  constructor(
    private alk: Alk,
    spot: Model,
    word: Value,
    takes: number,
    chain: string,
  ) {
    send(alk, { word, takes, chain })
    this.ptr = alk.x.alk_enroller(spot.ptr)
    if (!this.ptr) alk.fail('enroller')
  }

  push(pcm: Float32Array): { takes: Record<string, unknown>[]; done: boolean } {
    return receive(this.alk, floats(this.alk, pcm, (p, n) => this.alk.x.alk_enroller_push(live(this.ptr), p, n)))
  }

  result(): { word: Value; takes: Record<string, unknown>[]; warnings: Value[] } {
    return receive(this.alk, this.alk.x.alk_enroller_result(live(this.ptr)))
  }

  release(): void {
    if (this.ptr) this.alk.x.alk_enroller_free(this.ptr)
    this.ptr = 0
  }
}

export interface NativeCenter {
  embedding: number[]
  id: number[]
  seconds: number
}

// Timbre in the core: its windows, and a print from them
export class NativeTimbre {
  private ptr: number

  constructor(
    private alk: Alk,
    model: Model,
  ) {
    this.ptr = alk.x.alk_timbre(model.ptr)
    if (!this.ptr) alk.fail('timbre')
  }

  get handle(): number {
    return this.ptr
  }

  windows(pcm: Float32Array): NativeCenter[] {
    return receive(this.alk, floats(this.alk, pcm, (p, n) => this.alk.x.alk_timbre_windows(live(this.ptr), p, n)))
  }

  embed(pcm: Float32Array, source: string): Value {
    send(this.alk, { source })
    return receive(this.alk, floats(this.alk, pcm, (p, n) => this.alk.x.alk_timbre_embed(live(this.ptr), p, n)))
  }

  release(): void {
    if (this.ptr) this.alk.x.alk_timbre_free(this.ptr)
    this.ptr = 0
  }
}

// verify, identify, merge or read prints, given as JSON
export function voiceprintOp(alk: Alk, op: Record<string, unknown>): Value {
  send(alk, op)
  return receive(alk, alk.x.alk_voiceprint())
}

export interface LearnerState {
  progress: number
  ready: boolean
  windows: number
}

// a print learned window by window, in the core
export class NativeLearner {
  private ptr: number

  constructor(
    private alk: Alk,
    source: string,
    start: Value,
  ) {
    send(alk, { source, start })
    this.ptr = alk.x.alk_learner()
    if (!this.ptr) alk.fail('learner')
  }

  learn(timbre: NativeTimbre, pcm: Float32Array): LearnerState & { accepted: boolean; score: number; restarted: boolean } {
    return receive(this.alk, floats(this.alk, pcm, (p, n) => this.alk.x.alk_learner_learn(live(this.ptr), timbre.handle, p, n)))
  }

  offer(timbre: NativeTimbre, pcm: Float32Array): LearnerState & { accepted: number; rejected: number; restarted: boolean } {
    return receive(this.alk, floats(this.alk, pcm, (p, n) => this.alk.x.alk_learner_offer(live(this.ptr), timbre.handle, p, n)))
  }

  progress(): number {
    return receive<LearnerState>(this.alk, this.state()).progress
  }

  ready(): boolean {
    return receive<LearnerState>(this.alk, this.state()).ready
  }

  private state(): number {
    return this.alk.x.alk_learner_state(live(this.ptr))
  }

  // JSON, or raw: its float32 as they are and its windows unpacked
  voiceprint(raw = false): Value {
    return receive(this.alk, this.alk.x.alk_learner_voiceprint(live(this.ptr), +raw))
  }

  forget(): void {
    this.alk.x.alk_learner_forget(live(this.ptr))
  }

  release(): void {
    if (this.ptr) this.alk.x.alk_learner_free(this.ptr)
    this.ptr = 0
  }
}

export type PipelineOut = ['event', Record<string, unknown>] | ['window', Float32Array, number, number] | ['verify', number, Float32Array] | ['audio', Uint8Array] | ['utterance', Uint8Array, number, number, Record<string, unknown>?] | ['record', Uint8Array]

// a real-time input's own rules in the core: the chain, Spot, learning windows, verified wakes, utterances, enrollment, what it keeps
export class NativePipeline {
  private ptr: number
  private out = new Float32Array(256)

  constructor(
    private alk: Alk,
    models: { focus?: Model; cue?: Model; spot?: Model },
    options: Value,
  ) {
    send(alk, options)
    this.ptr = alk.x.alk_pipeline(models.focus?.ptr ?? 0, models.cue?.ptr ?? 0, models.spot?.ptr ?? 0)
    if (!this.ptr) alk.fail('pipeline')
  }

  // one hop; the result is valid until the next call
  step(hop: Float32Array, farQuietMs: number): Float32Array {
    const { alk } = this
    const ptr = live(this.ptr)
    new Float32Array(alk.heap, alk.x.alk_pipeline_hop(ptr), 256).set(hop)
    const out = alk.x.alk_pipeline_step(ptr, farQuietMs)
    this.out.set(new Float32Array(alk.heap, out, 256))
    return this.out
  }

  spot(model: Model): void {
    this.alk.x.alk_pipeline_spot(live(this.ptr), model.ptr)
  }

  raw(hop: Float32Array): void {
    new Float32Array(this.alk.heap, this.alk.x.alk_pipeline_hop(live(this.ptr)), 256).set(hop)
    this.alk.x.alk_pipeline_raw(live(this.ptr))
  }

  outs(): PipelineOut[] {
    if (!this.alk.x.alk_pipeline_pending(live(this.ptr))) return []
    return receive(this.alk, this.alk.x.alk_pipeline_outs(live(this.ptr)))
  }

  verified(key: number, same: boolean, score: number): void {
    this.alk.x.alk_pipeline_verified(live(this.ptr), key, +same, score)
  }

  set(change: Record<string, unknown>): void {
    send(this.alk, change)
    receive(this.alk, this.alk.x.alk_pipeline_set(live(this.ptr)))
  }

  record(seconds: number): void {
    receive(this.alk, this.alk.x.alk_pipeline_record(live(this.ptr), seconds))
  }

  recent(): Uint8Array | null {
    return receive(this.alk, this.alk.x.alk_pipeline_recent(live(this.ptr)))
  }

  stats(): Value {
    return receive(this.alk, this.alk.x.alk_pipeline_stats(live(this.ptr)))
  }

  get time(): number {
    return this.alk.x.alk_pipeline_get(live(this.ptr), 0)
  }

  get sleeping(): boolean {
    return this.alk.x.alk_pipeline_get(live(this.ptr), 1) === 1
  }

  get gainDb(): number {
    return this.alk.x.alk_pipeline_get(live(this.ptr), 2)
  }

  get hops(): number {
    return this.alk.x.alk_pipeline_get(live(this.ptr), 3)
  }

  get chained(): boolean {
    return this.alk.x.alk_pipeline_get(live(this.ptr), 4) === 1
  }

  release(): void {
    if (this.ptr) this.alk.x.alk_pipeline_free(this.ptr)
    this.ptr = 0
  }
}

export type CoachOut = ['event', Record<string, unknown>] | ['print', Float32Array]

// an input's background side in the core: the print learned from the user's windows, and the voice that verifies its wakes
export class NativeCoach {
  private ptr: number

  constructor(
    private alk: Alk,
    reference: Value,
  ) {
    send(alk, { reference })
    this.ptr = alk.x.alk_coach()
    if (!this.ptr) alk.fail('coach')
  }

  learner(start: Value): number {
    send(this.alk, { start })
    return receive(this.alk, this.alk.x.alk_coach_learner(live(this.ptr)))
  }

  forget(): void {
    this.alk.x.alk_coach_forget(live(this.ptr))
  }

  window(timbre: NativeTimbre, pcm: Float32Array, farQuietMs: number, t: number): CoachOut[] {
    return receive(this.alk, floats(this.alk, pcm, (p, n) => this.alk.x.alk_coach_window(live(this.ptr), timbre.handle, p, n, farQuietMs, t)))
  }

  verify(timbre: NativeTimbre, pcm: Float32Array): { same: boolean; score: number } {
    return receive(this.alk, floats(this.alk, pcm, (p, n) => this.alk.x.alk_coach_verify(live(this.ptr), timbre.handle, p, n)))
  }

  release(): void {
    if (this.ptr) this.alk.x.alk_coach_free(this.ptr)
    this.ptr = 0
  }
}
