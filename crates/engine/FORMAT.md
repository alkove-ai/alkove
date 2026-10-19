# The `.alk` format, version 1

An `.alk` file is a model fixed offline: its weights, and one or more **plans**. A plan is one arena of `f32` and a list of instructions that read and write the arena at fixed places. Running a plan means writing its inputs into the arena, running the instructions in order, and reading its outputs. There is no graph to schedule, no shape to infer and no memory to allocate at run time.

Most models have one plan. A model run in several steps has one plan per step, over the same weights: Lex reads a word with its `encoder` plan, then runs its `decoder` plan once per phoneme.

This document is enough to read the weights of a published model, to check a file, or to write another engine that gives the same results. The reference reader is `src/format.rs` and `src/ops.rs`; the reference semantics, `src/kernels.rs` and `src/math.rs`.

Version 1 is frozen: a file that needs anything else carries another version number, and this engine refuses it.

## Conventions

- Every number is little-endian. A **word** is 4 bytes: `u32`, `i32` or `f32`.
- Sizes, offsets and strides are counted in `f32` elements of the arena (or of a weight), never in bytes, except in the weight descriptors where noted.
- A `usize` word is a `u32` below 2³¹. A reader refuses any larger value, and any size computed from several (a product, a sum) that reaches 2³¹: this keeps every offset representable on 32-bit targets.
- `NONE` is the word `0xFFFFFFFF`: no weight, no residual, no initial state.
- A **string** is a `usize` length in bytes, then the UTF-8 bytes, padded with zeros to a multiple of 4.

## File layout

| Bytes | Content |
|---|---|
| 0–3 | magic `ALK\0` (`41 4C 4B 00`) |
| 4–7 | version, `u32`: `1` |
| 8–11 | `M`, the model section's length in bytes, a multiple of 4 |
| 12–15 | `B`, the offset of the weight data from the start of the file, with `16 + M ≤ B ≤` file length |
| 16 … 16 + M | the model section |
| B … end | the weight data (the **blobs**) |

## The model section

In order, with nothing between the parts and nothing after the last one:

1. `name`, a string: the model's name.
2. Weights: a count, then one weight descriptor each (below). Weight `i` is the `i`-th, counted from 0. Every plan reads the same weights.
3. Plans: a count, at least 1, then each plan (below). Two plans never have the same name. A model of one plan names it `main`.

## A plan

In order:

1. `name`, a string: the plan's name.
2. `arena`, `usize`: the arena's length in `f32`.
3. Inputs and outputs: a count, then for each:
   - `kind`, `u32`: `0` input, `1` output;
   - `name`, a string;
   - `off`, `usize`: where it starts in the arena;
   - `rank`, `usize`, then `rank` dimensions, each a `usize`.

   It lies at `arena[off .. off + product(dims)]`, row-major.
4. State pieces: a count, then for each `off`, `len`, `flat` (`usize`). The plan's state vector has `max(flat + len)` elements; piece `p` is `state[flat .. flat + len]`, kept at `arena[off .. off + len]` between runs.
5. Variable length: a `u32`, `0` when every size is fixed. Otherwise six words follow:

   | Word | Type | Meaning |
   |---|---|---|
   | `input` | `usize` | index of the input whose length varies |
   | `min` | `usize` | shortest length accepted |
   | `max` | `usize` | longest length accepted, at most that input's size |
   | `add` | `i32` | |
   | `div` | `u32` | not 0 |
   | `plus` | `i32` | |

   For an input of `len` elements, the plan's **frame count** is `T = max(0, floor((len + add) / div) + plus)`, with floor division. Until a length is given, `len = max`.
6. Instructions: a count, then for each: `code` (`u32`), `n` (`usize`), then `n` words, the instruction's fields (below). An instruction whose fields do not fill exactly its `n` words is refused.

### Sizes that follow the frame count

Some fields are a **Dim**, one `u32`:

- bit 31 clear: the size itself;
- bit 31 set: `coef` = bits 16 to 30, `offset` = bits 0 to 15 as an `i16`, and the size is `max(0, coef × T + offset)`, computed in 64 bits and capped at 2³¹ − 1.

A reader checks every instruction with `T` at its largest (`len = max`). Every Dim grows with `T`, so a shorter input stays within the checked bounds.

## Weights

A descriptor starts with `dtype` (`u32`), `len` (`usize`, the number of `f32` values once unpacked) and `off` (`usize`, a byte offset into the blobs). The engine unpacks every weight to `f32` when it loads the file.

