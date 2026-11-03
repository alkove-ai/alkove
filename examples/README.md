# Examples

## React

Apps on the hooks of `alkove/react`, in the workspace (`pnpm install` at the root, `pnpm --filter alkove build`, then `pnpm dev` in the example's folder). They load the models from Hugging Face.

| App | What it shows |
|---|---|
| [`next-agent`](./next-agent) | Next.js, `useAgentInput`: each sentence you say back as a clip, the ends of turns Cue hears |
| [`vite-wake`](./vite-wake) | Vite, `useWakeListener`: a word typed in the page, compiled by `Spot.compile`, and each time it is heard |

Outside this repository, replace `"alkove": "workspace:*"` with the published version.

## HTML

Plain HTML pages on the built package (`pnpm build` first), served from the repo root, for instance with `npx serve .` then `/examples/`.

| Page | What it shows |
|---|---|
| [`call.html`](./call.html) | `createCallInput`: listen to your own voice after Focus, raw / processed toggle |
| [`wake-then-voice.html`](./wake-then-voice.html) | `createAgentInput` and `agent.spot()`: only the sentence opened by the word is kept |

They load the models from `../models/` (`npx alkove models models`); remove `modelBaseUrl` to load them from Hugging Face.
