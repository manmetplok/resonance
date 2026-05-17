# resonance-svs — DiffSinger ONNX inference

Renders a DiffSinger `.ds` JSON score into a WAV file by running the voicebank's
acoustic ONNX → vocoder ONNX pipeline. Consumed by the Resonance app's Compose
vocal generator (`resonance-app::compose::vocal_svs`) and usable on its own via
the `resonance-svs` CLI.

## TL;DR — get singing in under 5 minutes

```bash
# 1. Fetch TIGER (English DiffSinger) + its companion vocoder (~580 MB total).
resonance-svs/scripts/download_models.sh

# 2. Render the bundled sample .ds file.
cargo run -p resonance-svs --release -- \
  --ds-file        resonance-svs/models/sample/hello_tiger.ds \
  --acoustic-config resonance-svs/models/singer/extracted/dsacoustic/dsconfig.yaml \
  --vocoder-config  resonance-svs/models/singer/extracted/dsvocoder/vocoder.yaml \
  --out            resonance-svs/models/sample/hello_tiger.wav \
  --speaker        tiger_disco \
  --speedup        20

# 3. Listen.
xdg-open resonance-svs/models/sample/hello_tiger.wav
```

A 2-second clip renders in ~0.5 s on a modern CPU. The sample says "hello tiger"
on A4 → C5 in the `tiger_disco` voice. Other speakers shipped with TIGER:
`tiger_electric`, `tiger_fresh`, `tiger_glam`, `tiger_mystic`, `tiger_royal`,
`tiger_vinyl`.

## What it does

```
.ds  →  parse + preprocess  →  acoustic ONNX (mel)  →  vocoder ONNX (waveform)  →  WAV
```

The combined-acoustic path (Jobsecond/diffsinger-onnx-infer style) is what's
exercised end-to-end. Linguistic / duration / pitch / variance stages exist as
module scaffolding for the split openvpi exporter format, but the smoke test
does not drive them — phoneme durations and `f0_seq` are read directly from the
`.ds` file.

## Build

```
cargo build -p resonance-svs                # CPU only
cargo build -p resonance-svs --release
cargo build -p resonance-svs --features cuda --release   # requires CUDA toolchain
cargo build -p resonance-svs --features rocm --release   # requires ROCm + matching ort binary
```

The default build links a CPU-only `onnxruntime` binary pulled by the `ort`
crate at compile time — no system install is needed for plain CPU rendering.

## Models bundled by the download script

`scripts/download_models.sh` downloads and extracts into `resonance-svs/models/`:

| What | URL | License |
|------|-----|---------|
| **TIGER** English DiffSinger v106 (~529 MB) | <https://github.com/spicytigermeat/tiger_diffsinger> | CC BY-NC-SA 4.0 |
| **tgm_hifigan v110** standalone vocoder (~50 MB, redundant — TIGER ships its own) | <https://github.com/mrtigermeat/tgm_hifigan> | CC BY-NC-SA 4.0 |

Both are non-commercial. Re-read the licences before doing anything beyond
personal experiments. The TIGER bundle includes 41 community voices; ready-to-go
speaker embeddings (`.emb` files) are only shipped for the 7 `tiger_*` speakers.

## Writing your own `.ds`

`models/sample/hello_tiger.ds` is the smallest possible example: one segment,
ten phonemes, four notes, two seconds. Field semantics:

| Field | Meaning |
|-------|---------|
| `ph_seq` | space-separated phonemes (matching TIGER's `phonemes.txt` ARPAbet-X set) |
| `ph_dur` | per-phoneme durations in seconds; must sum to total length |
| `ph_num` | phonemes per word for the duration predictor (advisory in this PoC) |
| `note_seq` | per-note pitches as `C4`, `D#5`, `Bb3`, `rest`, etc. |
| `note_dur` | per-note durations in seconds |
| `note_slur` | 0 = note start, 1 = slur into previous note |
| `f0_seq` | sampled fundamental frequency in Hz, space-separated |
| `f0_timestep` | seconds between f0 samples |

For multi-segment songs, the JSON top-level is an array of segments; each
segment carries its own `offset` in seconds along the global timeline.

## Execution providers

| Flag | Behaviour |
|------|-----------|
| `--execution-provider cpu` (default) | CPU-only, works on any machine. |
| `--execution-provider cuda` | Registers the CUDA provider, falls back to CPU on failure. Build with `--features cuda`. |
| `--execution-provider rocm` | Same, ROCm. Build with `--features rocm`. |

The acoustic model is the heavy stage (diffusion sampling steps); the vocoder
always runs on CPU because the per-call dispatch overhead of a GPU outweighs
its compute advantage for typical vocoders.

## Debugging

```
cargo run -p resonance-svs --example probe --release -- path/to/model.onnx
```

Prints every input / output of an ONNX file with its dtype and shape. Useful
when a voicebank from a different exporter throws "Missing Input: foo" — the
stage modules use runtime introspection of these names to decide which
optional inputs to populate.

## Tests

```
cargo test -p resonance-svs                                                          # parser-only smoke
SVS_POC_VOICEBANK_DIR=resonance-svs/models/singer/extracted/dsacoustic \
  SVS_POC_DS_FILE=resonance-svs/models/sample/hello_tiger.ds \
  SVS_POC_SPEAKER=tiger_disco \
  cargo test -p resonance-svs end_to_end_render -- --nocapture
```

The render test relies on the bundled vocoder shipped alongside `dsacoustic` —
ensure `singer/extracted/dsvocoder/vocoder.yaml` is present (the download
script puts it there).

## Layout

```
resonance-svs/
├── Cargo.toml
├── README.md
├── NOTES.md
├── .gitignore                 # excludes models/, *.onnx, *.zip, *.wav
├── scripts/
│   └── download_models.sh     # fetches TIGER + vocoder
├── models/                    # NOT committed; populated by the script
│   ├── singer/                # TIGER acoustic + bundled vocoder
│   ├── vocoder/               # standalone tgm_hifigan vocoder
│   └── sample/                # hello_tiger.ds, hello_tiger.wav
├── src/
│   ├── main.rs                # CLI entry
│   ├── lib.rs                 # exposes modules to tests
│   ├── audio.rs               # WAV writer (hound) and timeline-mix helper
│   ├── config.rs              # dsconfig.yaml + vocoder.yaml + phonemes.txt parsers
│   ├── ds.rs                  # .ds JSON parser, SampleCurve, note-name → MIDI
│   ├── pipeline.rs            # stage orchestration
│   └── stages/
│       ├── mod.rs
│       ├── common.rs          # session builder, execution-provider parsing
│       ├── acoustic.rs        # combined acoustic ONNX (Jobsecond-style)
│       ├── vocoder.rs         # mel + f0 → waveform
│       ├── linguistic.rs      # split-pipeline stage scaffold
│       ├── duration.rs        # split-pipeline stage scaffold
│       ├── pitch.rs           # split-pipeline stage scaffold
│       └── variance.rs        # split-pipeline stage scaffold
├── examples/
│   ├── probe.rs               # ONNX I/O inspector
│   └── render_resonance_vocal.rs  # CLI that mirrors the resonance-app pipeline
└── tests/
    └── smoke.rs
```