| `dtype` | Words after `off` | Data at `off` | Value `i` (0 ≤ `i` < `len`) |
|---|---|---|---|
| `0`, f32 | none | `len` × `f32` | as stored |
| `1`, int8 per channel | `channels`, `inner`, `scales`, `zeros` | `len` × `i8` | `c = (i / inner) mod channels`, then `(q[i] − zero[c]) × scale[c]`, the difference computed in integers |
| `2`, 4-bit blocks | `k`, `n`, `block`, `zeros`, `scales` | `n × k / 2` bytes of codes | see below |

For int8:

- `scales` is a byte offset to `channels` × `f32`;
- `zeros` is a byte offset to `channels` × `i8`;
- `len` is a multiple of `channels × inner`.

For 4-bit blocks, the result is a `k × n` matrix stored row-major, `len = k × n`:

- `block` is even and divides `k`; `nb = k / block`.
- `zeros` is a byte offset to `n × nb` unsigned bytes, `scales` to `n × nb` × IEEE `f16`.
- The codes run column by column, then block by block, then byte by byte.
- Byte `j` of block `b` of column `col` (at `(col × nb + b) × block / 2 + j`) holds:
  - in its low 4 bits, row `b × block + 2j`;
  - in its high 4 bits, row `b × block + 2j + 1`.
- Each row's value is `(code − zero) × scale`, with the zero and the scale of `(col, b)`.

## Running

- A session runs one plan, on its own arena; sessions of the same model share its weights. Its arena starts at zero.
- **One call:**
  1. Write the inputs; for a variable input, give its length, which sets `T`.
  2. Run every instruction in order.
  3. Read the outputs.
- **State:** the state pieces keep their values from one call to the next; nothing else is guaranteed to.
- **Before the first call**, the state is zero. Setting the state means writing the pieces; clearing it means zeroing them, inputs left as they are.

## Instructions

Every field is a `usize` unless marked. The kinds of field that recur:

- **Dim** fields are marked `Dim`.
- A **matrix** is two words, `off` and `rs`: row `i`, column `j` is at `off + i × rs + j`.
- A **view4** is five `i32` words, `off`, `s0`, `s1`, `s2`, `s3`: element `(i0, i1, i2, i3)` is at `off + i0 × s0 + i1 × s1 + i2 × s2 + i3 × s3`. Strides may be zero (broadcast) or negative.
- An **operand** is a weight id (or `NONE`) and a view4: it reads that weight when there is one, else the arena.
- A weight id is a `u32` index into the weights.

### How results are computed

These rules make the result the same to the bit on every target. An engine that only needs close values may ignore them; one that wants this engine's bits may not.

- `f32` throughout, rounding to nearest even. A product is rounded, then added: **no fused multiply-add**.
- **Sums of products** (`LINEAR`, `CONV`, `GRU`, `MATMUL`, the mel filters) start at `+0.0` and add one product after another in the order of `k`.
- **Lane sums.** The other sums use four lanes: element `i` of the body goes to lane `i mod 4`, the body being the largest multiple of 4. The lanes are then combined as `(l0 + l1) + (l2 + l3)`, and the tail added one by one. This covers:
  - the means and variances of `LAYER_NORM`;
  - `REDUCE` sum, mean and L2;
  - the softmax denominator;
  - the fbank's DC removal and mean normalization.
- **Functions:** `exp`, `ln`, `erf`, `tanh`, `sigmoid` and `gelu` are the polynomials of `src/math.rs`, not a math library's.
  - `sigmoid(x) = 1 / (1 + exp(−x))`;
  - `gelu(x) = (x × (erf(x / √2) + 1)) × 0.5`;
  - `exp(x)` is `0` below `−87`, so hosts that flush subnormals agree.
- **Comparisons** (`Max`, `Min`, `Relu`, `Clip`, `PRelu`) follow the formulas below exactly. That fixes what they give for NaN and for −0.

### 1, `FBANK`: log-mel filter bank

**Fields:**

| Field | Meaning |
|---|---|
| `x` | where the audio starts |
| `frames` (Dim) | `F`, the number of frames |
| `len` | samples per frame, ≥ 2 |
| `hop` | samples between frame starts |
| `out` (matrix) | `F` rows, one column per mel band |
| `mels` | number of mel bands |
| `nfft` | FFT size: a power of two, ≥ 4, ≥ `len`, with `bins ≤ nfft / 2 + 1` |
| `bins` | power bins computed |
| `window` (weight id, `len` values) | analysis window |
| `fft` (weight id, `3 × nfft / 2` values) | FFT tables, as for `RFFT` |
| `mel` (weight id) | the filters' values, end to end |
| `mels` × (`start`, `count`) | each band's bins, `start + count ≤ bins` |
| `mean`, `std` (weight ids or `NONE`, `mels` values) | per-band normalization |
| `preemph`, `first`, `floor` (`f32`) | pre-emphasis, its first-sample factor, power floor |
| `dc`, `cmn` (`u32`) | flags |
| `scratch` | `2 × nfft + bins` floats of work space |

