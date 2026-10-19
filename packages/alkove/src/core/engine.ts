import type { Alk, Model, Session, Tensor } from '../engine/alk'
import type { ModelName } from '../manifest'

export type StreamModel = 'focus' | 'cue' | 'spot'

export interface StreamNet {
  f32(name: string): Float32Array
  run(): void
  reset(): void
  release(): void
}

export interface Net {
  run(feeds: Record<string, Tensor>, wanted?: string[]): Record<string, Tensor>
}

// what the primitives run on: the wasm engine by default, a native bridge later
export interface Engine {
  stream(model: StreamModel): StreamNet
  net(model: 'timbre' | 'lex'): Net
}

// one model per name (weights unpacked once), one session per stream: each keeps its own state
export class WasmEngine implements Engine {
  private models = new Map<ModelName, Model>()
  private calls = new Map<ModelName, Session>()

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

  net(model: 'timbre' | 'lex'): Net {
    let s = this.calls.get(model)
    if (!s) this.calls.set(model, (s = this.model(model).session()))
    const session = s
    return { run: (feeds, wanted) => session.call(feeds, wanted) }
  }

  release(): void {
    for (const s of this.calls.values()) s.release()
    for (const m of this.models.values()) m.release()
    this.calls.clear()
    this.models.clear()
  }
}
