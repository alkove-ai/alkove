# Alkove

On-device voice input: keep the user's voice, detect speech, hear a wake word, recognize a voice. In the browser, without a server. [alkove.ai](https://alkove.ai)

| Package | |
|---|---|
| [`alkove`](./packages/alkove) | the web library (npm) |
| [`alkove`](./python) | the Python package (PyPI) |
| [`livekit-plugins-alkove`](./integrations/livekit) | Focus and Cue for LiveKit Agents |
| [`pipecat-alkove`](./integrations/pipecat) | Focus and Cue for Pipecat |
| [`alkove-engine`](./crates/engine) | the inference engine and the `.alk` format |
| [`alkove-core`](./crates/core) | the features, written once in Rust for every package |

Models: [huggingface.co/alkove-ai](https://huggingface.co/alkove-ai). Code and weights under Apache-2.0.

Building needs Node 22, pnpm and [rustup](https://rustup.rs): `rust-toolchain.toml` pins the Rust version and its wasm target, installed on the first build.

```sh
pnpm install
pnpm build
pnpm test
cargo test --release
```

The Python package builds with [maturin](https://www.maturin.rs): `cd python && maturin develop --profile python`.
