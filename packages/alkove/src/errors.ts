export type ErrorCode = 'mic.denied' | 'mic.not-found' | 'mic.busy' | 'page.insecure' | 'browser.unsupported' | 'models.fetch' | 'models.hash' | 'engine.failed' | 'lang.unsupported' | 'word.outdated'

// only what leaves the app without any sound fails an input; the rest is `mode`, `diag` and retries
export class AlkoveError extends Error {
  readonly name = 'AlkoveError'

  constructor(
    readonly code: ErrorCode,
    message: string,
    readonly cause?: unknown,
  ) {
    super(message)
  }
}

export interface Diag {
  code: string
  level: 'info' | 'warn'
  message?: string
  [key: string]: unknown
}
