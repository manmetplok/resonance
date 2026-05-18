#TODO

##Architecture / code health (from code review)

    - Full engine replay on undo is expensive for large projects
      (`resonance-app/src/update/project_io/replay.rs` — `replay_loaded_project`).
      Every undo/redo tears down all engine state and re-instantiates plugins.
      Short-term: diff old and new snapshots and only replay changed tracks/plugins.
      Long-term: consider command-based undo with inverse operations.

##Code review 2026-05-16 — deferred items

### Deeper merges / splits (P2 leftovers)

    - [ ] `resonance-app` 24 remaining inline tests (spread across binary-crate
      submodules: `recent.rs` 2, `undo.rs` 6, `compose/invariants.rs` 5,
      `compose/tests.rs` 8, `update/project_io/replay.rs` 3) need a library
      target before they can become integration tests. Either promote
      `Resonance` + helpers to a `resonance-app` library crate, or accept the
      inline tests as a documented exception.
    - [ ] `resonance-audio/src/recording.rs` inline test exposes private
      `RecordingState`/`TrackRecordingBuf` types; would need to expand public
      surface to migrate.

###P3 — library / idiom

    - [ ] Wrap the lane_inspector body in `iced::widget::lazy` fingerprinted on
      `(selected_lane, definition.id, version_counter)`. **Blocked**: iced 0.13
      `lazy` requires `View: Into<Element<'static, ...>>`. The vocal lyrics block
      uses `text_editor(&Content)` whose widget keeps a borrow alive, so the
      element isn't `'static`. Fixing requires moving `Content` behind `Rc<RefCell<>>`
      (behaviour change) or splitting the inspector so the text_editor lives
      outside the lazy boundary.
    - [ ] Drop `get_unchecked` behind power-of-two masks (benchmark first):
      `resonance-dsp/src/lfo.rs:76-77`,
      `plugins/resonance-wavetable/src/oscillator.rs`,
      `plugins/resonance-amp/src/nam/mod.rs` matvec.
    - [ ] Compile-time CMU dict via `phf_codegen`. **Deferred**: 135 k entries
      blow up build time (phf perfect-hash search is exponentially slower past
      ~100 k keys). Runtime parse is ~50 ms once via `OnceLock` — not load-bearing.

###P4 — dependencies

    - [ ] Major-version upgrades: `ureq 2 → 3`, `rand 0.8 → 0.10`, `dirs 5 → 6`,
      `iced 0.13 → 0.14`, `cpal 0.15 → 0.17`, `ringbuf 0.4 → 0.5`, `symphonia 0.5 → 0.6`.

###P5 — smaller wins

    - [ ] Replace `super::super::super::chord::Chord` at
      `resonance-music-theory/src/derive/vocal/style/mod.rs:89` with `crate::chord::Chord`.
    - [ ] Build `HashMap<PluginInstanceId, PluginLocator>` side-index in `Resonance` to replace
      the linear scan in `with_plugin_mut` (`resonance-app/src/main.rs:305`).
    - [ ] Group `vocal_audio_clips`/`vocal_clip_lyrics`/`vocal_render_epoch` into a
      `VocalAudioRegistry` struct in `resonance-app/src/compose/state.rs:77-112`.
    - [ ] Drop `pub(super)` on internals of `resonance-music-theory/src/derive/motif_engine.rs`
      once it becomes a directory (most should be private).
    - [ ] Replace `#[allow(clippy::too_many_arguments)]` on `apply_motif_pitches`,
      `shape_velocity`, `realize_phrase` with config structs.
    - [ ] Add `VocalParams::validate(&self) -> Result<()>` and call it at the app boundary
      (108 LOC of pub fields with hidden invariants).
    - [ ] Add `// SAFETY:` comments at:
      `resonance-audio/src/types/clip.rs:119` (Mmap::map),
      `wayland-plugin-gui/src/egl_context.rs:46,51,117`,
      `resonance-common/src/denormal.rs:30-35` (FTZ/DAZ asm).

##Code review 2026-05-18 — structural findings

### Separation-of-concerns violations

    - [x] `resonance-audio/src/recording.rs:108-160` — `create_track_buf()`
      mixes `TrackRecordingBuf` struct construction with `std::fs::create_dir_all`
      and `WavWriter::create`. Split: a pure constructor returning the buf, plus
      a separate `open_track_wav_file(path)` I/O helper.
    - [x] `resonance-audio/src/recording.rs:247-320` — `finalize_recording()`
      interleaves writer finalization with `std::fs::remove_file`. Extract
      `finalize_wav_file(path)` so the state mutation is testable without the
      filesystem.

### mod.rs files still housing real logic (>200 LOC)

The mastering plugin's `chain.rs` + `stages/` + `params/` decomposition is the
reference. After the earlier P2 splits, the *files* moved but the orchestrator
mod.rs in several places still owns the work it dispatched.

    - [x] `plugins/resonance-reverb/src/dsp/mod.rs` (423 LOC). Holds the full
      `ReverbDsp` impl: state init + 23 setters/getters + the 350-line process
      hot path. Move the orchestrator to `chain.rs`; promote diffusion/FDN math
      already in submodules to carry the per-stage state too. mod.rs should only
      re-export.
    - [x] `plugins/resonance-amp/src/nam/wavenet/mod.rs` (507 LOC). The
      `WaveNetModel` impl (incl. 120-line `from_config_and_weights`) belongs in
      a sibling `model.rs` alongside `conv_layer.rs`, `head.rs`, `ring.rs`.
    - [ ] `plugins/resonance-amp/src/editor/mod.rs` (237 LOC),
      `plugins/resonance-delay/src/editor/mod.rs` (226 LOC),
      `plugins/resonance-ir/src/editor/mod.rs` (220 LOC),
      `plugins/resonance-drums/src/editor/mod.rs` (256 LOC). Each houses
      `EditorFactory` + `EditorApp` + dispatch helpers. Extract the factory to
      `editor/factory.rs` (as the eq/compressor/wavetable splits already did).
      For drums, also lift the `reload_kit` helper out of `mod.rs`.

### Oversized source files worth a split

    - [ ] `resonance-app/src/compose/vocal_svs/segment.rs` (571 LOC) is one
      537-line `build_segment()`. Suggested split: `duration.rs` (phoneme
      durations), `f0.rs` (pitch curve + portamento/vibrato), `tension.rs`.
    - [ ] `resonance-app/src/update/compose/lane_inspector.rs` is still a giant
      match across Bass/Melody/Pad/Drum/Vocal parameter updates. Split per
      generator: `bass_params.rs`, `melody_params.rs`, `pad_params.rs`,
      `vocal_params.rs` under `update/compose/lane_inspector/`.
    - [ ] `resonance-app/src/view/compose/vocal_roll/draw.rs` (893 LOC). Split
      into `keyboard.rs`, `notes.rs`, `grid.rs` — each section already paints
      a disjoint region of the canvas.
