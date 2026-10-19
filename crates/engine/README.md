# alkove-engine

The inference engine of [Alkove](https://alkove.ai): it runs the five Alkove models (Focus, Cue, Spot, Timbre, Lex) from their `.alk` files, in the browser through WebAssembly and natively. It is not a general runtime: it runs what the [`.alk` format](./FORMAT.md) can express, which is what these models need.

Rust, no dependencies, Apache-2.0.

## What it guarantees

- **The same result to the bit on every target.** It has been checked on ARM (NEON), x86-64 (SSE2), WebAssembly with SIMD, and WebAssembly without SIMD. To get there:
  - no fused multiply-add, unless the `fma` feature is turned on;
  - every sum in a fixed order;
  - its own `exp`, `ln`, `erf`, `tanh`, `sigmoid` and `gelu`, with four-lane versions equal to the scalar ones.

  The rules are in [FORMAT.md](./FORMAT.md#how-results-are-computed).
- **No allocation while it runs.** A session owns one arena of `f32`, sized when the file loads; inputs, outputs, state and work space live at fixed places in it.
- **A file is checked when it loads.** Every instruction is checked against the arena and the weights before anything runs, as is every size. A malformed or crafted file is refused; it cannot make the engine read or write outside its memory.
- **One thread, float compute.** Weights stored as int8 or 4 bits are unpacked to `f32` when the file loads.

## Use

```rust
use std::sync::Arc;
use alkove_engine::{Model, Session};

let model = Arc::new(Model::load(&std::fs::read("alkove-cue-1-preview.alk")?)?);
let mut s = Session::new(model)?;
for chunk in audio.chunks_exact(480) {
    s.input_mut("audio").unwrap().copy_from_slice(chunk);
    s.run();
    let speech = s.output("speech").unwrap()[0];
}
```

The other calls on a session:

- the state persists from one `run` to the next; `clear_state` sets it back to zero and keeps the inputs, and `state` / `set_state` read and write it as one vector;
- a model whose input length varies (Timbre) takes `set_len` before `run`.

A model run in several steps holds one plan per step, over the same weights. Lex reads a word with its `encoder` plan, then runs its `decoder` plan once per phoneme. `Session::new` opens the first plan, `Session::of(model, i)` the `i`-th of `model.plans()`. Opening a session allocates its arena, and an allocation that fails is an error.

## Build

```sh
cargo build --release                                     # the native library (crates/wasm, a C ABI)
RUSTFLAGS="-C target-feature=+simd128,+bulk-memory" \
  cargo build --release -p alkove-wasm --target wasm32-unknown-unknown
```

The web SDK (`packages/alkove`) builds the wasm itself (`pnpm build`). It needs no imports, and every browser with WebAssembly SIMD runs it.

## C ABI

`crates/wasm` exports the engine to JavaScript and to other languages, next to [the core's functions](../core/README.md#c-abi). Every name is a byte pointer and a length.

| Function | |
|---|---|
| `alk_alloc(len)`, `alk_free(p, len)` | memory for bytes passed in |
| `alk_model(bytes, len)` | a model, or null (then `alk_error(out, cap)` writes why) |
| `alk_model_free(m)` | |
| `alk_plans(m)`, `alk_plan_name(m, p, out, cap)` | number of plans, name of plan `p` |
| `alk_session(m, p)`, `alk_session_free(s)` | a session on plan `p`: one arena, at zero (null past the last plan or out of memory, then `alk_error` says why) |
| `alk_io_count(m, p, kind)` | number of inputs (`kind` 0) or outputs (1) of plan `p` |
| `alk_io_name(m, p, kind, i, out, cap)` | name of the `i`-th one |
| `alk_io_shape(m, p, kind, i, dims, cap)` | its shape |
| `alk_input(s, name, len)`, `alk_output(s, name, len)` | where an input or an output lives, for the session's life |
| `alk_run(s)` | one call |
| `alk_variable(m, p)` | index of the input whose length varies, or −1 |
| `alk_set_len(s, len)` | that input's length for the next runs; −1 when out of range |
| `alk_state_len(m, p)` | length of the state vector |
| `alk_state_get(s, out)`, `alk_state_set(s, x)` | read or write the state vector |
| `alk_state_clear(s)` | the state back to zero, the inputs kept |
| `alk_reset(s)` | the whole arena back to zero |
| `alk_steps(m, p)`, `alk_step(s, i)` | the instructions one at a time, for debugging |
| `alk_arena(s)`, `alk_arena_len(m, p)` | the arena itself |

Pointers into the arena stay valid until the session is freed. A wasm memory that grows detaches JavaScript views, so take a fresh view after loading another model.

## Tests

```sh
cargo test --release
./crates/engine/fingerprints.sh model.alk …
```

- **`cargo test`** builds small plans in memory and compares them, to the bit, with plain loops summing in the same order: linear, convolutions in 1D and 2D (grouped, dilated, depthwise, transposed), sizes that do not fall on multiples of 4. It also checks the four-lane functions against the scalar ones, and that crafted files are refused. CI runs it on Linux x86-64, macOS arm64 and wasm32 (with and without SIMD).
- **`fingerprints.sh`** hashes the outputs of 200 runs of each plan on fixed inputs, natively, on x86-64 through Rosetta, and in Node. The three hashes must be equal.

## Files

The `.alk` files of the published models are on [Hugging Face](https://huggingface.co/alkove-ai), with their model cards. [FORMAT.md](./FORMAT.md) describes the format: enough to read the weights without this engine, or to write another one.
