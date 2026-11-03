import { Spot, type Lang, type Word } from 'alkove'
import { useWakeListener } from 'alkove/react'
import { useState } from 'react'

const LANGS: Lang[] = ['en', 'fr', 'es', 'it', 'pt', 'ca', 'ro']

export function App() {
  const [text, setText] = useState('Hey Nova')
  const [lang, setLang] = useState<Lang>('en')
  const [word, setWord] = useState<Word | null>(null)
  const [wakes, setWakes] = useState<string[]>([])
  const { status, error, resume } = useWakeListener({
    enabled: !!word,
    words: word ? [word] : [],
    on: { wake: ({ word }) => setWakes((l) => [`${new Date().toLocaleTimeString()}: « ${word} »`, ...l]) },
  })

  // any typed word, no training: Lex turns it into phonemes for Spot
  const listen = async () => setWord(await Spot.compile(text, { lang }))

  return (
    <main>
      <h1>Wake word</h1>
      <p>
        <input value={text} onChange={(e) => setText(e.target.value)} />{' '}
        <select value={lang} onChange={(e) => setLang(e.target.value as Lang)}>
          {LANGS.map((l) => (
            <option key={l}>{l}</option>
          ))}
        </select>{' '}
        {word ? <button onClick={() => setWord(null)}>Stop</button> : <button onClick={listen}>Listen</button>} {status}
        {status === 'blocked' && <button onClick={resume}>Turn the audio on</button>}
      </p>
      {word?.warnings.map((w) => <p key={w.message}>{w.message}</p>)}
      {error && <p role="alert">{error.message}</p>}
      <ul>
        {wakes.map((w, i) => (
          <li key={i}>{w}</li>
        ))}
      </ul>
    </main>
  )
}
