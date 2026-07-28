# NAM LSTM reference-parity fixture

Fixture for the LSTM reference-parity test (ba todo #1115, epic #197, doc
#258): the real NAM LSTM example model plus its reference output generated
with the upstream C++ implementation, NeuralAmpModelerCore, compiled/run
WITH fast tanh enabled — the condition the official NAM plugin runs under
(`nam::activations::Activation::enable_fast_tanh()`), same rationale as
the `../a1/` fixtures.

Collected 2026-07-28.

## Provenance & licensing

`lstm.nam` is the official example model shipped in the
NeuralAmpModelerCore repository, MIT License (Copyright (c) 2023 Steven
Atkinson), copied verbatim from `example_models/` at the same commit as
the `../a1/` and `../a2/` fixtures:

- Repository: https://github.com/sdatkinson/NeuralAmpModelerCore
- Commit used: `3cde95c354d5ba6da01316cad90b05cfc4855053` (library version 0.5.5)
- License: MIT (see ../a2/README.md for the full pointer)

| File | Architecture | Notes |
|---|---|---|
| `lstm.nam` | LSTM v0.5.4, 1 layer, input size 1, hidden size 3, 48 kHz | Real trainer export ("Test LSTM", Darkglass Microtubes 900 v2): 70 weights in the NAM LSTM layout — per cell one combined `[4h, in+h]` matrix, one summed bias, learned initial `h0`/`c0`; `[1, h]` head + bias; NO trailing head scale |

## Deterministic input

The shared `../a2/input.f32` (impulse + gap + 220 Hz sine, 4096 samples @
48 kHz; sha256 `c142e443...` — see ../a2/README.md). The reference render
below consumed the `input.wav` twin written by `../a2/gen_input.py`.

## Reference output (fast tanh enabled)

Raw little-endian float32, 4096 samples, produced by the upstream `render`
tool at the commit above with the `--fast-tanh` patch documented in
`../a1/README.md` (the patched call precedes `nam::get_dsp`; for LSTM the
flag selects the `fast_sigmoid`/`fast_tanh` branch of
`LSTMCell::process_`):

| Output | Command (from Core checkout) |
|---|---|
| `lstm.fasttanh.f32` | `build/tools/render --fast-tanh example_models/lstm.nam input.wav out.wav` |

then `python3 ../a2/wav_to_f32.py out.wav` to strip the WAV header.
Verified bit-deterministic across runs (two renders, identical sha256).

Reference-engine conditions (see `tests/common/mod.rs` for the harness
contract): `NAM_SAMPLE` = double with a per-sample f32 cast at the output,
`Reset(48000, 64)` with prewarm-on-reset, 64-sample blocks, x86-64 Linux
gcc `-O3` Release. **LSTM prewarm is exact, not merely sufficient**:
`LSTM::GetPrewarmSamples()` is `0.5 * sample_rate` = 24000 zeros (375
whole 64-blocks), fed starting from the LEARNED initial `h0`/`c0` state.
An LSTM is recurrent — its state converges to the zero-input fixed point
only asymptotically — so the parity test replays exactly 24000 zeros (and
separately asserts the fixed point is in fact reached by then, so the
count would only start to matter if a future model converged slower).

## Measured parity (2026-07-28, engine f32 vs reference render)

| Model | max abs err | first-divergence scale |
|---|---|---|
| `lstm.nam` | 2.757e-7 | build-noise (accumulation-order/FMA-level) |

Pinned by `tests/nam_lstm_reference_parity.rs` at tolerance 1e-6.

For history: before ba todo #1115 the engine's LSTM weight reader used a
PyTorch-style layout (separate `w_ih`/`w_hh`, separate `b_ih`/`b_hh`,
zero initial state, optional trailing head scale) that does NOT match the
NAM export — this very file (70 weights) failed to load at all (the
reader wanted 72 recurrent weights), and no LSTM output test existed to
notice.

## File hashes (sha256)

```
b19fe75bf26d5f86ed2936b4a91deca055af43c040c34e0f000bd5844f0a0d7b  lstm.fasttanh.f32
df9f78c49f49c2bb32411df47e3f53746075adb206b92d017e06379d1e56234a  lstm.nam
```
