# NAM A1 reference-parity fixtures

Fixtures for the A1 (classic WaveNet) reference-parity tests (ba todo
#1116, epic #197, doc #258): real A1 `.nam` files plus reference outputs
generated with the upstream C++ implementation, NeuralAmpModelerCore,
compiled/run WITH fast tanh enabled — the condition the official NAM
plugin runs under (`nam::activations::Activation::enable_fast_tanh()`),
which is what "correct" means for A1 files.

Collected 2026-07-28.

## Provenance & licensing

Both `.nam` files are official example models shipped in the
NeuralAmpModelerCore repository, MIT License (Copyright (c) 2023 Steven
Atkinson), copied verbatim from `example_models/` at the same commit as
the `../a2/` fixtures:

- Repository: https://github.com/sdatkinson/NeuralAmpModelerCore
- Commit used: `3cde95c354d5ba6da01316cad90b05cfc4855053` (library version 0.5.5)
- License: MIT (see ../a2/README.md for the full pointer)

Note: upstream `example_models/my_model.nam` is byte-identical to
`wavenet_a1_standard.nam` (same sha256), so only the latter is committed —
it covers all three real A1 example models the repository ships.

| File | Architecture | Notes |
|---|---|---|
| `wavenet_a1_standard.nam` | WaveNet v0.5.0, 2 layer arrays (16ch/8ch, kernel 3, dilations 1..512), no `sample_rate` field (48 kHz by NAM convention) | The standard A1 capture architecture; receptive field 4092 |
| `wavenet.nam` | WaveNet v0.5.4, 2 tiny layer arrays (3ch/2ch), 48 kHz | Small smoke model |

## Deterministic input

The shared `../a2/input.f32` (impulse + gap + 220 Hz sine, 4096 samples @
48 kHz; sha256 `c142e443...` — see ../a2/README.md). The reference
renders below consumed the `input.wav` twin written by `../a2/gen_input.py`.

## Reference outputs (fast tanh enabled)

Raw little-endian float32, 4096 samples each, produced by the upstream
`render` tool at the commit above, modified ONLY to add a `--fast-tanh`
flag that calls `nam::activations::Activation::enable_fast_tanh()` before
`nam::get_dsp` (upstream `tools/render.cpp` has no such switch; the
activation map is consulted at model construction, so the call must
precede the load). Patch essence:

```cpp
#include "NAM/activations.h"
// in main(), before nam::get_dsp(...):
if (fastTanh) nam::activations::Activation::enable_fast_tanh();
```

| Output | Command (from Core checkout) |
|---|---|
| `wavenet_a1_standard.fasttanh.f32` | `build/tools/render --fast-tanh example_models/wavenet_a1_standard.nam input.wav out.wav` |
| `wavenet.fasttanh.f32` | `build/tools/render --fast-tanh example_models/wavenet.nam input.wav out.wav` |

then `python3 ../a2/wav_to_f32.py out.wav` to strip the WAV header.

Reference-engine conditions (identical to the A2 fixtures, see
../a2/README.md): `NAM_SAMPLE` = double (per-sample f32 cast at the
output; the WaveNet internals are Eigen f32 either way), `Reset(48000, 64)`
with prewarm-on-reset, 64-sample blocks, x86-64 Linux gcc `-O3` Release.
`enable_fast_tanh()` swaps the `Tanh` activation for the reference
`fast_tanh` rational approximation — the engine's `nam::fast_tanh` is that
exact formula (ba todo #1116).

## Measured parity (2026-07-28, engine f32 vs reference render)

| Model | max abs err | first-divergence scale |
|---|---|---|
| `wavenet_a1_standard.nam` | 7.302e-7 | build-noise (FMA-contraction/libm-level) |
| `wavenet.nam` | 1.509e-7 | build-noise |

For history: before ba todo #1116 the legacy engine's A1 path measured
**max abs err 0.32** on this ±0.3 signal against the same reference
(structural divergences recorded in doc #258), and still 4.4e-3 after the
structural fix alone (the engine's previous Padé fast-tanh approximation
differed from the plugin's rational one). The committed pins are the
correct-vs-plugin state.

## File hashes (sha256)

```
1dc758fd30aa05292bd91d63a1a079e0588501f8b6689d6c77386131f97bf82e  wavenet.fasttanh.f32
66bda2b379289eff079c0755588bc9a92760654d9cc9af1b97cf30d0e92b167d  wavenet.nam
94cf77834b96555e116995967be7af5e715869c66792f9c75449f3ab1b73ac82  wavenet_a1_standard.fasttanh.f32
ceb53469a19ce278e2235da982ae676cb8d5451a8de22a7ecc7a2617d07224d1  wavenet_a1_standard.nam
```
