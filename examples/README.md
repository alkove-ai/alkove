# Examples

Plain HTML pages on the built package (`pnpm build` first), served from the repo root, for instance with `npx serve .` then `/examples/`.

| Page | What it shows |
|---|---|
| [`call.html`](./call.html) | `createCallInput`: listen to your own voice after Focus, raw / processed toggle |
| [`wake-then-voice.html`](./wake-then-voice.html) | `createWakeListener` then `createAgentInput`: nothing goes out before the word, then the next sentence |

They load the models from `../models/` (`npx alkove models models`); remove `modelBaseUrl` to load them from Hugging Face.
