import type { Alk, Model, Session, Tensor } from '../engine/alk'
import type { ModelName } from '../manifest'
import { Chain, type ChainOptions } from './chain'
import { compileNative, NativeCoach, NativeEnroller, NativeLearner, NativeLex, NativeListener, NativePipeline, NativeProcessor, NativeTimbre, type Value, voiceprintOp } from './native'

export type StreamModel = 'focus' | 'cue' | 'spot'

export interface StreamNet {
  f32(name: string): Float32Array
  run(): void
  reset(): void
  release(): void
}

export interface Net {
  run(feeds: Record<string, Tensor>, wanted?: string[]): Record<string, Tensor>
  reset(): void
}

// what the primitives run on: the wasm engine by default, a native bridge later
export interface Engine {
  stream(model: StreamModel): StreamNet
  net(model: 'timbre' | 'lex', plan?: string): Net
  chain(opts?: ChainOptions): Chain
  // the core's processor, on the models its options need
  processor(options: { focus: unknown; cue: boolean | null; spot: unknown }): NativeProcessor
  listener(words: Value[], context: string): NativeListener
  enroller(word: Value, takes: number, chain: string): NativeEnroller
  lex(): NativeLex
  // a typed word with Lex, phonemes between slashes without
  compile(text: string, lang: string[], chain?: string): { word: Value; warnings: Value[] }
  timbre(): NativeTimbre
  learner(source: string, start: Value): NativeLearner
  pipeline(options: { chain: unknown; words?: unknown[]; enroll?: unknown; [k: string]: unknown }): NativePipeline
  coach(reference: Value): NativeCoach
  voiceprint(op: Record<string, unknown>): Value
}

// one model per name (weights unpacked once), one session per stream: each keeps its own state
export class WasmEngine implements Engine {
  private models = new Map<ModelName, Model>()
  private calls = new Map<string, Session>()
  private lexer: NativeLex | null = null
  private voices: NativeTimbre | null = null

  constructor(
    readonly alk: Alk,
    private bytes: (model: ModelName) => Uint8Array,
  ) {}

  has(model: ModelName): boolean {
    return this.models.has(model)
  }

  model(name: ModelName): Model {
    let m = this.models.get(name)
    if (!m) this.models.set(name, (m = this.alk.model(this.bytes(name))))
    return m
  }

  stream(model: StreamModel): StreamNet {
    return this.model(model).session()
  }

  chain(opts?: ChainOptions): Chain {
    return new Chain(this.alk, this.model('focus'), this.model('cue'), opts)
  }

  processor(options: { focus: unknown; cue: boolean | null; spot: unknown }): NativeProcessor {
    const focus = !!options.focus
    const models = { focus: focus ? this.model('focus') : undefined, cue: focus || options.cue ? this.model('cue') : undefined, spot: options.spot ? this.model('spot') : undefined }
    return new NativeProcessor(this.alk, models, options as Value)
  }

  listener(words: Value[], context: string): NativeListener {
    return new NativeListener(this.alk, this.model('spot'), words, context)
  }

  enroller(word: Value, takes: number, chain: string): NativeEnroller {
    return new NativeEnroller(this.alk, this.model('spot'), word, takes, chain)
  }

  // one per engine: its sessions are reused from word to word
  lex(): NativeLex {
    return (this.lexer ??= new NativeLex(this.alk, this.model('lex')))
  }

  // one per engine, as Lex
  timbre(): NativeTimbre {
    return (this.voices ??= new NativeTimbre(this.alk, this.model('timbre')))
  }

  learner(source: string, start: Value): NativeLearner {
    return new NativeLearner(this.alk, source, start)
  }

  // Spot comes with the options when they need it, else later through `pipeline.spot()`
  pipeline(options: { chain: unknown; words?: unknown[]; enroll?: unknown; [k: string]: unknown }): NativePipeline {
    const chain = !!options.chain
    const spot = !!options.words?.length || !!options.enroll
    return new NativePipeline(this.alk, { focus: chain ? this.model('focus') : undefined, cue: chain ? this.model('cue') : undefined, spot: spot ? this.model('spot') : undefined }, options as Value)
  }

  coach(reference: Value): NativeCoach {
    return new NativeCoach(this.alk, reference)
  }

  voiceprint(op: Record<string, unknown>): Value {
    return voiceprintOp(this.alk, op)
  }

  compile(text: string, lang: string[], chain?: string): { word: Value; warnings: Value[] } {
    return compileNative(this.alk, /^\s*\/.+\/\s*$/.test(text) ? null : this.lex(), text, lang, chain)
  }

  net(model: 'timbre' | 'lex', plan?: string): Net {
    const key = `${model} ${plan ?? ''}`
    let s = this.calls.get(key)
    if (!s) this.calls.set(key, (s = this.model(model).session(plan)))
    const session = s
    return { run: (feeds, wanted) => session.call(feeds, wanted), reset: () => session.reset() }
  }

  release(): void {
    this.lexer?.release()
    this.voices?.release()
    this.lexer = null
    this.voices = null
    for (const s of this.calls.values()) s.release()
    for (const m of this.models.values()) m.release()
    this.calls.clear()
    this.models.clear()
  }
}
