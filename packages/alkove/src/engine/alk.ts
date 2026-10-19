// the Alkove engine and core's wasm (crates/wasm): no imports, synchronous runs, nothing allocated while it runs
export interface AlkExports {
  memory: WebAssembly.Memory
  alk_alloc(len: number): number
  alk_free(p: number, len: number): void
  alk_model(bytes: number, len: number): number
  alk_model_free(m: number): void
  alk_error(out: number, cap: number): number
  alk_plans(m: number): number
  alk_plan_name(m: number, i: number, out: number, cap: number): number
  alk_session(m: number, plan: number): number
  alk_session_free(s: number): void
  alk_input(s: number, name: number, len: number): number
  alk_output(s: number, name: number, len: number): number
  alk_run(s: number): void
  alk_state_clear(s: number): void
  alk_variable(m: number, plan: number): number
  alk_set_len(s: number, len: number): number
  alk_io_count(m: number, plan: number, kind: number): number
  alk_io_name(m: number, plan: number, kind: number, i: number, out: number, cap: number): number
  alk_io_shape(m: number, plan: number, kind: number, i: number, dims: number, cap: number): number
  alk_chain(focus: number, cue: number, gate: number, normalize: number, sleep: number, strength: number, confidence: number, exitConfidence: number, startSecs: number, stopSecs: number, print: number, printLen: number): number
  alk_chain_free(c: number): void
  alk_chain_hop(c: number): number
  alk_chain_step(c: number): number
  alk_chain_focus(c: number): number
  alk_chain_events(c: number): number
  alk_chain_event_count(c: number): number
  alk_chain_print(c: number, p: number, len: number): number
  alk_chain_strength(c: number, s: number): void
  alk_chain_enabled(c: number, on: number): void
  alk_chain_get(c: number, what: number): number
  alk_value_in(len: number): number
  alk_value_out(): number
  alk_value_len(): number
  alk_lex(m: number): number
  alk_lex_free(l: number): void
  alk_lex_phonemize(l: number): number
  alk_compile(l: number): number
  alk_processor(focus: number, cue: number, spot: number): number
  alk_processor_free(p: number): void
  alk_processor_push(p: number, pcm: number, len: number): number
  alk_processor_end(p: number): number
  alk_processor_audio(p: number): number
  alk_processor_audio_len(p: number): number
  alk_listener(m: number): number
  alk_listener_free(l: number): void
  alk_listener_words(l: number): number
  alk_listener_push(l: number, pcm: number, len: number): number
  alk_enroller(m: number): number
  alk_enroller_free(e: number): void
  alk_enroller_push(e: number, pcm: number, len: number): number
  alk_enroller_result(e: number): number
  alk_timbre(m: number): number
  alk_timbre_free(t: number): void
  alk_timbre_windows(t: number, pcm: number, len: number): number
  alk_timbre_embed(t: number, pcm: number, len: number): number
  alk_voiceprint(): number
  alk_learner(): number
  alk_learner_free(l: number): void
  alk_learner_learn(l: number, t: number, pcm: number, len: number): number
  alk_learner_offer(l: number, t: number, pcm: number, len: number): number
  alk_learner_voiceprint(l: number, raw: number): number
  alk_learner_forget(l: number): void
  alk_learner_state(l: number): number
  alk_pipeline(focus: number, cue: number, spot: number): number
  alk_pipeline_free(p: number): void
  alk_pipeline_hop(p: number): number
  alk_pipeline_step(p: number, farQuietMs: number): number
  alk_pipeline_raw(p: number): void
  alk_pipeline_spot(p: number, m: number): void
  alk_pipeline_outs(p: number): number
  alk_pipeline_pending(p: number): number
  alk_pipeline_verified(p: number, key: number, same: number, score: number): void
  alk_pipeline_set(p: number): number
  alk_pipeline_record(p: number, seconds: number): number
  alk_pipeline_recent(p: number): number
  alk_pipeline_stats(p: number): number
  alk_pipeline_get(p: number, what: number): number
  alk_coach(): number
  alk_coach_free(c: number): void
  alk_coach_learner(c: number): number
  alk_coach_forget(c: number): void
  alk_coach_window(c: number, t: number, pcm: number, len: number, farQuietMs: number, at: number): number
  alk_coach_verify(c: number, t: number, pcm: number, len: number): number
}

export type TypedData = Float32Array | Int32Array

export interface Tensor {
  data: TypedData
  dims: number[]
}

export interface Io {
  name: string
  dims: number[]
  size: number
}

// one way through a model's weights (Lex: its encoder, its decoder), with its own inputs, outputs and state
export interface Plan {
  index: number
  name: string
  inputs: Io[]
  outputs: Io[]
  variable: string | null
}

export class EngineError extends Error {}

export class Alk {
  private constructor(readonly x: AlkExports) {}

  // the compiled module is shared: one download and one compilation for every worker
  static async create(module: WebAssembly.Module): Promise<Alk> {
    const instance = await WebAssembly.instantiate(module, {})
    return new Alk(instance.exports as unknown as AlkExports)
  }

