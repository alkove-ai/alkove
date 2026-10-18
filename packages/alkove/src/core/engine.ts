import type { Binding, Ort, Session, Tensor } from '../engine/ort'
import { type ModelName, STREAMS } from '../manifest'

export type StreamModel = keyof typeof STREAMS

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

export class WasmEngine implements Engine {
  private sessions = new Map<ModelName, Session>()

  constructor(
    readonly ort: Ort,
    private bytes: (model: ModelName) => Uint8Array,
  ) {}

  has(model: ModelName): boolean {
    return this.sessions.has(model)
  }

  session(model: ModelName): Session {
    let s = this.sessions.get(model)
    if (!s) this.sessions.set(model, (s = this.ort.session(this.bytes(model))))
    return s
  }

  stream(model: StreamModel): Binding {
    const spec = STREAMS[model]
    return this.session(model).bind({ ...spec.shapes }, spec.loops)
  }

  net(model: 'timbre' | 'lex'): Net {
    return this.session(model)
  }

  release(): void {
    for (const s of this.sessions.values()) s.release()
    this.sessions.clear()
  }
}
