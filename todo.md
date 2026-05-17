#TODO

##General
    - [x] Conformation when a user wants to kill the app, but there are unsaved changes
    - [x] Conformation when a user wants to delete a track which has content.
    - [x] I want to create presets for track (bas guitar, rhythm guitar, solo, etc) we should have user presets, and a number of default presets.

##Arrange tab
    - [x] We need a place for global tracks like tempo track, and signature changes, my idea would an collapsable area between the normal tracks and the time indication
    - [x] Implement tempo track and signature track
    - [x] Selecting a track should highlight it
    - [x] Move the delete track button to the top right position. Its different from the rest of the buttons
    - [x] The solo functionality does not seem to work.

##Mix tab
    - [x] We need a new solution for subtracks. My suggestion would be to make the collapsed view the default. But then make it a bit wider (maybe two slots, and show db meters of all subtracks. When expendanded the user can modify gain etc.
    - [x] Something goes wrong when saving with busses. When opening a saved project the bus is there, but does not work. When removing the bus, audio can be heard again.

##Compose tab
    - [x] We need a solution about the editing of instruments, the current view is to small so its hard to pick the correct notes. We need to brainstorm about this.
    

##Plugins general
    - [x] We need better controls, the current control boxes run out of screen, maybe knobs are better then slider. Also the visuals (scopes etc) can be smaller, and maybe stacked. Lets come up with a coherent design for each plugin and inplement it.

##Delay plugin
    - [x] Feedback range is to large afaik


##Drum plugin
    - [x] We should be able to download the drumkit as a zip from a server. (see https://resonance.plok.org/index.json)
    - [x] Check if round robin is executed correctly (also add unit tests)
    - [x] Implement all pads/parts found in /home/jorrit/Documents/Guitar/drummica
    - [x] Add ability to delete installed drumkits from the download panel

##Architecture / code health (from code review)

    - [x] Break up the `Resonance` god object (`resonance-app/src/main.rs:25-82`).
      The struct holds 30+ fields and every message routes through a single 1000-line
      `dispatch()` in `update.rs`. Give each sub-state (`TransportState`, `MixerUiState`,
      `ClipInteractionState`, etc.) its own `handle()` method that takes the relevant
      message variant and an `&AudioEngine`, so `dispatch()` becomes a thin router.
      This keeps Iced's single-state-tree but makes ownership clear.

    - Full engine replay on undo is expensive for large projects
      (`resonance-app/src/undo.rs` — `replay_loaded_project`).
      Every undo/redo tears down all engine state and re-instantiates plugins.
      Short-term: diff old and new snapshots and only replay changed tracks/plugins.
      Long-term: consider command-based undo with inverse operations.

    - [x] Deduplicate tempo/signature event removal logic in `update.rs:141-233`.

    - [x] Add debug-build warnings for silent track/clip lookup misses.

    - [x] Promote sub-tracks to an engine concept or document the gap.

    - [x] Document `fetch_max` invariant on peak level atomics.

    - [x] Cap `ClapInstance.pending_params` to prevent unbounded growth.

    - [x] Centralize and document hard-coded limits.

##Code review 2026-05-16 — deferred items

### Deeper merges / splits (P2 leftovers)

    - [x] Full merge of `midi_editor.rs` (bottom piano roll) and
      `view/compose/expanded_editor.rs` (compose-tab piano roll). Coordinate
      helpers, hit testing, keyboard column, and the rounded-rect note draw
      are now shared via `piano_roll::{PianoRollLayout, PianoRollViewport,
      hit_test_note, draw_note, draw_keyboard, NoteStyle, NoteEdge}`. Each
      canvas still owns its `Program` impl (event routing, message types) and
      its canvas-specific bits (velocity lane, scale row tint, toolbar, per-bar
      beat-grid walking).
    - [x] Split `resonance-plugin/src/clap_bridge.rs` into
      `clap_bridge/{mod,shared,ports,params,state,gui,process}.rs`.
    - [x] Split `resonance-audio/src/types/tempo.rs` into
      `types/tempo/{map,conversion,bars,format}.rs`.
    - [x] Split `resonance-audio/src/engine/bounce.rs` into
      `bounce/{render,wav,clip}.rs`.
    - [x] Split `wayland-plugin-gui/src/window_thread.rs` into
      `window_thread/{event_loop,state,delegates,paint,debug}.rs`
      (`event_loop.rs` because `loop` is a reserved keyword).
    - [x] Move MIDI-hardware fields out of `HandlerState` into `MidiHardwareState`
      in `engine/midi/state.rs`; `HandlerState.midi_hw` replaces the inline fields.
    - [x] Split `plugins/resonance-drums/src/sampler.rs` into `dsp/{sampler,voice_pick,janitor}.rs`.
    - [x] Split `plugins/resonance-drums/src/editor/mod.rs` into
      `editor/{header,pad_grid,pad_inspector,kit_browser}.rs`.
    - [x] Split `plugins/resonance-drums/src/kit_loader.rs` into
      `kit_loader/{manifest,decode,fallback}.rs`.
    - [x] Split `plugins/resonance-wavetable/src/params.rs` into
      `params/{osc,env,lfo,filter,unison,mod_slot,fx,modulation}.rs`.
    - [x] Split `plugins/resonance-wavetable/src/editor/mod.rs` into
      `editor/{factory,app,chrome}.rs`.
    - [x] Split `plugins/resonance-reverb/src/dsp.rs` into
      `dsp/{diffusion,er,fdn,modulation}.rs`.
    - [x] Split `plugins/resonance-amp/src/nam/wavenet.rs` into
      `nam/wavenet/{ring,conv_layer,head,mod}.rs`.
    - [x] Split `plugins/resonance-eq/src/editor/mod.rs` and
      `plugins/resonance-compressor/src/editor/mod.rs` into
      `editor/{factory,app,control_strip}.rs`.
    - [x] Promote `experiments/svs-poc` out of `experiments/` (it's a direct
      runtime dep of `resonance-app::compose::vocal_svs`, so the
      "isolated PoC" description is false). Rename to `resonance-svs`,
      tighten the `pub` surface, move the two `bin/` targets to `examples/`.
    - [x] Tighten `view/mod.rs`: `view_midi_editor_panel` + `classify_editor_variant`
      moved into `view/editor_panel.rs`; `view_timeline` moved into
      `view/timeline_panel.rs`.
    - [x] Tighten `view/compose/mod.rs`: `view_compose` body moved into
      `view/compose/page.rs`; `group_header` moved into `view/compose/group_header.rs`.
    - [ ] `resonance-app` 9 remaining inline tests (in `main.rs` binary crate)
      need a library target before they can become integration tests. Either
      promote `Resonance` + helpers to a `resonance-app` library crate, or
      accept the inline tests as a documented exception.
    - [ ] `resonance-audio/src/recording.rs` inline test exposes private
      `RecordingState`/`TrackRecordingBuf` types; would need to expand public
      surface to migrate.

###P3 — library / idiom

###P3 — library / idiom

    - [x] Extract `AmpProcessor` from `plugins/resonance-amp/src/lib.rs` into
      `dsp/processor.rs`; `ResonancePlugin::process` is now a thin wrapper around
      `AmpProcessor::process_block(...) -> BlockPeaks`.
    - [x] Lift duplicated `load_preset` into `resonance-plugin::presets::load(...)`
      (eq, compressor, reverb, delay, wavetable now all call the shared helper).
    - [x] Replace hand-rolled `Display` impls in `resonance-music-theory/src/derive/{vocal,bass,melody}`
      with `strum::Display + strum::EnumString + strum::IntoStaticStr`.
    - [x] Extract `display_pick!(NewName, Inner, accessor, options_fn)` macro for the
      verbatim `OnceLock`-as-cache pick_list patterns in
      `resonance-app/src/view/compose/lane_inspector/vocal/common.rs`.
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
    - [x] Replace `canonical_phoneme` 48-line match in `resonance-music-theory/src/g2p.rs`
      with `phf::phf_map!` lookup (`ARPABET_INVENTORY`).
    - [ ] Compile-time CMU dict via `phf_codegen`. **Deferred**: 135 k entries
      blow up build time (phf perfect-hash search is exponentially slower past
      ~100 k keys). Runtime parse is ~50 ms once via `OnceLock` — not load-bearing.
    - [x] Swap `HashMap` for `BTreeMap`/`indexmap` in seeded-RNG paths
      (`generate_lyrics`, `MarkovTable::degrees`) for determinism.
    - [x] Changed `OutputPortSpec.name` from `String` to `Cow<'static, str>` so
      plugin `output_layout()` doesn't allocate for static names.

###P4 — dependencies

    - [x] Promote `iced 0.13`, `hound`, `rand`, `egui_glow` to `[workspace.dependencies]`
      and replace inline pins with `{ workspace = true }`.
    - [x] Replace `serde_yaml` (deprecated) with `serde_yml` (resonance-svs).
    - [x] Remove unused deps: `url` (resonance-amp), `libspa` (resonance-audio),
      `parking_lot` (resonance-delay; wavetable already clean).
    - [ ] Major-version upgrades: `ureq 2 → 3`, `rand 0.8 → 0.10`, `dirs 5 → 6`,
      `iced 0.13 → 0.14`, `cpal 0.15 → 0.17`, `ringbuf 0.4 → 0.5`, `symphonia 0.5 → 0.6`.
    - [x] Add `codegen-units = 1` to `[profile.release]` for CLAP dylibs; added
      `[profile.dev.package.resonance-dsp] opt-level = 2` for debug-build audio perf.

###P5 — smaller wins

    - [ ] Replace `super::super::chord::Chord` at
      `resonance-music-theory/src/derive/vocal.rs:482` with `crate::chord::Chord`.
    - [ ] Build `HashMap<PluginInstanceId, PluginLocator>` side-index in `Resonance` to replace
      the linear scan in `with_plugin_mut` (`resonance-app/src/main.rs:297-330`).
    - [ ] Group `vocal_audio_clips`/`vocal_clip_lyrics`/`vocal_render_epoch` into a
      `VocalAudioRegistry` struct in `resonance-app/src/compose/state.rs:68-112`.
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
