# Wavetable synth

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: none in `docs/history/specs/`. The "synth character" work
> (filter models, user wavetables, new mod sources, oscillator
> interaction, phase warp, sub and noise) was built on `synth/integration`
> and merged to master in `0957ccfd` (2026-09-28); the branch is gone.

`com.resonance.wavetable` is the first-party polyphonic synth: two wavetable
oscillators with warp and interaction modes, sub and noise, five filter
models, two envelopes, three LFOs plus a sample-and-hold generator, an
8-slot mod matrix, and a distortion → chorus → delay chain. Users play it
from MIDI tracks and edit it in its editor. Agents drive it by param name
through the plugin-param tools.

## Behaviour

- **Voices.** Up to 32 voices (`Max Voices`, default 16), unison up to 7
  sub-voices, mono legato with a 16-note held stack. Stealing considers only
  sounding voices; a retriggered or stolen voice continues instead of
  clicking; unison start phases are spread.
- **Oscillators.** Two oscillators. Each reads one of 10 factory tables
  (Basic, Saw Stack, PWM, Formant, Digital, Harmonic Sweep, Metallic, Organ,
  Noise Cycle, Sync Sweep) or table index 10, the user wavetable. Tables are
  2048 samples with 12 mip levels and are generated at build time
  (`build.rs` → `wavetables.bin`), embedded with `include_bytes!` and
  `mlock`ed once per process. Low mip levels use 6-point Lagrange reads.
- **Character.** Warp per oscillator: Off, Bend, Mirror, PWM, Formant,
  Quantize. Oscillator interaction (`Osc Mix Mode`): Sum, FM, Ring, Sync.
  Sub (sine or square, −1 or −2 octaves) and noise (white or pink, with
  colour). `Analog` adds drift and per-note cutoff and level spread;
  `Osc Phase Random` randomises start phase. The character kernel runs only
  when an interaction mode or warp is active.
- **User wavetables.** "Load…" in the Osc tab imports a WAV (hound). A Serum
  `clm ` chunk sets the frame length (16–65536, resampled to 2048); otherwise
  the file is cut into 2048-sample frames. Short files become one cycle, at
  most 256 frames are kept (thinned evenly), each frame is DC-removed, the
  table is peak-normalised and mixed to mono. The table is built off the
  audio thread and handed over through a `Mailbox`; the old table is freed
  on a janitor thread. The plugin state stores the frames themselves, so a
  project does not depend on the WAV file staying put.
- **Filter.** Models Clean (SVF), Ladder, Diode, MS-20 and Nonlinear SVF;
  types LP, HP, BP, Notch; `Filter FM` for audio-rate cutoff modulation.
- **Envelopes.** Amp and mod ADSR with curve; times are time-to-target
  (DSP2-12). Factory presets were rescaled for that change with
  `plugins/resonance-wavetable/tools/rescale_envelope_times_dsp2_12.py`.
- **Modulation.** Three LFOs (Sine, Tri, Saw, Square, S&H; Free, Retrig or
  Sync to 17 divisions from 8 bars to 1/64) and a global S&H generator
  (`mod_sh_*`). Mod matrix: 8 slots, 13 sources (None, LFO1–3, Env2,
  Velocity, KeyTrack, ModWheel, Aftertouch, Random bipolar/unipolar,
  SampleHold, Alternate) and 18 destinations, including Dist Drive, Voice
  Drive, Filter FM, Osc Mod Amount and Osc 1/2 Warp.
  **Mod Wheel and Aftertouch are marked unavailable and do nothing**
  (`ModSource::unavailable_reason`).
- **Effects.** Distortion modes Soft, Tube, Fold, Hard, Crush, Rectify with
  Off/2x/4x oversampling, tone, auto gain, bits and crush rate; a per-voice
  pre-filter `Voice Drive`. Chorus modes Classic, Juno I, Juno II, Juno I+II,
  Ensemble (BBD band-limiting and noise). Delay in ms, not tempo-synced.
- **Params.** 122 (`PARAM_COUNT`). `Master Volume` 0–1, default 0.8,
  smoothed. Stepped params take numbers over the control API.
- **Factory presets.** 28 format-1 JSON files in
  `plugins/resonance-wavetable/presets/` (Init, Lead, Bass, Pad, Pluck,
  Keys, Arp, FX, Brass, Strings), each a complete snapshot of all params.
- **Editor tabs.** Osc, Env · Filter, LFO, Mod, FX, with live views
  (waveform, frame strip, envelope, filter response, LFO shape, scope) fed
  by `WavetableVizState`.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Plugin | `plugins/resonance-wavetable/src/lib.rs` | `ResonanceWavetable`, `CLAP_ID = first_party::WAVETABLE` |
