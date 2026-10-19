import { WasmEngine } from '../core/engine'
import { Alk } from '../engine/alk'
import { loadFile, loadModel } from '../loader'
import { ENGINE_WASM, type ModelName } from '../manifest'
import { makeCalls } from './calls'

// Node: the same wasm engine as the browser, in the caller's thread
export class NodeBackend {
  private engine: WasmEngine | null = null
  private bytes = new Map<ModelName, Uint8Array>()
  readonly calls = makeCalls(() => this.engine!)

  async ready(names: ModelName[]): Promise<this> {
    if (!this.engine) {
      const wasm = await loadFile(ENGINE_WASM, undefined, new URL('./alkove-engine.wasm', import.meta.url).href)
      const alk = await Alk.create(await WebAssembly.compile(wasm as Uint8Array<ArrayBuffer>))
      this.engine = new WasmEngine(alk, (name) => this.bytes.get(name)!)
    }
    for (const n of names) if (!this.bytes.has(n)) this.bytes.set(n, await loadModel(n))
    return this
  }
}