  get heap(): ArrayBuffer {
    return this.x.memory.buffer
  }

  // bytes copied into wasm memory for the length of one call
  with<T>(bytes: Uint8Array, fn: (p: number, len: number) => T): T {
    const p = this.x.alk_alloc(bytes.length)
    new Uint8Array(this.heap, p, bytes.length).set(bytes)
    try {
      return fn(p, bytes.length)
    } finally {
      this.x.alk_free(p, bytes.length)
    }
  }

  fail(what: string): never {
    const cap = 512
    const p = this.x.alk_alloc(cap)
    try {
      const n = this.x.alk_error(p, cap)
      throw new EngineError(`${what}: ${new TextDecoder().decode(new Uint8Array(this.heap, p, n))}`)
    } finally {
      this.x.alk_free(p, cap)
    }
  }

  model(bytes: Uint8Array): Model {
    return new Model(this, bytes)
  }
}

export class Model {
  readonly ptr: number
  readonly plans: Plan[]

  constructor(
    readonly alk: Alk,
    bytes: Uint8Array,
  ) {
    const x = alk.x
    this.ptr = alk.with(bytes, (p, n) => x.alk_model(p, n))
    if (!this.ptr) alk.fail('model')
    const buf = x.alk_alloc(256)
    const text = (n: number) => new TextDecoder().decode(new Uint8Array(alk.heap, buf, n))
    try {
      this.plans = Array.from({ length: x.alk_plans(this.ptr) }, (_, plan) => {
        const ios = (kind: number): Io[] =>
          Array.from({ length: x.alk_io_count(this.ptr, plan, kind) }, (_, i) => {
            const name = text(x.alk_io_name(this.ptr, plan, kind, i, buf, 256))
            const rank = x.alk_io_shape(this.ptr, plan, kind, i, buf, 64)
            const dims = Array.from(new Uint32Array(alk.heap, buf, rank))
            return { name, dims, size: dims.reduce((a, b) => a * b, 1) }
          })
        const inputs = ios(0)
        return { index: plan, name: text(x.alk_plan_name(this.ptr, plan, buf, 256)), inputs, outputs: ios(1), variable: inputs[x.alk_variable(this.ptr, plan)]?.name ?? null }
      })
    } finally {
      x.alk_free(buf, 256)
    }
  }

  // by name, or the first plan
  session(plan?: string): Session {
    const p = plan === undefined ? this.plans[0] : this.plans.find((q) => q.name === plan)
    if (!p) throw new EngineError(`no plan named ${plan}`)
    return new Session(this, p)
  }

  release(): void {
    this.alk.x.alk_model_free(this.ptr)
  }
}

// one arena: the stream's state, its inputs and outputs, at fixed places for the session's life
export class Session {
  private readonly ptr: number
  private readonly at = new Map<string, Io & { p: number }>()

  constructor(
    readonly model: Model,
    readonly plan: Plan,
  ) {
    const { alk } = model
    const x = alk.x
    this.ptr = x.alk_session(model.ptr, plan.index)
    if (!this.ptr) alk.fail('session')
    const name = new TextEncoder()
    for (const [ios, find] of [
      [plan.inputs, x.alk_input],
      [plan.outputs, x.alk_output],
    ] as const)
      for (const io of ios) this.at.set(io.name, { ...io, p: alk.with(name.encode(io.name), (p, n) => find(this.ptr, p, n)) })
  }

  private io(name: string): Io & { p: number } {
    const io = this.at.get(name)
    if (!io) throw new EngineError(`no tensor named ${name}`)
    return io
  }

  // a fresh view each time: growing the wasm memory detaches the old ones
  f32(name: string): Float32Array {
    const io = this.io(name)
    return new Float32Array(this.model.alk.heap, io.p, io.size)
  }

  run(): void {
    this.model.alk.x.alk_run(this.ptr)
  }

  reset(): void {
    this.model.alk.x.alk_state_clear(this.ptr)
  }

  // one call on any inputs: Timbre's windows of any length, Lex's int32 codes (held as floats, exact below 2^24)
  call(feeds: Record<string, Tensor>, wanted: string[] = this.plan.outputs.map((o) => o.name)): Record<string, Tensor> {
    const { alk } = this.model
    const { variable } = this.plan
    for (const io of this.plan.inputs) {
      const feed = feeds[io.name]
      if (!feed) throw new EngineError(`missing input ${io.name}`)
      if (io.name === variable) {
        if (alk.x.alk_set_len(this.ptr, feed.data.length) !== 0) alk.fail(io.name)
      } else if (feed.data.length !== io.size) throw new EngineError(`${io.name}: ${feed.data.length} values for ${io.size}`)
      this.f32(io.name).set(feed.data)
    }
    this.run()
    const out: Record<string, Tensor> = {}
    for (const name of wanted) out[name] = { data: this.f32(name).slice(), dims: [...this.io(name).dims] }
    return out
  }

  release(): void {
    this.model.alk.x.alk_session_free(this.ptr)
    this.at.clear()
  }
}