**Computing frame `f`.** Let `s = x + f × hop`.

1. If `dc` is set, `m` = the lane-sum mean of the `len` samples; otherwise `m = 0`.
2. Build the frame:
   - `y[0] = ((a[s] − m) × first) × window[0]`;
   - `y[j] = ((a[s+j] − m) − preemph × (a[s+j−1] − m)) × window[j]` for `j ≥ 1`;
   - zeros up to `nfft`.
3. The power of bin `k` is `re² + im²`, from the real FFT of `y` (see `RFFT`).
4. Band `b` sums `power[start + i] × mel[at + i]` for `i < count`, in order. `at` is the band's first value in `mel`: the sum of the earlier counts.
5. The band's value:
   - takes `ln` of the sum, or of `floor` when the sum is below it;
   - minus `mean[b]` if given;
   - divided by `std[b]` if given;
   - is written at `out[f][b]`.

If `cmn` is set, each band then loses its lane-sum mean over the `F` frames.

### 2, `LINEAR`: matrix times weights

**Fields:** `x` (matrix), `out` (matrix), `m` (Dim, rows), `k`, `n`, `w` (weight id, `k × n` values, row `kk` holding the `n` weights of input `kk`), then an **epilogue**.

**Computing:** `out[i][j] = Σ_kk x[i][kk] × w[kk × n + j]`, then the epilogue.

**The epilogue** is eight words:

| Word | Meaning |
|---|---|
| `bias` (weight id or `NONE`) | added per column |
| `act` (`u32`) | `0` none, `1` gelu, `2` relu, `3` sigmoid, `4` tanh, `5` PReLU |
| `slope` (weight id) | PReLU's slope per column, read only when `act` is 5 |
| `scale` (weight id or `NONE`) | multiplied per column |
| `shift` (weight id or `NONE`) | added per column |
| `res` (`u32`, `NONE` for none) | where a residual starts |
| `rs0`, `rs1` | the residual's row and column strides |

Each output `v` of row `i`, column `j` becomes, in this order:

1. `v + bias[j]`;
2. the activation:
   - relu is `v < 0 ? 0 : v`;
   - PReLU is `v < 0 ? v × slope[j] : v`;
3. `× scale[j]`;
4. `+ shift[j]`;
5. `+ arena[res + i × rs0 + j × rs1]`.

Each step applies only when its field is given.

### 3, `CONV`: 2D convolution, plain or transposed

**Fields:**

- `x` (`off`, `c`, `h`, `w`: offset and the channel, row and column strides), `c_in`, `h_in` (Dim), `w_in` (Dim);
- `out` (`off`, `c`, `h`, `w`), `c_out`, `h_out` (Dim), `w_out` (Dim);
- `k0`, `k1`, `stride0`, `stride1`, `dil0`, `dil1`, `pad0`, `pad1`;
- `groups`, `transposed` (`u32`);
- `w` (weight id), then an epilogue (per output channel; `i` = `y × w_out + x`).

`groups` divides `c_in` and `c_out`. Kernel sizes, strides and dilations are not 0. Unless the conv is depthwise (`groups = c_in = c_out`), it has at most 16 taps: `k0 × k1 ≤ 16`.

The weight holds `k0 × k1 × (c_in / groups) × c_out` values. With `cig = c_in / groups` and the tap `t = ky × k1 + kx`, the weight of tap `t`, input channel `ci` of the group, output channel `co` is `w[(t × cig + ci) × c_out + co]`.

Output `(co, y, x)`, in group `g = co / (c_out / groups)`, sums over `ky`, then `kx`, then `ci`, in that order: input channel `g × cig + ci`, at row `iy` and column `ix`, times the weight. Along each axis, with output position `o`, tap `k`, stride `s`, dilation `d`, padding `p`:

- **plain:** `i = o × s + k × d − p`;
- **transposed:** `i = (o + p − k × d) / s`, only when `o + p − k × d ≥ 0` and divisible by `s`.

A tap whose `i` falls outside the input reads `0`. Weights are finite: where a weight is infinite or NaN, the output at such a tap is unspecified.

### 4, `LAYER_NORM`

