import type { Action, Command, Lang } from '../types'

type Row = [Action, string[], string?]

const DEFAULTS: Record<Lang, Row[]> = {
  en: [
    ['abort', ['stop', 'cancel']],
    ['send', ['send', 'send it']],
    ['clear', ['clear', 'clear the prompt']],
    ['run', ['continue', 'go on'], 'continue'],
    ['run', ['compact'], '/compact'],
    ['run', ['run the tests'], 'Run the tests.'],
    ['run', ['commit'], 'Commit the changes.'],
  ],
  fr: [
    ['abort', ['stop', 'arrête', 'annule']],
    ['send', ['envoie']],
    ['clear', ['efface']],
    ['run', ['continue', 'vas-y'], 'continue'],
    ['run', ['compacte'], '/compact'],
    ['run', ['lance les tests'], 'Run the tests.'],
    ['run', ['commit', 'commite'], 'Commit the changes.'],
  ],
  es: [
    ['abort', ['para', 'stop', 'cancela']],
    ['send', ['envía', 'envíalo']],
    ['clear', ['borra']],
    ['run', ['continúa', 'sigue'], 'continue'],
    ['run', ['compacta'], '/compact'],
    ['run', ['lanza los tests', 'ejecuta los tests'], 'Run the tests.'],
    ['run', ['commit', 'haz commit'], 'Commit the changes.'],
  ],
  it: [
    ['abort', ['ferma', 'stop', 'annulla']],
    ['send', ['invia', 'mandalo']],
    ['clear', ['cancella']],
    ['run', ['continua', 'vai avanti'], 'continue'],
    ['run', ['compatta'], '/compact'],
    ['run', ['lancia i test', 'esegui i test'], 'Run the tests.'],
    ['run', ['commit', 'fai il commit'], 'Commit the changes.'],
  ],
}

// what an earlier version stored: `continue`, `compact` and `prompt` became `run`
export function upgrade(c: Omit<Command, 'action'> & { action: string }): Command | undefined {
  if (c.action === 'continue') return { ...c, action: 'run', text: 'continue' }
  if (c.action === 'compact') return { ...c, action: 'run', text: '/compact' }
  if (c.action === 'prompt') return { ...c, action: 'run' }
  return c.action === 'abort' || c.action === 'send' || c.action === 'clear' || c.action === 'run' ? { ...c, action: c.action } : undefined
}

export function defaults(lang: Lang): Command[] {
  return DEFAULTS[lang].map(([action, phrases, text], i) => ({ id: `${action}-${i}`, action, phrases, text: text ?? null }))
}

// a command is short: anything longer is dictation, never compared
export const MAX_WORDS = 4

// lower case, no accents, no punctuation, single spaces
export function norm(s: string): string {
  return s
    .normalize('NFD')
    .replace(/[̀-ͯ]/g, '')
    .toLowerCase()
    .replace(/[^\p{L}\p{N}' -]+/gu, ' ')
    .replace(/['-]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
}

export function distance(a: string, b: string): number {
  const row = Array.from({ length: b.length + 1 }, (_, j) => j)
  for (let i = 1; i <= a.length; i++) {
    let prev = row[0]!
    row[0] = i
    for (let j = 1; j <= b.length; j++) {
      const next = Math.min(prev + (a[i - 1] === b[j - 1] ? 0 : 1), row[j - 1]! + 1, row[j]! + 1)
      prev = row[j]!
      row[j] = next
    }
  }
  return row[b.length]!
}

// the command a phrase says, allowing what Whistle gets slightly wrong ("stopp" for "stop"); none for anything longer or farther
export function match(text: string, commands: Command[]): Command | undefined {
  const said = norm(text)
  if (said.split(' ').length > MAX_WORDS) return undefined
  let best: { c: Command; d: number } | undefined
  for (const c of commands) {
    for (const p of c.phrases) {
      const want = norm(p)
      // one edit for a short word: two would take "sort" for "stop" and "and" for "send"
      const allowed = want.length < 10 ? 1 : Math.floor(want.length / 5)
      // Whistle often splits a word in two, misheard ("est face" for "efface"): one edit more once joined
      const joined = distance(said.replaceAll(' ', ''), want.replaceAll(' ', '')) - (said.includes(' ') && !want.includes(' ') ? 1 : 0)
      const d = Math.min(distance(said, want), joined)
      if (d <= allowed && (!best || d < best.d)) best = { c, d }
    }
  }
  return best?.c
}
