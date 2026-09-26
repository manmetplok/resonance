# ARCH-01 / ARCH-02 / ARCH-03 — incremental implementation plan

Read-only architecture pass, 2026-09-26, against master `bb45ccad` (F1, G3 and D
merged while this was written; the review text is from `09041eee`). Every claim
below was re-checked against the current tree; where the finding is out of date
it says so.

Conventions used in each step: **Goal · Files/symbols · Approach · Test · Diff ·
Conflict risk · Behaviour change**. "NOW" = land in this fix campaign; "EPIC" =
file as a ba epic and drain it after the campaign.

---

## Cross-cutting facts that shape all three plans

* Master moves fast: 128 of the last 300 commits touch `message.rs`, 85 touch
  `lib.rs`. Unmerged branches that still touch the hub files: `ba/epic-14`,
  `ba/epic-40`, `ba/epic-202`, `ba/epic-33`, `ba/todo-1051`, `ba/designer-wip`
  (stranded QA-split branches — see memory; they are not being merged soon, so
  they bound conflict risk only for whoever eventually rebases them).
* The active fix batches still open at the time of writing: M1 (plugin
  framework), M2 (DSP), M3 (just started). None of them touch
  `resonance-audio/tests/`, `mixer/callback`, `engine/bounce`, `message.rs` or
  `undo/snapshot.rs` per `git diff --name-status master...fix/review-M{1,2}`.
  F1 (which did touch callback/transport/bounce) is merged (`2c811666`).
* Each `resonance-audio` test binary is ~116–125 MB on disk; the app's group
  binaries are 700–940 MB; `target/debug/deps` is 510 GB (seven stale hash
  variants of every binary are lying around — unrelated, but `cargo clean -p`
  of the big crates would recover hundreds of GB).

---

## ARCH-03 — test-binary sprawl + `#[doc(hidden)]` engine internals

### Evidence, re-verified

| Crate | test files | LOC | per-binary size | `[[test]]` groups |
|---|---|---|---|---|
| `resonance-audio` | **124** (was 121 at review; F1 added `offline_render_gate`, `playhead_seek_race`, …) + `multi_out_harness/` | 31k | 116–125 MB | 0 |
| `plugins/resonance-mastering` | 22 | 4.5k | **~374 MB** | 0 |
| `plugins/resonance-granular-delay` | 23 | 6.8k | **~351 MB** | 0 |
| `plugins/resonance-amp` | 30 (+`common/`, `fixtures/`, `golden/`) | 9.4k | ~196 MB | 0 |
| `resonance-music-theory` | 41 | 11.6k | ~18 MB | 0 |
| `resonance-plugin` | 21 | 7.9k | — | 0 |
| `plugins/resonance-drums` | 19 | 4.9k | — | 0 |

The three plugins are big because `default = ["editor"]` links egui + the
Wayland runtime into every test binary; that is a separate lever (ARCH-08) and
is not needed for the win here.

New audio test files land at 6–52 per month (52 in June, 27 in August), so the
count only goes up until the group shape exists and `run-tests.py`/CLAUDE.md
say "add a module, not a file" for this crate too.

**Measured (this machine, 16 cores, load avg ~15 from other agents' builds —
treat as an upper bound):**

* no-op `cargo test -p resonance-audio --no-run` on a fresh target: 0.6 s
* rebuild of one test target after deleting its executable (codegen + link of
  the test crate only; the lib is fresh; default GNU ld, `target-cpu=native`):
  **0.31 s** (`solo_predicate`, 44 LOC)
* rebuild of all 122 test targets after deleting their executables (this is
  exactly the "one-line change to `resonance-audio/src`" cost minus the lib
  compile): **8.4 s wall**, load avg 34→40 during the run — i.e. roughly
  122 × ~1 s CPU spread over 16 cores. 122 × ~120 MB = **~14.6 GB written**
  per rebuild.

Reference point from the app's consolidation commit `21ea8f0d` (the same
operation): 241 → 13 binaries took a one-line-change relink from 193 s to 8 s
(the lib-only floor); lld did not help (205 s) because the cost is rustc
codegen per target, not the linker.

