#TODO

    - [x] Drum CLAP plugin: implement the new design from the design bundle
      (https://api.anthropic.com/v1/design/h/FYbDo_boPFREv9MfhjU0eg?open_file=Resonance+Drums.html).
      Fetch the bundle, read its README for context + intent, and land the
      relevant aspects of the design in `plugins/resonance-drums/src/editor/**`
      (and `plugins/resonance-drums/src/dsp/**` if the design implies DSP
      changes). Route the visual work through `ux-design` (theme constants,
      spacing, hover/active states) and have `e2e-tester` rebaseline / add
      `iced_test` snapshot coverage for the drum plugin editor once the
      visuals are final.

      **Also fix:** drum outputs don't sound when a non-built-in kit is
      selected. The built-in kit plays correctly, but switching to a
      different kit produces silence. This used to work — almost certainly
      a regression from the recent sub-track refactor (engine fan-out via
      `output_port_index`, parent → sub-track routing). Audit the kit-load
      path end-to-end: kit selection in the editor → sample-pack load →
      per-pad sample assignment → engine sub-track output routing. The
      `plugins/resonance-drums/src/dsp/sampler.rs` per-pad output and the
      engine's sub-track fan-out (how the drum plugin's N output ports
      get mapped to the parent track's sub-tracks) are the likely
      suspects. Add a regression test that loads a non-built-in kit and
      asserts a sub-track produces non-silent output for a triggered pad.

      _Resolved 2026-05-20_. Root cause was **not** the sub-track refactor
      but a regression in `resonance-common/src/wav.rs`: the symphonia
      decode loop called `decoded.copy_to_vec_interleaved(&mut samples)`,
      which **resizes** the destination to the current packet's sample
      count rather than appending. Multi-packet WAVs (Drummica's 24-bit
      44.1 kHz samples decode in ~256-frame packets) silently kept only
      the last packet's audio — a few hundred frames out of hundreds of
      thousands. Built-in fallback samples are tiny single-packet WAVs
      so they survived; "non-built-in" Drummica samples became
      sub-millisecond clicks. Fix: decode into a per-packet scratch and
      `extend_from_slice` into `samples`. Regression tests:
      `resonance-common/tests/wav.rs::decode_long_wav_keeps_all_packets`
      (1 s of synthetic audio in multi-packet form) and
      `plugins/resonance-drums/tests/drummica_routing.rs` (loads Drummica,
      asserts every pad class produces non-silent audio on the expected
      output port, including the cymbal → Overhead-only route).

      Design bundle URL returned 404 on every probed path so the editor
      redesign turned into a focused polish pass: typography/spacing
      tokens in `plugins/resonance-drums/src/editor/theme.rs`,
      `section_label`/`hint_text` helpers used consistently across the
      pad inspector, installed-kit + overhead combo collapsed onto one
      row to claw back vertical space, selected pad now reads with
      accent-colored bold text. No iced surfaces touched — the drum
      editor is egui-only, so no `iced_test` snapshots need rebaselining.

    - [ ] Wrap the lane_inspector body in `iced::widget::lazy` fingerprinted on
      `(selected_lane, definition.id, version_counter)`. **Blocked**: iced 0.14
      `lazy` still requires `View: Into<Element<'static, ...>>` (re-verified
      after the 0.13 → 0.14 upgrade). The vocal lyrics block uses
      `text_editor(&Content)` whose widget keeps a borrow alive, so the
      element isn't `'static`. Fixing requires moving `Content` behind `Rc<RefCell<>>`
      (behaviour change) or splitting the inspector so the text_editor lives
      outside the lazy boundary.
    - [ ] Compile-time CMU dict via `phf_codegen`. **Deferred**: 135 k entries
      blow up build time (phf perfect-hash search is exponentially slower past
      ~100 k keys). Runtime parse is ~50 ms once via `OnceLock` — not load-bearing.

## Code review 2026-05-19 — High (deferred refactors)

These were flagged High by the review but are bigger refactors than the
inline fix pass tackled. Each is documented enough to pick up later.

### resonance-audio — engine

- [x] `engine/bounce/render.rs:78-82,169,236,278,356,401,428` — bounce thread
  takes blocking plugin locks per chunk, glitching live playback. Use
  `try_lock` + retry, or freeze a snapshot.
- [x] `types/track.rs:57` — `plugin_ids: Vec<PluginInstanceId>` mutated under
  tracks write lock while audio reads. ArcSwap the chain.

## Code review 2026-05-19 — Medium

### resonance-audio
- [x] `engine/bounce/render.rs:78-82,169,236,278,356,401,428` — bounce thread takes
  blocking `mutex.lock()` per plugin per chunk, glitching live playback. Use
  `try_lock` + retry, or freeze a snapshot.
- [x] `engine/mod.rs:435-437` — `AudioEngine::send` silently drops commands.
  Return `Result` or emit an `EngineDisconnected` event.
- [x] `types/track.rs:57` — `plugin_ids: Vec<PluginInstanceId>` mutated under
  tracks write lock while audio thread reads. ArcSwap the chain.
- [x] `engine/clips.rs:67,184` — `compute_waveform_peaks` blocks engine thread
  while pushing into `clips_arc.write()`.
- [x] `engine/midi/clips.rs:127-164` — `MidiClipMoved`/`MidiClipTrimmed` events
  fire even when the clip wasn't found. Move event emission inside the `Some`
  branch (already correct for audio clips).
- [ ] `platform.rs:359-368,495-500` — env-var manipulation racy with anything
  else that reads env mid-stream-build. Cpal-workaround; long-term needs a cpal
  API change.

### resonance-app
- [x] `update.rs:106-208` — `update()` mixes undo bookkeeping + gate logic +
  dispatch. Extract `record_undo` / `gates_message` helpers in `undo.rs` so
  `update()` reads as a 10-line orchestrator.

  _Resolved 2026-05-27_. `is_gated_message` and `bounce_blocks_message`
  moved verbatim into `resonance-app/src/undo.rs` as private free fns,
  combined behind a `Resonance::gates_message(&Message) -> bool` method.
  The classify + dirty-flag + record/begin block became
  `Resonance::record_undo(&Message) -> bool` (returning the
  `commit_after` flag). `update()` is now ~10 lines: gate check,
  Undo/Redo meta-shortcut, `record_undo`, dispatch, conditional commit.
  No behaviour change — the existing 6 undo unit tests still pass.
- [x] `state.rs:676-690` — `sorted_tracks()` / `sorted_busses()` allocate a Vec
  per call despite the invariant. Return `&[T]`.

  _Resolved 2026-05-27_. `TrackRegistry::sorted_tracks` / `sorted_busses`
  now return `&[TrackState]` / `&[BusState]`, borrowing the backing
  `Vec` (which is already kept sorted by `.order` — every mutation
  pushes monotonic `next_track_order` / `next_bus_order`, and the
  two replay paths call `resort_tracks` / `resort_busses` after their
  load loops, so the `debug_assert!` in the getter is preserved as the
  invariant guard). The `Resonance::sorted_tracks` / `sorted_busses`
  thin wrappers in `lib.rs` were updated to match. Call-site fixes:
  `view/mixer/mod.rs` dropped `.iter().copied()` (now `.iter()` already
  yields `&TrackState`) and switched `for track in &sorted_tracks` to
  `for track in sorted_tracks` so `IntoIterator` resolves on the slice;
  `view/track_header.rs` swapped `.into_iter()` for `.iter()`; the
  `mixer_sub_track_grouping` integration test got the same `.copied()`
  removal + `for t in &sorted` → `for t in sorted` fix. No behaviour
  change — 38 unit tests + the existing mixer/timeline snapshot tests
  still pass.
- [x] `update/project_io/replay.rs:387-409` — plugin slot reconstruction races
  `PluginAdded` events. Defensive sort by saved order after all replays complete.

  _Resolved 2026-05-28_. `replay_loaded_project` now stashes the saved
  plugin-slot order per track / bus + the master chain into three
  scratch collections before any `replay_*` runs, then re-applies that
  order to `track.plugins` / `bus.plugins` / `r.master_plugins` at the
  very end of replay (just before `rebuild_plugin_index`). A new private
  helper `sort_plugins_by_saved_order` does a stable
  `sort_by_key(position)` — slots whose `instance_id` isn't in the saved
  list get `usize::MAX` and sink to the end (preserving the race-arrival
  order of any late `PluginAdded` events), and saved ids that never
  produced a slot (plugin failed to load → filtered placeholder) are
  silently ignored. Six inline unit tests cover the helper: scrambled
  chain restored, already-sorted no-op, late appends sink to end,
  missing saved ids tolerated, empty saved list no-op, single-slot
  no-op. No behaviour change for chains that already arrived in order
  (Rust's adaptive sort is O(n) on a sorted slice).
- [x] `update/clips.rs:316-318` — `samples_per_tick` ignores tempo-map variation
  when trimming MIDI clips. Use `tempo_map.tick_to_abs_sample`.
  _Resolved 2026-05-28_ — `update_midi_clip_trim` now projects the
  right-edge sample via `TempoMap::tick_to_abs_sample` and converts
  snapped sample deltas back to tick deltas via `sample_to_abs_tick`
  (mirroring the audio-clip path). Left-edge `new_start_sample` is
  re-projected through the tempo map after the trim_start clamp so
  it stays consistent with the engine's playback projection (see
  `engine/midi/outbound.rs:180`). Covered by
  `tests/midi_clip_trim_tempo.rs` (ramp + flat-tempo cases).
- [x] `timeline_draw.rs:219-244` — `draw_global_tracks` rasterises a trapezoidal
  fill as hundreds of `fill_rectangle` calls. Use a single `canvas::Path`.
  _Resolved 2026-05-28_. The trapezoidal fill (and the 2 px polyline on
  top) was the tempo-lane area-under-graph in `draw_global_tracks`, at
  the current `timeline_draw.rs:288-339`. Each pair of tempo events
  spawned up to 400 `fill_rectangle` calls for the fill plus 800 for
  the line, and the right-edge extension added two more. Replaced with
  a single polyline: collect every visible tempo `Point` (plus the
  horizontal extension to the right edge when the last point sits
  inside the canvas), build one `canvas::Path` that traces the polyline
  then closes along `graph_bot` back to the starting x, and `frame.fill`
  it once with the existing `fill_color`. The line itself is a second
  `canvas::Path` of the same polyline, `frame.stroke`d at 2 px with the
  existing `line_color` and `LineJoin::Round` (matches the visual of
  the overlapping 1 px rect stack at sharp corners). No theme constants
  or colors changed. Five timeline goldens were rebaselined under
  `tests/snapshots/` because the path-rasterised fill rounds pixel
  coverage slightly differently than the 1 px rect stack — that's
  expected drift, the tempo line/fill remain in the same position,
  height, and color (verified by re-reading the regenerated PNGs).
  Snapshots rebaselined: `global_tracks_shelf_expanded`,
  `global_tracks_shelf_after_update_tempo`,
  `global_tracks_shelf_after_cycle_signature`,
  `timeline_lane_clip_globals_expanded_scrolled`,
  `track_header_no_bleed_into_chrome_expanded`. Full
  `cargo check`/`cargo clippy -p resonance-app --no-deps`/`cargo test
  -p resonance-app --no-fail-fast` pass.
- [ ] `view/mixer/mod.rs:23-107` — `view_mixer` builds two scrollable rows of
  strips with no row-level virtualization. Wrap in `lazy` keyed on track ids.
  **Blocked** (2026-06-09): same iced 0.14 constraint as the lane_inspector
  item above — `lazy` requires `View: Into<Element<'static, ...>>` and a
  `'static` closure, but every strip is built by `&self` methods
  (`view_channel_strip` / `view_bus_strip`) whose elements borrow
  `self.view_caches` pick_list options and per-track state. Worse, a
  track-ids-only fingerprint is *wrong* here, not just hard: each strip
  embeds a `canvas::Cache`-backed `StereoMeterCanvas` (via `fader_section`
  / `meter_v`, `view/controls.rs`) driven by `track.level_l/_r`, which
  update at peak-snapshot rate — a lazy region whose hash omits that live
  data freezes the VU meters (the exact failure mode called out in
  `.claude/agents/ux-design.md` §Performance), and folding the levels
  into the key would rebuild every strip on every snapshot, defeating
  virtualization. Unblocking needs either iced relaxing the `'static`
  bound, or splitting meters out of the strips so the static chrome can
  be keyed separately. The strips already follow the cheap-rebuild rules
  (canvas meters, `Rc` pick_list caches from `UiViewCaches`).
- [x] `view/compose/page.rs:104-113` — `track_count` computed via
  `iter().filter().count()` every frame. Cache on `ComposeState`.

  _Resolved 2026-06-09_. New `ComposeState.track_count` field + a
  `refresh_track_count(&[TrackState])` method that re-runs the old
  filter (top-level Instrument/Vocal, `sub_track.is_none()`). Refresh
  sites: the three engine track-add handlers + `removed` in
  `engine_events/tracks.rs`, the full replay path (`replay.rs`, right
  after `resort_tracks`), the diff-replay fast path (`replay_diff.rs`),
  and all four demo seed fixtures (which bypass engine events).
  Sub-track creation in `engine_events/plugins.rs` deliberately does
  not refresh — sub-tracks always carry `sub_track` and can never
  change the count. `view_compose` now reads the cached field.
  Covered by `tests/compose_track_count.rs` (demo seed = 5, drum
  sub-tracks excluded = 4, 7 synth tracks = 7, each cross-checked
  against the original filter).
- [ ] `main.rs:169-194` — `parse_startup_tab()` / `parse_demo_flag()` iterate
  `std::env::args()` twice. Parse once.
- [ ] `engine_events/mod.rs:21` — `impl Resonance` god-object. ARCHITECTURE.md
  marks it as a historical exception; convert to a free
  `pub fn handle_engine_event(r, ev)` in `engine_events/dispatch.rs`.
- [ ] `recent.rs:54-57` — re-runs `e.path.exists()` for every entry on load.
  Slow on NFS/removable media. Defer to user click on "Open".

### resonance-music-theory
- [ ] `generator/markov.rs:186` — `history.remove(0)` inside fill loop is O(n²).
  Use `VecDeque` or sliding-window index.
- [ ] `generator/markov.rs:244` — order-2 back-off iterates `&table.transitions`
  per slot. Cache per `suffix`.
- [ ] `derive/motif_engine/build.rs:44-46` — pattern-index `ceil` should be
  `floor` so low `complexity` actually produces simple patterns.
- [ ] `derive/motif_engine/phrase.rs:289-296` — consequent resolves to "lowest
  chord tone" not "root". Use `nearest_midi_to(chord.root, last.note)`.
- [ ] `derive/motif_engine/harmony.rs:71` — `apply_gap_fill` uses `Vec::insert`
  in a loop. O(n²) but ~16 notes per phrase; comment the assumption.
- [ ] `derive/motif_engine/build.rs:130-131` — `snap_to_chord_interval` does
  `i8` subtraction that could overflow if `chord_intervals` ever returned a
  value outside 0..12. Use `i16` defensively.

### resonance-metering / resonance-dsp / resonance-common
- [ ] `resonance-metering/src/spectrum/ring.rs:78-88` — `push_slice` issues N
  atomic RMW pairs per sample. Add a "reserve + bulk write + commit one Release"
  variant.
- [ ] `resonance-metering/src/spectrum/fft_worker.rs:65-70` — fixed-size buffers
  stored as `Vec<f32>`. Use `Box<[f32; FFT_SIZE]>` for unambiguous BCE.
- [ ] `resonance-metering/src/spectrum/fft_worker.rs:81` — worker uses
  `park_timeout(16ms)`; push side never `unpark()`s. Either unpark on
  HOP_SIZE crossings or document the latency.
- [ ] `resonance-metering/src/true_peak/polyphase.rs:64-74` — 48 modulos per
  input sample. Use linear history.
- [ ] `resonance-metering/src/lufs/integrated.rs:55-60` — `debug_assert!(false,
  …)` on 60-min cap. Replace with `log::warn!`; long sessions are not bugs.
- [ ] `resonance-metering/src/crest.rs:66` — full linear scan of 100 ms ring on
  every `crest_db()`. Either monotonic-deque or document UI-rate readout only.
- [ ] `resonance-dsp/src/delay.rs:23-26` — `tap()` silently aliases when
  `delay > mask`. Add `debug_assert!(delay <= self.mask)`.
- [ ] `resonance-dsp/src/lfo.rs:73-86` — `LFO::next` accepts non-finite rate
  and poisons output. Validate in `new`/`set_rate`.
- [ ] `resonance-dsp/src/biquad.rs:189-193` — `clamp_params` can panic if
  `sr == 0`. `let nyquist = (sr * 0.5).max(20.0);`.
- [ ] `resonance-dsp/src/dynamics.rs:60-67` — `Ballistics::from_times` divides
  by potential 0 sample rate. Add `sample_rate.max(1.0)`.
- [ ] `resonance-common/src/registry.rs:80-93` — `is_installed` re-reads JSON
  per call. Provide batched API.

### plugins/*
- [ ] `resonance-mastering/src/stages/linear_phase_eq/convolver.rs:73-74,109-114,
  132-137` — uses `VecDeque::push_back`/`pop_front` per sample. Capacities are
  pre-reserved so no reallocation, but the deque indirection adds per-sample
  overhead vs. a flat ring with two indices. Worth profiling.
- [ ] `resonance-mastering/src/stages/multiband/mod.rs:101-117` — silently caps
  `frames` if the host exceeds the construction-time `max_buffer`. Reallocate
  scratch on `initialize` (off the audio thread) or assert.
- [ ] `resonance-drums/src/dsp/sampler.rs:282-288` — per-pad `volume`, `pan`,
  `oh_blend`, `balance` are snapshotted once per block but applied per sample
  without smoothing. Pad volume jumps still click on automation; master volume
  is now block-linear-ramped, but the pad params still aren't. Add per-pad
  `prev_*` state and interpolate across the block (NUM_PADS × 4 floats).
- [ ] `resonance-mastering/src/assistant/mod.rs` (150 lines) — `Assistant` type
  + impl in `mod.rs`. Move to `assistant/state.rs`.
- [ ] `resonance-wavetable` — every DSP file lives at crate root
  (`effects.rs`, `engine.rs`, …). Fold under `dsp/`.
- [ ] `resonance-ir/src/convolver.rs` — should be `dsp.rs` (per plugin layout
  convention). Also restructure to consume slices instead of per-sample dispatch
  from `lib.rs:221-271`.
- [ ] `resonance-eq/src/band.rs:151-177` — 24/48 dB cuts use Q=0.707 cascades,
  sagging at cutoff vs. true Butterworth. Use per-stage Q tables.
- [ ] `resonance-eq/src/dsp.rs:84-87` — `db_to_linear(output_gain.next())` per
  sample. Smooth in linear-gain space.

### resonance-plugin / wayland-plugin-gui / resonance-svs
- [ ] `resonance-plugin/src/clap_bridge/process.rs:212-217` — manual
  `MaybeUninit` slice reinterpretation; also silently truncates >8 output ports.
  Use `MaybeUninit::slice_assume_init_mut` and `debug_assert!`.
- [ ] `resonance-plugin/src/clap_bridge/process.rs:71-74` — automation
  param-change events bypass any smoother. Document that plugins must drive
  `smoother.set_target` from `set_plain`.
- [ ] `resonance-plugin/src/clap_bridge/state.rs:80-86` — state load races
  in-flight process-block param events. Verify against CLAP threading rules.
- [ ] `resonance-plugin/src/clap_bridge/ports.rs:67-72` — port-name copy
  includes trailing NUL. Check `AudioPortInfoWriter::set` semantics.
- [ ] `wayland-plugin-gui/src/editor.rs:92-100` — `set_size` never updates
  `self.size`. Subsequent `get_size` returns stale data.
- [ ] `wayland-plugin-gui/src/window_thread/paint.rs:80-93` — EGL sized as
  integer `scale`, viewport as float `pixels_per_point`. Either clamp or wire
  `wp-fractional-scale-v1` through.
- [ ] `resonance-svs/src/audio.rs:29` — negative segment offsets silently
  clamped to 0. Either trim leading samples or document.
- [ ] `resonance-svs/src/stages/vocoder.rs:61-63` — `mel.data.clone()` + f0
  collect per segment. Take by value with `mem::take` like the acoustic stage.

## Code review 2026-05-19 — Low

### resonance-audio
- [ ] `types/clip.rs:252-262` — `pre_touch` reads one byte per 4 KiB page; on
  THP-backed mmaps step by the huge-page size.

### resonance-app
- [ ] `main.rs:300,313,350` — `debug_assert!(result.is_some(), …)` after
  `with_track_mut`. If truly an invariant, use `.expect()` so release fails too.

### resonance-music-theory
- [ ] `fretboard.rs:81` — search caps at `start..=7u8`; voicings above fret 11
  unreachable for barre-chord variations in the upper register.
- [ ] `fretboard.rs:128-129` — `start_fret <= 1 → 0` collapses fret-1 voicings
  to "open" display. Document or distinguish.
- [ ] `derive/vocal/lyrics.rs:178-184` — locked-line rhyme recovery uses
  exact-text match; editing the locked line silently falls back to slot pattern.
- [ ] `chord.rs:113-119` — `Chord::pitch_classes` allocates per call. Return
  `impl Iterator` or `SmallVec`.
- [ ] `derive/motif_bass.rs:283-296` — `chord_tones_in_register` is
  O(register-span × |pcs|). Use a `[bool; 12]` PC bitmap.
- [ ] `rng.rs:36-41` — `next_range` has modulo bias; undetectable in practice
  for n ≤ 256 but worth rejection-sampling if "uniform" is ever claimed.
- [ ] `generator/markov.rs:11-12` — crate uses both `rand::SmallRng` and the
  custom `XorShift`. Consolidate to one determinism contract.
- [ ] `lib.rs` re-exports — `VocalSinger`, `VocalVoicebank`, `g2p.rs` etc. are
  SVS configuration, not music theory. Consider splitting into a
  `resonance-vocal` crate.

### resonance-metering
- [ ] `correlation.rs:51-73,72-82` — `samples_pushed` unused; first 99 ms of
  output is biased toward 0 from zero-history. Gate readout until full window.
- [ ] `crest.rs:55` — same unused-`samples_pushed` field.
- [ ] `lufs/mod.rs:126-128` — `_LOUDNESS_OFFSET_RE_EXPORT` dead workaround.
- [ ] `k_weighting.rs:84,105` — `assign_prefilter` / `assign_rlb` reach into
  `Biquad`'s `pub` fields. Add an `assign_raw` constructor.
- [ ] `spectrum/octave.rs:60-63` — band-fallback path uses `f32::max` (NaN
  propagating), other branches use manual `if v > peak`. Unify.

### resonance-common
- [ ] `wav.rs:198,221` — `target_len = (input.len() / ratio) as usize`
  truncates to zero when `input.len() < ratio`. Clamp to 1 when input non-empty.
- [ ] `scan.rs:2` — `scan_directory` swallows `read_dir` errors. Return
  `io::Result` or log via `tracing`.
- [ ] `registry.rs:117` — hand-rolled `today_iso` date math. Pull `chrono` from
  another transitive dep instead.

### resonance-dsp
- [ ] `delay.rs:29-35` — `tap_linear` casts negative input to `usize` (saturates
  to 0). `debug_assert!(delay_frac.is_finite() && delay_frac >= 0.0)`.
- [ ] `filter.rs:18` — `OnePole::set_cutoff` doesn't clamp against
  `sample_rate/2`. Document the near-identity passthrough at Nyquist.
- [ ] `rng.rs:13-18` — xorshift32 with seed=0 relies on `| 1` recovery.
  Assert non-zero in debug.

### plugins/*
- [ ] `resonance-wavetable/src/viz.rs:218-229` — `ScopeCollector::publish`
  builds a fresh 2 KB stack buffer + double-copy per block.
- [ ] `resonance-wavetable/src/render.rs:218-229` — inner sample loop iterates
  voices even with both oscs disabled. Early-skip.
- [ ] `resonance-reverb/src/viz.rs:51-55` — `TailHistory::push` stores
  `write_pos` with `Relaxed`; other plugins use `Release`/`Acquire`. Inconsistent.
- [ ] `resonance-compressor/src/viz.rs:50-54` — same Relaxed-store pattern.
- [ ] `resonance-delay/src/dsp.rs:103-105` — LFO uses `(self.lfo_phase *
  TAU).sin()` per sample. Acceptable; polynomial sine if ever profiled hot.
- [ ] `resonance-delay/src/lib.rs:170-185` — `fb.powf(n_taps)` per block for 8
  taps; replace with `fb * fb` accumulation.

### resonance-plugin / wayland-plugin-gui / resonance-svs
- [ ] `resonance-plugin/src/param.rs:347-378` — `TempParamOwned` only used by
  `clap_bridge::state`. Move under `clap_bridge`.
- [ ] `resonance-plugin/src/clap_bridge/gui.rs:57-61` — `set_scale` silently
  ignores the host's scale hint.
- [ ] `wayland-plugin-gui/src/widgets.rs:218-233` — `draw_arc` allocates a
  `Vec<Pos2>` of 49 points per arc per frame.
- [ ] `wayland-plugin-gui/src/window_thread/event_loop.rs:176-202` — main loop
  uses a fixed 16 ms tick; doesn't pace via `wl_surface.frame()` callbacks.
- [ ] `resonance-svs/src/lib.rs:8` — `write_mono_f32_wav` exported but
  `mix_into_timeline` is crate-private. Either expose both or rename.
- [ ] `resonance-svs/src/ds.rs:227` — note-name regex compiled per call. Use
  `OnceLock<Regex>`.

