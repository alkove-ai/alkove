import { Agent } from './agent'

export default function Page() {
  return (
    <main>
      <h1>Voice agent input</h1>
      <p>Each sentence you say comes back as its own clip, after Focus. A line marks where Cue heard the end of your turn.</p>
      <Agent />
    </main>
  )
}
