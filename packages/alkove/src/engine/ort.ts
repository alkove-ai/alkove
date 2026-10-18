// onnxruntime's wasm C API (onnxruntime/wasm/api.h, 1.30) called directly: synchronous runs, tensors allocated once.
export interface OrtModule {
  HEAPU8: Uint8Array
  _malloc(size: number): number
  _free(ptr: number): void
  stackSave(): number
  stackRestore(ptr: number): void
  stackAlloc(size: number): number
  UTF8ToString(ptr: number): string
  _OrtInit(threads: number, logLevel: number): number
  _OrtGetLastError(codePtr: number, messagePtr: number): number
  _OrtCreateSessionOptions(
    optimization: number,
    arena: boolean,
    memPattern: boolean,
    executionMode: number,
    profiling: boolean,
    profilePrefix: number,
    logId: number,
    logSeverity: number,
    logVerbosity: number,
    optimizedPath: number,
  ): number
  _OrtReleaseSessionOptions(handle: number): number
  _OrtCreateSession(data: number, length: number, options: number): number
  _OrtReleaseSession(handle: number): number
  _OrtGetInputOutputCount(session: number, inPtr: number, outPtr: number): number
  _OrtGetInputOutputMetadata(session: number, index: number, namePtr: number, metaPtr: number): number
  _OrtFree(ptr: number): number
  _OrtCreateTensor(type: number, data: number, length: number, dims: number, rank: number, location: number): number
  _OrtGetTensorData(tensor: number, typePtr: number, dataPtr: number, dimsPtr: number, rankPtr: number): number
  _OrtReleaseTensor(tensor: number): number
  _OrtRun(
    session: number,
    inNames: number,
    inValues: number,
    inCount: number,
    outNames: number,
    outCount: number,
    outValues: number,
    runOptions: number,
  ): number
}

export type OrtFactory = (args: Record<string, unknown>) => Promise<OrtModule>

export type Dtype = 'float32' | 'int32'
export type TypedData = Float32Array | Int32Array

export interface Tensor {
  data: TypedData
  dims: number[]
}

const TYPES: Record<Dtype, number> = { float32: 1, int32: 6 }
const CPU = 1
const OPTIMIZE_ALL = 99
const LOG_ERROR = 3

export class OrtError extends Error {}

export class Ort {
  private constructor(readonly m: OrtModule) {
    if (m._OrtInit(1, LOG_ERROR) !== 0) this.fail('init')
  }

  // the compiled module is shared: one download and one compilation for every worker
  static async create(factory: OrtFactory, wasm: WebAssembly.Module): Promise<Ort> {
    const m = await factory({
      numThreads: 1,
      instantiateWasm(imports: WebAssembly.Imports, done: (instance: WebAssembly.Instance, module: WebAssembly.Module) => void) {
        void WebAssembly.instantiate(wasm, imports).then((instance) => done(instance, wasm))
        return {}
      },
      locateFile: (name: string) => name,
    })
    return new Ort(m)
  }

  get heap(): ArrayBuffer {
    return this.m.HEAPU8.buffer as ArrayBuffer
  }

  u32(ptr: number, value?: number): number {
    const view = new Uint32Array(this.heap, ptr, 1)
    if (value !== undefined) view[0] = value
    return view[0]!
  }

  fail(what: string): never {
    const m = this.m
    const stack = m.stackSave()
    try {
      const p = m.stackAlloc(8)
      m._OrtGetLastError(p, p + 4)
      const code = this.u32(p)
      const msg = this.u32(p + 4)
      throw new OrtError(`onnxruntime ${what}: ${code} ${msg ? m.UTF8ToString(msg) : ''}`.trim())
    } finally {
      m.stackRestore(stack)
    }
  }

  string(text: string): number {
    const bytes = new TextEncoder().encode(text + '\0')
    const p = this.m._malloc(bytes.length)
    this.m.HEAPU8.set(bytes, p)
    return p
  }

  session(bytes: Uint8Array): Session {
    return new Session(this, bytes)
  }
}

interface Slot {
  name: number
  type: Dtype
}

export class Session {
  readonly inputs: string[] = []
  readonly outputs: string[] = []
  readonly handle: number
  private slots = new Map<string, Slot>()

