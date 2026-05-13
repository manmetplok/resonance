# svs-poc — implementation notes

## Reused Resonance types

**None — by design.** The Resonance workspace was surveyed (see inventory
below) but no Resonance crate type proved a clean fit for the PoC pipeline.
The PoC therefore declares no Resonance crate as a dependency. This is the
strictest possible interpretation of the one-way-coupling rule: removing the
PoC is purely a `rm -rf experiments/svs-poc/` plus one workspace-members
edit, with zero risk of orphaned references.

A future real integration that wants to share code with this PoC would more
likely *adapt* Resonance types to ONNX needs than reuse them as-is. The
candidates I considered, and why each was rejected for this PoC:

| Candidate                           | Why not reused                                                                 |
|-------------------------------------|--------------------------------------------------------------------------------|
| `resonance_common::wav::*`          | Decode-only; provides no WAV writer. PoC uses `hound` for output (same writer Resonance would land on if it ever needs one). |
| `resonance_music_theory::PitchClass`| No MIDI-note-number ↔ Hz helper, and DiffSinger note names ("C#4", "Bb3") need a regex parser anyway. The 30-line Jobsecond port is simpler than juggling `PitchClass` + manual octave/accidental math. |
| `resonance_dsp::Biquad` / `Lfo`     | The DiffSinger pipeline doesn't filter or modulate. Vocoder output is final. |
| `resonance_audio::decode::*`        | We *write* audio, not read it. |
| `resonance_audio::types::*`         | DAW-shaped (clips, tracks, tempo). Heavyweight for a CLI tool. |

The version-matching exercise was useful regardless: `hound 3.5.1`, `serde 1`,
`serde_json 1`, `anyhow 1`, `thiserror 1`, `tracing 0.1`, `regex 1` were
already in `Cargo.lock` and were pinned to compatible versions in the PoC's
`Cargo.toml`, so `cargo build` compiles each at most once across the
workspace. Newly introduced to the dependency tree: `ort`, `ndarray`,
`serde_yaml`, `clap`, `tracing-subscriber`.

## Inventory of Resonance shared-code touches

**Only one:** `Cargo.toml` workspace `members` list — added
`"experiments/svs-poc"`. That's an additive change with no semantic effect on
any other crate's build. No source file in any existing Resonance crate was
modified. `Cargo.lock` updated automatically by `cargo build`.

## Pipeline shape

```
.ds JSON  →  preprocess (token map, frame-quantize durations, resample f0)
         →  acoustic ONNX  (tokens, durations, f0, speedup, [optional inputs])  →  mel
         →  vocoder ONNX   (mel, f0)                                            →  waveform
         →  timeline mix → WAV
```

The combined-acoustic ONNX path is what's wired end-to-end. The split-pipeline
stages (`linguistic.rs`, `duration.rs`, `pitch.rs`, `variance.rs`) are loaded
on demand but the smoke-test path doesn't traverse them — `.ds` files from
the openvpi sample set carry `ph_dur` and `f0_seq` directly, so the variance
chain isn't needed for an audible result.

## Performance numbers

Measured locally on x86_64 CPU against TIGER v106 acoustic + bundled
tgm_hifigan vocoder, single-threaded ONNX Runtime, `--speedup 20`:

| Segment | Audio length | Wall time | Real-time factor |
|---------|--------------|-----------|------------------|
| `hello_tiger.ds` (1 segment, 10 phonemes, 4 notes) | 2.00 s | **0.47 s** | ~4.25× faster than realtime |

Output sanity check (RMS per 200 ms window):

```
  0.00s: rms=0.008  (AP silence — correct)
  0.20s: rms=0.13   (hh-ah onset — "h" of hello)
  0.40s: rms=0.17   (l-ow — sustained on A4)
  0.60s: rms=0.17
  0.80s: rms=0.15
  1.00s: rms=0.17   (t-ay onset — "ti" of tiger)
  1.20s: rms=0.08   (closure between syllables)
  1.40s: rms=0.14   (g-er — "ger" on C5)
  1.60s: rms=0.08   (release)
  1.80s: rms=0.007  (AP silence — correct)
```

