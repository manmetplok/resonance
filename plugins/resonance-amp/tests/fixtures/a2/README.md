# NAM A2 reference-parity fixtures

Fixtures for the A2 (Architecture 2) reference-parity test suite (ba todo
#1115, epic #197, doc #258): real A2 `.nam` files plus reference outputs
generated with the upstream C++ implementation, NeuralAmpModelerCore.

Collected 2026-07-28 (ba todo #1114).

## Provenance & licensing

All `.nam` files are the official test/example models shipped in the
NeuralAmpModelerCore repository, MIT License (Copyright (c) 2023 Steven
Atkinson):

- Repository: https://github.com/sdatkinson/NeuralAmpModelerCore
- Commit used: `3cde95c354d5ba6da01316cad90b05cfc4855053` (library version 0.5.5)
- License: https://github.com/sdatkinson/NeuralAmpModelerCore/blob/3cde95c354d5ba6da01316cad90b05cfc4855053/LICENSE (MIT)
- Files copied verbatim from `example_models/` at that commit, e.g.
  https://github.com/sdatkinson/NeuralAmpModelerCore/blob/3cde95c354d5ba6da01316cad90b05cfc4855053/example_models/A2.nam

MIT permits redistribution; the license notice above satisfies its
attribution requirement.

Note on TONE3000: user-uploaded captures from https://www.tone3000.com were
considered as additional fixtures, but every TONE3000 download path (REST
API `/api/v1/models` + model file URLs) requires an OAuth 2.0 + PKCE
browser login — there is no anonymous access — so none are included.
The upstream example models cover strictly more A2 features than a typical
TONE3000 A2-Full capture. The `.nam` weights here are synthetic test
weights, not trained amp captures; parity testing only needs determinism,
not realism (expect DC offsets / unmusical output).

## Fixture matrix

| File | Architecture | A2 features exercised |
|---|---|---|
| `A2.nam` | `SlimmableContainer`, 2 WaveNet submodels (3ch "Lite", 8ch "Full"), v0.7.0, 48 kHz | Plain-A2 era: `activation` objects (LeakyReLU 0.01), `bottleneck`, `head1x1`, per-layer `gating_mode` arrays (`none`), container submodel selection |
| `slimmable_wavenet.nam` | WaveNet, v0.7.0, 48 kHz | `slimmable` packed weights, `method: "slice_channels_uniform"`, `allowed_channels: [1, 2, 3]` |
| `wavenet_a2_max.nam` | WaveNet, v0.6.0, 48 kHz | Maximal A2: `bottleneck`, activation objects (Softsign), `secondary_activation`, `groups_input` / `groups_input_mixin` / grouped `layer1x1` / `head1x1`, all 8 FiLM insertion points, `condition_dsp` |
| `wavenet_condition_dsp.nam` | WaveNet, v0.6.0, 48 kHz | `condition_dsp` in isolation (string activations, no FiLM/bottleneck) |

## Deterministic input

`input.f32` — raw little-endian float32, mono, 4096 samples, 48 000 Hz:

- `n == 0`: `1.0` (unit impulse)
- `1 <= n < 1024`: `0.0` (decay gap)
- `1024 <= n < 4096`: `0.5 * sin(2*pi*220*(n-1024)/48000)`, evaluated in
  IEEE-754 double precision, then cast to float32

Regenerate with `python3 gen_input.py` (also writes `input.wav`, a 48 kHz
mono IEEE-float32 WAV used to feed the reference renderer; it is not
committed — the raw `.f32` is canonical). sha256 of `input.f32`:
`c142e4432f37fc575aa8e3ee2abcd411e227bd8e05255d43210c3b9299dbc630`.

## Reference outputs

Raw little-endian float32, 4096 samples each, produced by the upstream
`render` tool at the commit above:

| Output | Command (from Core checkout) |
|---|---|
| `A2.slim0.f32` | `build/tools/render --slim 0.0 example_models/A2.nam input.wav out.wav` |
| `A2.slim1.f32` | `build/tools/render --slim 1.0 example_models/A2.nam input.wav out.wav` |
| `slimmable_wavenet.slim0.f32` | `build/tools/render --slim 0.0 example_models/slimmable_wavenet.nam input.wav out.wav` |
| `slimmable_wavenet.slim1.f32` | `build/tools/render --slim 1.0 example_models/slimmable_wavenet.nam input.wav out.wav` |
| `wavenet_a2_max.default.f32` | `build/tools/render example_models/wavenet_a2_max.nam input.wav out.wav` |
| `wavenet_condition_dsp.default.f32` | `build/tools/render example_models/wavenet_condition_dsp.nam input.wav out.wav` |

Default (no `--slim`) output is byte-identical to `--slim 1.0` for both
slimmable fixtures (full size is the load-time default), so only the
slim0/slim1 pair is committed for those.

### Exact reproduction

```sh
git clone https://github.com/sdatkinson/NeuralAmpModelerCore
cd NeuralAmpModelerCore
git checkout 3cde95c354d5ba6da01316cad90b05cfc4855053
git submodule update --init --recursive   # eigen 5.0.1, AudioDSPTools v0.1.1
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build --target render -j
python3 <fixtures>/gen_input.py           # writes input.wav next to it
build/tools/render [--slim X] example_models/<model>.nam input.wav out.wav
python3 <fixtures>/wav_to_f32.py out.wav  # strips the WAV header -> out.f32
```

Reference-engine conditions that a Rust parity test must match:

- `NAM_SAMPLE` is **double** (default build; `NAM_SAMPLE_FLOAT` not defined).
  Internal processing is f64; each output sample is cast to f32 once.
- `DSP::Reset(48000.0, 64)` is called once before processing, with
  prewarm-on-reset enabled (Core default) — the model is warmed up on
  zeros for its receptive field before the first real sample.
- Audio is processed in fixed 64-sample blocks (4096 = 64 x 64).
- Renders are bit-deterministic across runs (verified); outputs were
  produced on x86-64 Linux, gcc, `-O3` Release. Cross-platform float
  differences in the last ulp are conceivable, so parity tests should
  compare with a small tolerance (e.g. <= 1e-6 absolute) rather than
  bit equality.

## File hashes (sha256)

```
2d2d744516dc0197737a2c5001010429692d4cb20d72b08264781de626fcf4ca  A2.nam
9fef7599bdc9c066da400919c032b42c085460914b814a9432bc37c023d8a76d  A2.slim0.f32
1a62f9d898ee3ce6923dd162c2b7cc08e982b9579dfdd0fe9158ee30ba60e3a3  A2.slim1.f32
c142e4432f37fc575aa8e3ee2abcd411e227bd8e05255d43210c3b9299dbc630  input.f32
735c1a86e18140b7cfe90c08427ca6a85f62c32d34cc4048997933652aa774b4  slimmable_wavenet.nam
483d0349f3dd0a33009842bc7fc675d99fd18ff46ef9a585ea78d18fcf12a339  slimmable_wavenet.slim0.f32
e4e51c2df9c6893ff71d1bc6c9063aa936db91013fb3a3a5fceebad861029404  slimmable_wavenet.slim1.f32
64ea27a229dc11e69b4648710e23f285b5dc340635bf15b91c2ca44631186774  wavenet_a2_max.default.f32
fb9d6a0be38dc6f5df29fa52b0d575ddf6c79fa8f0f2e02a9816cb8a82dbce7c  wavenet_condition_dsp.default.f32
12384c6640e1126907b366584024c4abb129ac5920b3dc2d31b29e39315e820d  wavenet_a2_max.nam
1af5a5d4eb079b894e095882738c102fd2d9eeced387a16d0d24cd73a07de718  wavenet_condition_dsp.nam
```
