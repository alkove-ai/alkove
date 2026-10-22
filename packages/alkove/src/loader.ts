import { AlkoveError, type Diag } from './errors'
import { HF_ORG, type ModelFile, type ModelName, MODELS } from './manifest'

export type Source = ArrayBuffer | Uint8Array | URL | string

export interface Config {
  models?: Partial<Record<ModelName | 'engine', Source>>
  modelBaseUrl?: string | URL
  assetBaseUrl?: string | URL
  modelDir?: string
}

export interface Progress {
  loaded: number
  total: number
}

const CACHE = 'alkove'
const CACHE_HOST = 'https://cache.alkove.invalid/'
const UNUSED_DAYS = 30

let config: Config = {}
let locked = false
const diags = new Set<(d: Diag) => void>()

export function configure(c: Config): void {
  if (locked) report({ code: 'config.late', level: 'warn', message: 'Alkove.configure() after the first input: models already loaded keep their source.' })
  config = { ...config, ...c }
}

export const currentConfig = () => config
export const lockConfig = () => void (locked = true)

export function onDiag(fn: (d: Diag) => void): () => void {
  diags.add(fn)
  return () => void diags.delete(fn)
}

const warned = new Set<string>()

// with nobody listening, not an input either, a warning goes to the console once per code
export function report(d: Diag): void {
  for (const fn of diags) fn(d)
  if (diags.size || d.level !== 'warn' || warned.has(d.code)) return
  warned.add(d.code)
  console.warn(`alkove: ${d.message ?? d.code}`, d)
}

export async function sha256(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', bytes as Uint8Array<ArrayBuffer>)
  return Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, '0')).join('')
}

const isNode = () => typeof process !== 'undefined' && !!process.versions?.node && typeof window === 'undefined'
const hasCache = () => typeof caches !== 'undefined' && (typeof isSecureContext === 'undefined' || isSecureContext)

// a relative base ('/models/') is read against the page
export function under(base: string | URL, file: string): string {
  const page = (globalThis as { location?: { href: string } }).location?.href
  return new URL(file, new URL(String(base).replace(/\/?$/, '/'), page)).href
}

function urlOf(file: ModelFile): string {
  if (config.modelBaseUrl) return under(config.modelBaseUrl, file.file)
  if (!file.revision) throw new AlkoveError('models.fetch', `${file.name}: not on Hugging Face in this version, set modelBaseUrl`)
  report({ code: 'models.default-host', level: 'warn', message: `${file.name} comes from Hugging Face: each user sends it their IP. Self-host the models in production (modelBaseUrl).` })
  return `https://huggingface.co/${HF_ORG}/${file.name}/resolve/${file.revision}/${file.file}`
}

async function fetchBytes(url: string, onProgress?: (p: Progress) => void, total = 0): Promise<Uint8Array> {
  let res: Response
  try {
    res = await fetch(url)
  } catch (e) {
    throw new AlkoveError('models.fetch', `fetch failed: ${url}`, e)
  }
  if (!res.ok || !res.body) throw new AlkoveError('models.fetch', `HTTP ${res.status}: ${url}`)
  const size = Number(res.headers.get('content-length')) || total
  const reader = res.body.getReader()
  const parts: Uint8Array[] = []
  let loaded = 0
  for (;;) {
    const { done, value } = await reader.read()
    if (done) break
    parts.push(value)
    loaded += value.length
    onProgress?.({ loaded, total: Math.max(size, loaded) })
  }
  const out = new Uint8Array(loaded)
  let o = 0
  for (const p of parts) {
    out.set(p, o)
    o += p.length
  }
  return out
}

async function verified(file: ModelFile, bytes: Uint8Array): Promise<Uint8Array> {
  const hash = await sha256(bytes)
  if (hash !== file.sha256) throw new AlkoveError('models.hash', `${file.name}: sha256 ${hash}, expected ${file.sha256} (truncated download, or a CDN that rewrites files)`)
  return bytes
}

async function fromSource(file: ModelFile, src: Source, onProgress?: (p: Progress) => void): Promise<Uint8Array> {
  if (src instanceof ArrayBuffer) return verified(file, new Uint8Array(src))
  if (src instanceof Uint8Array) return verified(file, src)
  return verified(file, await fetchBytes(String(src), onProgress, file.bytes))
}

// a file checked once by its sha256 is never downloaded again; files no version asked for in 30 days are dropped
async function cached(file: ModelFile, load: () => Promise<Uint8Array>): Promise<Uint8Array> {
  if (!hasCache()) return load()
  let cache: Cache
  try {
    cache = await caches.open(CACHE)
  } catch {
    return load()
  }
  const key = CACHE_HOST + file.sha256
  const hit = await cache.match(key).catch(() => undefined)
  if (hit) {
    const bytes = new Uint8Array(await hit.arrayBuffer())
    if ((await sha256(bytes)) === file.sha256) {
      void touch(cache, file.sha256)
      return bytes
    }
    await cache.delete(key).catch(() => {})
  }
  const bytes = await load()
  await cache.put(key, new Response(bytes as Uint8Array<ArrayBuffer>, { headers: { 'content-type': 'application/octet-stream' } })).catch(() => {})
  void touch(cache, file.sha256)
  return bytes
}

async function touch(cache: Cache, hash: string): Promise<void> {
  try {
    const key = CACHE_HOST + 'used.json'
    const used: Record<string, number> = (await (await cache.match(key))?.json().catch(() => null)) ?? {}
    const now = Date.now()
    used[hash] = now
    for (const [h, at] of Object.entries(used))
      if (now - at > UNUSED_DAYS * 86400e3) {
        await cache.delete(CACHE_HOST + h)
        delete used[h]
      }
    await cache.put(key, new Response(JSON.stringify(used), { headers: { 'content-type': 'application/json' } }))
  } catch {}
}

export async function isCached(file: ModelFile): Promise<boolean> {
  if (config.models?.[nameOf(file)] instanceof ArrayBuffer || config.models?.[nameOf(file)] instanceof Uint8Array) return true
  if (!hasCache()) return false
  try {
    return !!(await (await caches.open(CACHE)).match(CACHE_HOST + file.sha256))
  } catch {
    return false
  }
}

const nameOf = (file: ModelFile) => (Object.entries(MODELS).find(([, f]) => f === file)?.[0] ?? 'engine') as ModelName | 'engine'

export async function loadFile(file: ModelFile, onProgress?: (p: Progress) => void, fallback?: Source): Promise<Uint8Array> {
  const own = config.models?.[nameOf(file)] ?? fallback
  if (own instanceof ArrayBuffer || own instanceof Uint8Array) return fromSource(file, own)
  if (isNode()) {
    if (own && String(own).startsWith('file:')) {
      const { readFile } = await import('node:fs/promises')
      return verified(file, new Uint8Array(await readFile(new URL(String(own)))))
    }
    if (config.modelDir) {
      const { readFile } = await import('node:fs/promises')
      const { join } = await import('node:path')
      return verified(file, new Uint8Array(await readFile(join(config.modelDir, file.file))))
    }
    return fromSource(file, own ?? urlOf(file), onProgress)
  }
  return cached(file, () => fromSource(file, own ?? urlOf(file), onProgress))
}

export function loadModel(name: ModelName, onProgress?: (p: Progress) => void): Promise<Uint8Array> {
  return loadFile(MODELS[name], onProgress)
}
