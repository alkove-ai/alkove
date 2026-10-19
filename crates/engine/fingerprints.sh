#!/bin/sh
# The fingerprint of each .alk given, on this Mac (NEON), on x86-64 (SSE, through Rosetta) and in Node (wasm): the three must be equal.
set -e
cd "$(dirname "$0")/../.."
cargo build -q --release
cargo build -q --release --example bench
cargo build -q --release --example bench --target x86_64-apple-darwin
RUSTFLAGS="-C target-feature=+simd128,+bulk-memory" cargo build -q --release -p alkove-engine-wasm --target wasm32-unknown-unknown
for f in "$@"; do
  ./target/release/examples/bench "$f" 20 | head -1
  arch -x86_64 ./target/x86_64-apple-darwin/release/examples/bench "$f" 20 | head -1
  node crates/engine-wasm/fingerprint.mjs target/wasm32-unknown-unknown/release/alkove_engine_wasm.wasm "$f" 20
done
