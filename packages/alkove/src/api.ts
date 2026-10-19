import { Lex as LexCore, type Pronunciation } from './core/lex'
import { type CompileOptions, type Word, type WordJSON, wordOf } from './core/spot/spot'
import { Enrollment, type EnrollTarget } from './recipes/enroll'
import { type CommonOptions, createEnrollInput } from './recipes/input'
import { type Center, type Level, Timbre as TimbreCore, Voiceprint, type VoiceprintJSON } from './core/timbre'
import { Resampler } from './dsp/resample'
import { decodeWav, encodeWav } from './dsp/wav'
import { AlkoveError } from './errors'
import { configure, isCached, onDiag, type Progress } from './loader'
import { ENGINE_WASM, type Lang, type ModelName, MODELS, RATE } from './manifest'
import type { ProcessOptions, ProcessPart, ProcessResult } from './process'
import { micPermission, supported } from './recipes/mic'
import type { CallName } from './runtime/calls'
import { Host } from './runtime/host'

const isNode = () => typeof process !== 'undefined' && !!process.versions?.node && typeof window === 'undefined'

interface Backend {
  call<T>(name: CallName, args: unknown[], transfer?: Transferable[], onProgress?: (p: { done: number; total: number }) => void): Promise<T>
  // keeps the worker that holds a learner's or a stream's state; what it returns lets it go
  hold(): () => void
}

let ids = 0

let node: Promise<import('./runtime/node').NodeBackend> | null = null

// the background worker in a browser, the caller's thread in Node
async function backend(models: ModelName[], onProgress?: (p: Progress) => void): Promise<Backend> {
  if (isNode()) {
    const nb = await (node ??= import('./runtime/node').then((m) => new m.NodeBackend()))
    await nb.ready(models)
    return { call: async <T>(name: CallName, args: unknown[], _t?: Transferable[], progress?: (p: { done: number; total: number }) => void) => (nb.calls[name] as (...a: unknown[]) => T)(...args, progress), hold: () => () => {} }
  }
  const host = Host.get()
  await host.models('background', models, onProgress)
  // asked again at each call: the worker is closed once no input is left, and comes back on the next call
  return { call: async <T>(name: CallName, args: unknown[], transfer: Transferable[] = [], progress?: (p: { done: number; total: number }) => void) => (await host.models('background', models)).call<T>(name, args, transfer, progress), hold: () => host.hold() }
}

export type Usage = 'call' | 'agent' | 'wake'

export interface UsageOptions {
  voiceprint?: boolean
  typedWords?: boolean
}

function modelsFor(usage: Usage, o: UsageOptions = {}): ModelName[] {
  const m: ModelName[] = usage === 'wake' ? ['spot'] : ['focus', 'cue']
  if (o.voiceprint) m.push('timbre')
  if (o.typedWords) m.push('lex')
  return m
}

export type ProcessInput = Blob | ArrayBuffer | Uint8Array | AudioBuffer | { pcm: Float32Array; rate: number }

async function decode(input: ProcessInput): Promise<Float32Array> {
  if ('pcm' in input && 'rate' in input) return Resampler.all(input.pcm, input.rate, RATE)
  if (typeof AudioBuffer !== 'undefined' && input instanceof AudioBuffer) return Resampler.all(mono(input), input.sampleRate, RATE)
  const bytes = typeof Blob !== 'undefined' && input instanceof Blob ? new Uint8Array(await input.arrayBuffer()) : input instanceof Uint8Array ? input : new Uint8Array(input as ArrayBuffer)
  if (String.fromCharCode(...bytes.subarray(0, 4)) === 'RIFF') {
    const { pcm, rate } = decodeWav(bytes)
    return Resampler.all(pcm, rate, RATE)
  }
  if (typeof OfflineAudioContext === 'undefined') throw new AlkoveError('browser.unsupported', 'Outside a browser only WAV is decoded: decode the file and pass { pcm, rate }.')
  const ctx = new OfflineAudioContext(1, 1, 44100)
  const buf = await ctx.decodeAudioData(bytes.slice().buffer)
  return Resampler.all(mono(buf), buf.sampleRate, RATE)
}

function mono(buf: AudioBuffer): Float32Array {
  if (buf.numberOfChannels === 1) return buf.getChannelData(0).slice()
  const out = new Float32Array(buf.length)
  for (let c = 0; c < buf.numberOfChannels; c++) {
    const d = buf.getChannelData(c)
    for (let i = 0; i < out.length; i++) out[i]! += d[i]! / buf.numberOfChannels
  }
  return out
}

