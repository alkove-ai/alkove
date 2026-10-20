# Contributing

Bugs and questions: [open an issue](https://github.com/alkove-ai/alkove/issues). Security issues: see [SECURITY.md](./SECURITY.md).

## Building

Building needs Node 22, pnpm and [rustup](https://rustup.rs): `rust-toolchain.toml` pins the Rust version and its wasm target, installed on the first build.

```sh
pnpm install
pnpm build
pnpm test
cargo test --release
```

The Python package builds with [maturin](https://www.maturin.rs): `cd python && maturin develop --profile python`.
