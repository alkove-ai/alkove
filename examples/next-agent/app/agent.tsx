'use client'

import { useAgentInput } from 'alkove/react'
import { useState } from 'react'

type Item = { kind: 'phrase'; url: string; seconds: number } | { kind: 'turn'; probability: number }

export function Agent() {
  const [on, setOn] = useState(false)
  const [items, setItems] = useState<Item[]>([])
  const { status, mode, error, resume } = useAgentInput({
    enabled: on,
    on: {
      // what a speech-to-text would get: 16 kHz mono, one segment per sentence
      utterance: ({ pcm, start, end }) => setItems((l) => [...l, { kind: 'phrase', url: URL.createObjectURL(wav(new Float32Array(pcm))), seconds: end - start }]),
      turnend: ({ probability }) => setItems((l) => [...l, { kind: 'turn', probability }]),
    },
  })

  return (
    <>
      <p>
        <button onClick={() => setOn(!on)}>{on ? 'Stop' : 'Talk'}</button> {status} {mode && `· ${mode}`}
        {status === 'blocked' && <button onClick={resume}>Turn the audio on</button>}
      </p>
      {error && <p role="alert">{error.message}</p>}
      <ol>
        {items.map((it, i) =>
          it.kind === 'phrase' ? (
            <li key={i}>
              <audio controls src={it.url} /> {it.seconds.toFixed(1)} s
            </li>
          ) : (
            <li key={i}>end of turn ({it.probability.toFixed(2)})</li>
          ),
        )}
      </ol>
    </>
  )
}

function wav(pcm: Float32Array, rate = 16000): Blob {
  const view = new DataView(new ArrayBuffer(44 + pcm.length * 2))
  const text = (at: number, s: string) => [...s].forEach((c, i) => view.setUint8(at + i, c.charCodeAt(0)))
  text(0, 'RIFF')
  view.setUint32(4, 36 + pcm.length * 2, true)
  text(8, 'WAVEfmt ')
  view.setUint32(16, 16, true)
  view.setUint16(20, 1, true)
  view.setUint16(22, 1, true)
  view.setUint32(24, rate, true)
  view.setUint32(28, rate * 2, true)
  view.setUint16(32, 2, true)
  view.setUint16(34, 16, true)
  text(36, 'data')
  view.setUint32(40, pcm.length * 2, true)
  pcm.forEach((x, i) => view.setInt16(44 + i * 2, Math.max(-1, Math.min(1, x)) * 0x7fff, true))
  return new Blob([view], { type: 'audio/wav' })
}
