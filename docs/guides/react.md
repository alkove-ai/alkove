---
title: React
description: useAgentInput, useCallInput and useWakeListener open the microphone with the component and close it with it, strict mode and Next.js included.
sidebar:
  order: 2.5
---

```tsx
import { useState } from 'react'
import { useAgentInput } from 'alkove/react'

function Agent({ ws }: { ws: WebSocket }) {
  const [on, setOn] = useState(false)
  const { status, error, resume } = useAgentInput({
    enabled: on,
    near: true,
    on: {
      utterance: ({ pcm }) => ws.send(pcm),
      turnend: () => ws.send(JSON.stringify({ type: 'reply' })),
    },
  })
  if (error) return <p>{error.message}</p>
  if (status === 'blocked') return <button onClick={resume}>Turn the microphone on</button>
  return <button onClick={() => setOn(!on)}>{on ? 'Stop' : 'Talk'}</button>
}
```

The hooks come with the `alkove` package, under `alkove/react`, for React 18 and later. Each one wraps a function of the package: `useAgentInput` [`createAgentInput`](/docs/start/agent/), `useCallInput` [`createCallInput`](/docs/start/call/), `useWakeListener` [`createWakeListener`](/docs/start/wake-word/). They take the same options, plus two of their own:

- **`enabled`**: `false` opens nothing, no microphone and no model; switching it to `false` closes the input. `true` by default.
- **`on`**: the events, by name (`speechend`, `turnend`, `utterance`, `wake`…). The latest functions are always called: an arrow written inline never reopens the input.

## What they return

| | |
|---|---|
| `status` | `'off'` before the input opens or after it closed, then the input's own: `'starting'`, `'running'`, `'blocked'`, `'interrupted'` |
| `mode` | `'focus'`, `'focus+print'`, `'spot'`, `'spot+print'` or `'raw'`; `null` while closed |
| `error` | what kept the input from opening, an [`AlkoveError`](/docs/guides/errors/) such as `mic.denied` or `page.insecure`; `null` otherwise |
| `track` | the output track, for `RTCPeerConnection.addTrack` or LiveKit; follows `trackchange`, which a bound input (`bind()`) does not send: it swaps the sender's track itself |
| `input` | the input itself once open, for `listen()`, `hold()`, `record()`, `bind()`… |
| `resume` | to call in a click when `status` is `'blocked'`: the browser kept the audio suspended until a user gesture |

## When the options change

`strength` and `voiceprint` apply to the running input. Any other option closes the input and opens a new one: `deviceId` to switch microphones, `near`, `turn`, the words of a wake listener. Options are compared by content, so an object written inline in the component reopens nothing.

## Strict mode and Next.js

React's strict mode mounts each component twice in development: the first opening is cancelled. Inputs open one after the other, across every hook of the page, a new one once the previous has opened or failed: two inputs never hold the microphone at once, even when a component is mounted again while its input still opens.

`alkove/react` is marked `'use client'`: import it from a client component. Nothing touches the browser until a hook runs.

## A call

```tsx
import { useEffect } from 'react'
import { useCallInput } from 'alkove/react'

function Call({ pc }: { pc: RTCPeerConnection }) {
  const { input } = useCallInput({ near: true })
  useEffect(() => {
    if (!input) return
    const sender = pc.addTrack(input.track)
    input.bind(sender)
    return () => pc.removeTrack(sender)
  }, [input, pc])
  return null
}
```

An input opened again, after an option changed, comes as a new `input`: the effect moves the call to its track. `bind()` lets the input swap the sender's track itself when the audio clock stalls; see [calls over WebRTC](/docs/guides/webrtc/).

## A wake word

```tsx
import { useWakeListener } from 'alkove/react'

function Wake({ onWake }: { onWake: () => void }) {
  useWakeListener({ words: ['hey nova'], on: { wake: onWake } })
  return null
}
```