  constructor(
    readonly ort: Ort,
    bytes: Uint8Array,
  ) {
    const m = ort.m
    const logId = ort.string('')
    const options = m._OrtCreateSessionOptions(OPTIMIZE_ALL, true, true, 0, false, 0, logId, LOG_ERROR, 0, 0)
    m._free(logId)
    if (!options) ort.fail('session options')
    const data = m._malloc(bytes.byteLength)
    m.HEAPU8.set(bytes, data)
    try {
      this.handle = m._OrtCreateSession(data, bytes.byteLength, options)
      if (!this.handle) ort.fail('session')
    } finally {
      m._free(data)
      m._OrtReleaseSessionOptions(options)
    }
    const stack = m.stackSave()
    try {
      const p = m.stackAlloc(8)
      if (m._OrtGetInputOutputCount(this.handle, p, p + 4) !== 0) ort.fail('io count')
      const nIn = ort.u32(p)
      const nOut = ort.u32(p + 4)
      for (let i = 0; i < nIn + nOut; i++) {
        if (m._OrtGetInputOutputMetadata(this.handle, i, p, p + 4) !== 0) ort.fail('io metadata')
        const name = ort.u32(p)
        const meta = ort.u32(p + 4)
        const type = new Int32Array(ort.heap, meta, 1)[0] === TYPES.int32 ? 'int32' : 'float32'
        m._OrtFree(meta)
        const text = m.UTF8ToString(name)
        ;(i < nIn ? this.inputs : this.outputs).push(text)
        this.slots.set(text, { name, type })
      }
    } finally {
      m.stackRestore(stack)
    }
  }

  slot(name: string): Slot {
    const s = this.slots.get(name)
    if (!s) throw new OrtError(`no tensor named ${name}`)
    return s
  }

  // any shapes, every call allocates: Timbre's windows, Lex's beams
  run(feeds: Record<string, Tensor>, wanted: string[] = this.outputs): Record<string, Tensor> {
    const { ort } = this
    const m = ort.m
    const names = Object.keys(feeds)
    const owned: number[] = []
    const tensors: number[] = []
    const stack = m.stackSave()
    try {
      for (const name of names) {
        const { data, dims } = feeds[name]!
        const p = m._malloc(Math.max(4, data.byteLength))
        owned.push(p)
        m.HEAPU8.set(new Uint8Array(data.buffer, data.byteOffset, data.byteLength), p)
        tensors.push(createTensor(ort, this.slot(name).type, p, data.byteLength, dims))
      }
      const inNames = m.stackAlloc(4 * names.length)
      const inValues = m.stackAlloc(4 * names.length)
      const outNames = m.stackAlloc(4 * wanted.length)
      const outValues = m.stackAlloc(4 * wanted.length)
      names.forEach((n, i) => {
        ort.u32(inNames + 4 * i, this.slot(n).name)
        ort.u32(inValues + 4 * i, tensors[i]!)
      })
      wanted.forEach((n, i) => {
        ort.u32(outNames + 4 * i, this.slot(n).name)
        ort.u32(outValues + 4 * i, 0)
      })
      if (m._OrtRun(this.handle, inNames, inValues, names.length, outNames, wanted.length, outValues, 0) !== 0) ort.fail('run')
      const out: Record<string, Tensor> = {}
      wanted.forEach((n, i) => {
        const t = ort.u32(outValues + 4 * i)
        try {
          out[n] = readTensor(ort, t)
        } finally {
          m._OrtReleaseTensor(t)
        }
      })
      return out
    } finally {
      m.stackRestore(stack)
      for (const t of tensors) m._OrtReleaseTensor(t)
      for (const p of owned) m._free(p)
    }
  }

  bind(shapes: Record<string, number[]>, loops: [input: string, output: string][] = []): Binding {
    return new Binding(this, shapes, loops)
  }

  release(): void {
    const m = this.ort.m
    m._OrtReleaseSession(this.handle)
    for (const s of this.slots.values()) m._OrtFree(s.name)
    this.slots.clear()
  }
}

function createTensor(ort: Ort, type: Dtype, data: number, bytes: number, dims: number[]): number {
  const m = ort.m
  const stack = m.stackSave()
  try {
    const p = m.stackAlloc(4 * Math.max(1, dims.length))
    dims.forEach((d, i) => ort.u32(p + 4 * i, d))
    const t = m._OrtCreateTensor(TYPES[type], data, bytes, p, dims.length, CPU)
    if (!t) ort.fail('tensor')
    return t
  } finally {
    m.stackRestore(stack)
  }
}