export const Alkove = {
  configure,
  stream,
  supported,
  micPermission,
  onDiag,

  // a settings or onboarding screen: everything a usage needs, downloaded and compiled ahead
  async preload(usage: Usage, opts: UsageOptions & { onProgress?: (p: Progress) => void } = {}): Promise<void> {
    const names = modelsFor(usage, opts)
    if (isNode()) {
      await backend(names)
      return
    }
    const host = Host.get()
    const rt = names.filter((n) => n !== 'timbre' && n !== 'lex')
    const bg = names.filter((n) => n === 'timbre' || n === 'lex')
    const parts = [{ loaded: 0, total: 0 }, { loaded: 0, total: 0 }]
    const part = (i: number) => (p: Progress) => {
      parts[i] = p
      opts.onProgress?.({ loaded: parts[0]!.loaded + parts[1]!.loaded, total: parts[0]!.total + parts[1]!.total })
    }
    await Promise.all([host.models('realtime', rt, part(0)), bg.length ? host.models('background', bg, part(1)) : null])
  },

  // true when everything is already here: works offline
  async cached(usage: Usage, opts: UsageOptions = {}): Promise<boolean> {
    const files = [ENGINE_WASM, ...modelsFor(usage, opts).map((n) => MODELS[n])]
    return (await Promise.all(files.map(isCached))).every(Boolean)
  },

  // a file, faster than real time, in the background worker: the same output as the real time for the same 16 kHz audio
  async process(input: ProcessInput, opts: ProcessOptions = {}): Promise<ProcessResult> {
    const pcm = await decode(input)
    opts.signal?.throwIfAborted()
    const b = await backend(modelsOf(opts))
    const r = await b.call<Omit<ProcessResult, 'toWav'>>('process', [pcm, wireOf(opts)], [], opts.onProgress)
    return {
      ...r,
      toWav(which = 'audio') {
        return encodeWav(which === 'both' ? [r.raw, r.audio] : [which === 'raw' ? r.raw : r.audio], RATE)
      },
    }
  },
}

function modelsOf(opts: ProcessOptions): ModelName[] {
  const names: ModelName[] = []
  if (opts.focus) names.push('focus', 'cue')
  else if (opts.cue) names.push('cue')
  if (opts.spot?.words.length) names.push('spot')
  return names
}

// what crosses to the worker: words and prints as JSON
function wireOf(opts: ProcessOptions) {
  const { onProgress: _p, signal: _s, ...rest } = opts
  return {
    ...rest,
    focus: typeof opts.focus === 'object' ? { ...opts.focus, voiceprint: opts.focus.voiceprint instanceof Voiceprint ? opts.focus.voiceprint.toJSON() : opts.focus.voiceprint } : opts.focus,
    spot: opts.spot ? { ...opts.spot, words: opts.spot.words.map((w) => wordOf(w).toJSON()) } : undefined,
  }
}

export type StreamSource = AsyncIterable<Float32Array> | ReadableStream<Float32Array>

// long files (an hour) without keeping them in memory: blocks in, parts out, the same as process() on the whole
async function* stream(source: StreamSource, opts: ProcessOptions & { rate?: number } = {}): AsyncGenerator<ProcessPart> {
  const b = await backend(modelsOf(opts))
  const id = ++ids
  const release = b.hold()
  await b.call('streamOpen', [id, wireOf(opts)]).catch((e) => {
    release()
    throw e
  })
  const rate = opts.rate ?? RATE
  const resampler = new Resampler(rate, RATE, true)
  let given = 0
  let taken = 0
  const iter: AsyncIterable<Float32Array> = Symbol.asyncIterator in source ? (source as AsyncIterable<Float32Array>) : readable(source as ReadableStream<Float32Array>)
  try {
    for await (const chunk of iter) {
      opts.signal?.throwIfAborted()
      const out: number[] = []
      given += chunk.length
      resampler.push(chunk, (x) => out.push(x))
      taken += out.length
      if (out.length) yield await b.call<ProcessPart>('streamPush', [id, Float32Array.from(out)])
    }
    // the last half window, up to the length the input had
    const tail: number[] = []
    const want = Math.round((given * RATE) / rate) - taken
    if (want > 0) resampler.push(new Float32Array(Resampler.HALF * 2 + 8), (x) => tail.length < want && tail.push(x))
    if (tail.length) yield await b.call<ProcessPart>('streamPush', [id, Float32Array.from(tail)])
  } finally {
    try {
      yield await b.call<ProcessPart>('streamEnd', [id])
    } finally {
      release()
    }
  }
}

async function* readable(s: ReadableStream<Float32Array>): AsyncIterable<Float32Array> {
  const reader = s.getReader()
  try {
    for (;;) {
      const { done, value } = await reader.read()
      if (done) return
      yield value
    }
  } finally {
    reader.releaseLock()
  }
}

export class Timbre {
  private constructor(private b: Backend) {}

  static async create(): Promise<Timbre> {
    return new Timbre(await backend(['timbre']))
  }

  // 1.5 s or more advised: windows of 1.5 s, mean without the 10 % least alike
  async embed(pcm: Float32Array, opts: { source?: 'raw' | 'focus' } = {}): Promise<Voiceprint> {
    return Voiceprint.fromJSON(await this.b.call<VoiceprintJSON>('embed', [pcm, opts.source ?? 'raw']))!
  }

  verify(a: Voiceprint, b: Voiceprint, opts: { level?: Level } = {}): { same: boolean; score: number } {
    return TimbreCore.verify(a, b, opts)
  }