**Fields:** `x` (matrix), `out` (matrix), `m` (Dim, rows), `n` (not 0), `gamma`, `beta` (weight ids, `n` values), `eps` (`f32`).

**Computing, per row:**

- `mean` = the lane-sum of the row, `/ n`;
- `var` = the lane-sum of `(x − mean)²`, `/ n`;
- `out[j] = ((x[j] − mean) / sqrt(var + eps)) × gamma[j] + beta[j]`.

### 5, `GRU`: one direction of an ONNX GRU, `linear_before_reset = 1`

**Fields:**

| Field | Meaning |
|---|---|
| `x` (`off`, `s0`, `s1`) | the input: step `t`, batch row `b` at `off + t × s0 + b × s1`, `i` values |
| `t`, `batch`, `i` | steps, batch rows, input size |
| `h0` (`u32` offset or `NONE`, then `rs`) | the initial state of row `b` at `h0 + b × rs`; zeros when `NONE` |
| `y` (`off`, `s0`, `s1`) | where each step's state goes, laid out as `x` |
| `h` | state size |
| `W` (weight id, `i × 3h`) | input weights |
| `R` (weight id, `h × 3h`) | recurrent weights |
| `bW`, `bR` (weight ids, `3h`) | biases |
| `reverse` (`u32`) | run the steps backwards |
| `scratch` | `(t + 1) × batch × 3h` floats of work space |

