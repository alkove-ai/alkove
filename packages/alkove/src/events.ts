type Handler<T> = (value: T) => void

// one style of events: on(type, fn) returns its unsubscribe
export class Emitter<M extends object> {
  private handlers = new Map<keyof M, Set<Handler<never>>>()

  on<K extends keyof M>(type: K, fn: Handler<M[K]>): () => void {
    let set = this.handlers.get(type)
    if (!set) this.handlers.set(type, (set = new Set()))
    set.add(fn as Handler<never>)
    return () => void set.delete(fn as Handler<never>)
  }

  protected emit<K extends keyof M>(type: K, value: M[K]): void {
    for (const fn of this.handlers.get(type) ?? []) {
      try {
        ;(fn as Handler<M[K]>)(value)
      } catch (e) {
        queueMicrotask(() => {
          throw e
        })
      }
    }
  }

  protected listens(type: keyof M): boolean {
    return !!this.handlers.get(type)?.size
  }

  protected clear(): void {
    this.handlers.clear()
  }
}
