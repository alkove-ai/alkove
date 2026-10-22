import { readFile } from 'node:fs/promises'
import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http'
import type { AddressInfo } from 'node:net'
import { extname, join, normalize, sep } from 'node:path'

// the CSP of the CSP guide, exactly: the network test page is served with it
export const STRICT_CSP = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval' blob:; worker-src blob:; connect-src 'self'"

const TYPES: Record<string, string> = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.json': 'application/json',
  '.wasm': 'application/wasm',
  '.alk': 'application/octet-stream',
  '.wav': 'audio/wav',
  '.f32': 'application/octet-stream',
}

export interface Roots {
  dist: string
  models: string
  fixtures: string
  pages: string
}

interface Hit {
  path: string
  status: number
  at: number
}

// static files for the browser tests, plus model routes that fail on demand; every request is logged
export async function startServer(roots: Roots): Promise<{ origin: string; server: Server }> {
  const log: Hit[] = []
  // /flaky/<token>/…: 404 until the token is switched on
  const flaky = new Map<string, boolean>()

  const file = (root: string, rel: string) => {
    const p = normalize(join(root, decodeURIComponent(rel)))
    if (!p.startsWith(root + sep) && p !== root) throw Object.assign(new Error('outside'), { status: 403 })
    return p
  }

  const send = async (res: ServerResponse, path: string, headers: Record<string, string> = {}, mutate?: (b: Buffer) => Buffer) => {
    const raw: Buffer = await readFile(path)
    const body = mutate ? mutate(raw) : raw
    res.writeHead(200, { 'content-type': TYPES[extname(path)] ?? 'application/octet-stream', 'content-length': body.length, 'cache-control': 'no-store', ...headers })
    res.end(body)
  }

  const route = async (req: IncomingMessage, res: ServerResponse, url: URL): Promise<void> => {
    const p = url.pathname
    let m: RegExpMatchArray | null
    if (p === '/__log') {
      const token = url.searchParams.get('token') ?? ''
      res.writeHead(200, { 'content-type': 'application/json' })
      res.end(JSON.stringify(log.filter((h) => !token || h.path.includes(token))))
      return
    }
    if ((m = p.match(/^\/__flaky\/([\w-]+)\/(on|off)$/))) {
      flaky.set(m[1]!, m[2] === 'on')
      res.writeHead(204).end()
      return
    }
    if (p === '/' || p === '/index.html') return send(res, join(roots.pages, 'index.html'))
    if (p === '/csp.html') return send(res, join(roots.pages, 'csp.html'), { 'content-security-policy': STRICT_CSP })
    if (p.startsWith('/browser/')) return send(res, file(roots.pages, p.slice('/browser/'.length)))
    if (p.startsWith('/dist/')) return send(res, file(roots.dist, p.slice('/dist/'.length)))
    if (p.startsWith('/fixtures/')) return send(res, file(roots.fixtures, p.slice('/fixtures/'.length)))
    if (p.startsWith('/models/')) return send(res, file(roots.models, p.slice('/models/'.length)))
    // one byte changed in the middle: the sha256 no longer matches
    if (p.startsWith('/corrupt/'))
      return send(res, file(roots.models, p.slice('/corrupt/'.length)), {}, (b) => {
        const c = Buffer.from(b)
        const i = c.length >> 1
        c.writeUInt8(c.readUInt8(i) ^ 0xff, i)
        return c
      })
    // /held/<token>/…: waits until the token is switched on, a slow first visit
    if ((m = p.match(/^\/held\/([\w-]+)\/(.+)$/))) {
      while (!flaky.get(m[1]!)) await new Promise((r) => setTimeout(r, 50))
      return send(res, file(roots.models, m[2]!))
    }
    if ((m = p.match(/^\/flaky\/([\w-]+)\/(.+)$/))) {
      if (!flaky.get(m[1]!)) {
        res.writeHead(404, { 'content-type': 'text/plain' }).end('not found')
        return
      }
      return send(res, file(roots.models, m[2]!))
    }
    res.writeHead(404, { 'content-type': 'text/plain' }).end('not found')
  }

  const server = createServer((req, res) => {
    const url = new URL(req.url ?? '/', 'http://x')
    const hit: Hit = { path: url.pathname, status: 0, at: Date.now() }
    if (!url.pathname.startsWith('/__')) log.push(hit)
    res.on('finish', () => (hit.status = res.statusCode))
    route(req, res, url).catch((e: Error & { status?: number; code?: string }) => {
      const status = e.status ?? (e.code === 'ENOENT' || e.code === 'EISDIR' ? 404 : 500)
      if (!res.headersSent) res.writeHead(status, { 'content-type': 'text/plain' })
      res.end(String(e.message))
    })
  })
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve))
  const { port } = server.address() as AddressInfo
  return { origin: `http://127.0.0.1:${port}`, server }
}