function readTensor(ort: Ort, tensor: number): Tensor {
  const m = ort.m
  const stack = m.stackSave()
  try {
    const p = m.stackAlloc(16)
    if (m._OrtGetTensorData(tensor, p, p + 4, p + 8, p + 12) !== 0) ort.fail('tensor data')
    const type = ort.u32(p)
    const data = ort.u32(p + 4)
    const dimsPtr = ort.u32(p + 8)
    const rank = ort.u32(p + 12)
    const dims = Array.from(new Uint32Array(ort.heap, dimsPtr, rank))
    m._OrtFree(dimsPtr)
    const size = dims.reduce((a, b) => a * b, 1)
    const view = type === TYPES.int32 ? new Int32Array(ort.heap, data, size) : new Float32Array(ort.heap, data, size)
    return { data: view.slice(), dims }
  } finally {
    m.stackRestore(stack)
  }
}

interface Buffer {
  ptr: number
  size: number
  type: Dtype
  tensor: number
}

// fixed shapes: inputs and outputs live in wasm memory for the session's life, each loop (state -> next_state) swaps its two buffers after a run
export class Binding {
  private buffers = new Map<string, Buffer>()
  private inNames: number
  private inValues: number
  private outNames: number
  private outValues: number
  private order: { inputs: string[]; outputs: string[] }

  constructor(
    readonly session: Session,
    shapes: Record<string, number[]>,
    private loops: [string, string][],
  ) {
    const { ort } = session
    const m = ort.m
    const make = (name: string): Buffer => {
      const dims = shapes[name]
      if (!dims) throw new OrtError(`no shape for ${name}`)
      const type = session.slot(name).type
      const size = dims.reduce((a, b) => a * b, 1)
      const ptr = m._malloc(4 * size)
      new Uint8Array(ort.heap, ptr, 4 * size).fill(0)
      return { ptr, size, type, tensor: createTensor(ort, type, ptr, 4 * size, dims) }
    }
    for (const name of session.inputs) this.buffers.set(name, make(name))
    for (const [input, output] of loops) shapes[output] ??= shapes[input]!
    for (const name of session.outputs) this.buffers.set(name, make(name))
    this.order = { inputs: session.inputs, outputs: session.outputs }
    this.inNames = m._malloc(4 * session.inputs.length)
    this.inValues = m._malloc(4 * session.inputs.length)
    this.outNames = m._malloc(4 * session.outputs.length)
    this.outValues = m._malloc(4 * session.outputs.length)
    session.inputs.forEach((n, i) => ort.u32(this.inNames + 4 * i, session.slot(n).name))
    session.outputs.forEach((n, i) => ort.u32(this.outNames + 4 * i, session.slot(n).name))
    this.write()
  }

  private write(): void {
    const { ort } = this.session
    this.order.inputs.forEach((n, i) => ort.u32(this.inValues + 4 * i, this.buffers.get(n)!.tensor))
    this.order.outputs.forEach((n, i) => ort.u32(this.outValues + 4 * i, this.buffers.get(n)!.tensor))
  }

  // a fresh view each time: growing the wasm memory detaches the old ones
  f32(name: string): Float32Array {
    const b = this.buffers.get(name)!
    return new Float32Array(this.session.ort.heap, b.ptr, b.size)
  }

  i32(name: string): Int32Array {
    const b = this.buffers.get(name)!
    return new Int32Array(this.session.ort.heap, b.ptr, b.size)
  }

  run(): void {
    const { ort, handle } = this.session
    const n = this.order
    if (ort.m._OrtRun(handle, this.inNames, this.inValues, n.inputs.length, this.outNames, n.outputs.length, this.outValues, 0) !== 0) ort.fail('run')
    if (!this.loops.length) return
    for (const [input, output] of this.loops) {
      const a = this.buffers.get(input)!
      this.buffers.set(input, this.buffers.get(output)!)
      this.buffers.set(output, a)
    }
    this.write()
  }

  reset(): void {
    for (const [input] of this.loops) this.f32(input).fill(0)
  }

  release(): void {
    const m = this.session.ort.m
    for (const b of this.buffers.values()) {
      m._OrtReleaseTensor(b.tensor)
      m._free(b.ptr)
    }
    this.buffers.clear()
    for (const p of [this.inNames, this.inValues, this.outNames, this.outValues]) m._free(p)
  }
}
