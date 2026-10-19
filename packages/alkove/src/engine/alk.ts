// the Alkove engine's wasm (crates/engine-wasm): no imports, synchronous runs, nothing allocated while it runs
export interface AlkExports {
  memory: WebAssembly.Memory
  alk_alloc(len: number): number
  alk_free(p: number, len: number): void
  alk_model(bytes: number, len: number): number
  alk_model_free(m: number): void
  alk_error(out: number, cap: number): number
  alk_session(m: number): number
  alk_session_free(s: number): void
  alk_input(s: number, name: number, len: number): number
  alk_output(s: number, name: number, len: number): number
  alk_run(s: number): void
  alk_state_clear(s: number): void
  alk_variable(m: number): number
  alk_set_len(s: number, len: number): number
  alk_io_count(m: number, kind: number): number
  alk_io_name(m: number, kind: number, i: number, out: number, cap: number): number
  alk_io_shape(m: number, kind: number, i: number, dims: number, cap: number): number
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
  readonly inputs: Io[]
  readonly outputs: Io[]
  readonly variable: string | null

  constructor(
    readonly alk: Alk,
    bytes: Uint8Array,
  ) {
    const x = alk.x
    this.ptr = alk.with(bytes, (p, n) => x.alk_model(p, n))
    if (!this.ptr) alk.fail('model')
    const ios = (kind: number): Io[] => {
      const out: Io[] = []
      const buf = x.alk_alloc(256)
      try {
        for (let i = 0; i < x.alk_io_count(this.ptr, kind); i++) {
          const name = new TextDecoder().decode(new Uint8Array(alk.heap, buf, x.alk_io_name(this.ptr, kind, i, buf, 256)))
          const rank = x.alk_io_shape(this.ptr, kind, i, buf, 64)
          const dims = Array.from(new Uint32Array(alk.heap, buf, rank))
          out.push({ name, dims, size: dims.reduce((a, b) => a * b, 1) })
        }
      } finally {
        x.alk_free(buf, 256)
      }
      return out
    }
    this.inputs = ios(0)
    this.outputs = ios(1)
    this.variable = this.inputs[x.alk_variable(this.ptr)]?.name ?? null
  }

  session(): Session {
    return new Session(this)
  }

  release(): void {
    this.alk.x.alk_model_free(this.ptr)
  }
}

// one arena: the stream's state, its inputs and outputs, at fixed places for the session's life
export class Session {
  private readonly ptr: number
  private readonly at = new Map<string, Io & { p: number }>()

  constructor(readonly model: Model) {
    const { alk } = model
    const x = alk.x
    this.ptr = x.alk_session(model.ptr)
    const name = new TextEncoder()
    for (const [ios, find] of [
      [model.inputs, x.alk_input],
      [model.outputs, x.alk_output],
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
  call(feeds: Record<string, Tensor>, wanted: string[] = this.model.outputs.map((o) => o.name)): Record<string, Tensor> {
    const { alk, variable } = this.model
    for (const io of this.model.inputs) {
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
