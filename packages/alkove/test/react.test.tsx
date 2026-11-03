// @vitest-environment jsdom
import { act, render, renderHook } from '@testing-library/react'
import { StrictMode, createElement, type ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

class Fake {
  static all: Fake[] = []
  handlers = new Map<string, Set<(v: unknown) => void>>()
  status = 'running'
  mode = 'focus'
  track = { id: 'out' } as unknown as MediaStreamTrack
  disposed = false
  strength: number[] = []
  prints: unknown[] = []
  resumed = 0

  constructor(readonly opts: Record<string, unknown>) {
    Fake.all.push(this)
  }

  on(type: string, fn: (v: unknown) => void) {
    let set = this.handlers.get(type)
    if (!set) this.handlers.set(type, (set = new Set()))
    set.add(fn)
    return () => void set.delete(fn)
  }

  emit(type: string, v: unknown) {
    for (const fn of this.handlers.get(type) ?? []) fn(v)
  }

  listens(type: string) {
    return !!this.handlers.get(type)?.size
  }

  dispose() {
    this.disposed = true
    this.emit('status', 'disposed')
  }

  setStrength(s: number) {
    this.strength.push(s)
  }

  setVoiceprint(p: unknown) {
    this.prints.push(p)
  }

  async resume() {
    this.resumed++
  }
}

let gate: Promise<void> | null = null
let fail: Error | null = null
const calls: string[] = []
const make = (kind: string) =>
  vi.fn(async (opts: Record<string, unknown>) => {
    calls.push(`${kind}:start`)
    if (gate) await gate
    calls.push(`${kind}:end`)
    if (fail) throw fail
    return new Fake(opts)
  })

vi.mock('../src/recipes/input', () => ({ createAgentInput: make('agent'), createCallInput: make('call'), createWakeListener: make('wake') }))

const { useAgentInput, useCallInput, useWakeListener, same } = await import('../src/react')
const input = await import('../src/recipes/input')

const flush = () => act(() => new Promise<void>((r) => setTimeout(r, 0)))
const alive = () => Fake.all.filter((f) => !f.disposed)

beforeEach(() => {
  Fake.all = []
  gate = null
  fail = null
  calls.length = 0
  vi.mocked(input.createAgentInput).mockClear()
})

afterEach(() => vi.restoreAllMocks())

describe('alkove/react', () => {
  it('opens the input with its options, gives its state and forwards events to the latest handler', async () => {
    const seen: string[] = []
    const { result, rerender } = renderHook(({ tag }) => useAgentInput({ near: true, on: { turnend: () => seen.push(tag) } }), { initialProps: { tag: 'a' } })
    expect(result.current.status).toBe('starting')
    await flush()
    const f = Fake.all[0]!
    expect(f.opts.near).toBe(true)
    expect(f.opts.signal).toBeInstanceOf(AbortSignal)
    expect(result.current).toMatchObject({ input: f, status: 'running', mode: 'focus', track: f.track, error: null })
    act(() => f.emit('turnend', {}))
    rerender({ tag: 'b' })
    act(() => f.emit('turnend', {}))
    expect(seen).toEqual(['a', 'b'])
    expect(Fake.all).toHaveLength(1)
    act(() => f.emit('mode', { mode: 'raw', reason: 'overload' }))
    act(() => f.emit('status', 'blocked'))
    expect(result.current).toMatchObject({ mode: 'raw', status: 'blocked' })
    await act(() => result.current.resume())
    expect(f.resumed).toBe(1)
  })

  it('enabled: false opens nothing; switching it off disposes the input', async () => {
    const { result, rerender } = renderHook(({ enabled }) => useCallInput({ enabled }), { initialProps: { enabled: false } })
    await flush()
    expect(Fake.all).toHaveLength(0)
    expect(result.current.status).toBe('off')
    rerender({ enabled: true })
    await flush()
    expect(alive()).toHaveLength(1)
    rerender({ enabled: false })
    expect(Fake.all[0]!.disposed).toBe(true)
    expect(result.current).toMatchObject({ input: null, status: 'off', mode: null, track: null })
  })

  it('unmounted while opening: the mic request is aborted and the input disposed once it comes', async () => {
    let open!: () => void
    gate = new Promise((r) => (open = r))
    const { unmount } = renderHook(() => useAgentInput())
    await flush()
    unmount()
    const signal = vi.mocked(input.createAgentInput).mock.calls[0]![0]!.signal!
    expect(signal.aborted).toBe(true)
    open()
    await flush()
    expect(Fake.all).toHaveLength(1)
    expect(alive()).toHaveLength(0)
  })

  it("strict mode's double mount opens one input", async () => {
    const { result } = renderHook(() => useAgentInput(), { wrapper: ({ children }: { children: ReactNode }) => createElement(StrictMode, null, children) })
    await flush()
    expect(calls).toEqual(['agent:start', 'agent:end'])
    expect(alive()).toEqual([result.current.input])
  })

  it('switched off and on while opening: the second input opens once the first settled, never two at once', async () => {
    let open!: () => void
    gate = new Promise((r) => (open = r))
    const { result, rerender } = renderHook(({ enabled }) => useAgentInput({ enabled }), { initialProps: { enabled: true } })
    await flush()
    rerender({ enabled: false })
    rerender({ enabled: true })
    await flush()
    expect(calls).toEqual(['agent:start'])
    gate = null
    open()
    await flush()
    expect(calls).toEqual(['agent:start', 'agent:end', 'agent:start', 'agent:end'])
    expect(Fake.all[0]!.disposed).toBe(true)
    expect(alive()).toEqual([result.current.input])
  })

  it('an inline options object reopens nothing; a structural option reopens, strength and voiceprint apply live', async () => {
    const print = { alkove: 'voiceprint', embedding: [1, 2] }
    const { rerender } = renderHook((p: { near: boolean; strength: number; print: object }) => useAgentInput({ near: p.near, speech: { stopSecs: 0.8 }, strength: p.strength, voiceprint: p.print as never }), {
      initialProps: { near: true, strength: 1, print },
    })
    await flush()
    rerender({ near: true, strength: 1, print: { ...print, embedding: [1, 2] } })
    await flush()
    expect(Fake.all).toHaveLength(1)
    expect(Fake.all[0]!.prints).toEqual([])
    rerender({ near: true, strength: 0.5, print: { ...print, embedding: [3] } })
    await flush()
    expect(Fake.all).toHaveLength(1)
    expect(Fake.all[0]!.strength).toEqual([0.5])
    expect(Fake.all[0]!.prints).toEqual([{ ...print, embedding: [3] }])
    rerender({ near: false, strength: 0.5, print })
    await flush()
    expect(Fake.all).toHaveLength(2)
    expect(Fake.all[0]!.disposed).toBe(true)
    expect(Fake.all[1]!.opts).toMatchObject({ near: false, strength: 0.5, voiceprint: print })
  })

  it('a failed opening gives the error and status off', async () => {
    fail = new Error('mic.denied')
    const { result } = renderHook(() => useWakeListener({ words: ['hey nova'] }))
    await flush()
    expect(result.current).toMatchObject({ input: null, status: 'off', error: fail })
  })

  it('diag is subscribed only while the app has a handler, so the SDK warns in the console otherwise', async () => {
    const { rerender } = renderHook(({ diag }: { diag?: () => void }) => useAgentInput({ on: { diag } }), { initialProps: {} })
    await flush()
    expect(Fake.all[0]!.listens('diag')).toBe(false)
    const fn = vi.fn()
    rerender({ diag: fn })
    expect(Fake.all[0]!.listens('diag')).toBe(true)
    act(() => Fake.all[0]!.emit('diag', { code: 'x' }))
    expect(fn).toHaveBeenCalledWith({ code: 'x' })
    rerender({})
    expect(Fake.all[0]!.listens('diag')).toBe(false)
  })

  it('trackchange gives the new track', async () => {
    const { result } = renderHook(() => useCallInput())
    await flush()
    const t = { id: 'raw' } as unknown as MediaStreamTrack
    act(() => Fake.all[0]!.emit('trackchange', t))
    expect(result.current.track).toBe(t)
  })

  it('renders in a component', async () => {
    function Status() {
      const { status } = useAgentInput()
      return createElement('p', null, status)
    }
    const { container } = render(createElement(Status))
    await flush()
    expect(container.textContent).toBe('running')
  })

  it('same() compares content, toJSON() included, and other objects by identity', () => {
    const word = { toJSON: () => ({ text: 'hey' }) }
    expect(same({ a: [1, { b: 2 }], c: undefined }, { a: [1, { b: 2 }] })).toBe(true)
    expect(same(word, { text: 'hey' })).toBe(true)
    expect(same({ a: 1 }, { a: 2 })).toBe(false)
    const s1 = new (class Stream {})()
    const s2 = new (class Stream {})()
    expect(same(s1, s1)).toBe(true)
    expect(same(s1, s2)).toBe(false)
  })
})
