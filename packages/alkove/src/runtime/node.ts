import { WasmEngine } from '../core/engine'
import { Alk } from '../engine/alk'
import { loadFile, loadModel } from '../loader'
import { ENGINE_WASM, type ModelName } from '../manifest'
import { makeCalls } from './calls'

// Node: the same wasm engine as the browser, in the caller's thread
export class NodeBackend {
  private engine: WasmEngine | null = null
  private boot: Promise<void> | null = null
  private bytes = new Map<ModelName, Uint8Array>()
  readonly calls = makeCalls(() => this.engine!)

  async ready(names: ModelName[]): Promise<this> {
    // one engine for calls that start together: handles of one instance mean nothing to another
    await (this.boot ??= (async () => {
      const wasm = await loadFile(ENGINE_WASM, undefined, new URL('./alkove-engine.wasm', import.meta.url).href)
      const alk = await Alk.create(await WebAssembly.compile(wasm as Uint8Array<ArrayBuffer>))
      this.engine = new WasmEngine(alk, (name) => this.bytes.get(name)!)
    })().catch((e) => {
      this.boot = null
      throw e
    }))
    for (const n of names) if (!this.bytes.has(n)) this.bytes.set(n, await loadModel(n))
    return this
  }
}
