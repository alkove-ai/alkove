# Examples

Plain HTML pages on the built package (`pnpm build` first), served from the repo root, for instance with `npx serve .` then `/examples/`.

| Page | What it shows |
|---|---|
| [`call.html`](./call.html) | `createCallInput`: listen to your own voice after Focus, raw / processed toggle |
| [`wake-then-voice.html`](./wake-then-voice.html) | `createAgentInput` and `agent.spot()`: only the sentence opened by the word is kept |

They load the models from `../models/` (`npx alkove models models`); remove `modelBaseUrl` to load them from Hugging Face.
