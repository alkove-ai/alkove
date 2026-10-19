import { AlkoveError } from '../errors'
import { currentConfig, loadFile, loadModel, lockConfig, type Progress, report, under } from '../loader'
import { ENGINE_WASM, type ModelName } from '../manifest'

type Reply = { resolve: (v: any) => void; reject: (e: unknown) => void; progress?: (p: any) => void }

// a worker spoken to by messages; replies matched by id
export class WorkerLink {
  private next = 1
  private replies = new Map<number, Reply>()
  private listeners = new Set<(m: any) => void>()

  constructor(readonly worker: Worker) {
    worker.onmessage = (e: MessageEvent) => {
      const m = e.data
      if (m?.type === 'reply') {
        const r = this.replies.get(m.id)
        this.replies.delete(m.id)
        if (m.error) r?.reject(Object.assign(new Error(m.error.message), { code: m.error.code }))
        else r?.resolve(m.result)
      } else if (m?.type === 'progress') this.replies.get(m.id)?.progress?.(m.p)
      else for (const fn of this.listeners) fn(m)
    }
    worker.onerror = (e) => {
      for (const r of this.replies.values()) r.reject(new AlkoveError('engine.failed', e.message || 'worker failed'))
      this.replies.clear()
      for (const fn of this.listeners) fn({ type: 'crash', message: e.message })
    }
  }

  listen(fn: (m: any) => void): () => void {
    this.listeners.add(fn)
    return () => void this.listeners.delete(fn)
  }

  get busy(): boolean {
    return this.replies.size > 0
  }

  request<T>(message: Record<string, unknown>, transfer: Transferable[] = [], progress?: (p: any) => void): Promise<T> {
    const id = this.next++
    return new Promise<T>((resolve, reject) => {
      this.replies.set(id, { resolve, reject, progress })
      this.worker.postMessage({ ...message, id, progress: !!progress }, transfer)
    })
  }

  call<T>(method: string, args: unknown[], transfer: Transferable[] = [], progress?: (p: any) => void): Promise<T> {
    return this.request<T>({ type: 'call', method, args }, transfer, progress)
  }

  post(message: Record<string, unknown>, transfer: Transferable[] = []): void {
    this.worker.postMessage(message, transfer)
  }
}

interface Assets {
  WORKER: string
  WORKLET: string
}

let assets: Promise<Assets> | null = null
const loadAssets = () => (assets ??= import('./assets') as Promise<Assets>)

export async function scriptUrl(kind: 'worker' | 'worklet'): Promise<string> {
  const base = currentConfig().assetBaseUrl
  if (base) return under(base, `alkove-${kind}.js`)
  const a = await loadAssets()
  return URL.createObjectURL(new Blob([kind === 'worker' ? a.WORKER : a.WORKLET], { type: 'text/javascript' }))
}

let wasm: Promise<WebAssembly.Module> | null = null

export function engineModule(onProgress?: (p: Progress) => void): Promise<WebAssembly.Module> {
  return (wasm ??= (async () => {
    const own = currentConfig().models?.engine
    const base = currentConfig().assetBaseUrl
    const url = own ?? (base ? under(base, ENGINE_WASM.file) : new URL('./alkove-engine.wasm', import.meta.url).href)
    const bytes = await loadFile(ENGINE_WASM, onProgress, url)
    return WebAssembly.compile(bytes as Uint8Array<ArrayBuffer>)
  })().catch((e) => {
    wasm = null
    throw e
  }))
}

// one engine for the page: two workers (real time, background) sharing one compiled wasm
export class Host {
  private static current: Host | null = null
  private sent = { realtime: new Set<ModelName>(), background: new Set<ModelName>() }
  private links: Partial<Record<'realtime' | 'background', Promise<WorkerLink>>> = {}
  // learners and streams whose state lives in the background worker: it stays while they do
  private holds = 0

  static get(): Host {
    lockConfig()
    return (Host.current ??= new Host())
  }

  link(role: 'realtime' | 'background'): Promise<WorkerLink> {
    return (this.links[role] ??= (async () => {
      const [module, url] = await Promise.all([engineModule(), scriptUrl('worker')])
      const link = new WorkerLink(new Worker(url, { type: 'module', name: `alkove-${role}` }))
      const mine = this.links[role]
      link.listen((m) => m.type === 'crash' && this.links[role] === mine && this.drop(role, link))
      await new Promise<void>((resolve, reject) => {
        const off = link.listen((m) => {
          if (m.type === 'booted') {
            off()
            resolve()
          } else if (m.type === 'crash') reject(new AlkoveError('engine.failed', m.message))
        })
        link.post({ type: 'boot', module })
      })
      return link
    })().catch((e) => {
      delete this.links[role]
      throw e
    }))
  }

  private drop(role: 'realtime' | 'background', link: WorkerLink): void {
    delete this.links[role]
    this.sent[role].clear()
    link.worker.terminate()
  }

  hold(): () => void {
    this.holds++
    let held = true
    return () => {
      if (held) this.holds--
      held = false
    }
  }

  // no input left: the workers and their sessions go (the background one only when no call waits and nothing is held); the next use starts them again
  async idle(): Promise<void> {
    for (const role of ['realtime', 'background'] as const) {
      const p = this.links[role]
      const link = p && (await p.catch(() => null))
      if (link && this.links[role] === p && (role === 'realtime' || (!link.busy && !this.holds))) this.drop(role, link)
    }
  }

  async models(role: 'realtime' | 'background', names: ModelName[], onProgress?: (p: Progress) => void): Promise<WorkerLink> {
    const link = await this.link(role)
    const missing = names.filter((n) => !this.sent[role].has(n))
    if (!missing.length) return link
    const totals = new Map<ModelName, Progress>()
    const bytes = await Promise.all(
      missing.map((n) =>
        loadModel(n, (p) => {
          totals.set(n, p)
          let loaded = 0
          let total = 0
          for (const v of totals.values()) {
            loaded += v.loaded
            total += v.total
          }
          onProgress?.({ loaded, total })
        }),
      ),
    )
    const payload = Object.fromEntries(missing.map((n, i) => [n, bytes[i]!.slice().buffer]))
    await link.request({ type: 'models', models: payload }, Object.values(payload))
    for (const n of missing) this.sent[role].add(n)
    return link
  }

  static report = report
}
