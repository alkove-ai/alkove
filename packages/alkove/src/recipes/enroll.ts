import type { Warning } from '../core/lex'
import type { Take } from '../core/spot/spot'
import { Word, type WordJSON, wordOf } from '../core/spot/spot'
import type { Voiceprint } from '../core/timbre'
import { AlkoveError } from '../errors'
import { Emitter } from '../events'

export interface EnrollTarget {
  on(type: 'take', fn: (t: Take) => void): () => void
  on(type: 'enrolled', fn: (r: { word?: WordJSON; warnings?: Warning[]; audio?: Float32Array; error?: string }) => void): () => void
  stop(): void
  source: 'raw' | 'focus'
}

export interface EnrollTools {
  compile(): Promise<Word>
  open(word: WordJSON): Promise<EnrollTarget>
  embed(pcm: Float32Array, source: 'raw' | 'focus'): Promise<Voiceprint>
}

// the user says the word a few times: its thresholds come from these takes and from what is heard between them; the takes' voice comes back apart, never in the word
export class Enrollment extends Emitter<{ take: Take; warning: Warning }> {
  readonly done: Promise<{ word: Word; voiceprint: Voiceprint | null }>
  private target: EnrollTarget | null = null
  private cancelled = false
  private abort: (() => void) | null = null

  constructor(tools: EnrollTools) {
    super()
    this.done = this.run(tools)
    this.done.catch(() => {})
  }

  private async run(tools: EnrollTools): Promise<{ word: Word; voiceprint: Voiceprint | null }> {
    const typed = await tools.compile()
    for (const w of typed.warnings) this.emit('warning', w)
    if (this.cancelled) throw new DOMException('cancelled', 'AbortError')
    const target = (this.target = await tools.open(typed.toJSON()))
    const offs = [target.on('take', (t) => this.emit('take', t))]
    try {
      const r = await new Promise<{ word?: WordJSON; warnings?: Warning[]; audio?: Float32Array; error?: string }>((resolve, reject) => {
        offs.push(target.on('enrolled', resolve))
        this.abort = () => reject(new DOMException('cancelled', 'AbortError'))
      })
      if (r.error || !r.word) throw new AlkoveError('engine.failed', r.error ?? 'enrollment failed')
      for (const w of r.warnings ?? []) this.emit('warning', w)
      const voiceprint = r.audio && r.audio.length >= 4000 ? await tools.embed(r.audio, target.source) : null
      const w = wordOf(r.word)
      const word = new Word(w.text, w.lang, w.phonemes, w.thresholds, w.calibrated, w.chain, [...typed.warnings, ...(r.warnings ?? [])])
      return { word, voiceprint }
    } finally {
      for (const off of offs) off()
      target.stop()
    }
  }

  cancel(): void {
    this.cancelled = true
    this.abort?.()
    this.target?.stop()
  }
}