Peak amplitude 0.49, no clipping, dynamic envelope tracks the `.ds` phoneme
durations exactly. ROCm and CUDA not measured (no GPU in test environment); the
session-builder code paths register the requested provider but rendering
correctness on accelerators is deferred per the PoC spec.

The pipeline reports `per_segment_seconds` in its `RunSummary`; collect this
across longer / more elaborate `.ds` files to build out a richer perf table.

## Model-loading quirks

- **ort version pin.** Pinned to `=2.0.0-rc.12` rather than `2.0.0-rc.10`
  because cargo picked rc.12 (latest matching `^2.0.0-rc.10`) and rc.12
  bumped its internal `ndarray` dependency from 0.16 to 0.17. Mixing 0.16
  and 0.17 in one tree produces `OwnedTensorArrayData` trait-bound mismatches
  that the compiler diagnoses as "multiple different versions of crate
  `ndarray` in the dependency graph". The fix is to match the ort-internal
  ndarray version exactly; the exact pin makes future bumps explicit.
- **`ort::Error` and `anyhow::Context`.** ort 2.x's `Error<C>` is generic in a
  context type and consequently does not implement the trait bounds anyhow's
  `Context` extension requires. The PoC converts via
  `.map_err(|e| anyhow::anyhow!("…: {e}"))` instead. Annoying boilerplate,
  but explicit.
- **`Session::inputs` / `Session::outputs`.** Used to be public fields in
  older ort; in 2.0.0-rc.12 they are methods returning `&[Outlet]`. Outlet's
  `name` and `dtype` are also methods, not fields. The PoC adapts via small
  `input_names()` / `output_names()` helpers that turn them into
  `HashSet<String>` for "does this model declare input X?" feature-detection.
- **CPU fallback for non-default providers.** The session builder is given
  the requested provider followed by `CPUExecutionProvider::default()`, so
  if the requested provider isn't registered (e.g. ROCm requested on a CPU
  machine), session creation still succeeds and the model runs on CPU. The
  PoC logs a warning when the `rocm` feature flag is off.
- **`speedup` vs `steps`.** Two generations of openvpi exporter coexist in
  the wild. Older ONNX takes a `speedup: int64` scalar (PNDM stride); newer
  "continuous-acceleration" exports take a `steps: int64` scalar (direct
  diffusion step count). TIGER v106 is the newer kind. The PoC introspects
  the model's declared inputs and supplies whichever it asks for, using the
  same `--speedup` CLI value as the underlying number. Models that declare
  neither are accepted too.
- **`depth` dtype changed.** Older shallow-diffusion exports take
  `depth: int64` (absolute step count, e.g. 1000); newer variable-depth
  exports take `depth: float32` (fractional 0–1 of the total schedule, e.g.
  0.6). The PoC introspects the dtype of the `depth` input and serialises
  accordingly. `max_depth` in dsconfig.yaml mirrors this (int vs float).
- **TIGER's bundled vocoder beats the standalone one.** The standalone
  `tgm_hifigan v110` vocoder.yaml declares `mel_base: e`, while TIGER's
  acoustic dsconfig.yaml declares `mel_base: '10'`. Feeding TIGER mels into
  the v110 vocoder produces buzz / distortion. The `dsvocoder/` directory
  inside the TIGER zip ships its own matched vocoder that consumes the
  acoustic's native mel scale — use that instead. (The PoC doesn't insert
  an automatic log10 ↔ logE conversion; documenting and using the matched
  vocoder is the workable path.)

## dsconfig.yaml edge cases hit

- **Field aliases.** Older voicebanks use `use_energy_embed` /
  `use_breathiness_embed` (acoustic-side embedding flags); newer ones add
  `predict_energy` / `predict_breathiness` (variance-side prediction flags).
  Both groups are represented in `config::DsAcousticConfigRaw` with
  `Option<bool>` so missing keys don't fail parsing.
- **`augmentation_args.random_time_stretching.domain`** is sometimes the
  string `"log"` and sometimes absent. Parsed but not surfaced to the
  pipeline because the PoC doesn't perform augmentation.
- **`speakers` may be missing entirely** for single-speaker voicebanks. The
  pipeline treats an empty list as "single speaker, no spk_embed input
  needed" and lets the acoustic model's `input_names()` decide whether
  `spk_embed` is required.
