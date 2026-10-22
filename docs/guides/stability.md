---
title: Stability and versions
description: What a version of Alkove fixes, what may change from one version to the next, and what 1.0 waits for.
sidebar:
  order: 13
---

Alkove is in preview: the package is `0.x` and the models end in `-preview`. This page says what you can count on today, and what can still move.

## What a version fixes

A version of `alkove` is a fixed set of files: its code, its engine (`alkove-engine.wasm`, inside the package) and its five models, each pinned by SHA-256 and read at a fixed Hugging Face commit. Nothing it loads can change under it.

- **The models.** A file whose hash does not match is refused with `models.hash`, whether it comes from Hugging Face, your server or `Alkove.configure({ models })`. Your users run the models you tested, wherever you host them. From 0.2 on, Hugging Face keeps every commit, so an old version still finds its models after newer ones are published.
- **The results.** The engine gives the same output, to the bit, in every browser and on every CPU: x86 or ARM, with or without WebAssembly SIMD, in Node too. For one version, the same audio gives the same events everywhere, so a bug seen on one machine replays on another.
- **The thresholds.** Spot's word thresholds, Timbre's verify levels and the gate's timings were measured on that version's models, and ship with it.

## From one version to the next

Versions follow semver, with its rule for `0.x`:

| Version | What it may contain |
|---|---|
| Patch (`0.2.0` → `0.2.1`) | Fixes, and additions that do not break code written for the previous version. The same models. |
| Minor (`0.2` → `0.3`) | Breaking changes: to the API, the models, the defaults. |

`npm install alkove` saves a `^0.3.2` range, which takes patches and stops before the next minor: a breaking version never arrives on its own. Each version is in the [changelog](https://github.com/alkove-ai/alkove/blob/main/CHANGELOG.md), with what to change in your code and which models moved.

The npm package, the Python package and its LiveKit and Pipecat plugins, and the Android library share the minor version: `alkove` 0.3 on the web, in Python and on Android runs the same core and the same models, and reads the same voiceprints and words. Their patches are numbered on their own.

Some things move in any version, patches included. Write your code so they do not break it:

- **New diag codes** (`diag` events, `Alkove.onDiag`). Log the codes you do not know rather than switching over a closed list.
- **New fields** in events and results.
- **The text of messages.** Match on `code`, never on `message`.

## What you store

Two things an app stores outlive a session. Each is bound to the model that made it, by name and by hash:

| Stored | Bound to | When that model changes |
|---|---|---|
| Voiceprints (`Voiceprint.toJSON()`) | Timbre | `Voiceprint.fromJSON()` returns `null`, and an input given the old print ignores it with a `voiceprint.reset` diag. Learn it again: about 6 s of voice with `learn: true`. |
| Compiled and enrolled words | Spot | Refused with an `AlkoveError` `word.outdated`. Compile the text again; enroll a recorded word again. |

A version that keeps Timbre and Spot keeps your prints and words, whatever else changes. See [voiceprints](/docs/guides/voiceprints/#when-prints-expire) and [wake words](/docs/guides/wake-words/#saving-words).

## The models

A model's name is `alkove-<model>-<generation>`, for example `alkove-focus-1-preview`.

- **With `-preview`**, the weights may still change under the same name. The hash tells them apart: a print or a word made with other weights is refused, as above.
- **From generation 1 on**, one name means one set of weights. New weights get a new name.

Each [model page](/docs/models/) gives its benchmarks and its limits for the weights this version ships.

## The engine and its format

The engine is open source (Apache-2.0), in [`crates/engine`](https://github.com/alkove-ai/alkove/tree/main/crates/engine). It ships inside the package, so it changes only with a version of `alkove`.

- **Outputs change only on purpose.** The SDK's tests compare every model's outputs, to the bit, with references made by the native engine. A version whose engine moves any output says so in the changelog.
- **The `.alk` format is frozen at version 1.** Its [specification](https://github.com/alkove-ai/alkove/blob/main/crates/engine/FORMAT.md) gives the layout of a file and the meaning of every instruction, and neither will change. Anything new takes a new version number; an engine refuses a version it does not know rather than misreading it.

## What 1.0 waits for

From 1.0, the API and the models stop moving: breaking changes wait for 2.0, and the models drop `-preview`. Before that, these must be in and measured:

1. **Interruptions**: telling a real interruption from a "mm-hmm" while an agent speaks, with Spot and the user's print.
2. **End of turn**: knowing when the user is done, not just pausing.
3. **Close voices**: Focus and Timbre telling apart two voices of the same register.
4. **Lex in English and Portuguese**: better pronunciations in these two languages.
5. **Diagnostics**: events for what degrades the input, such as loud noise or another voice nearby.