**So the audio crate is not the app.** Its per-target cost is ~0.3–1 s
(no iced, no app monomorphisation), and the grouping brings ~8.4 s down to an
estimated **1.5–3 s** (10 targets of 2–9k test LOC each, compiled in
parallel — the per-group cost is dominated by the test code, not the link).
The finding's "link time dominates the audio suite" is not borne out on this
machine: the win is ~6 s per engine change, plus ~13 GB less disk churn per
rebuild (relevant: `target/debug/deps` is 510 GB and holds seven stale hash
variants of every audio test binary, ~100 GB for this crate's tests alone),
plus 112 fewer processes for `run-tests.py` to spawn. That is still worth a
mechanical commit — and it is the prerequisite for stopping the sprawl — but
it is a housekeeping win, not the 25× the app got. The plugin crates with
~350–375 MB binaries (egui + Wayland linked in through `default = ["editor"]`)
are the better per-target targets; see the mastering measurement at the end.

### Process-wide state that needs its own binary (audio)

| File | Why |
|---|---|
| `sidechain_taps.rs` | `#[global_allocator]` counting allocator (only one per binary; changes allocator for every test in the process). |
| `recording_write_failure.rs` | lowers `RLIMIT_FSIZE` for the process (`FSIZE_LOCK` serialises only *its own* tests; any other module writing a file concurrently would hit `EFBIG`); also `#![cfg(target_os = "linux")]`. |
| `engine_send_disconnected.rs` | resets the process-global `ENGINE_DISCONNECT_REPORTED` latch (`engine/mod.rs:479`) and asserts "reported once"; another module exercising `for_test_disconnected` concurrently would flake it. |

Fine to group despite looking special: `pw_output_smoke.rs` (both tests are
`#[ignore]`, so they cost nothing in a group and still run with `-- --ignored`);
`plugin_rescan.rs` / `plugin_bypass.rs` / `clap_*` (they `dlopen` first-party
cdylibs; several already share a process today — nothing is unloaded); the six
files with `static FAKE_*_EXT: clap_plugin_*` (file-local statics, no
cross-talk); `bounce_plugin_lock.rs` (timing test; FU-F2d flakiness is
load-related and unchanged by grouping — if anything it gets *less* parallel
pressure when fewer binaries run at once).

Things that are not a "pure move" and must be handled in the same commit:

1. `mod multi_out_harness;` in four files (`sidechain_key_delivery`,
   `sub_track_parent_fader`, `stem_sub_track_render`, `stem_bus_sub_track_render`)
   resolves against the *child's* directory once the file is a submodule. Same
   fix the app used: the group root owns `#[path = "multi_out_harness/mod.rs"] mod multi_out_harness;`
   and the four children say `use crate::multi_out_harness::{…}` (one-line edit each).
   All four are in the `mixer` group, so it lives there only.
2. `mod sys { … }` in `recording_write_failure.rs` is fine (stays standalone).
3. Duplicate free-function names across files (`make_tempdir` ×8, `tmp_path` ×5,
   `empty_engine_state` ×5, `tone` ×4, …) and 12 duplicate `#[test]` names are
   harmless — each file becomes its own module, names are namespaced. No
   renames needed. (The app commit verified "same 1837 test functions" by
   diffing `--list` output before/after; do the same: `cargo test -p
   resonance-audio -- --list | sort` must be an identical multiset of
   `<module>::<fn>` names modulo the new module prefix.)

### Proposed grouping (7 groups + 3 standalone = **10 binaries**, down from 124)

Mirrors `src/`: `engine/` handlers, `mixer/` render path, `engine/bounce`,
`clap_host/`, `midi_hardware`+`midi_io`+`midi_clock`, `types/`, `io/`+decode.

**mixer** (34 files, 8908 LOC): audition_preview, automation_comp_delay, automation_live_value, automation_render, aux_send_render, clip_fade_gain_render, cycle_load, freeze_playback_substitution, graph_rate_assert, latency_comp, measure_mix, midi_event_cap, midi_event_window, midi_stash, mix_audio_parity, mixer_gain_ramp, monitor_fallback_resample, monitor_ring_alignment, recorded_monitor_gating, recorded_outbound_gating, recording_drain, recording_overflow_event, recording_start_latch, recording_whole_frame_push, reference_monitor, render_block_parity, sidechain_key_delivery, solo_predicate, stem_bus_sub_track_render, stem_render, stem_sub_track_render, sub_track_parent_fader, take_comp_render, underrun_rate_limiter (+ `multi_out_harness/`)

**engine** (23 files, 7240 LOC): automation_handlers, aux_send_cycle, bus_plugin_move, clip_fade_gain_handlers, clip_warp_handlers, deferred_clip_commands, device_params_handler, external_instrument_handlers, external_instrument_ping, external_recorded_playback, freeze_command_plumbing, loop_record_takes, master_plugin_move, midi_bulk_edits, midi_clip_handlers, midi_map_command_plumbing, offline_render_gate, playback_source_handler, playhead_seek_race, reference_handlers, take_removal, track_plugin_chain, track_plugin_move

**clap_host** (15 files, 4094 LOC): clap_all_notes_off, clap_bundle_path, clap_factory_presets, clap_ffi_hardening, clap_latency_tracking, clap_note_event_order, clap_param_flush, clap_param_meta, clap_plugin_drop_order, plugin_bypass, plugin_editor_state, plugin_id_ranges, plugin_load_failure, plugin_output_scrub, plugin_rescan

**bounce** (15 files, 3374 LOC): bounce_external_offsets, bounce_midi_events, bounce_plugin_lock, bounce_render_range_tempo, bounce_tail_and_master_latency, bounce_transport_guard, export_encoders, export_normalize, export_settings, freeze_cache_read, freeze_render_core, midi_export_project, reference_export_exclusion, stem_export, vocal_tuning_bounce

**types** (14 files, 2983 LOC): aux_send_model, bar_length_shared, clip_warp, fade_curve, pw_latency_math, quantize_engine, sample_to_abs_tick, tempo_bar_at_sample_exact, tempo_map, tempo_position_to_bars, tempo_reanchor_math, transport_pos_beats, types_track, vocal_tuning_model

**midi_hw** (12 files, 2105 LOC): control_surface_parse, device_param_automation, live_arrival_offset, live_note_retry_order, midi_clock_parse, midi_hardware_emit, midi_hardware_parse, midi_io, midi_program_change, outbound_note_pairing, outbound_step_start, smf_import

**io** (8 files, 1718 LOC): clip_pitch_analysis, clip_tempo_detect, import_audio_to_pool, load_clip_offthread, load_wav_rate_mismatch, pw_output_smoke, reference_analysis, wav_chunk_parse

**standalone** (3): engine_send_disconnected, recording_write_failure, sidechain_taps

(The mapping is by file name + a skim of each file's imports; the implementer
should re-check any file whose primary subject is ambiguous — e.g.
`latency_comp` tests the `latency` module through the render path, `quantize_engine`
is pure `quantize::` — but a wrong group is a cosmetic mistake, not a broken
test.)

### Steps

**A3-1. Group `resonance-audio/tests/` into 10 binaries.**
Goal: the done-when in the finding (`Executable` count ≤ 10).
Files: `git mv resonance-audio/tests/<f>.rs resonance-audio/tests/<group>/<f>.rs`
for 121 files; new `resonance-audio/tests/{mixer,engine,clap_host,bounce,types,midi_hw,io}.rs`
roots with `#[path = "<group>/<f>.rs"] mod <f>;` lines (exactly `resonance-app/tests/mixer.rs`'s
shape, including the doc comment explaining why `#[path]` is load-bearing); the
four `multi_out_harness` edits; `resonance-audio/Cargo.toml` comment update
(the dev-dependency comments name `tests/import_audio_to_pool.rs` etc. — fix
the paths). Move `multi_out_harness/` to `tests/mixer/multi_out_harness/`.
Approach: script the moves (the grouping list above is machine-readable in
`scratchpad/arch/grouping.md`); do not touch test bodies. Cargo autodiscovers
`tests/*.rs` only at the top level, so subdirectory files stop being targets
with no `autotests = false` needed — same as the app.
Test: `cargo test -p resonance-audio -- --list` before/after → same function
multiset; `./scripts/run-tests.py -p resonance-audio` green; record the
`--no-run` wall clock before/after in the commit message like `21ea8f0d` did.
Diff: ~121 renames + ~150 added lines + 4 one-line edits. Mechanical.
Conflict risk: **medium and time-sensitive** — any branch that *adds* a file to
`resonance-audio/tests/` after this lands re-creates a top-level binary
silently (the merge won't fail). Land it at a quiet point (no open batch
touches the dir right now: M1/M2/M3 don't) and add the rule to CLAUDE.md's
test section in the same commit ("resonance-audio: add a module to
`tests/<group>.rs`, never a top-level file"). Branches with in-flight edits to
existing audio tests (`ba/epic-14`, `ba/epic-40`, `ba/epic-202` — stranded)
will rebase through git rename detection.
Behaviour change: none.

**A3-2. Extend `run-tests.py`/CLAUDE.md guidance + a guard test.**
Goal: keep it consolidated.
Files: `CLAUDE.md` (tests section, one bullet), `scripts/run-tests.py` docstring
("~620" → new count), and a tiny assertion in an existing app group binary
(`resonance-app/tests/control.rs` already has cross-cutting checks; ARCH-10
proposes `tests/architecture.rs` — put it there when ARCH-10 lands, otherwise
a `#[test]` that globs `resonance-audio/tests/*.rs` and asserts the set equals
the 10 known roots).
Diff: ~30 lines. Conflict: none. Behaviour: none.

**A3-3. Same for `resonance-mastering`, `resonance-granular-delay`, `resonance-amp`.**
Goal: the three largest per-binary costs in the workspace (351–374 MB × 22–23).
Grouping: mastering → `stages` (12 `stages_*` + `multiband_band_gr`),
`assistant` (4), `editor_and_params` (`editor_*`, `params_labels`, `dsp_golden`,
`integration`) = 3. granular-delay → `dsp` (align, density_sync, granulate,
quantize, sync_divisions, pitch_sync, shimmer, diffusion, feedback, freeze,
time_modes, stereo, quality, dsp_regression) and `plugin_and_editor`
(plugin, presets, state, viz, viz_snapshot, editor_groups, editor_knobs,
hero_layout) = 2. amp → `nam` (all `nam_*`, `matvec_null`, `dsp_golden`),
`plugin` (state, tuner, tuner_view, editor_param_binding, tone3000_browser)
+ standalone `nam_rt_safety` (`#[global_allocator]`) and `model_selector`
(`env::set_var("XDG_DATA_HOME")` — process-wide) = 4.
Not pure moves: granular-delay has **nine** copies of `#[global_allocator] static ALLOC: CountingAlloc`
(diffusion, feedback, freeze, pitch_sync, plugin, quality, shimmer, stereo,
time_modes) — one binary can hold only one, so hoist it into
`tests/dsp/common/alloc.rs` and have each file `use crate::common::alloc::…`
(~9 × 20-line edits; the allocator semantics are identical since all nine are
the same counting allocator). amp's `mod common;` (5 files) needs the same
`use crate::common` treatment as `multi_out_harness`.
Test: `--list` multiset per crate; golden PNG/WAV tests in `golden/` name their
files crate-relatively — `run-tests.py` already launches from the crate root, so
they are unaffected by the move (verify one golden test per crate).
Diff: ~75 renames + ~120 lines per crate. Conflict: low (M1 touches plugin
*framework*, not these three crates' tests; M2 touches DSP under `plugins/*/src`
— check `git diff --name-only master...fix/review-M2 -- plugins/*/tests` right
before landing). Behaviour: none.
`resonance-music-theory` (18 MB binaries) is not worth it now; do it only if
someone touches that crate's tests anyway (3 groups: `pitch_scale_chord`,
`progression_voicing`, `generators_vocal`).

**A3-4. Stop production app code reaching `__test_support`.**
Goal: `grep -rn '__test_support' resonance-app/src` returns only test-support
lines.
Files: `resonance-app/src/lib.rs:732` and `resonance-app/src/test_support/project.rs:45`
use `resonance_audio::__test_support::Receiver<AudioCommand>` — it is just
`crossbeam_channel::Receiver`, and `resonance-app` already depends on
`crossbeam-channel` (`Cargo.toml:17`). Replace the two type paths with
`crossbeam_channel::Receiver<AudioCommand>`; the 20 app test files that name
the hidden path keep working through `__test_support` until A3-5.
Diff: 2 lines. Conflict: none. Behaviour: none.

**A3-5. Gate the test surface behind a cargo feature and promote what
production needs.**
Goal: production builds of the app cannot see engine internals.
Files: `resonance-audio/Cargo.toml` (`[features] test-internals = []`),
`resonance-audio/src/lib.rs:56-201` (`pub mod __test_support` →
`#[cfg(feature = "test-internals")] pub mod test_support`) and the 24
`#[doc(hidden)] pub use` blocks at `:205-400`; `resonance-app/Cargo.toml`
`[dev-dependencies] resonance-audio = { path = …, features = ["test-internals"] }`
(resolver = "2" keeps dev-dep features out of `cargo build`); every
`resonance_audio::__test_support::` path in `resonance-audio/tests/` and
`resonance-app/tests/` → `resonance_audio::test_support::` (sed).
Promote to real public API instead of gating: the hidden items production app
code uses today — `PoolImportOutcome` (message.rs, 2 files), `import_one_to_pool`
(1), `MIN_CLIP_GAIN_DB`/`MAX_CLIP_GAIN_DB` (1), `ABMeterTap` (1). Everything
else in the hidden blocks (`push_take`, `chunk_span`, `to_freeze_cache_spawn`,
`try_lock_with_backoff`, `SharedState`, `EngineHandlerHarness`, the
`*_in_place` helpers, …) is test-only and moves under the feature.
Test: `cargo build -p resonance-app` (no feature) and `cargo test -p resonance-app`
both compile; `cargo doc -p resonance-audio` shows no `__` items.
Diff: ~60 lines in lib.rs + a sed across ~50 test files + 2 Cargo.toml lines.
Conflict risk: **medium** — `resonance-audio/src/lib.rs` is touched by the
stranded epic branches and by any batch that adds a hidden re-export (F1 did
just that for `OfflineRenderGuard`-adjacent helpers). Land right after A3-1
while the dir is quiet, or fold into A3-1's commit.
Behaviour: none (feature-gated compile surface only).

**A3-6. Make `EngineHandlerHarness` / `MixAudioHarness` the default surface.**
Goal: new tests stop needing new re-exports. Both harnesses already exist
(`engine/thread/test_support.rs`, `mixer/test_support/callback.rs`) and are
already the *documented* way ("widen it a method at a time"); 27 audio test
files still build `Arc<RwLock<…>>` maps by hand. This is a guideline change
(ARCHITECTURE.md *Test Layout* + the harness doc comments), not a code change —
and it becomes mandatory anyway when ARCH-02 changes the map types (see
A2-4: those 27 files are the blast radius).
Diff: docs only. Conflict: none.

### NOW vs EPIC for ARCH-03 (revised after measuring)

The finding bundles two things whose value turned out very different:

* **NOW: A3-4 + A3-5** — the *surface* half. Production app code reaching
  `__test_support` (2 lines) and 24 `#[doc(hidden)]` engine internals being
  public API is the part with real consequences (every private engine
  refactor is a public-API change; the app already depends on one). ~60 lines
  in `resonance-audio/src/lib.rs` + two `Cargo.toml` lines + a sed over
  ~50 test files; no behaviour change; medium conflict risk on `lib.rs`
  re-exports only, and nothing open touches them today.
* **NOW, optional, if a quiet window exists: A3-1 + A3-2** — the grouping.
  Mechanical and zero-risk, but the measured win is ~6 s wall and ~13 GB of
  disk writes per engine change, not minutes. Worth one commit because it
  also stops the count growing (F1 added three files in a week) and matches
  the app's convention, but it should not displace a bug fix. If it lands,
  fold A3-5's `lib.rs` edit into the same window.
* **DEFER: A3-3** (mastering / granular-delay / amp). 21 mastering targets
  rebuild in 1.5 s; the granular-delay grouping needs the nine
  `#[global_allocator]` copies hoisted — a code change for ~1 s of wall
  clock. Do it only if ARCH-08 (drop the editor deps from test builds) does
  not already shrink those binaries, and then as part of that work.
* **EPIC / opportunistic:** music-theory grouping, A3-6 docs.

---

## ARCH-02 — audio callback `try_read` vs control-thread `write()`

### Evidence, re-verified — and one correction to the mechanism

Still true: `engine/mod.rs:583-597` holds `tracks`, `busses`, `master`, `clips`,
`midi_clips` in `Arc<parking_lot::RwLock<…>>` and `plugins` in
`Arc<RwLock<PluginMap>>`; `mixer/callback/play.rs:25-46` `try_read`s five of
them (plus `master` in `mixer/master.rs:55`, and `tracks`+`plugins` in
`live_midi.rs:54`, `count_in.rs:37`, `stopped.rs:24`) and renders silence on
any failure, bumping `shared.render_skip_cycles`. There are now **83**
`.write()` sites under `engine/` (61 at review), of which the five hot maps
account for 16 (`engine/midi/clips.rs`), 12 (`engine/clips.rs`), 11
(`tracks.rs`), 9 (`busses.rs`), 6 (`take_park.rs` doc-contract), 6
(`midi/live.rs`), 2 each in `takes.rs`, `plugins.rs`, `master.rs`,
`bounce_realtime.rs`, `bounce/clip.rs`, `thread/mod.rs`, and 1 each in
`vocal_render.rs`, `vocal_analysis.rs`, `scan.rs`.

**Correction: the writes are short; the long holders are readers on worker
threads.** Reading every hot-map write site: each is "find element, set a few
fields, send an echo" or "insert/remove one element". The heavy work is already
done off-lock (`engine/clips.rs:679-689` moved clip decode to a worker,
`ClipSource::open_wav` for save-as happens before the guard at `:904`, plugin
instantiation happens before `plugins.write().insert` at `plugins.rs:324`,
`ClearAll` drains under the lock and drops outside at `tracks.rs:409-433`).
The only O(n)-under-write sites are `engine/midi/clips.rs` `quantize` /
`humanize` / `groove` / `replace_notes` (`:441-447`, `:495-497`, `:518-520`,
`:477-479`), which compute a new note vector while holding `midi_clips.write()`
— microseconds for realistic clips.

What actually produces a dropout is `parking_lot::RwLock`'s **task-fair
policy** (`parking_lot-0.12.5/src/rwlock.rs:17-19`): "readers trying to
acquire the lock will block even if the lock is unlocked when there are writers
waiting", and `try_read` returns `None` in that state. So the failure needs
*two* parties besides the callback:

1. a **reader holding the lock for a long time on a non-RT thread** — exactly
   what the offline paths do: `engine/bounce/render.rs:305-309` takes `read()`
   on all five maps for one `BOUNCE_CHUNK = 1024`-frame render (`:22`), i.e.
   every plugin's `process()` on every track, per chunk; `bounce/freeze.rs:98,125,341,364`,
   `bounce/stem.rs:453-529`, `bounce/wav.rs:336`, `bounce/clip.rs:77` likewise;
2. **any** engine-thread `write()` on that map arriving during that chunk — it
   queues, and from that instant until the worker's chunk finishes every
   callback `try_read` on that map fails.

So the realistic dropout scenarios are "edit anything while a freeze / bounce /
stem export / `measure_mix` is running" (a bounce-in-place of a heavy track
holds a read guard for several ms per chunk, far above the 2.7 ms quantum) and,
secondarily, the worker-thread *writers* (`vocal_render::ensure_tuning_caches`
runs on the freeze worker holding `clips.write()` for the whole FFT-shift pass
— `vocal_render.rs:71-72` called from `freeze.rs:93`; the clip-load worker's
`clips.write()` at `clips.rs:763`; `vocal_analysis.rs:81`). Undo/load
(`ClearAll` + replay) is a *burst* of short writes, each of which can collide
with the callback's own `try_read` only if the callback and the write are
simultaneous — parking_lot's `try_read` also fails while a writer *holds* the
lock, so a burst of 500 clip inserts costs at most a handful of skipped blocks,
not a dropout per insert. The finding's ranking should be flipped: the offline
read guards are the first target, structural undo second.

`try_lock_with_backoff` (`bounce/render.rs:71`) is about the **per-plugin
instance `Mutex`**, not these maps; it survives every step below (the
callback and the bounce still share plugin instances until MIX-02's exclusivity
gate, which F1 has now landed as `OfflineRenderGuard`). Step 4 of the finding
("delete `try_lock_with_backoff`") is therefore wrong as written; what goes
away is the *map* read guards in `ChunkCtx`.

`SharedState` already publishes `aux_sends`, `sidechain_routes`, `take_comp`,
`reference` via `ArcSwap`, and `Track` is internally RT-safe (atomics for
volume/pan/mute/solo/arm/monitor/output, `ArcSwap` plugin chain,
`ArcSwapOption<String>` input device — `types/track.rs:24-82`), so the
per-element hot path is already lock-free; only the *containers* are locked.

Instrumentation: `render_skip_cycles` is already folded into
`CycleLoadReport.lock_skips_{window,lifetime}` (`cycle_load.rs:159-167`) — the
finding's step 1 is half done; what is missing is *per-map* attribution.

### Target model

```
engine thread (single owner)                   audio thread / bounce worker
────────────────────────────                   ─────────────────────────────
EngineModel {                                  graph: ArcSwap<RenderGraph>
  tracks:     IndexMap<TrackId, Arc<Track>>,     ── load() once per block/chunk;
  busses:     IndexMap<BusId,  Arc<Bus>>,           no lock, never fails
  master:     Arc<MasterBus>,
  clips:      Vec<Arc<AudioClip>>,             RenderGraph = the same six fields,
  midi_clips: Vec<Arc<MidiClip>>,              immutable, built by EngineModel::publish()
  plugins:    IndexMap<PluginInstanceId, Arc<PluginSlot>>,
}
publish(): graph.store(Arc::new(self.snapshot()))   // O(n) Arc clones, one alloc
   └─ old Arc<RenderGraph> → Retired queue on the engine thread,
      dropped at the 16 ms loop tick when Arc::strong_count == 1
      (this is also MIX-04's fix for automation / tempo / latency_comp /
      take_comp / aux_sends / sidechain / frozen_source)
```

* Element mutation on the engine thread: `Arc::make_mut(&mut self.midi_clips[i])`
  (clones only if the audio thread still holds the previous graph) then
  `publish()`. `Track` needs no clone at all for fader/mute/etc. — its atomics
  are mutated through `&Track` as today; only insert/remove republish.
* The audio callback replaces five `try_read`s by one `inputs.graph.load()`;
  `BlockInputs` fields become `&graph.tracks` etc. — `render_core` is unchanged
  because it already takes `&IndexMap<…>`/`&[…]` borrows.
* `HandlerCtx` (`engine/thread/mod.rs:44`) gains `model: &mut EngineModel`
  (or the harness passes it); the five `&Arc<RwLock<…>>` fields go last.
  `EngineHandlerHarness` is the one other construction site
  (`test_support.rs:110`) — good, one place to adapt.
* Worker-thread writers become messages to the engine thread. There is
  already an internal command channel (`cmd_tx_retry`, `HandlerCtx`) and a
  precedent (`DeferredClipCommand`); add an internal `EngineInternal::ClipLoaded(AudioClip)`
  / `TuningCachesBuilt{…}` / `PitchAnalysed{…}` path. The `take_park`
  interlock (`take_park.rs:35-40`: "every mutation while holding
  `ctx.clips.write()`") collapses to plain engine-thread sequencing — the
  park stays (undo needs it) but its lock contract disappears.
* Bounce/freeze/stem: `ChunkCtx` (`bounce/render.rs:143-150`) holds
  `Arc<RenderGraph>` loaded once per chunk (or once per bounce if
  determinism during an offline render is preferred — today's behaviour is
  "sees edits mid-render", keep it per chunk to avoid a behaviour change).
  No read guard → no queued writer → no callback failure.
* `ClipSource::Memory(Vec<f32>)` (`types/clip.rs:111`) would deep-copy on
  `make_mut`; change it to `Arc<[f32]>` (or `Arc<Vec<f32>>`) first so an
  `AudioClip` clone is pointer-sized. `Mapped` already holds `Arc<Mmap>`.
* `MidiClip.notes: Vec<MidiNote>` clones per note edit under `make_mut` —
  O(notes) on the engine thread, acceptable (ARCH-09 step 2 wants
  `Arc<[MidiNote]>` on the app side anyway; do the engine side the same way
  if a profile ever shows it).

### Migration map, ordered by benefit ÷ risk

| Order | Map | Element type today | Engine-thread writers | Worker writers | Offline readers | Why this position |
|---|---|---|---|---|---|---|
| 1 | `midi_clips` | `MidiClip` plain data | 16 (`midi/clips.rs`) + 6 (`midi/live.rs`, live recording) | none | bounce/freeze/stem | Most frequent user edits during playback (note drags, quantize, control-API bulk writes); no cross-thread writers; simple element. |
| 2 | `busses` + `master` | `Bus`, `MasterBus` plain (plugin-id `Vec`s) | 9 + 4 | none | all | Tiny maps, plain types; unblocks removing two `try_read`s. |
| 3 | `tracks` | `Track` (already RT-safe internals) | 11 (insert/remove/clear) | `bounce_realtime.rs:327` (worker removes the target track on cancel) | all | Element needs no COW; only container inserts/removes; one worker writer to route through the engine thread. |
| 4 | `plugins` | `PluginSlot { Mutex<SyncClapInstance>, BypassFade, … }` | 8 | `scan.rs:52` (rescan drains) | all | `Arc<PluginSlot>` shares the instance mutex exactly as today; `Retired` must drop old graphs on the engine thread *after* `Arc::strong_count==1`, which also guarantees `ClapInstance::drop` never runs on the audio thread (a latent MIX-04-class bug today: `plugins.write().shift_remove` → the callback's guard is the last owner only if held across the swap, which `try_read` guards are not — fine today, must stay fine). |
| 5 | `clips` | `AudioClip { ClipSource, … }` | 12 | `clips.rs:223,763` (load worker), `vocal_render.rs:72` (freeze worker), `vocal_analysis.rs:81`, `bounce/clip.rs`, `bounce_realtime.rs:321` | all | Most cross-thread writers and the `take_park` lock contract — do it last, after the internal-message path exists from steps 3–4. |

### Steps

**A2-1. Per-map contention attribution (instrumentation).**
Goal: know which map fails before migrating; keep it after as a regression
guard.
Files: `engine/mod.rs` `SharedState` (`render_skip_cycles` → add
`render_skip_by_map: [AtomicU64; 6]` or six named atomics), `mixer/callback/play.rs:25-46`
(the `let (Some(..), ..) = (..) else` — evaluate the five `try_read`s
individually so the failing one is known; keep the all-or-nothing render
decision), same in `count_in.rs`, `stopped.rs`, `live_midi.rs`,
`mixer/master.rs`; `cycle_load.rs` `CycleLoadReport` (+`lock_skips_by_map`)
and `format_cycle_load_line`. Note there is **no** `AudioEvent` for the
report: `CycleLoadMeter` lives inside the mixer closure
(`engine/mod.rs:758`) and prints the line with `eprintln!` from the audio
thread at `engine/mod.rs:807` (rate-limited, but still an RT-thread print —
ARCH-05's "no logging on the RT thread" rule applies; move the print to the
engine loop by reading the `SharedState` atomics there while here). The app
reads only `dsp_load_ema_bits` / `dsp_load_peak_bits`; add the per-map
counters next to them so the existing meter can show them.
Test: extend `resonance-audio/tests/cycle_load.rs` (uses `MixAudioHarness::render_lock_contended`
already) to hold a write guard on one map and assert only that map's counter
moves.
Diff: ~80 lines. Conflict: **low now** (F1 merged; `play.rs` is quiet).
Behaviour: none audible; one more diagnostic field.

**A2-2. `Retired<T>` deferred-drop queue on the engine thread (= MIX-04 fix).**
Goal: no `Arc` whose last owner may be the audio thread is ever freed there;
this is the primitive every later step publishes through.
Files: new `engine/retire.rs` (`struct Retired { queue: VecDeque<Arc<dyn Any + Send + Sync>> }`,
`fn retire(&mut self, old: Arc<T>)`, `fn sweep(&mut self)` dropping entries
with `strong_count == 1`), `engine/thread/mod.rs:435-452` (call `sweep()` in
the 16 ms loop), and the existing publishers: `thread/mod.rs:321` (automation),
`:542` + `transport.rs:450ff` + `midi/clock.rs:165` (`rcu_tempo`), latency
republish in `plugins.rs`, `takes.rs:671`-ish `publish_take_comp`,
`busses.rs:249` `publish_aux_sends`, `tracks.rs:85,94` (freeze source), each
switching `store(new)` to `let old = swap(new); state.retired.retire(old)`.
`ArcSwap::swap` returns the old `Arc`, so this is one line per publisher.
Test: the MIX-04 verification — hold `automation.load()` on a "callback"
thread, publish twice, drop the guard, assert the retire list still owns the
old value and that a `Drop`-counting wrapper ran on the engine thread.
Diff: ~120 lines. Conflict: low–medium (touches `transport.rs` which F1
just changed — rebase is trivial, the change is one line at each publish
site). Behaviour: none audible; removes a real xrun source.

**A2-3. Shrink the only O(n) write sections and the freeze-worker write.**
Goal: cheap hygiene while the maps are still locked.
Files: `engine/midi/clips.rs:441-447, 495-497, 518-520, 477-479` — compute
`new_notes` from a clone taken under a short `read()`, then `write()` to swap
(`std::mem::replace`), so the lock is held for a pointer swap;
`engine/vocal_render.rs:71` `ensure_tuning_caches` — build caches into a
local `HashMap<ClipId, Cache>` under a `read()`, then one short `write()` to
attach (the freeze worker currently holds `clips.write()` across every
FFT shift). Optional: `engine/clips.rs:763-780` load-worker block is already
minimal.
Test: existing `midi_bulk_edits.rs`, `quantize_engine.rs`, `vocal_tuning_bounce.rs`
cover the semantics; add one `MixAudioHarness` case that renders while a
long `ensure_tuning_caches` runs and asserts zero lock skips.
Diff: ~60 lines. Conflict: low (`midi/clips.rs` was touched by VIEW-02, merged).
Behaviour: none.

**A2-4. Migrate `midi_clips` to the published graph.**
Goal: first map off the lock; proves the pattern end to end.
Files: `engine/mod.rs` (`midi_clips: Arc<RwLock<Vec<MidiClip>>>` →
`Arc<ArcSwap<Vec<Arc<MidiClip>>>>` **or** the first field of a new
`RenderGraph`; prefer starting the `RenderGraph` struct now with one field so
later steps only add fields), `engine/thread/mod.rs` `HandlerCtx.midi_clips`,
the 22 write sites in `engine/midi/{clips,live}.rs` (each becomes `model.midi_clips_mut(|v| …); model.publish()`),
`mixer/callback/play.rs` (drop one `try_read`, use `graph.load()`),
`bounce/render.rs:308`, `freeze.rs:126,341`, `stem.rs:454`, `mixer/render_core.rs`
`BlockInputs.midi_clips: &[Arc<MidiClip>]` (callers deref), the two test
harnesses, and the **27 audio test files** that construct `Arc<RwLock<Vec<MidiClip>>>`
by hand (mostly `empty_engine_state()` helpers — this is where A3-6 pays off:
route them through `EngineHandlerHarness` while touching them).
Test: `EngineHandlerHarness` + `MixAudioHarness` in one test: spawn a thread
hammering `AddMidiNote`/`ReplaceNotes` on a 500-clip project while the harness
renders 10 000 blocks; assert `render_skip_cycles == 0` (the finding's
done-when, scoped to this map). Existing `midi_*`/`bounce_midi_events`/`stem_*`
tests pin semantics.
Diff: ~400 lines across ~35 files. Conflict: **medium** (every batch that
touches an engine MIDI handler). Behaviour: none audible except *fewer*
dropouts.

**A2-5 … A2-8.** `busses`+`master`, `tracks`, `plugins`, `clips` in the order
of the table — each the same shape as A2-4, each ending with `grep '\.write()'`
empty for that map. A2-8 (`clips`) additionally introduces the internal
engine-thread message for the load worker / freeze worker / pitch analysis and
rewrites `take_park.rs`'s doc contract. After A2-8: delete `RwLock` from
`HandlerCtx`/`ChunkCtx`, delete the five `try_read`s, keep
`try_lock_with_backoff` (plugin mutex).

**A2-9. Done-when test.** `EngineHandlerHarness` loads a 500-clip project
(`LoadClipFromWav` ×500 through the real handler) while `MixAudioHarness`
renders continuously; assert zero contended blocks and zero
`Retired`-queue drops on the render thread.

### NOW vs EPIC for ARCH-02

* **NOW: A2-1** (instrumentation, ~80 lines, no behaviour change, `play.rs`
  is quiet since F1 merged) and **A2-3** (lock-scope hygiene in
  `midi/clips.rs` + `vocal_render.rs`, ~60 lines). Both are safe in a bug-fix
  campaign and A2-1's numbers decide how urgent the rest is.
* **NOW if there is appetite for one medium change: A2-2** — it is the MIX-04
  fix (a real, reproducible xrun source: freeing a `LatencyComp` with up to
  960 000-float delay lines or a frozen-track cache on the audio thread), and
  every later ARCH-02 step needs the primitive. ~120 lines, one line per
  publisher.
* **EPIC: A2-4 … A2-9** ("engine graph publishing"), one todo per map in the
  table's order, each independently landable and each verifiable by the
  per-map counter from A2-1 reaching zero under the hammer test.

---

## ARCH-01 — per-feature state tax / `UndoExtras` / hub files

### Evidence, re-verified — the shadow file is smaller than the finding says

`UndoExtras` (`undo/snapshot.rs:40-106`) has 11 fields. Checking each against
`ProjectFile` and the two restore paths on current master:

| Extras field | Persisted in `ProjectFile`? | Slow path (`replay_loaded_project`) already restores it from the file? | Fast path (`try_diff_replay`) restores it from the file? | Verdict |
|---|---|---|---|---|
| `clip_fade_gain` | **yes** — `ProjectClip.fade_in_frames/curve/gain_db` (`model.rs:719-736`), written by `serialize.rs:305-309` | yes — `replay/mod.rs:533-571` sends `SetClipFade`/`SetClipGain` | yes — `replay_diff.rs:897-934` diffs fade/gain per clip | **fully redundant**; both paths then re-apply the extras (`snapshot.rs:370`, `replay_diff.rs:166`) — a double restore. The doc comment ("isn't part of `ProjectFile` yet (#321)") is stale. |
| `vocal_clip_lyrics` | **yes** — `ProjectMidiClip.vocal_lyrics` (`model.rs:853`), `serialize.rs:317-341` | yes — `replay/mod.rs:617-618` | no (`apply_compose` takes it from extras, `replay_diff.rs:1028`) | redundant; fast path must read `b.midi_clips[*].vocal_lyrics` instead. |
| `automation_lanes` | **yes** — `model.rs:162` | yes — `replay/mod.rs:119-136` | from extras (`replay_diff.rs:178`) | duplicate, as the finding says. |
| `external_instruments` + `external_instrument_devices` | **yes** — `ProjectTrack.external_instrument: ProjectExternalInstrument { device_id, device_definition, bank, program, … }` (`model.rs:452, 470-494`) | yes — `replay/entity.rs:192-223` | from extras (`restore_external_instruments`, `snapshot.rs:434`) | redundant; the doc comment ("`ProjectFile` shape doesn't carry it yet") is stale. |
| `track_freeze` | **yes** — `ProjectTrack.freeze: TrackFreezeState` (`model.rs:463`) | deliberately **skipped** on undo (`engine_events/project_io.rs:189` `freeze_rehydrate = pending_undo_extras.is_none()`), then `apply_freeze_restore(extras)` | from extras (`replay_diff.rs:159`) | derivable: `apply_freeze_restore` can take the statuses from `ProjectTrack.freeze` (same data, different cache-cleanup semantics — keep the function, change its input). |
| `compose_arrangements` | **yes** — `ProjectSectionDefinition.arrangement: Vec<ProjectPatternEntry>` (`project/sections.rs:65-72`; the "flattened primary id" is the *legacy* field) | yes — via `load_from_project` | `load_from_project` then overwritten from extras (`replay_diff.rs:1033`) | redundant; comment stale. |
| `reference` (`ReferenceUndo`: entries, active_id, loudness_match, offset_db, trim_db) | **yes** — `ProjectFile.references` + `reference_settings` (`model.rs:124-129`) | yes — `restore_references` (`replay/restore.rs:363`) | from extras (`replay_diff.rs:149`) | redundant modulo the "don't yank the monitor" rule (`reference/state.rs:138-143`), which `restore_references` must honour on the fast path (check `ab_source`/`loop_to_mix` are not reset by it). |
| `chord_track` | **no** — not in `model.rs`, not in `serialize.rs`; only `replay_diff.rs:156` and `snapshot.rs:367` restore it | — | — | **genuinely unpersisted**: undoable but lost on save/reload (confirmed). Needs a `ProjectFile.chord_track` field. |
| `compose_derived_clips` + `compose_next_derived_clip_id` | derivable — `ComposeState::rebuild_derived_clips` (`compose/state.rs:647-681`) rebuilds the map from clips after a disk load | yes (rebuild) | from extras | derivable; the extras copy exists only so the fast path doesn't have to call `rebuild_derived_clips`. |

So: **8 of 11 fields are already in `ProjectFile` and already restored from
it on at least one path; 2 are derivable; 1 (`chord_track`) is the only real
gap.** Folding the extras is mostly *deleting* code plus writing the
invariant test, not adding persistence. The exception is the fast path's
reading of the extras for compose/vocal/reference/external state, which must be
switched to the snapshot's `ProjectFile` — that is the behaviour-bearing part
and the reason each field is its own commit.

Hub files, re-verified: `message.rs` 1821 lines, 31 sub-enums defined in it,
`Message` has 36 variants; 211 files import from `crate::message`. **Three
sub-enums already live in their domain module** — `ComposeMessage`
(`compose/messages.rs:36`), `ReferenceMessage` (`reference/messages.rs:11`),
`ControlMessage` (`control_socket.rs:66`) — and `message.rs` simply `use`s
them (`message.rs:6-10`). So the split is an established pattern, not a new
one. Churn since 2026-08-01 by enum (hunks): `PluginMessage` 6, `UiMessage` 3,
`VocalTuningMessage` 2, `ClipMessage` 2, everything else ≤ 1 — so the safe
first movers are the low-churn enums and `PluginMessage`/`UiMessage` go last.
`Resonance` has 89 fields; `undo/classify.rs` has 12 `Message::X(_) => Skip`
catch-alls (`:43-136`), so a new variant in `Ui`, `Browser`, `Drag`, `Group`,
`Viewport`, … is silently non-undoable — that is ARCH-06's item, listed here
only because step A1-3 sets it up.

### Target shape (for orientation; only the first three steps are planned)

* `UndoSnapshot = LoadedProject` (a `ProjectFile` + midi notes + plugin blobs).
  No extras. Both restore paths read only the `ProjectFile`.
* Per-domain declarations next to per-domain handlers: `update/track.rs`
  owns `TrackMessage`, `state/tracks.rs` owns the track sub-state, and
  `message.rs` is `Message` plus re-exports. Undo classification moves beside
  each enum as an exhaustive `fn undo_action(&self)` (ARCH-06 step 3).
* A per-domain `Reconcile` trait driving both `replay_loaded_project`
  (old = empty) and `try_diff_replay` (old = current) — the finding's step 3,
  unchanged, deferred to the epic.

### Steps (first three only)

**A1-1. Invariant test first: snapshot → restore → `build_project_file` is a fixed point.**
Goal: a failing test for every field that the extras currently paper over,
so each deletion in A1-2 is guarded.
Files: new module in the existing `io` group binary —
`resonance-app/tests/io/undo_snapshot_fixed_point.rs` (register in
`resonance-app/tests/io.rs`; `tests/io/replay.rs` and `replay_diff.rs` are the
neighbours; `pool_persistence.rs:79-165` already uses
`app.test_build_project_file()` and the round-trip shape to copy).
Approach: build with `Resonance::new_for_test_with_capture()`, load the demo
project and each template, mutate one thing per domain (fade, lyric,
arrangement entry, external device, freeze status, reference trim, chord,
automation breakpoint), `snapshot_for_undo`, mutate again, restore through
*both* paths (`try_diff_replay` and the `ClearAll`→`AllCleared` pipeline —
`engine_events::project_io::all_cleared` is reachable from tests via the
capture receiver), and assert `build_project_file(app) == snapshot.project.file`
(derive/compare `ProjectFile` via its serde JSON if it lacks `PartialEq`).
Expected to **fail today for `chord_track`** (not in the file at all) and to
pass for the rest — which is the proof they are redundant.
Diff: ~200 lines of test. Conflict: none (new module). Behaviour: none.

**A1-2. Fold `UndoExtras` into the snapshot's `ProjectFile`, one field per commit.**
Order (cheapest and most obviously redundant first):
1. `clip_fade_gain`: delete the field, `ClipFadeGain` struct,
   `apply_clip_fade_gain_restore` and its two call sites (`snapshot.rs:370`,
   `replay_diff.rs:166`). Both paths already restore from the file. ~-80 lines.
2. `compose_arrangements` + `restore_arrangements`: delete; `load_from_project`
   already rebuilds from `ProjectSectionDefinition.arrangement`. ~-50 lines.
3. `external_instruments`/`external_instrument_devices`: change
   `restore_external_instruments(&UndoExtras)` to take `&ProjectFile` and
   read `ProjectTrack.external_instrument` (`device_id` is there); slow path
   then has a double restore to remove (`entity.rs:192` vs
   `finalize_undo_restore`). ~-40/+20.
4. `vocal_clip_lyrics`: `apply_compose` reads `b.midi_clips[*].vocal_lyrics`
   (mirror `replay/mod.rs:617`). ~-15/+10.
5. `automation_lanes`: `replay_diff.rs:178` builds the lane map from
   `b.automation_lanes` exactly as `replay/mod.rs:127-136` does. ~-10/+10.
6. `track_freeze`: `apply_freeze_restore` takes statuses derived from
   `ProjectTrack.freeze`; then the `freeze_rehydrate` special case in
   `engine_events/project_io.rs:189` can go. ~-20/+20 — the one with real
   semantics (cache deletion for tracks no longer frozen); keep the function
   body, change its input.
7. `reference`: `restore_references(r, &b)` on the fast path, after
   confirming it leaves `ab_source`/`loop_to_mix`/meters alone (if not, split
   `restore_references` into content vs monitor state first). ~-15/+10.
8. `compose_derived_clips` + `next_derived_clip_id`: call
   `rebuild_derived_clips` on the fast path as the slow path does. Watch the
   counter: `next_derived_clip_id` must be `max(existing)+1`, which the
   rebuild should already compute — assert it in A1-1.
9. `chord_track`: add `ProjectFile.chord_track: ChordTrack` with
   `#[serde(default)]`, write it in `serialize.rs`, read it in
   `replay_globals`; then delete the last extras field, `UndoExtras`,
   `pending_undo_extras` (`state/project_io.rs:56`; the `is_none()` checks in
   `replay/mod.rs:75` and `engine_events/project_io.rs:189` become an explicit
   `io.restoring_undo: bool`), and `finalize_undo_restore`. This is the only
   step that changes the project format (additive) and the only one that
   changes behaviour (chord track survives save/reload — a bug fix).
Test: A1-1 stays green after each commit; `tests/timeline/clip_fade_gain_snapshot.rs`,
`tests/io/replay_diff.rs`, `tests/timeline/automation_*`, `take_group_mirror.rs`
pin the domains. Diff: ~-250/+120 total across 9 small commits. Conflict:
**low-medium** — `undo/snapshot.rs` and `replay_diff.rs` are touched by the
stranded `ba/todo-1051`/`ba/designer-wip` branches only; no open batch touches
them (A2 merged). Behaviour: none until commit 9 (chord track persisted).

**A1-3. Move sub-message enums beside their handlers, low-churn domains first.**
Goal: `message.rs` shrinks toward `Message` + re-exports, and the file stops
being the merge magnet; enables ARCH-06's exhaustive `undo_action` later.
Files: for each domain D with `update/D.rs`: cut `pub enum DMessage {…}` (and
only the `use`s it needs) from `message.rs` into `update/D.rs` (or
`update/D/messages.rs` when `update/D/` is a directory, mirroring
`compose/messages.rs`), add `pub use crate::update::D::DMessage;` in
`message.rs`. The 211 importers keep compiling because they import from
`crate::message`. `update.rs`'s dispatch arms are untouched.
Order by recent churn (hunks since Aug): `Transport`, `Bus`, `Master`,
`Freeze`, `Take`, `Automation`, `Mixer`, `ExternalInstrument`, `Marker`,
`MarkerUi`, `Arrangement`, `Group`, `GlobalTrack`, `ChordTrack`, `Viewport`,
`ProjectIo`, `Export`, `Import`, `Pool`, `Relink`, `Browser`, `Drag`,
`Track`, `Clip`, `MidiClip`, `MidiEditor`, `VocalTuning` — then `UiMessage`
and `PluginMessage` last (6 and 3 hunks; M1 is live in the plugin area).
One commit per 3–5 enums; each is a pure move verified by `cargo check` +
`git diff --stat` showing only `message.rs` and the target file.
Diff: ~1500 lines moved, ~40 added. Conflict: **medium, but only with
branches editing the moved enum** — a modify/delete conflict on the enum body
is a one-minute resolution (re-apply the variant in the new file), and the
low-churn ordering makes it rare; leave `Ui`/`Plugin`/`Clip`/`MidiEditor`
until the campaign's view/plugin batches are merged.
Behaviour: none.

Not planned here (epic): `Reconcile` trait (finding step 3), grouping the 89
`Resonance` fields into sub-states (ARCH-06 step 2), and exhaustive
`undo_action` per enum (ARCH-06 step 3) — A1-3 is the enabling move for the
last one.

### NOW vs EPIC for ARCH-01

* **NOW: A1-1** (the invariant test; it also documents which extras are dead)
  and **A1-2 commits 1–2** (delete the two fully redundant fields; ~-130
  lines, no behaviour change, both paths already covered by existing tests).
  Optionally A1-2 commit 9's *persistence half* alone — add
  `ProjectFile.chord_track` — since "undoable but not saved" is a data-loss
  bug the campaign would otherwise want as a STATE-xx item.
* **NOW, opportunistically:** A1-3 for the first 5–8 low-churn enums (pure
  move, one commit) — but only in a moment when no batch is touching
  `message.rs`; today none is.
* **EPIC ("state tax"):** the rest of A1-2 (commits 3–8, each touching a
  restore function with semantics), the rest of A1-3, and the `Reconcile`
  trait.

---

## Summary — recommended NOW steps

| Item | Step | Files | Size | Conflict | Behaviour |
|---|---|---|---|---|---|
| ARCH-03 | A3-4 drop `__test_support::Receiver` from app production code | `resonance-app/src/lib.rs:732`, `src/test_support/project.rs:45` | 2 lines | none | none |
| ARCH-03 | A3-5 `test-internals` feature + promote 5 items to real API | `resonance-audio/src/lib.rs`, both `Cargo.toml`s, sed over tests | ~60 lines + sed | medium (`lib.rs` re-export churn) | none |
| ARCH-03 (optional) | A3-1 group audio tests into 10 binaries (+A3-2 docs/guard) — measured win ~6 s + ~13 GB disk per engine change | `resonance-audio/tests/**` (121 renames, 7 new roots, 4 one-liners), `Cargo.toml` comments, `CLAUDE.md`, `run-tests.py` docstring | large but mechanical | medium *only* against branches adding audio test files — none open | none |
| ARCH-02 | A2-1 per-map `try_read` failure counters | `engine/mod.rs` `SharedState`, `mixer/callback/{play,count_in,stopped}.rs`, `mixer/{live_midi,master}.rs`, `cycle_load.rs`, tick log | ~80 lines | low (F1 merged) | none |
| ARCH-02 | A2-3 lock-scope hygiene | `engine/midi/clips.rs` (4 sites), `engine/vocal_render.rs:71` | ~60 lines | low | none |
| ARCH-02 | A2-2 `Retired` deferred-drop (= MIX-04) | new `engine/retire.rs`, `engine/thread/mod.rs` loop, 7 publish sites | ~120 lines | low–medium | none audible; removes RT frees |
| ARCH-01 | A1-1 snapshot/restore fixed-point test | `resonance-app/tests/io/undo_snapshot_fixed_point.rs` (+1 line in `io.rs`) | ~200 lines | none | none (expected to fail for `chord_track`) |
| ARCH-01 | A1-2 (1)+(2) delete `clip_fade_gain`, `compose_arrangements` extras | `undo/snapshot.rs`, `update/project_io/replay_diff.rs` | ~-130 lines | low | none |
| ARCH-01 | A1-2 (9a) persist `chord_track` | `project/model.rs`, `update/project_io/serialize.rs`, `replay/mod.rs` | ~40 lines | low | chord track survives save/reload (bug fix, additive format) |

Everything else (A2-4…A2-9 graph publishing per map; A1-2 (3)–(8); A1-3
remainder; `Reconcile`) goes to two ba epics: **"engine render-graph
publishing"** (ARCH-02, one todo per map in the table order) and **"state
tax: one declarative project model"** (ARCH-01 + ARCH-06 + ARCH-09 step 3).

---

## Measurement (this machine, 2026-09-26 13:19–13:25, 16 cores, load avg 15–40 from other agents' builds)

Method: build the crate's tests to a fresh state, enumerate the test
executables with `--message-format=json`, delete them, and time
`cargo test -p <crate> --no-run` again. Deleting an executable makes cargo
re-run rustc for that test target (codegen + link) with the library fresh —
which is exactly the cost a one-line change to the crate's `src/` pays on top
of the lib compile.

| Crate | test targets | per-binary size | one target | all targets (wall) | est. after grouping |
|---|---|---|---|---|---|
| `resonance-audio` | 122 (+2 since) | 116–125 MB | 0.31 s | **8.4 s** (load 34→40) | ~1.5–3 s (10 targets) |
| `plugins/resonance-mastering` | 21 | ~374 MB | 0.30 s | **1.5 s** (load 17) | ~0.5 s (3 targets) |
| `resonance-app` (from `21ea8f0d`, for scale) | 241 → 13 | 700–940 MB | ~13 s CPU | 193 s → 8 s | (done) |

Take-away: outside the app, a test target costs ~0.3 s wall / ~1 s CPU
regardless of binary size (ld is copying sections; there is no iced
monomorphisation to redo). The ~600 non-app test targets in the workspace
therefore cost on the order of 40 s CPU ≈ a few seconds wall per full
rebuild, not minutes. The measurable costs of the sprawl are disk churn
(~14.6 GB per audio rebuild, ~7.9 GB per mastering rebuild; `target/debug/deps`
is 510 GB with seven stale hash variants of each binary) and process count,
not developer wait time. Repro:

```sh
cargo test -p resonance-audio --no-run --message-format=json | jq -r 'select(.reason=="compiler-artifact" and .profile.test and (.target.src_path|test("/tests/"))).executable' > exes.txt
xargs rm -f < exes.txt
time cargo test -p resonance-audio --no-run     # all test targets, lib fresh
```


---

## Progress notes (orchestrator, 2026-09-26)

**H1 landed (A2-1, A2-2, A2-3) @ merge of `arch/H1-engine-locks`.** Corrections to the plan above:
- A2-1: per-map counters (`SharedState::lock_misses`, `StateMap`) count misses from *every* callback branch, not only playing skips; `render_skip_cycles` remains the skip total. The report reaches the engine loop via `SharedState::cycle_report` (a seqlock of atomics), not an `AudioEvent`; the RT `eprintln!` is gone.
- A2-3: the "bounce guards stall the callback" scenario is moot since MIX-02's offline-render gate (the callback never `try_read`s maps while an `OfflineRenderGuard` is held). The remaining value of A2-4+ for offline paths is engine-thread latency and determinism, not dropouts. Bounce per-chunk guards documented on `ChunkCtx`, unchanged.
- A2-2: `Retired` lives on `SharedState` (engine-side Mutex; the audio thread never touches it) rather than `HandlerState`, because publishers only have `&HandlerCtx` and a few run on workers. `Track::{push,retain,set,clear}_plugins` now return the replaced `Arc`; A2-4+ should publish `RenderGraph` through `retire::publish`.
- Signature changes: `rcu_tempo(ctx, f)`, `ReferencePlayer::publish(&SharedState, …)`, `apply_master_fx_chain` / `pickup_live_midi` gained a parameter.