- **TIGER's namespaced file names.** TIGER ships `dsconfig.yaml` referencing
  `tgm_acou_v106.onnx` / `tgm_acou_v106.phonemes.txt` / speaker names like
  `tgm_acou_v106.tiger_disco`, but the on-disk files are bare:
  `acoustic.onnx`, `phonemes.txt`, `tiger_disco.emb`. That's an OpenUTAU
  convention — OpenUTAU expects voicebanks to be namespaced so multiple
  voicebanks can coexist in one installation. The PoC's `dsconfig.yaml` is
  rewritten by `scripts/download_models.sh` to strip the namespace prefix.
  Original retained at `dsconfig.original.yaml` for reference.
- **Speakers may declare more `.emb` files than are shipped.** TIGER's
  dsconfig lists 41 speakers but only 7 `tiger_*.emb` files are bundled. The
  PoC logs a warning and continues; the user must pick a speaker whose
  embedding file actually exists.

## .ds parser quirks

- **`f0_timestep` is sometimes a number, sometimes a numeric string.**
  Handled with an untagged-enum `TimestepField` that accepts both. The
  openvpi sample files use the string form ("0.005"), but tools written in
  Rust / Go that round-trip the file often emit the bare number.
- **`note_seq` uses musical names with accidentals**: `"C4"`, `"D#4"`,
  `"Bb3"`, `"rest"`. Ported the regex parser from Jobsecond's
  `noteNameToMidi`. Rests, malformed names, and empty strings all map to
  MIDI 0, matching the reference.
- **`ph_dur` is in seconds, not frames.** Conversion is cumulative
  (accumulate → round → diff) to keep the cumulative position aligned
  exactly with `(sum(ph_dur_seconds) / frame_length).round()`, otherwise
  per-phoneme rounding drift causes the acoustic model to ingest a token
  count that doesn't match the f0 frame count.

## scripts/download_models.sh post-extraction step

The script rewrites the extracted `dsconfig.yaml` in place using `sed` to
strip the OpenUTAU `tgm_acou_v106.` prefix from file paths and speaker names.
This is the minimal, reproducible alternative to writing a name-mangling
loader into the PoC itself — keeps the PoC's config parser straight.

## Where the reference implementations diverged from documentation

- **openvpi's `ConfigurationSchemas.md`** documents the dsconfig.yaml fields
  but not their interaction with the acoustic ONNX's input set. The actual
  source of truth is what the exported model declares as inputs — hence the
  PoC uses runtime introspection (`session.inputs()`) and treats dsconfig
  flags as advisory.
- **OpenUtau's `DsConfig.cs`** carries additional fields (`use_continuous_acoustic_embed`,
  `use_variable_depth`, `use_lang_id`) that don't appear in
  diffsinger-onnx-infer's C++ struct. Both are accepted by `DsAcousticConfigRaw`
  but the PoC only acts on the subset Jobsecond's reference acts on,
  because that's the path it has working code for.
- **Jobsecond's vocoder always runs on CPU**, hard-coded in
  `main.cpp`. The PoC mirrors this — the `--execution-provider` flag only
  routes the acoustic model.
- **Sample `.ds` files in openvpi/DiffSinger/samples/** are Chinese-language
  examples. They depend on the phoneme dictionary of a Mandarin singer
  voicebank. Trying to render them against an English voicebank produces
  garbled output even when nothing crashes, because phoneme tokens that
  don't appear in the singer's `phonemes.txt` map to id 0 (silence/AP).
  This is a feature of DiffSinger's contract, not a PoC bug.

## What the PoC explicitly does *not* do

- No phonemizer. Input must already be phonemized via `.ds`.
- No streaming, chunked render, or incremental re-render.
- No project / clip / track / automation integration with the rest of
  Resonance.
- No variance ONNX execution. Voicebanks whose acoustic model declares
  required `energy` or `breathiness` inputs but whose `.ds` omits them will
  fail with a clear error pointing at the missing curve.
- No multi-segment crossfade. Segments are mixed by overlap-add at sample
  offsets; if two segments overlap, they sum.