| Params | `src/params/{mod,osc,character,analog,filter,env,lfo,sample_hold,mod_slot,unison,fx}.rs` | `PARAM_COUNT`, `param_at` |
| Engine + voices | `src/dsp/{engine,voice,oscillator,wavetable,wavetable_gen}.rs` | `SynthEngine`, `MAX_VOICES`, `NUM_OSCS`, `ensure_resident` |
| Render pipeline | `src/dsp/render/{snapshot,plan,kernel,character}.rs` | one atomic load per param per block; no alloc or locks per sample |
| Character DSP | `src/dsp/{warp,osc_mix,sub_noise,analog,filter_models,filter}.rs` | `WarpMode`, `OscMixMode`, `FilterModel` |
| Modulation | `src/dsp/{modulation,lfo,envelope}.rs` | `ModSource`, `ModDest`, `NUM_MOD_SLOTS` |
| Effects | `src/dsp/effects.rs` | `DistMode`, `ChorusMode` |
| User tables | `src/user_wavetable/{mod,import,state}.rs`, `src/dsp/user_table.rs` | `UserWavetables`, `UserTableSwap`, `MAX_USER_FRAMES` |
| Presets | `src/presets.rs`, `presets/*.json` | `PRESETS` |
| Editor | `src/editor/{app,chrome,factory,display_waves}.rs`, `editor/tabs/`, `editor/viz/` | `WtTab` |

Invariant: the per-sample kernel never calls `Param::value()`, allocates or
locks; `tests/user_wavetable_rt_alloc.rs` arms a counting allocator on the
process thread to prove table install, replace and clear do no heap work.

## Surfaces

- **GUI:** add as an instrument on a MIDI track; editor from the plugin
  slot. Shared preset bar in the editor and the app's preset browser
  ([plugin-preset-library.md](plugin-preset-library.md)).
- **Control API / MCP:** `track.add_instrument` →
  `mcp__resonance__track_add_instrument`; params through
  `track_plugin_params` / `track_set_plugin_param`; presets through
  `track_plugin_presets` / `track_load_plugin_preset` /
  `track_save_plugin_preset`. No method imports a user wavetable.
- **Agent skills:** the user-level skill `resonance-synth-design`
  (`~/.claude/skills/`, outside the repo) is written against this param
  set; every param it names exists on master. Its reference still says it
  matches "branch synth/integration", which is stale but harmless. No skill
  in `resonance-agent-plugin/` targets the synth.

## How it's tested

| What | Where | Run |
|---|---|---|
| Instantiation, state, labels, master vol | `plugins/resonance-wavetable/tests/{instantiation,state,labels,master_vol,mod_availability}.rs` | `./scripts/run-tests.py -p resonance-wavetable` |
| Voices and oscillators | `tests/{voice_allocation,retrigger_continuity,unison_onset,mip_aliasing,osc_character}.rs` | same |
| Envelopes, LFOs, mod | `tests/{envelope_times,lfo_sync,mod_sources,mod_destinations}.rs` | same |
| Character DSP | `tests/{analog,filter_models,distortion,chorus_modes}.rs` | same |
| User wavetables | `tests/{user_wavetable,user_wavetable_rt_alloc}.rs` | same |
| Presets | `tests/{presets,factory_preset_envelope_times}.rs` | same |
| Goldens | `tests/{null_test,render_block_regression}.rs` → `tests/golden/{null_test,render_block_regression}.f32` | `RESONANCE_BLESS=1` (or `RESONANCE_BLESS_NULL_TEST=1`, `RESONANCE_BLESS_RENDER_BLOCK=1`) |
| Editor | `tests/{editor_controls,editor_param_binding,viz}.rs` | same |
| Performance | `benches/synth.rs` (Criterion) | `cargo bench -p resonance-wavetable` |

Not covered: no test listens for musical quality; no app-level test drives a
user wavetable import end to end.

## Decisions and deviations

- Factory tables are generated, not shipped as audio files, and are
  zero-copy at runtime.
- The user table is stored inside the plugin state (up to 256 × 2048
  floats), so presets and projects are self-contained.
- Three places name the envelope rescale script differently;
  `tests/factory_preset_envelope_times.rs` points at
  `scripts/rescale_wavetable_envelope_times.py`, which does not exist. The
  real file is `plugins/resonance-wavetable/tools/rescale_envelope_times_dsp2_12.py`.

## Known gaps

- Mod Wheel and Aftertouch mod sources are dead:
  [wavetable-mod-wheel-aftertouch](../../intents/plugins-instruments/wavetable-mod-wheel-aftertouch.md).
- Delay is not tempo-synced.
- LFO shape labels live in the editor, not on the param (ba todo #1292
  note in `editor/tabs/mod.rs`).
