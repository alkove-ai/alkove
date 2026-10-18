import factory from 'alkove:ort-glue'

import { WasmEngine } from '../core/engine'
import { Ort } from '../engine/ort'
import { loadFile, loadModel } from '../loader'
import { type ModelName, ORT_WASM } from '../manifest'
import { makeCalls } from './calls'

// Node: the same wasm engine as the browser, in the caller's thread
export class NodeBackend {
  private engine: WasmEngine | null = null
  private bytes = new Map<ModelName, Uint8Array>()
  readonly calls = makeCalls(() => this.engine!)

  async ready(names: ModelName[]): Promise<this> {
    if (!this.engine) {
      const wasm = await loadFile(ORT_WASM, undefined, new URL('./alkove-ort.wasm', import.meta.url).href)
      const ort = await Ort.create(factory, await WebAssembly.compile(wasm as Uint8Array<ArrayBuffer>))
      this.engine = new WasmEngine(ort, (name) => this.bytes.get(name)!)
    }
    for (const n of names) if (!this.bytes.has(n)) this.bytes.set(n, await loadModel(n))
    return this
  }
}