  async identify(voice: Float32Array | Voiceprint, known: Record<string, Voiceprint>, opts: { level?: Level } = {}): Promise<{ who: string; score: number } | null> {
    const v = voice instanceof Voiceprint ? voice : await this.embed(voice)
    return TimbreCore.identify(v, known, opts)
  }
}

export class Lex {
  static readonly inventory = LexCore.inventory
  static readonly langs = LexCore.langs
  static readonly distance = LexCore.distance

  private constructor(private b: Backend) {}

  static async create(): Promise<Lex> {
    return new Lex(await backend(['lex']))
  }

  phonemize(text: string, lang: Lang): Promise<Omit<Pronunciation, 'ids'>[]>
  phonemize(texts: string[], lang: Lang): Promise<Omit<Pronunciation, 'ids'>[][]>
  async phonemize(text: string | string[], lang: Lang): Promise<unknown> {
    if (!LexCore.langs.includes(lang)) throw new AlkoveError('lang.unsupported', `No ${lang}: ${LexCore.langs.join(', ')} only.`)
    const all = await this.b.call<Pronunciation[][]>('phonemize', [Array.isArray(text) ? text : [text], lang])
    const out = all.map((ps) => ps.map(({ ipa, p }) => ({ ipa, p })))
    return Array.isArray(text) ? out : out[0]
  }
}

export const Spot = {
  // a typed word (Lex), or phonemes between slashes (no Lex)
  async compile(text: string, opts: CompileOptions = {}): Promise<Word> {
    for (const l of [opts.lang ?? []].flat()) if (!LexCore.langs.includes(l)) throw new AlkoveError('lang.unsupported', `No ${l}: ${LexCore.langs.join(', ')} only.`)
    const typed = /^\s*\/.+\/\s*$/.test(text)
    const b = await backend(typed ? [] : ['lex'])
    return wordOf(await b.call<WordJSON>('compile', [text, { lang: opts.lang, chain: opts.chain }]))
  },

  // the user says the word `takes` times: thresholds set from these takes and from what is heard between them
  enroll(text: string, opts: CompileOptions & CommonOptions & { takes?: number } = {}): Enrollment {
    return new Enrollment({
      compile: () => Spot.compile(text, opts),
      open: async (word) => {
        const input = await createEnrollInput(opts, word, opts.takes ?? 3)
        return { on: input.on.bind(input) as EnrollTarget['on'], stop: () => input.dispose(), source: 'raw' }
      },
      embed: async (pcm, source) => (await Timbre.create()).embed(pcm, { source }),
    })
  },
}

// an enrollment screen (« read this sentence »): the same rules as the inputs' learning, run in the background worker
type LearnerState = { progress: number; ready: boolean; voiceprint: RawPrint | null }
type RawPrint = { source: Voiceprint['source']; model: string; centers: Center[]; windows: Center[] }

const fromRaw = (p: RawPrint | null) => (p ? new Voiceprint(p.centers, p.source, p.model, p.windows) : null)
const toRaw = (p: Voiceprint) => ({ alkove: 'voiceprint', model: p.model, source: p.source, centers: p.centers, windows: p.windows })
const closing = typeof FinalizationRegistry === 'undefined' ? null : new FinalizationRegistry<() => void>((close) => close())

export class VoiceprintLearner {
  private state: { progress: number; ready: boolean; voiceprint: Voiceprint | null }

  private constructor(
    private b: Backend,
    private id: number,
    s: LearnerState,
    release: () => void,
  ) {
    this.state = { progress: s.progress, ready: s.ready, voiceprint: fromRaw(s.voiceprint) }
    closing?.register(this, () => void b.call('learnerClose', [id]).catch(() => {}).finally(release))
  }

  static async create(opts: { voiceprint?: Voiceprint | null } = {}): Promise<VoiceprintLearner> {
    const b = await backend(['timbre'])
    const id = ++ids
    const release = b.hold()
    try {
      const s = await b.call<LearnerState>('learnerOpen', [id, opts.voiceprint ? toRaw(opts.voiceprint) : null])
      return new VoiceprintLearner(b, id, s, release)
    } catch (e) {
      release()
      throw e
    }
  }

  get progress(): number {
    return this.state.progress
  }

  get ready(): boolean {
    return this.state.ready
  }

  get voiceprint(): Voiceprint | null {
    return this.state.voiceprint
  }

  async offer(pcm: Float32Array): Promise<{ accepted: number; rejected: number; progress: number; ready: boolean }> {
    const r = await this.b.call<LearnerState & { accepted: number; rejected: number }>('learnerOffer', [this.id, pcm])
    this.state = { progress: r.progress, ready: r.ready, voiceprint: fromRaw(r.voiceprint) }
    return { accepted: r.accepted, rejected: r.rejected, progress: r.progress, ready: r.ready }
  }

  forget(): void {
    this.state = { progress: 0, ready: false, voiceprint: null }
    void this.b.call('learnerForget', [this.id])
  }
}