The `3h` columns of `W`, `R`, `bW` and `bR` are the gates `z`, `r`, `n`, in that order (ONNX's z, r, h).

**At each step** (from the last one when `reverse` is set), with `xW = x · W` and `hR = h_prev · R`, both sums of products:

- `z = sigmoid((xW_z + bW_z) + (hR_z + bR_z))`
- `r = sigmoid((xW_r + bW_r) + (hR_r + bR_r))`
- `n = tanh((xW_n + bW_n) + r × (hR_n + bR_n))`
- `h = (1 − z) × n + z × h_prev`, written to `y`.

`h_prev` is the previous step's output in `y`, or `h0` at the first step.

### 6, `COPY`

**Fields:** `dims` (4 × Dim), `src` (view4), `dst` (view4).

**Computing:** `dst[i] = src[i]` for every index of `dims`, in index order (`i0` slowest). When both views run along the last axis with stride 1, each such run is moved as one block. Overlapping views are not refused: their result is that of this order.

### 7, `SOFTMAX`

**Fields:** `x` (matrix), `out` (matrix), `m` (Dim, rows), `n` (Dim, columns), `log` (`u32`).

**Computing, per row:**

1. `mx` = the largest value, starting from −∞, keeping a value only when it is greater.
2. `e[j] = exp(x[j] − mx)`.
3. `s` = the lane-sum of `e`.
4. The output:
   - `out[j] = e[j] / s`;
   - with `log` set, `out[j] = (x[j] − mx) − ln(s)` instead.

### 8, `BINARY`

**Fields:** `op` (`u32`), `dims` (4 × Dim), `a` (operand), `b` (operand), `out` (view4).

**Computing:** `out[i] = a[i] op b[i]`.

| `op` | Result |
|---|---|
| 0 | `a + b` |
| 1 | `a − b` |
| 2 | `a × b` |
| 3 | `a / b` |
| 4 | `b > a ? b : a` |
| 5 | `b < a ? b : a` |
| 6 | `a < 0 ? a × b : a` (PReLU) |
| 7 | `a == b` |
| 8 | `a > b` |
| 9 | `a < b` |

The comparisons give `1.0` or `0.0`.

### 9, `UNARY`

**Fields:** `op` (`u32`), `p0`, `p1` (`f32`), `dims` (4 × Dim), `a` (operand), `out` (view4).

**Computing:** `out[i] = f(a[i])`.

| `op` | `f(v)` |
|---|---|
| 0 | `v` |
| 1 | `v < 0 ? 0 : v` |
| 2 | `sigmoid(v)` |
| 3 | `tanh(v)` |
| 4 | `exp(v)` |
| 5 | `ln(v)` |
| 6 | `sqrt(v)` (IEEE) |
| 7 | `−v` |
| 8 | `erf(v)` |
| 9 | `gelu(v)` |
| 10 | clip: `u = v < p0 ? p0 : v`, then `u > p1 ? p1 : u` |
| 11 | `|v|` |
| 12 | `1 / v` |
| 13 | `v × v` |

### 10, `REDUCE`

**Fields:** `op` (`u32`), `rows` (Dim), `n` (Dim), `x` (view4), `out` (view4).

For row `r`, the values are `x.off + r × x.s0 + i × x.s1` for `i < n`. The result goes to `out.off + r × out.s0`. The other strides are unused.

| `op` | Result |
|---|---|
| 0 | the lane-sum |
| 1 | the lane-sum `/ n` |
| 2 | the largest value, as in `SOFTMAX` |
| 3 | `sqrt` of the lane-sum of the squares |

### 11, `RFFT`: real DFT

**Fields:** `x`, `n` (a power of two ≥ 4), `fft` (weight id, `3n / 2` values), `out`, `s`, `si` (not 0, `s ≠ si`), `scratch` (`n` floats).

**Computing:** for `k` from 0 to `n / 2`, `X[k] = Σ_j x[j] × e^(−2πi·jk/n)`. Bin `k`'s real part goes to `out + k × s`, its imaginary part to `out + k × s + si`.

**The tables:** with `m = n / 2`:

- the first `m` values are the pairs `(cos 2πj/m, −sin 2πj/m)` for `j < m / 2`, the twiddles of a complex FFT of `m` points;
- the next `2m` are the pairs `(cos 2πk/n, −sin 2πk/n)` for `k < m`, which split it into a real FFT of `n` points.

**The engine's algorithm,** for its bits (`rfft` and `butterflies` in `src/kernels.rs`):

1. Pair the samples into `m` complex values.
2. Permute them in bit-reversed order.
3. Run radix-2 butterflies, stage by stage.
4. Split the result into the `n / 2 + 1` bins.

### 12, `IRFFT`: inverse real DFT

**Fields:** `x`, `s`, `si`, `n`, `fft` (weight id, as `RFFT`), `out`, `scratch` (`n` floats).

**Computing:** the input is `n / 2 + 1` bins laid out as `RFFT`'s output. The `n` outputs are the real part of the inverse DFT of the Hermitian extension of these bins, times `1 / n`. The imaginary parts of bins `0` and `n / 2` are ignored. The engine's algorithm is `irfft` in `src/kernels.rs`.

### 13, `WHERE`

**Fields:** `dims` (4 × Dim), `c`, `a`, `b` (operands), `out` (view4).

**Computing:** `out[i] = c[i] ≠ 0 ? a[i] : b[i]`.

### 14, `GATHER`: rows picked by indices held as floats

**Fields:** `src` (weight id or `NONE`), `x` (view4), `outer`, `len` (not 0), `inner`, `idx`, `n`, `idx_s`, `out` (view4).

**Computing:** for each `i < n`:

1. `v = arena[idx + i × idx_s]`.
2. Make it a row number `k`:
   - NaN gives `0`;
   - otherwise `v` is truncated toward zero;
   - a negative `k` gets `len` added;
   - then `k` is clamped to `[0, len − 1]`.
3. For `o < outer`, `r < inner`: `out(o, i, r) = x(o, k, r)`, both view4 with the last index 0.

### 15, `MATMUL`: product of two arena matrices

**Fields:** `batch0`, `batch1`, `m`, `k`, `n`, then three groups of five words and `scratch`:

| Group | Five words |
|---|---|
| `a` | `off`, `s_b0`, `s_b1`, `s_m`, `s_k` |
| `b` | `off`, `s_b0`, `s_b1`, `s_k`, `s_n` |
| `out` | `off`, `s_b0`, `s_b1`, `s_m`, `s_n` |

`out`'s `s_n` must be `1` when `n > 1`. `scratch` holds `ceil(n / 8) × 8 × k` floats.

**Computing,** for every batch index: `out[i][j] = Σ_kk a[i][kk] × b[kk][j]`, a sum of products.

## What a reader must refuse

The reference reader refuses, before running anything:

- **Header:** a bad magic, version or section bounds.
- **Structure:**
  - a model section that ends early or has words left over;
  - no plan, or two plans of the same name;
  - an unknown instruction, operation or activation code;
  - an instruction that does not fill exactly its words.
- **Sizes:** any size or offset of 2³¹ or more, alone or computed.
- **Weights:**
  - a weight id that does not exist;
  - a weight whose unpacked length is not the one the instruction needs;
  - blob data past the end of the file.
- **Bounds:** any view of any instruction that reaches outside the arena (or outside its weight) at the largest frame count. This also covers inputs, outputs and state pieces.
- **Overlaps:** an output that overlaps one of the instruction's inputs or its scratch, except for `COPY`. The element-by-element instructions (`BINARY`, `UNARY`, `WHERE`) may write over an input read through exactly the same view.
- **Instruction rules:** the conditions given above (powers of two, `groups`, `s ≠ si`, …).

With these checks done, no instruction can read or write outside its arena, whatever the file holds.
