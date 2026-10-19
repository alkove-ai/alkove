import type { Alk, Model } from '../engine/alk'
import { AlkoveError, type ErrorCode } from '../errors'

// a value of the core: what JSON can say, crossing the wasm as tagged bytes (0 null, 1 false, 2 true, 3 f64, 4 string, 5 array, 6 object)
export type Value = null | boolean | number | string | Value[] | { [key: string]: Value }

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
    return receive(this.alk, this.alk.x.alk_lex_phonemize(this.ptr))
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
    const audio = new Float32Array(this.alk.heap, x.alk_processor_audio(this.ptr), x.alk_processor_audio_len(this.ptr)).slice()
    const { start, ...rest } = v
    return { start, audio, ...rest }
  }

  push(pcm: Float32Array): Record<string, unknown> {
    return this.part(floats(this.alk, pcm, (p, n) => this.alk.x.alk_processor_push(this.ptr, p, n)))
  }

  end(): Record<string, unknown> {
    return this.part(this.alk.x.alk_processor_end(this.ptr))
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
    receive(this.alk, this.alk.x.alk_listener_words(this.ptr))
  }

  push(pcm: Float32Array): HeardFrame[] {
    return receive(this.alk, floats(this.alk, pcm, (p, n) => this.alk.x.alk_listener_push(this.ptr, p, n)))
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
    return receive(this.alk, floats(this.alk, pcm, (p, n) => this.alk.x.alk_enroller_push(this.ptr, p, n)))
  }

  result(): { word: Value; takes: Record<string, unknown>[]; warnings: Value[] } {
    return receive(this.alk, this.alk.x.alk_enroller_result(this.ptr))
  }

  release(): void {
    if (this.ptr) this.alk.x.alk_enroller_free(this.ptr)
    this.ptr = 0
  }
}
