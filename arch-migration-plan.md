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

> **Progress 2026-09-26 (branch `arch/H3-test-grouping`):** A3-4, A3-5,
> A3-1, A3-2 landed. 129 test files → 7 groups + 4 standalone (the plan's
> 3 plus `retire_queue`, a second `#[global_allocator]`); the five files
> added since the plan went to bounce (`offline_render_fidelity`) and
> mixer (`playhead_discontinuity_flush`, `silent_advance_loop_seam`,
> `stopped_instrument_preview`, with `note_recorder/`). 1028 tests
> before/after; 130 → 12 executables; touch-lib rebuild+run 12.1 s → 3.7 s.
> `__test_support` is now `test_support` behind `test-internals`; the
> featureless build allows `dead_code`/`unused_imports` crate-wide instead
> of cfg-gating each test-only helper (feature-on builds lint as before).
> Guard: `tools/arch-invariants` → `audio_test_binaries_are_the_known_groups`.

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

**H2 landed (A1-1, A1-2 (1)(2)(9a), A1-3 first slice).** Corrections:
- A1-1's `build_project_file(restore(s)) == s.file` cannot see chord_track (it was on neither side); the disk round-trip test (`tests/io/chord_track_persistence.rs`) and the full `same_state` check are what cover it. The fixed-point test (`tests/io/undo_snapshot_fixed_point.rs`) runs demo + 4 templates through both paths.
- `compose_arrangements` was fully redundant (`to_project_definitions` already writes the full arrangement).
- `vocal_clip_lyrics` normalisation differs between paths in live state (slow path pads to note count) — compare after normalising in A1-2 (4).
- `restore_performance` runs only on the slow path — fold into the Reconcile work.
- Slow-path `freeze.reset()` wipes UPD-05 content baselines — note for A1-2 (6).
- A1-3: `TransportMessage`, `BusMessage`, `MasterMessage` now live in `update/{transport,bus,master}.rs` with `pub use` re-exports.

**H4 landed (ARCH-08 + ARCH-10) on `arch/H4-sdk-invariants`.** Notes:
- ARCH-08: the 11 plugin manifests no longer name `wayland-plugin-gui` or `egui` (`editor = ["dep:plugin-gui-core", "resonance-plugin/editor-widgets", …]`); `resonance_plugin::editor_host` was already the only source-level platform reference. `plugin-gui-core` stays a direct plugin dep on purpose — it is the platform-neutral half and ~100 `use plugin_gui_core::…` lines read it directly; routing those through `resonance-plugin` would be a large mechanical diff for no layering gain. The one remaining runtime mention in `plugins/` is `resonance-gate`'s macOS-only *dev*-dep on `cocoa-plugin-gui` (the NSApplication pump for `editor_open_cocoa`); the SDK could re-export `test_support` to remove it. `resonance-plugin/src/ui.rs` moved to `resonance_app::plugin_ui`; the SDK has no `iced` dependency. `latency.rs` header corrected.
- ARCH-10: `tools/arch-invariants` (workspace member; `cargo test -p arch-invariants`) encodes: crate DAG (allowed edges per crate, any dep kind, manifest-declared so macOS edges are checked from Linux), GUI-toolkit/windowing/CLAP ownership, plugins never naming a runtime (manifest + source), `plugins/*` ⇔ members + cdylib (bundle.sh's checks, in the suite), the 11 app test groups closed, no inline `#[cfg(test)]` beyond the ARCHITECTURE.md exception, no `.engine.` under `view/`, no `_ =>` in `control/view_model/`. A3-2's "assert the resonance-audio test roots" belongs here once A3-1 lands.

**A-1 landed (A1-2 step 3) @ 37ae683a.** Notes:
- `UndoExtras` is down to 6 fields. `restore_external_instruments(&ProjectFile)` is now fast-path only (`replay_diff.rs`); the slow path's only restore is `replay_track` in `replay/entity.rs` (`finalize_undo_restore` no longer re-asserts, since `replay_loaded_project` already clears `r.external_instruments`).
- Device-param resolution is shared as `ProjectExternalInstrument::device_params` (embedded user definition first, then registry). The fast path used to consult the registry only — the paths differ only if a user device definition was rescanned with different params between snapshot and undo.
- `Resonance::test_snapshot_undo_extras` has no callers left; delete with `UndoExtras` in A-7.

**A-2 landed (A1-2 step 4) @ cb850b0c.** Notes:
- Canonical lyric forms on `VocalAudioRegistry`: file form `file_lyrics(clip, n)` (cut to note count, trailing empties stripped) and live form `restore_clip_lyrics` (padded to note count, no entry when empty). Disk load and both undo paths share the latter; the serializer uses the former (now also cuts at note count).
- Unplanned: UPD-05's `freeze_content_fingerprint` now hashes the file form, else a padding-only restore marked frozen vocal tracks stale. `compute_track_freeze_fingerprint` (test-support only) still hashes the raw entry — switch it too if it is ever wired live.
- Lyric entries for clips no longer in `midi_clips` are dropped on restore (vocal clips join `midi_clips` at install time, so no FU-H2a-style loss). `UndoExtras` is down to 5 fields.

**A-3 landed (A1-2 step 5) @ 5479f8cf.** `restore_automation_lanes(&[AutomationLane])` takes the file form and is the one lane restore for disk load and both undo paths; the slow path's second (extras) restore was a no-op and is gone. The file form is lossless (serializer writes the mirror verbatim, sorted by lane id) — no normalisation needed. `UndoExtras` left: `compose_derived_clips`, `compose_next_derived_clip_id`, `reference`, `track_freeze`.

**A-4 landed (A1-2 step 6) @ 8e10c24c.** Notes:
- `FreezeStatus::from_persisted` is the inverse of `to_persisted`; disk rehydrate and both undo paths use it. Canonical form: `Freezing`/`Failed` have no file form and restore as `Idle`; the cache ref's inner `status` matches the variant.
- `apply_freeze_restore(&[ProjectTrack], project_path)`; the slow path's freeze restore runs last inside `replay_loaded_project` (`replay_freeze`, after lanes since the fingerprint reads them), which now `take()`s `io.project_path`. `rehydrate_frozen_tracks` moved from `all_cleared` into the replay; the `freeze_rehydrate` capture is gone. On undo, replay start only clears the freeze queue, so the restore sees live statuses (FU-H2b baselines still survive).
- Fixed real asymmetry: slow-path undo of a freeze never deleted the cache or sent `UnfreezeTrack` (`freeze.reset()` wiped statuses first).
- Behaviour: undo no longer restores `Failed`/`Freezing` (was able to wedge `any_in_flight`); freeze progress no longer counts as a gesture change.
- **FU-A4a fixed @ 5dc2fa1e:** the `Freeze` domain passes `after_clear_all` to `reconcile_freeze_statuses`, which attaches (`attach_freeze_cache`, shared with rehydrate) every target Frozen/Stale track not already attached (none after `ClearAll`; diff path infers attachment from the live Frozen/Stale status). Missing/undecodable cache → Stale. Remaining edge: a diff-path track live-Stale only because its cache failed to load is treated as attached. Original note: after `ClearAll`, a track restored `Frozen` by a full-replay undo never gets `SetTrackFrozenSource` — shows Frozen, plays the live chain; fast path has the same gap for redo of a freeze whose cache exists. Fix = reconcile like rehydrate (decode + attach; undecodable → Stale); fixtures need real WAVs. Not a pure refactor, so left out.
- `UndoExtras` left: `compose_derived_clips`, `compose_next_derived_clip_id`, `reference`. `pending_undo_extras.is_some()` branches remain for A-7.

**A-5 landed (A1-2 step 7) @ db57a50c.** Notes:
- Content (`entries`, `active_id`, `loudness_match`, `trim_db`) was already persisted; the real work was engine re-sync, which neither undo path did (fast path restored the GUI only; slow path overwrote reallocated ids with stale ones). Monitor state lives in `ReferenceState::monitor` (`ab_source`, `loop_to_mix`, `offset_db`, `ab_meter`, `momentary_restore`) and is cleared from the undo file (`undo_project_file()`); it stays on disk.
- Two restores: slow path `restore_references` (after `ClearAll` reset the engine allocator) and fast path `reconcile_references` (match by path, keep ids/analysis, `LoadReferenceTrack` with a hint from app-side `next_engine_id`). New silent engine command `AudioCommand::ClearActiveReference`.
- Fixed: a project saved mid-analysis (`integrated_lufs: null`) failed to load — now reads back as −inf.
- Behaviour: undo no longer flips A/B / loop-to-mix; undo of a reference edit now reaches the engine; undo of a remove reloads under a fresh id (or `Missing`).
- **FU-A5a–c fixed @ 71ed3a6b:** `AddRefMarker { marker_id }` from the app (`alloc_marker_id`, session-monotonic; engine keeps no marker counter); every `LoadReferenceTrack` is id-hinted from a never-rewound `next_engine_id`, echoes for issued-but-unlisted ids are dropped (`is_stale`); a load is listed at once as `Analyzing(Decoding)` so it is in every later snapshot — `pending_loads` is gone. Guard `tests/io/reference_echo_races.rs` runs the app against the real `ReferencePlayer`. Original note: reference markers restore to the GUI only, and the engine's marker allocator restarts at 1 (collision); a late `ReferenceAnalysisProgress`/`Loaded` for a removed reference re-creates a ghost entry; an undo taken while a load echo is in flight lets that entry land afterwards.
- `UndoExtras` left: `compose_derived_clips`, `compose_next_derived_clip_id` (A-6).

**A-6 landed (A1-2 step 8) @ 4c76bb78.** Design: `docs/design/A-6-derived-clips.md`. Notes:
- Top-level `ProjectFile.derived_clips: Option<Vec<ProjectDerivedClip>>` rather than per-clip `derived_from`: a per-clip field can't hold in-flight (`OnEcho` drum install) or dangling (user-deleted) entries. `None` (old files, templates) → positional `rebuild_derived_clips`, as before.
- One restore `restore_derived_clips(&ProjectFile, echoes_in_flight, counter_floor)` for fast path (keep all), slow path + disk load (keep replayed). Supersedes the plan's "rebuild on the fast path".
- Derived counter is session-monotonic, not snapshot state: undo never lowers it; reserves past restored MIDI/audio clips and map values. Fixed: the slow path rewound it below ids issued after the snapshot (derived-range twin of STATE-08; vocal WAVs are `audio/clip_<id>.wav`).
- Behaviour: a moved derived clip stays claimed after reload; a hand-drawn clip on a generator placement bar is no longer claimed (and deleted) by regenerate.
- `UndoExtras` is `{}`; `undo_extras()` / `extras_equal` deleted; `finalize_undo_restore` a stub. Type, `pending_undo_extras`, `try_diff_replay`'s `_extras`, `test_snapshot_undo_extras` left for A-7.
- **Open:** FU-A6a — the engine bumps `next_clip_id` past any id it is handed, so after the first derived clip, engine-allocated clips (GUI, recordings) land at `DERIVED_CLIP_ID_BASE + k`, where `fresh_derived_clip_id` allocates next without an in-use skip; `state/ids.rs` names a non-existent `ComposeState::allocate_derived_clip_id`. Unverified. FU-A6b/c fixed @ 1da0beba (`forget_deleted_derived_clip` at user-delete sites only — not in the `MidiClipDeleted` echo, which regeneration also triggers; `reserve_derived_clip_ids_on_disk` on disk load + Save As; a resize never resurrects a user-deleted clip). Still open: FU-A6d — the vocal audio-clip map likely has the same dangling-entry issue on user delete. Was: FU-A6b — dangling map entries for user-deleted derived clips survive a fast-path undo (suspend UPD-05, may resurrect on resize). FU-A6c — derived counter not seeded from `audio/clip_*.wav` on load (STATE-12 for the derived range).

**A-7 landed (A1-2 step 9b) @ 3e4496e9.** `UndoExtras`, `pending_undo_extras`, `finalize_undo_restore`, `try_diff_replay`'s extras param gone; `UndoSnapshot { project: LoadedProject }`. `io.restoring_undo: bool` is set at the slow-path `ClearAll` and `mem::take`n in `all_cleared` exactly where the Option was; 7 sites mapped 1:1. Guard: `tests/io/undo_restore_flag.rs`.
- **FU-A7a fixed @ f0c07231:** reachable from GUI open/recent (control API is gated by `busy_guard`); `restoring_undo` cleared beside `pending_load = Some` in `ProjectLoaded(Ok)` and `begin_instantiate` (the only two non-undo sites). "`AllCleared` never arrives" only happens when the command channel is dead, which already latches the restart banner — left alone. Original note: a GUI `ProjectLoaded`/template `begin_instantiate` arriving while an undo's `ClearAll` is in flight replaces `pending_load` but not the flag, so the disk project replays with undo branches (no patch resend, rehydrate, relink modal or job completion). Fix: clear `restoring_undo` in `ProjectLoaded(Ok)` and `begin_instantiate`. An undo whose `AllCleared` never arrives leaves `loading` + flag set.
- **A-13 roadmap (from the A-7 agent):** the slow-path undo is not "old = empty" — freeze statuses, reference monitor and derived counter keep live state — so the trait needs `ReconcileCtx { origin: DiskLoad | UndoFull | UndoDiff }` (absorbs `restoring_undo`). Order: (1) app-side-only domains restored whole on both paths — quantize, performance, chord track, markers, track groups, pool, take groups, tempo events; (2) shared fns with origin-dependent behaviour — automation lanes, derived clips, external instruments, references, freeze (FU-A4a), missing-plugins; (3) routing — sends, sidechain routes; (4) globals/transport; (5) audio clips, MIDI clips, compose/vocal; (6) structural — tracks, busses, master, plugins; only then can `structurally_compatible` and the `ClearAll` fallback go. Driver keeps a fixed order: lanes after external instruments, freeze last, derived clips after clips.

**FU-A6a fixed @ 6942f712.** Reproduced (engine: derived `base`, drawn clip, derived `base+1` → ids `[base, base+1, base+1]`; reopening a project with a vocal render also pushed the engine counter into the range via the STATE-08 WAV scan). `DERIVED_CLIP_ID_BASE` moved to `resonance-audio/src/types/mod.rs`; `engine/clips.rs::reserve_clip_id` bumps only below it (4 sites) and the WAV scan skips the range. App-side `fresh_derived_clip_id` is the only allocator in the range. For D-6/D-7: `fresh_derived_clip_id` is already the app's general clip allocator (import, bounce target, control clip/notes, vocal installs) — rename and fold recordings/takes/imports into it rather than add a second range. FU-A6c still open.

**A-13a landed @ b18300be.** Design: `docs/design/A-13-reconcile.md`; code `update/project_io/reconcile/`. `trait Reconcile { const NAME; fn reconcile(r, old: Option<&ProjectFile>, new, ctx) }`, `ReconcileCtx { origin: DiskLoad|UndoFull|UndoDiff, project_dir }`; one `const DOMAINS` table tagged by `Stage` (`Timeline`, `Content`), run by `reconcile_stage` on both paths. Stages exist only for the not-yet-migrated inline code between domains, and collapse as slices land. `io.reconcile_trace` + `tests/io/reconcile_order.rs` guard the order. Group (1) done. Reorders (checked, no behaviour change): full path markers/chord/tempo-events after `SetBpm`; diff path chord/markers and Content domains later. The diff path's tempo stays after external instruments (moving it would change the tempo map `restore_derived_clips` sees) — converge in group (4). **Group (2) needs:** a `live` carry struct on the ctx for state captured before `ClearAll` (freeze statuses, reference monitor, derived-counter floor, project path); new stages `Clips` (derived clips) and `Tail` (external instruments → lanes → missing-plugins → freeze); external instruments leave `replay_track` (check `tests/io/replay.rs` order assertions).

**A-13b landed @ 5aeec0c2.** Group (2) on the driver; design doc §7. `ReconcileCtx.live: LiveCarry { project_path, derived_counter_floor }` — freeze statuses and reference monitor turned out not to need carrying. Stages `Timeline, Clips, Content, Tail`; 14 domains pinned by `reconcile_order::the_table_is_the_agreed_order`. External instruments left `replay_track`: one body `restore_external_instruments(file, after_clear_all)`, file order (was HashMap order on diff path); on the full path the config is now sent after tracks/clips/Content, before lanes (engine reads it at render time). FU-A4a's fix point is the `Freeze` domain's undo arm. **Group (3):** a `Routing` stage between Timeline and Clips; `wipe_registry`'s `aux.sends.clear()`/`sidechain.clear()` move into the domains; first domains to use `old`; keep full-path endpoint validation and diff-path removals-first. **Group (4):** `Globals` stage before Timeline; keep `restore_drum_patterns`' clear-when-empty flag difference and full-path-only UI reset; diff-path tempo converging before clips is a real engine-order change.

**A-13c landed @ 3150564e.** Group (4); design doc §8. New `Stage::Globals`; 20 domains. `transport` is the first domain diffing against `old` (send all after `ClearAll`, only changed scalars on the diff path). Diff-path tempo/chord/markers now precede track and clip commands (evidence: no intervening engine handler or app step reads the tempo map; pinned by `a_diff_undo_sends_tempo_before_the_clips_and_only_changed_scalars`). `replay_globals`, `apply_global`, `apply_compose` deleted. Full-path-only kept: UI reset, drum-roll focus, playhead reset, chord trim. **Next:** group (3) `Routing` stage between Timeline and Clips (full path after `replay_master`; copy `transport`'s old-diff shape); group (5) clip domains at the head of `Clips` + vocal audio-clip map rebuild; group (6) structural (tracks, busses, master, plugins), after which `structurally_compatible` and the `ClearAll` fallback can go.

**A-13d landed @ be5c201a.** Group (5); design doc §9. 23 domains; `Clips` = audio_clips, midi_clips, clip_lyrics, derived_clips, vocal_audio_clips. `ReconcileCtx.midi_notes` (the file carries no notes). Full path command order unchanged; diff path's vocal-map rebuild moved to right after `DerivedClips` (guard `a_diff_undo_rebuilds_the_vocal_audio_clip_map_from_the_target`). Only entities and routing remain inline between Timeline and Clips. **Group (3):** `Routing` after master on both paths — puts diff-path sends after master (unverified; the slice must show independence). **Group (6):** once entities add/remove on the diff path, clip diff arms add "load what old lacks / delete what new lacks" (`load_audio_clip`/`load_midi_clip` are ready); then `structurally_compatible` + `ClearAll` fallback go.

**A-13e landed @ ee067a01.** Group (3); design doc §10. `Stage::Routing` (sends, sidechain_routes); 25 domains. Only entities remain inline between Timeline and Clips. **Group (6) plan (strictly sequential, after D-4):** A-13f entity scalars → `Stage::Entities` (tracks, busses, master, track_outputs, plugin_state, entity_order) keeping the shape check; A-13g shrink `structurally_compatible` (drop sections/placements, drum groups/patterns, track groups, markers one at a time); A-13h diff-path add/remove for busses + plugin instances (edge removals before entity removal); A-13i add/remove tracks (incl. sub-tracks, type change = remove+add) and clips; A-13j delete `structurally_compatible`, the undo `ClearAll`, `restoring_undo`, `Origin::UndoFull`, merge entry points — a structural undo stops re-instantiating plugins.

**A-8 landed (A9-3 cheap half) @ cc2fd3b4.** `same_state` / `gesture_changed_since` compare `ProjectFile` with `==`; `files_equal` is deleted. The gesture-end check is now ~287 µs, down from 677 µs, which is roughly `build_project_file` alone. Old JSON semantics mapped NaN/inf to `null` (NaN == NaN); the only reachable field that can realistically hold NaN is `ProjectPluginParam.value` (plugin-reported), so it has a hand-written `PartialEq` (`==`, then `to_bits()`). No `#[serde(skip)]` / `serialize_with` in the tree. The tempo-map `PartialEq` derives turned out not to be reachable from `ProjectFile`; they are harmless. Guard: `struct_equality_agrees_with_json_equality_across_fixtures`.

**A-10 landed (A6-4) @ fd8a3c6e.** Every sub-message enum (plus `UiMessage`, `PluginMessage`, `ControlMessage`) has an exhaustive `undo_action`; `classify.rs` is one delegating arm per enum. `arch-invariants::undo_classification_has_no_catch_all_arms` guards it (≥30 impls). Behaviour changes (Record → Skip, guarded by `tests/timeline/undo_transient_dialog_state.rs`): bounce dialog `PickDevice`/`PickPort`/`SetMono`/`Cancel`/`CancelInProgress`; drum-manager view-state variants (a 5-keystroke pattern rename was 7 undo entries, now 1). Re-blessed `compose_drum_rail_snare_meter_open-wgpu.png` (title bar "unsaved" → "saved" only).
- **FU-A10a fixed @ a8a5f234** (new `CoalesceKey`s `TrackName`, `DrumGroupName`, `DrumGroupParam(group, DrumGroupKnob)`, `VocalTheme`, `VocalLineText`; `LaneInspectorMsg::undo_action(def, track)`; `BounceInPlace` is Skip, the handler force-records `BounceInPlaceOffline`). FU-A10b fixed @ e4869177 (`LaneParam`, `ChordParam`, `VocalBulkLyrics` keys; `ChordInspectorMsg::undo_action(def)`). FU-A10c fixed @ 0e82b6fc. Was (FU-A10c): `DrumGroupsMessage::SetGroupCycle`/`SetGroupPhase`/`SetPadWeight` and `ExternalInstrumentMessage::SetLatencyOffset` sliders record per step. Was (FU-A10b): ~16 numeric sliders in `LaneInspectorMsg`/`ChordInspectorMsg` (bass/melody/pad velocity+register, vocal delivery, chord complexity/leap/substitution) record per step; the bulk-lyrics `text_editor` records on cursor moves. Original note: per-keystroke/per-step messages still record one entry each and need `CoalesceKey` variants in `undo/snapshot.rs`: `TrackMessage::SetTrackName`, `DrumGroupsMessage::RenameGroup`, drum group knobs (`SetGroupDensity`, swing, …), probably `LaneInspectorMsg::SetVocalLineText`. `TrackMessage::BounceInPlace` on an external-MIDI track only opens the dialog but records (realtime bounce = 2 entries); the classifier can't see track state.

**A-12a landed (A6-2 batch 1) @ 62086b6b.** `state::{PluginCatalog, MidiDevices, Banners, InputDevices}`; `Resonance` 90 → 79 fields. Batches 1–3 keep the original (stuttering) field names inside the sub-structs (`r.plugin_catalog.available_plugins`); `InputDevices` uses `devices` / `default_name`. A cosmetic pass can drop the prefixes. Remaining: `PresetState`, `ModalState`, `PluginMirror`, `MasterState`.

**A-12b landed (A6-2 batch 2) @ 7c9af9de.** `state::PresetState` (8 fields) and `state::ModalState` (7); `Resonance` 79 → 66. `import_progress_modal_open` left beside `import_progress` on purpose. Remaining: `PluginMirror` (after D-1), `MasterState` (last).

**A-12c landed @ 0768c633.** `state::PluginMirror { state_cache, index, next_id }`; `Resonance` 66 → 64. `master_plugins` belongs to `MasterState` (plan's table), not here. Only `MasterState` remains in A-12; reaching ≤ 40 fields needs more groups beyond the plan's 8 (a follow-up survey).

**A-12d landed (A6-3 last).** `state::MasterState { volume, level_l, level_r, plugins, fx_bypassed }` — field names match `BusState`'s, not the batch-1/2 stuttering convention (same choice A-12c made for `PluginMirror`); `Resonance` 64 → 60. This closes the plan's original 8-group table. Survey of the remaining 60 fields (with proposed homes and the next 4 groupings to reach ≤ 40) in `docs/design/A-12-resonance-fields.md`: `MediaState` (−6), `DeviceState` (−4), `UiTransientState` (−7), `SessionMetaState` (−4, high blast radius — `revision` alone touches ~57 sites — recommend landing it alone). `SongStructureState` (`clips`/`midi_clips`/`tempo_events`/`signature_events`/`tempo_map`/`chord_track`/`markers`) is deliberately deferred: every field in it is what A-13/A-13d's Reconcile-trait migration is actively working through. Also flags `Resonance::groove_library` as dead (superseded by `quantize.groove_library`, kept in sync only for a test hook).

**A-12d landed @ 5281dd48.** `state::MasterState { volume, level_l, level_r, plugins, fx_bypassed }`; `Resonance` 64 → 60. All 8 planned sub-states done. Survey (`docs/design/A-12-resonance-fields.md`) proposes a second tier to reach ≤ 40: MediaState (−6, ~177 refs), DeviceState (−4, ~137), UiTransientState (−7, ~332), SessionMetaState (−4, ~203; `revision` touches ~57 sites — land alone, maybe behind `bump_revision()` first) → 39. SongStructureState deferred until A-13 group (6) is done. Dead: `Resonance::groove_library` (superseded by `quantize.groove_library`) — delete.

**A-12e landed @ e7a1b28a.** Dead `groove_library` deleted; `state::MediaState` (`r.media`: pool, pool_import, import_progress, import_progress_modal_open, browser, drag_placement, relink); `state::UiTransientState` (`r.ui`: view_mode, pre_performance_view, view_caches, transport_labels, mixer, interaction, last_arrangement_shift) — `Debug` only (members aren't Clone/Default). `update_depth` stays top-level (dispatch re-entrancy guard, not UI). `Resonance` 60 → 47. Left: DeviceState (−4), SessionMetaState (−4, alone) → 39; SongStructureState after A-13.

**C-1 landed (A5-3 first half) @ 8339eecd.** 39 emit sites (not 41). `PluginEditorFailure::engine_error_kind()` classifies editor-open failures per variant. 12 sites stay `Internal` because their `Result<_, String>` source mixes causes: 4 "no project directory", 2 worker-panic recoveries, 2 `platform::build_input_stream`, 4 MIDI hardware/clock `configure`/`set_track_*` — C-3's per-module `thiserror` should split these. `midi/hardware.rs`'s `set_track_input` `Err` arm is unreachable today.

**C-2 landed @ 4f3342dd.** `BounceError { kind: ExportErrorKind, message }` (its sites already went through `ExportReporter::error` with a kind); `TrackBounceError`/`StemExportError` carry `EngineError`. `JobStatus.error` is now `Option<JobError { message, kind: Option<ErrorKind> }>`; its `Deserialize` also accepts the old bare string. **Wire note:** the *serialized* shape changed (string → object), so a pre-C-2 reader cannot parse a new app's failure; accepted without a `PROTOCOL_VERSION` bump since app and `resonance-mcp` ship together and there are no external clients. `ExportErrorKind` → control `ErrorKind` mapping is in the app (`update/control/job.rs::export_kind_to_rpc`). Only `render.mixdown` reaches a control job today; track/stem bounce kinds are only logged.

**D-1 landed (A4-4 plugins) @ 630b5197.** `AddPlugin*` take a mandatory `id`, so the compiler found every site. `Resonance::allocate_plugin_id` (monotonic counter + in-use scan over the mirror; replay's placeholder slots are eager, so replayed ids are seen) serves GUI, control, presets, replace and replay. The engine's `reject_if_plugin_id_in_use` refuses a collision with `EngineErrorKind::Internal` (not transient, so not `Busy`). The plugin row is gone from `state/ids.rs`. Template for D-2..D-5: mandatory id field, delete the engine counter, per-container reject helper, no counter reset on `ClearAll`. `tests/clap_host/plugin_id_duplicate_rejected.rs` replaces `plugin_id_ranges.rs`.

**C-3 part 1 landed @ 4ef24291.** `MidiClockError`, `MidiHardwareError`, `InputStreamError` (`.kind()`), `MidiIoError`, `RecordingError`, `WavParseError`/`WavIoError`, `ClipError`, `ClapBundleError`, each with `From<…> for EngineError`. Messages textually unchanged. Of C-1's 12 `Internal` sites, 6 now carry real kinds (2 input-stream, 4 MIDI hw/clock); 6 stay `Internal` by design (4 "no project directory" preconditions, 2 worker panics — no `Result` source). **Remaining (C-3b):** 12 pub fns — `audition::load_audition_source`, `import_queue::submit`, `AudioEngine::new`, `clips::transcode_to_wav`, `bounce/freeze::read_freeze_cache`, `plugins::ensure_bundle`, `bounce/stem::{render_stem,write_stem_wav}`, `import_pool::{import_one_to_pool,run_pool_import_with}`, 2 `#[doc(hidden)]` bounce test helpers; plus pub(crate) PipeWire `build`s and `plugins::resolve_plugin_id`.

**C-4 landed @ 0d8a7504.** `AtomicWriteError`, `RegistryError`, `MidiMapError`, `DeviceJsonError`, `DeviceLoadError`/`DeviceSaveError`, `WavDecodeError`, `AudioProbeError`; messages verbatim. `From<WavDecodeError|AudioProbeError> for EngineError` (Io) in resonance-audio. All four feature combos and standalone drums/ir/amp builds pass. resonance-common has no `Result<_, String>` left.

**C-3b + C-5 landed @ 82469dbd — Epic C done.** `AuditionError`, `ImportQueueError`, `TranscodeError`, `ImportError`, `PluginBundleError`, `PartialFileError`, `FreezeError`/`FrozenCacheError` (`freeze_terminal_event` now matches `FreezeError::Cancelled`, not a string compare), `StemError`, `EngineInitError`, `PwInputError`/`PwOutputError`, test-only `TestEncodeError`. Only private `recording.rs::take_clip_source` keeps `String`. `arch-invariants::engine_common_public_fns_dont_return_result_string` walks audio + common `pub fn` signatures; both allow-lists empty. `ImportFailed.reason` is still a `String` (via `to_string()`).

**D-2 + D-3 landed @ fdef85fd.** Sends: `AddAuxSend` (rejects collision) / `SetAuxSend` (quiet no-op on unknown id) split; the diff path picks by whether the id existed pre-diff; `CONTROL_SEND_ID_BASE` gone. Busses: engine has no bus counter; `BUS_ID_BASE` (= old `RETURN_BUS_ID_BASE`, 2e9) kept as an **app-only** convention because `song.summary`/`song.tracks` list tracks and busses in one id-addressed sequence — a bus on a live track's id would be silently unreachable via control. D-4 (tracks) must keep that in mind. Replay mirrors busses and sends eagerly, so no bump-on-load is needed. Also fixed `id_allocation.rs::add_round`'s `gui_bus` pick, which had been testing the control bus.

**D-4 landed @ c44abfdb.** `AddTrack`/`AddInstrumentTrack`/`AddVocalTrack` take a mandatory `id`; engine `reject_if_track_id_in_use` (`Internal`); `SUB_TRACK_ID_BASE` deleted; `TrackRegistry::next_track_id` starts at 1 (fresh projects unchanged) and `debug_assert`s below `BUS_ID_BASE`. Replay mirrors eagerly. Fixed: `demo::seed_demo_content` never bumped the counter past its hand-picked ids 1–6. Remaining engine counters: clip, take-group, asset, reference, marker (D-5 in flight, D-6).

**FU-D4a landed.** Closed the startup race D-4 left open: the real engine thread created a default track id 1 unprompted at startup (`engine/thread/mod.rs`, right after `SampleRateDetected`), and the app's counter also starts at 1, so a first GUI "Add Track" handled before the app mirrored that `TrackAdded` echo allocated id 1 too — the engine refused it as a collision (`Internal`), a click that visibly did nothing. Fix: the engine thread no longer creates a track unprompted at all; the app sends its own `AddTrack` for the startup default (`Resonance::send_startup_default_track`, `state/ids.rs`), called from `Resonance::new` synchronously before iced's event loop can run, so nothing can ever again race it for id 1. Tests: `resonance-audio/tests/engine/startup_no_default_track.rs` drives the real `engine_thread` via a new `EngineHandlerHarness::startup_events` (fake channels, no audio device, `ShutDown` queued before spawn) and asserts no `TrackAdded` fires; `resonance-app/tests/io/startup_default_track.rs` drives `Resonance::new_for_test_with_capture` + the new `test_send_startup_default_track` hook and asserts the startup send and the very next GUI "Add Track" never collide. No visible behaviour change: a fresh session still ends up with one "Track 1", id 1.

**E landed (A7-3) @ 38d66942.** `dirs` and `time` stay unconditional deps, not optional under `model`: the ungated `registry` module (used by drums) calls `dirs::data_dir()` and `time::OffsetDateTime`. `symphonia` is optional under `decode`. amp and `resonance-plugin` use no features; drums and ir use `decode`. `arch-invariants::plugins_disable_default_features_on_resonance_common` guards it.



---

# Part 2 — ARCH-04/05/06/07/09 (planned 2026-09-26, after H1/H2/H4)

# ARCH-04 / 05 / 06 / 07 / 09 — incremental implementation plan (H5)

Read-only architecture pass, 2026-09-26, against master `f57c6793` (H1, H2, H4
merged; H3 branch `arch/H3-test-grouping` has no commits yet; `fix/review-M12`
open with 9 commits). Every number below was re-measured on this tree; where
the review text (from `09041eee`) is out of date it says so.

Conventions per step: **Goal · Files/symbols · Approach · Test · Diff ·
Conflict · Behaviour**. "NOW" = land in this campaign; "EPIC" = file in ba and
drain later.

---

## Cross-cutting facts

* **Open branches that bound conflict risk.** `fix/review-M12` (in progress)
  touches `undo/classify.rs` (+21), `undo/mod.rs` (+10), `update/track.rs` (+4),
  `update/clips.rs` (+5), `update/project_io/{mod,autosave}.rs`,
  `engine_events/{clips,mod,project_io,tracks}.rs`, `state/project_io.rs`,
  `project/io.rs`, `resonance-audio/src/engine/{clips,tracks,thread/mod}.rs`.
  `arch/H3-test-grouping` will touch `resonance-audio/src/lib.rs` (A3-5) and
  `resonance-audio/tests/**`. Everything marked NOW below avoids those files or
  touches them by one additive line.
* **`ba/todo-1059` "Split message.rs into per-domain submodules"** is an
  approved, open ba todo with a stale `[X verify-failed @35fc6bf5]` tag (July).
  Its branch made a `message/` *directory* (`message/{browser,clip,plugin,…}.rs`)
  and is 124 files / 17k lines diverged from master. H2 already moved
  `Transport`/`Bus`/`Master` the other way (enum beside its handler). Do **not**
  rebase #1059; re-scope its text to A6-1's shape and let A6-1 close it.
* Churn since 2026-09-01: `message.rs` 7 commits, `resonance-audio/src/lib.rs`
  13, `resonance-app/src/lib.rs` 5, `undo/history.rs` 4, `undo/classify.rs` 2,
  `types/events.rs` 1, `resonance-common/src` 4, `state/{tracks,plugin_index,aux_sends}.rs` 0.
* No build was run (memory-exhaustion crash earlier today); all evidence is
  `grep`, `git`, `cargo metadata`/`cargo tree --offline`.

---

## ARCH-04 — entity ids allocated in two places with hand-partitioned bases

### Evidence, re-verified (still valid; one correction)

Engine side — `HandlerState` (`resonance-audio/src/engine/thread/mod.rs:101-123`)
owns **7 counters**: `next_track_id`, `next_bus_id`, `next_clip_id`,
`next_asset_id`, `next_plugin_id`, `next_send_id`, `next_take_group_id`; plus
`ReferencePlayer.next_ref_id` / `next_marker_id` (`engine/reference.rs:77,105`).
**15 high-water bump sites** (`state.next_x_id = max(id + 1)`) in
`engine/{midi/clips,reference,busses,plugins,takes,tracks,clips}.rs` and
`thread/dispatch/clips.rs:25`, plus two "reserve" commands with no echo:
`AudioCommand::ReserveAssetIds` (`commands.rs:95`) and `RestoreTakeGroups`
(`commands.rs:492-505`, bumps group + clip counters).

App side — **five allocators in four files, three copies of the same
"skip while in use" loop**, and five numeric bases spread over five files:

| Space | Allocator | Base | Where |
|---|---|---|---|
| plugin (control adds) | `allocate_control_plugin_id` | `CONTROL_PLUGIN_ID_BASE = 3_000_000_000` | `state/plugin_index.rs:158`; base in `resonance-audio/src/types/mod.rs:34`; engine honours hints below the base only (`engine/plugins.rs:282-310`) |
| send (control adds) | `allocate_control_send_id` | `CONTROL_SEND_ID_BASE = 2_000_000_000` | `state/aux_sends.rs:106,121` |
| sub-track / bounce target / **track group** | `allocate_sub_track_id` | `1_000_000_000` (unnamed, seeded `lib.rs:881`) | `state/tracks.rs:556`; groups via `update/group.rs:80` |
| FX return bus | `allocate_return_bus_id` | `2_000_000_000` (unnamed, `lib.rs:882`) | `state/tracks.rs:571` |
| derived (compose) clip | `next_derived_clip_id` | `DERIVED_CLIP_ID_BASE = 1 << 40` | `compose/state.rs:31` |
| missing reference | local counter | `MISSING_ID_BASE = 1_000_000_000` | `replay/restore.rs:369` (app-only space, fine) |

App-only spaces (never cross to the engine): markers `allocate_id`
(`state/markers.rs:115`), automation `next_lane_id` (`state/automation.rs:42`),
groove ids. STATE-04 (group-id counter) was fixed by three load-time bumps
(`replay/mod.rs:287`, `replay/restore.rs:25`, `replay_diff.rs:1059`); FU-A1c
(`allocate_sub_track_id` doesn't check the group registry) is still open.

Senders: `id_hint: Some(..)` at 25 sites / 11 files (replay 10, `update/track.rs` 6,
`mixer.rs` 3, `bus.rs` 2, …); `id_hint: None` (engine allocates) at 12 sites /
8 files (`track.rs` 3, `mixer.rs` 2, `bus.rs` 2, `plugin.rs`, `master.rs`,
`reference.rs`, `view/menus.rs`, `engine_events/presets.rs`).

**Correction to the finding's "decide one owner: the app".** Two engine spaces
are allocated *inside the engine at times the app cannot pre-decide*: clip ids
for recorded takes (`recording.rs:484` at record-stop, per armed track, per
loop pass) and take-group ids (`takes.rs:191`), and asset ids on the import
worker (`import_pool.rs`). Moving those to the app means pre-reserving a block
of ids per arm/import and passing it in the command — real design work, not a
mechanical move. Everything else (track, bus, plugin, send, reference) is
mechanical. So the epic is worth 5 todos, not 7, and the recording/import pair
is the one to design first.

### Steps

**A4-1. Collision invariant test (NOW).**
Goal: pin the partition that keeps the two owners apart today, so the epic can
move spaces one at a time against a failing test.
Files: new module `resonance-app/tests/io/id_allocation.rs` (register in
`tests/io.rs`; neighbours `chord_track_persistence.rs`, `undo_snapshot_fixed_point.rs`).
Approach: `Resonance::new_for_test_with_capture()`, load the demo, add one of
each entity twice — once via the GUI message (`TrackMessage::AddTrack`,
`AddBus`, `PluginMessage::Add…`, `MixerMessage::AddSend`, create-group,
compose-derived clip) and once via control (`run_via_update` → the
`allocate_control_*` paths) — drain echoes with the existing capture helpers,
`test_build_project_file`, reload through `replay_loaded_project`, add again;
assert every id space is a set (no duplicates) and every app counter is
`> max(existing)`. Add a `const` assertion that the five bases are pairwise
disjoint and ordered (`SUB_TRACK < RETURN_BUS == CONTROL_SEND < CONTROL_PLUGIN
< DERIVED_CLIP`) — today the sub-track base (1e9) and the return-bus base (2e9)
are unnamed literals.
Test: itself. Diff: ~150 lines. Conflict: none (new module). Behaviour: none.
Expected: green today except possibly the FU-A1c group case — write that case
so it fails, then A4-3 fixes it.

**A4-2. One table for the app side: `state/ids.rs` (NOW).**
Goal: the "who allocates what, from where" answer lives in one file, and the
three copied skip-loops become one function.
Files: new `resonance-app/src/state/ids.rs` with the five base constants
(re-export `CONTROL_PLUGIN_ID_BASE` from audio, name the two literals
`SUB_TRACK_ID_BASE`, `RETURN_BUS_ID_BASE`, move `CONTROL_SEND_ID_BASE`,
`DERIVED_CLIP_ID_BASE`, `MISSING_REFERENCE_ID_BASE`) and
`pub fn allocate_unused(next: &mut u64, in_use: impl Fn(u64) -> bool) -> u64`;
`state/tracks.rs:484-585`, `state/plugin_index.rs:150-170`,
`state/aux_sends.rs:100-125`, `compose/state.rs:31`, `replay/restore.rs:369`,
`lib.rs:881-894` (seed from the constants). Keep the allocator *methods* where
they are (their callers don't move); only the bodies and constants change.
Test: A4-1 stays green; existing `take_group_mirror`, control `track.add`
tests. Diff: ~+80/-60. Conflict: low — the four state files have 0 commits
since September; `lib.rs` seed lines are not in M12's diff. Behaviour: none.

**A4-3. FU-A1c in the same place (NOW, optional, ~10 lines).**
`allocate_sub_track_id` also skips ids present in `track_groups` (pass the
registry's id set as the `in_use` closure from `update/group.rs:80` and the
control `track.add` path). Test: the A4-1 group case flips green.

**A4-4. App-owned ids, one space per todo (EPIC "app-owned entity ids").**
Order by mechanical-ness: (1) plugins — flip the 5 `id_hint: None` plugin adds
to `allocate_control_plugin_id` (rename to `allocate_plugin_id`), engine
handler rejects a present id with `AudioEvent::Error`, delete `next_plugin_id`
+ the hint-vs-base rule (`plugins.rs:282-310`) + `CONTROL_PLUGIN_ID_BASE`;
(2) sends; (3) busses (+ return-bus base folds in); (4) tracks (+ demo.rs,
templates, sub-track base folds in); (5) references; (6) **design todo**:
recording clip ids + take-group ids + import asset ids — app reserves a block
at arm/import time (`AudioCommand::ArmTrack { clip_ids: Range }` or similar),
engine never invents one; then delete `ReserveAssetIds`, `RestoreTakeGroups`'
bump, and the 15 `max(id+1)` sites. Each todo ~100-300 lines; conflict medium
(engine handlers are hot); behaviour none. Done-when as in the finding
(`grep next_[a-z_]*_id resonance-audio/src/engine` empty).

**Verdict:** NOW = A4-1 + A4-2 (+A4-3). The two-owner split is *stable* today
because the ranges are disjoint; the epic's payoff is deleting 15 bump sites and
2 reserve commands and making new entity types not re-decide ownership. It is
not urgent.

---

## ARCH-05 — no error taxonomy / logging facade

### Evidence, re-verified (counts updated; one consequence not borne out)

| Crate | `Result<_, String>` | `eprintln!` | logging crate |
|---|---|---|---|
| resonance-audio | **46** (was 41) | **36** (root 21: `recording.rs` 9, `platform.rs` 8, `stream_errors.rs` 2, `output_pipewire.rs` 2; `engine/` 13; `mixer/callback/mod.rs` **1, latched**; `types/` 1) | none |
| resonance-app | 25 | **49** (`src/*.rs` 16, `update/project_io/**` 16, `engine_events/` 9, …) | none (the review's `log::` hits were `rfd::…Dialog::new`) |
| resonance-common | 20 | 6 | none |
| resonance-plugin | 8 | 1 | none |
| wayland-plugin-gui | 0 | 6 | none |
| plugins/resonance-amp | 52 | 6 | none |
| resonance-mcp, resonance-svs | 0 | 0 | `tracing 0.1` + `tracing-subscriber 0.3` (not in the workspace deps table; `thiserror = "2"` is) |

`AudioEvent` still has four bare-string error variants — `Error(String)`
(**38 emit sites**: `engine/clips.rs` 12, `plugins.rs` 7, `transport.rs` 4, …),
`BounceError`, `TrackBounceError`, `StemExportError` — but it also already has
**nine typed precedents**: `ExportError { kind: ExportErrorKind }` (5 kinds),
`PluginLoadFailed { reason }`, `PluginScanFailed`, `ImportFailed`,
`MixMeasureError`, `FreezeError`, `ReferenceLoadFailed`,
`StemExportTargetError`, `PluginEditorError`. The app consumes `Error(String)`
in exactly one place (`engine_events/dispatch.rs:42` → `transport::error`, a
banner).

**Not borne out:** "the control layer's `ErrorKind` mapping has to string-match
or default". `update/control/` constructs `RpcError` 299 times
(`invalid_params` 187, `not_found` 60, `busy` 25, `needs_confirmation` 11,
`internal` 9, `unsupported` 7) and every one is decided *synchronously against
the app mirror before any engine round-trip*; there is no `contains("…")` on
engine text anywhere under `update/control/`. Engine failures reach a control
client only through jobs (`render.*`, `project.*`, `vocal.render`), where
`JobState::Failed` carries the string as `error`. So the taxonomy's consumer is
weaker than claimed; what remains real is: (a) no level / filter / routing for
85 stderr prints in the two big crates, (b) one RT-thread print, (c) every new
failure path re-invents formatting, (d) the `Error(String)` catch-all lets the
app only show a banner.

### Steps

**A5-1. Logging facade, no taxonomy (NOW; one commit per crate).**
Goal: `eprintln!` gone from library crates; `RUST_LOG` works; default output
unchanged.
Files: workspace `Cargo.toml` (`tracing = "0.1"`, `tracing-subscriber = { version
= "0.3", features = ["env-filter"] }` — same versions mcp/svs already pin);
`resonance-app/src/main.rs` installs
`tracing_subscriber::fmt().with_env_filter(EnvFilter::try_from_default_env().unwrap_or(LevelFilter::WARN.into())).with_writer(std::io::stderr).init()`;
then sed per crate: `eprintln!(` → `tracing::warn!(` (or `error!`/`info!` by
reading the message — most are warnings) in `resonance-audio` (36),
`resonance-common` (6), `resonance-plugin` (1), `wayland-plugin-gui` (6),
`resonance-app` (49). Tests: no subscriber → output dropped, which is what
`run-tests.py` wants; `new_for_test` may call
`tracing_subscriber::fmt::try_init()` behind `RESONANCE_TEST_LOG` if anyone
misses the prints. Add to `tools/arch-invariants`: *no `eprintln!`/`println!`
under `resonance-{audio,app,common,plugin}/src`* (allow-list: `main.rs`, the
`svs` CLI, `bin/`), and *no `tracing::` macro under `resonance-audio/src/mixer/`*
(RT rule — see A5-2).
Diff: ~1 line per site + a `use` per file ≈ 120 lines; the invariant ~40.
Conflict: **audio + common + plugin + wayland now** (none of those sites is in
M12/H3's diff except `engine/clips.rs`/`tracks.rs` — check the 3 sites there
against M12 before landing, or leave those two files for the app commit);
**app after M12 merges** (16 of its 49 sites are under `update/project_io/`,
which M12 edits). Behaviour: none by default (same text, same stderr, now with
a `WARN resonance_audio::…` prefix).

**A5-2. The one RT-thread print → engine-loop log (NOW, ~20 lines; fold into
A5-1's audio commit).**
`mixer/callback/mod.rs:171-180` `log_oversize_buffer` keeps its `AtomicBool`
latch but stores `(requested, scratch)` into two `SharedState` atomics instead
of printing; the engine loop's 16 ms tick (where H1 already reads
`cycle_report`) logs it once. Test: extend `resonance-audio/tests/cycle_load.rs`
(or the H3 `mixer` group) — render with an oversize buffer, assert the atomics
are set and nothing was printed on the render thread (the invariant test is the
static guard). Behaviour: none audible.

**A5-3. `EngineError { kind, message }` for `AudioEvent::Error` (EPIC step 1).**
New `resonance-audio/src/types/error.rs`: `EngineErrorKind { NotFound, Busy,
Unsupported, Io, Plugin, Internal }` (mirrors `resonance_control::ErrorKind` +
`Io`/`Plugin`), `EngineError::internal(msg)` so the 38 sites are one-token
changes first, classified in a second pass (`clips.rs` is mostly `NotFound`,
`plugins.rs` mostly `Plugin`). Then `BounceError`/`TrackBounceError`/
`StemExportError` carry `EngineError` (or fold into the existing
`ExportErrorKind`). `JobStatus.error` gains an optional `kind` so a control
client sees `not_found` vs `io` for a failed render. Consumers: `transport::error`
(banner keeps `.message`), `update/control/job.rs`. Diff ~150 lines. Conflict:
medium (`engine/clips.rs`, `plugins.rs` are hot; M12 touches `clips.rs`).
Behaviour: none.

**A5-4. `Result<_, String>` → `thiserror` per module (EPIC step 2).**
Audio: 20 files / 46 sites (`midi_io` 5, `midi_hardware` 5, `recording` 4,
`io/wav` 4, `types/clip` 3, `clap_host/bundle` 3, …); common: 20 sites. One
file per commit; each error type lives beside its module and converts into
`EngineError` at the event boundary. Leave `resonance-amp` (52, internal to
the NAM loader) alone. Diff ~20 lines per file. Conflict: low per file.

**Verdict:** NOW = A5-1 (audio/common/plugin/wayland crates) + A5-2 + the
invariant test; app-crate sweep right after M12 merges. Taxonomy = EPIC
"engine error taxonomy" (A5-3, A5-4 as 3 todos). The finding's severity should
be read as *consistency*, not *correctness*: nothing string-matches today.

**Progress (H6, branch `arch/H6-tracing`):** A5-1 done for resonance-audio
(32 sites), resonance-common (5), resonance-plugin (1), wayland-plugin-gui (6),
cocoa-plugin-gui (4, unbuilt on Linux beyond `cargo check`); app `main.rs`
installs the subscriber (default `warn,resonance=info,resonance_svs=warn,
wayland_plugin_gui=info,cocoa_plugin_gui=info`, no `log` bridge). Deviation:
`resonance-plugin` also depends on `tracing-subscriber` and installs the same
subscriber from `ClapBridge::new_shared` (`src/logging.rs`) — a CLAP cdylib has
its own tracing dispatcher, so without it every swept `resonance-common` /
plugin-SDK / editor-runtime event inside a bundle would be dropped. A5-2 done:
`cycle_load::OversizeBufferLatch` on `SharedState`, logged by the engine tick.
Invariants: `library_crates_log_through_tracing_not_stderr`,
`audio_callback_never_logs`. Remaining: the app sweep (49 sites), plugin crates
(amp 6 / drums 2 / ir 1 — none in `process()`, allow-listed by file), and the
cpal `err_fn` logs (`engine/mod.rs`, `platform.rs`), which on ALSA run on the
audio thread's worker — rate-limited, but still formatted there.

---

## ARCH-06 — `Resonance` and `message.rs` hub files

### Evidence, re-verified

* `message.rs` **1764 lines** (was 1821; H2 moved `Transport`/`Bus`/`Master`
  to `update/{transport,bus,master}.rs` with `pub use` re-exports —
  `message.rs:30-32`). Still defines **27 sub-enums + `DropTarget` + `Message`**
  (36 variants). 211 files import via `crate::message`, so moves stay invisible
  to importers. Handler for every enum is a single `update/<domain>.rs` file
  (`BounceMessage` is nested inside `TrackMessage::Bounce`, handled in
  `update/track.rs:438-463`; `ArrangementMessage` is handled in
  `update/compose/drum_groups.rs`).
* Churn since 2026-08-01 by enum (hunks): `PluginMessage` **8**, `UiMessage` 4,
  `Clip` 2, `Take` 2, `Arrangement`/`Track`/`Mixer`/`Viewport`/`Import`/`Pool`/
  `Relink` 1, **the other 14 zero**. Sizes: `MidiEditor` 175 lines, `Ui` 151,
  `Plugin` 137, `Clip` 119, `Track` 109, `Automation` 85, `ExternalInstrument` 82,
  `Pool` 76, `Browser` 63, `Relink` 61, `Import` 58, `Take` 49, `Group` 47, …
* `lib.rs` 997 lines; `Resonance` has **93 fields** (was 89); one `impl
  Resonance` block from `:525` (constructors + `new_for_test*`) — the
  "giant impl" half of the finding is gone, the "93 loose fields" half is not.
  Loose-field groups with reference counts in `src/`: **master** 5 fields /
  108 refs (`master_plugins` 66, also in `undo/snapshot.rs` and the control
  view model); **midi devices** 7 / 55 (`midi_*`); **plugin catalog** 3 / 29
  (`available_plugins`, `plugin_scan_failures`, `plugin_scan_in_progress`);
  **banners** 3 / 60 (`error_message` 53); **input devices** 2 / 21;
  **presets** 8 / 41; **modal/dialog** 7 / 68; **plugin mirror** 3 / 34.
  `state/` already has 30 sub-state modules — the pattern exists.
* `undo/classify.rs` 547 lines: **12 `Skip` catch-alls** (`Control`, `Viewport`,
  `Ui`, `ProjectIo`, `Export`, `Import`, `Relink`, `Browser`, `Drag`, `Group`,
  `MarkerUi`, `VocalTuning`) and **6 `Record` catch-alls** (`Pool`, `GlobalTrack`,
  `ChordTrack`, `Arrangement`, `Master`, `Take`). Since STATE-07 (M4) a `Record`
  that changes nothing is dropped by `same_state` — at the cost of a **second
  full snapshot** per message (`undo/mod.rs:158`), so a `Record` catch-all on a
  transient variant costs 2× O(project), not a bogus history entry. M12 adds 21
  lines to `classify.rs`.

### Steps

**A6-1. Move the 14 zero-churn enums beside their handlers (NOW; = A1-3 slice 2).**
Goal: `message.rs` ≈ 850 lines; the merge magnet loses half its mass.
Files: cut each `pub enum XMessage` (+ its doc comment and only the `use`s it
needs) from `message.rs` into its handler file, add `pub use
crate::update::<d>::XMessage;` in `message.rs`. Set: `Group`→`update/group.rs`,
`Marker`→`marker.rs`, `MarkerUi`→`marker_ui.rs`, `Export`→`export.rs`,
`ExternalInstrument`→`external_instrument.rs`, `Freeze`→`freeze.rs`,
`MidiClip`→`midi_clip.rs`, `MidiEditor`→`midi_editor.rs`,
`VocalTuning`→`vocal_tuning.rs`, `Automation`→`automation.rs`,
`GlobalTrack`→`global_track.rs`, `Browser`→`browser.rs`, `Drag`→`drag.rs`
(+ `DropTarget`, or `state/drag.rs` since 5 of its 7 users are state/view),
`ChordTrack`→`chord_track.rs`. **Skip this round:** `ProjectIo`
(`update/project_io/mod.rs` is in M12), `Bounce` (nested in `Track`), and the
1+-hunk enums (`Track`, `Clip`, `Mixer`, `Take`, `Pool`, `Relink`, `Import`,
`Viewport`, `Arrangement`) until M12 merges; `Ui`/`Plugin` last.
Approach: two commits of 7; each verified by `cargo check -p resonance-app
--tests` and `git diff --stat` showing only `message.rs` + the target files.
Test: compile is the test; the `io`/`timeline` group binaries exercise the
handlers. Diff: ~-900 / +950 moved, ~14 lines added. Conflict: **low** — none of
the 14 target files is in M12's or H3's diff and the enums have had zero hunks
in eight weeks. Behaviour: none.

**A6-2. Two smallest sub-states (NOW, optional).**
`state::PluginCatalog { available, scan_failures, scan_in_progress }` (3 fields,
29 refs) and `state::MidiDevices { inputs, outputs, last_refresh, clock_send_*,
clock_recv_* }` (7 fields, 55 refs). Mechanical `r.available_plugins` →
`r.plugin_catalog.available` sed over ~85 lines in `update/`, `view/`,
`engine_events/`, `lib.rs`. Before landing: `git diff master...fix/review-M12 |
grep -c 'available_plugins\|midi_'` must be 0 (M12 edits `engine_events/`).
Diff ~120 lines. Behaviour: none.

**A6-3. Remaining sub-states (EPIC "state tax", one todo each):** `MasterState`
(108 refs — touches `undo/snapshot.rs`, `serialize.rs`, `view_model`),
`Banners` (60), `InputDevices` (21), `PresetState` (41), `ModalState` (68),
`PluginMirror` (34). Each a mechanical rename of 20-110 one-liners.
Done-when: `Resonance` ≤ 40 fields.

**A6-4. Exhaustive `undo_action` per enum (EPIC, after A6-1 completes and M12
merges).** `impl XMessage { pub fn undo_action(&self) -> UndoAction }` beside
each moved enum, exhaustive match (no `_`); `classify.rs` collapses to
`Message::X(m) => m.undo_action()`. Start with the 12 `Skip` catch-all enums
(each becomes an all-`Skip` exhaustive match — that *is* the point: a new
variant fails to compile until classified), then the 6 `Record` ones (which
also stops the double-snapshot on transient variants). Diff ~-400/+500.
Conflict: `classify.rs` is in M12 → strictly after. Done-when:
`grep -nE '\(_\) => UndoAction::(Skip|Record)' undo/classify.rs` empty.

**Verdict:** NOW = A6-1 (+A6-2). Also: close or re-scope ba #1059 to A6-1's
shape; its branch is unsalvageable.

**Progress (H8a, branch `arch/H8a-message-split`):** A6-1 done — all 14 enums (+ `DropTarget` → `update/drag.rs`) moved beside their handlers with `pub use` re-exports; `message.rs` 1771 → 1051 lines.

---

## ARCH-07 — `resonance-common` is a model crate every plugin links

### Evidence, re-verified (valid, but the compile-time cost is overstated)

`resonance-common/src` = 3 999 LOC in 19 modules. **Model** (2 347 LOC, 59 %):
`take` 733, `midi_map` 334, `device_definition` 328, `automation` 270,
`freeze` 216, `device_registry` 162, `group_identity` 119,
`external_instrument` 102, `track_group` 83. **Utilities** (1 589):
`audio_probe` 402, `resample` 355, `wav` 279, `registry` 193, `atomic_file` 115,
`drum_map` 88, `factory_presets` 73, `denormal` 52, `scan` 32. Heavy deps:
`symphonia` → `wav`, `audio_probe`; `serde_json` → `device_definition`,
`registry`, `factory_presets`, `midi_map`; `dirs` → `midi_map`,
`device_registry`, `registry`; `time` → `registry`.

What plugins actually import: `flush_denormals` ×12 (**all 11 plugins** — their
only universal import), `scan_directory` (amp, ir), `registry` (drums, 4 files),
`drum_map` (drums), `decode_wav_stereo`/`_channels` (drums, ir). 8 of 11
plugins import *nothing but* `flush_denormals`. `resonance-plugin` itself
depends on common for `scan_directory` (`loader.rs:57`) and `factory_presets`
(`presets.rs:178-190`; 7 plugins use it through the SDK). All 11 plugins already
depend on `resonance-dsp`. Audio + app: **103 files** import `resonance_common`.
`cargo tree --offline -p resonance-eq --no-default-features`: `symphonia`,
`serde_json`, `dirs`, `time` all present via common.

**Correction:** `scripts/bundle.sh:108-115` builds every plugin in *one* cargo
invocation, and the suite builds the workspace — with resolver 2, `symphonia`
& co. are compiled once for the graph either way. The per-plugin *compile*
cost the finding describes only exists for a standalone `cargo build -p
resonance-eq`. The real, current costs are the **dependency surface** (a plugin
can `use resonance_common::Take` today and nothing objects) and the
**"lowest layer" rule having no home for utilities**. Both are fixable without
a new crate.

### Steps

**A7-1. Allow-list invariant (NOW, ~40 lines).**
`tools/arch-invariants/tests/architecture.rs`: new test
`plugins_reach_only_common_utilities` — every `resonance_common::<ident>` in
`plugins/*/src` and `resonance-plugin/src` must be in
`{flush_denormals, scan_directory, registry, drum_map, decode_wav_stereo,
decode_wav_channels, factory_presets, atomic_file}`; message: "DAW model types
are not plugin API — add the utility to the list or move it down". Add the
sentence to ARCHITECTURE.md's layering bullets. Exercise once (add `use
resonance_common::Take;` to a plugin → fails → revert), note it in the doc
comment like the existing tests do. Conflict: none. Behaviour: none.

**A7-2. `flush_denormals` → `resonance-dsp` (NOW, ~30 lines).**
`resonance-common/src/denormal.rs` → `resonance-dsp/src/denormal.rs` (+ `pub
use` at the dsp root); update 12 plugin sites + `resonance-audio` (1) to
`resonance_dsp::flush_denormals`; delete the module from common (no shim —
common must keep zero internal deps per the arch table). Then the 8 plugins
with no remaining `resonance_common::` import drop the manifest line (the SDK
still pulls common transitively; that is fine and A7-1 guards the source
level). Conflict: 1 line in each of 11 plugin files — low; the M-batches on
plugins are merged. Behaviour: none (same intrinsics).

**A7-3. Feature-gate the model inside common (EPIC, or NOW if a quiet slot
appears; ~60 lines).** `[features] default = ["model", "decode"]`; `model`
gates the nine model modules (+ optional `dirs`, `time`); `decode` gates `wav`
+ `audio_probe` (+ optional `symphonia`); `serde_json` stays unconditional
(`registry`/`factory_presets` need it). `resonance-plugin` and the plugins:
`default-features = false` (`features = ["decode"]` for drums, ir). Add to
A7-1: a plugin's `resonance-common` dep must set `default-features = false`.
This gives the type-level guarantee (a plugin *cannot* name `Take`) for 60
lines instead of a crate split, and makes a standalone `-p <plugin>` build
lean. Conflict: low (common: 4 commits since Sep). Behaviour: none.

**A7-4. `resonance-model` crate split (DEFER; probably never).** 2 347 LOC
moved, 103 importing files in audio/app (or a shim release), a new arch row,
ARCHITECTURE.md. A7-3 delivers the same guarantees; only do this if a second
model consumer appears (headless CLI, a `resonance-control` that wants the
types).

**Verdict:** NOW = A7-1 + A7-2. Recommend replacing the finding's crate split
with A7-3 (feature gates) as a single ba todo.

**Progress (H7, branch `arch/H7-plugin-deps`):** A7-2 done — `flush_denormals`
now in `resonance-dsp` (no shim); compressor, delay, eq, gate,
granular-delay, mastering, reverb, wavetable dropped `resonance-common`.
A7-1 done — `plugins_reach_only_common_utilities` (allow-list
`PLUGIN_COMMON_ITEMS`: scan_directory, registry, drum_map,
decode_wav_stereo, decode_wav_channels, factory_presets; scans every target
of each plugin + `resonance-plugin`; `atomic_file` left out as unused) and
`only_listed_plugins_depend_on_resonance_common` (`PLUGINS_ON_COMMON` = amp,
drums, ir). A7-3 still open.

---

## ARCH-09 — undo snapshots deep-copy the whole project per edit

### Evidence, re-verified (valid; one thing got *worse* since the review)

`snapshot_for_undo` (`undo/snapshot.rs:178-237`) = `build_project_file(self)`
+ a `Vec<MidiNote>` clone per MIDI clip + **7 remaining `UndoExtras` fields**
(H2 removed `clip_fade_gain`, `compose_arrangements` and persisted
`chord_track`; left: `compose_derived_clips`, `compose_next_derived_clip_id`,
`vocal_clip_lyrics`, `automation_lanes`, `reference`, `track_freeze`,
`external_instruments` + `_devices`) + a `Vec<u8>` clone per live plugin blob
from `plugin_state_cache`. **New since the review:** STATE-07's no-op detection
(`undo/mod.rs:158` → `same_state`, `snapshot.rs:101-140`) takes a *second* full
snapshot at every gesture commit and compares note vectors element-wise — every
gesture now costs 2× O(project). Coalescing (`record_coalesced`) and the control
compound (`with_compound_undo`, one snapshot per `notes.insert_many`) already
bound the burst cases the finding worried about.

`DEFAULT_HISTORY_CAPACITY = 200` still lives in `resonance-audio/src/limits.rs:67`,
re-exported by `resonance-audio/src/lib.rs:53` and `resonance-app/src/undo/mod.rs:22`,
consumed only in `undo/history.rs:9,68` (no test imports it from audio).
`MidiClipState.notes: Vec<MidiNote>` (`state/clips.rs:169`) — ~135 read sites,
**~23 mutation sites in 13 files** (`engine_events/midi.rs` 8, `compose/regenerate.rs` 2,
`project/io.rs` 2, `demo.rs` 2, 9 singles). `LoadedProject.midi_notes:
HashMap<ClipId, Vec<MidiNote>>` (`project/model.rs:894`) used in 8 files;
`plugin_state_cache: HashMap<_, Vec<u8>>` 17 refs / 13 files;
`LoadedProject.plugin_states` 4 refs; `SaveCollector.plugin_states: Vec<(_, Vec<u8>)>`.

### Steps

**A9-1. Move `DEFAULT_HISTORY_CAPACITY` into the app (NOW, ~10 lines).**
`resonance-app/src/undo/history.rs` gets `pub const DEFAULT_HISTORY_CAPACITY:
usize = 200;`; delete `limits.rs:67`, `resonance-audio/src/lib.rs:53` (and the
comment at `lib.rs:6`); `undo/mod.rs:22` re-exports from `history`. Test:
`cargo check -p resonance-app --tests`; `tests/timeline/undo_history.rs` pins
capacity behaviour. Conflict: M12 adds 10 lines to `undo/mod.rs` (additive,
different region); H3 will edit audio `lib.rs` (one-line removal, trivially
rebased either way). Behaviour: none.

**A9-2. Plugin blobs as `Arc<[u8]>` (NOW, ~40 one-liners).**
`plugin_state_cache: HashMap<PluginInstanceId, Arc<[u8]>>` (`lib.rs`),
`LoadedProject.plugin_states` likewise, `SaveCollector.plugin_states` converts
once at the `PluginStatesSaved` echo (`.into()`), `snapshot_for_undo`'s
`collect` clones become refcount bumps, `same_state` compares
`Arc::ptr_eq(a, b) || a == b`. Files: `undo/snapshot.rs` (5),
`engine_events/plugins.rs` (5), `update/plugin_replace.rs` (2),
`update/project_io/{serialize,instantiate,mod}.rs`, `replay/entity.rs` (2),
`replay_diff.rs`, `state/tracks.rs`, `test_support/mixer_plugins.rs`,
`engine_events/{presets,project_io}.rs`, `project/model.rs`, `project/io.rs`.
Test: extend `tests/timeline/undo_history.rs` — load a plugin with a 1 MB fake
blob (the `test_support/mixer_plugins.rs` fakes), record 50 note edits, assert
`Arc::ptr_eq` between consecutive snapshots' blobs (deterministic, no memory
measurement). Conflict: low — `engine_events/project_io.rs` and `project/io.rs`
are in M12 (one line each; land after M12 or accept a trivial rebase).
Behaviour: none. This is the part that matters for NAM/IR/wavetable blobs
(KB–MB each × 200 entries).

**A9-3. Notes as `Arc<Vec<MidiNote>>` (EPIC step).**
`MidiClipState.notes` and `LoadedProject.midi_notes` values become
`Arc<Vec<MidiNote>>`; the ~23 mutation sites use `Arc::make_mut`; the ~135
reads compile unchanged through `Deref`; `same_state` short-circuits on
`Arc::ptr_eq` before `midi_notes_equal`. Done-when test: snapshot a 2 000-note
project 200 times after editing one clip and assert every *other* clip's notes
are `ptr_eq` across snapshots. Diff ~80 lines / ~20 files. Conflict: medium —
`engine_events/midi.rs` and the compose/vocal paths are app-vocal territory.
Behaviour: none.

**A9-4. Delta snapshots from the `Reconcile` diff (EPIC, only after ARCH-01
step 3).** Unchanged from the finding: not before, two diff engines would be
worse than one deep copy.

**Verdict:** NOW = A9-1 + A9-2. A9-3 goes into the "state tax" epic next to
the extras removal (A1-2 (3)-(8)), because once `UndoExtras` is gone
`same_state` is `ProjectFile == ProjectFile` + Arc pointer checks and the
second snapshot becomes cheap for free.

---

## ARCH-01 / ARCH-02 — remaining steps (from `arch-migration-plan.md`, post-H1/H2)

ARCH-01: A1-2 (3) `external_instruments` → read `ProjectTrack.external_instrument`;
(4) `vocal_clip_lyrics` (normalise padding first — H2 note); (5)
`automation_lanes`; (6) `track_freeze` (keep UPD-05 baselines — FU-H2b); (7)
`reference` (split content vs monitor state); (8) `compose_derived_clips` +
counter (call `rebuild_derived_clips` on the fast path — FU-H2a); (9b) delete
`UndoExtras`, `pending_undo_extras`, `finalize_undo_restore`; then the
`Reconcile` trait (epic). A1-3 remainder = A6-1 above.
ARCH-02: A2-4 `midi_clips` → `RenderGraph` via `retire::publish` (H1's
primitive); A2-5 `busses`+`master`; A2-6 `tracks`; A2-7 `plugins`; A2-8 `clips`
(+ internal engine-thread message for the load/freeze/analysis workers); A2-9
the 500-clip hammer test. Per H1's correction the offline-path dropout is moot
since MIX-02's gate; **read `SharedState::lock_misses` from a real session
before scheduling A2-4** — if the per-map counters stay at zero under normal
editing, the whole series is engine-thread latency hygiene, not a dropout fix,
and belongs behind everything below.

---

## Next 10 architecture steps, in priority order (across all ARCH items)

| # | Step | Files | Size | Conflict | Why here |
|---|---|---|---|---|---|
| 1 | **A6-1** move 14 zero-churn message enums beside their handlers (2 commits) | `message.rs`, 14 `update/*.rs` | ~900 lines moved, 14 added | low; none in M12/H3 | halves the merge magnet; pure move; enables A6-4 |
| 2 | **A9-1 + A9-2** history capacity into the app; plugin blobs `Arc<[u8]>` | `undo/{history,mod,snapshot}.rs`, audio `limits.rs`/`lib.rs`, ~12 blob sites | ~50 lines | low (2 one-line touches in M12 files) | the one ARCH-09 cost that bites today (stateful plugins × 200) |
| 3 | **A7-1 + A7-2** plugin allow-list invariant; `flush_denormals` → dsp; drop 8 manifest deps | `tools/arch-invariants`, `resonance-dsp`, 11 plugin files | ~70 lines | low | closes the layering hole for good; 8 plugins stop naming common |
| 4 | **A5-1 (audio/common/plugin/wayland) + A5-2 + invariant** tracing facade, RT print off the callback | `Cargo.toml`, `main.rs`, ~50 sites, `mixer/callback/mod.rs`, `SharedState` | ~150 lines | low now (skip `engine/clips.rs`/`tracks.rs` until M12) | `RUST_LOG` for the first time; the RT rule becomes a test |
| 5 | **A4-1 + A4-2 (+A4-3)** id-collision invariant test; `state/ids.rs` one table; FU-A1c | `tests/io/id_allocation.rs`, `state/ids.rs`, 4 state files, `lib.rs` seeds | ~230 lines | low (0 commits since Sep on those files) | pins the partition before anyone moves a space; fixes an open FU |
| 6 | **A1-2 (3)(4)(5)** three more extras read from `ProjectFile` | `undo/snapshot.rs`, `replay_diff.rs`, `replay/entity.rs` | 3 commits, ~-70/+40 | low-medium (`replay_diff.rs` not in M12) | fixed-point test already guards; each shrinks `same_state` |
| 7 | **A5-1 (app crate)** the 49 app `eprintln!` sites | `resonance-app/src/**` | ~60 lines | **after M12** (`update/project_io/`) | completes the facade; invariant flips to all four crates |
| 8 | **A6-2 (+Banners)** `PluginCatalog`, `MidiDevices`, `Banners` sub-states | `lib.rs`, ~145 refs | ~170 one-liners | **after M12** (`engine_events/`) | 13 fields off `Resonance`; mechanical |
| 9 | **A6-4** exhaustive `undo_action` for the 12 `Skip` catch-all enums | `classify.rs`, the moved enums' files | ~-250/+300 | **after M12 + #1** | new variants can no longer be silently non-undoable |
| 10 | **A2-4** `midi_clips` onto `RenderGraph` (first map) | `engine/mod.rs`, `midi/{clips,live}.rs`, `play.rs`, `render_core.rs`, harnesses, ~27 tests | ~400 lines | medium; **after H3** (touches audio tests) and only if `lock_misses` shows misses | proves the ARCH-02 pattern end to end |

After these: A1-2 (6)(7)(8)(9b) + `Reconcile` (epic "state tax"), A9-3 Arc
notes (same epic), A6-3 remaining sub-states (same epic), A7-3 feature gates
(one todo), A5-3/A5-4 taxonomy (epic "engine error taxonomy"), A4-4 (epic
"app-owned entity ids", 5 todos + 1 design todo), A2-5…A2-9 (epic "engine
render-graph publishing").

### Epics to file in ba

1. **state tax: one declarative project model** — A1-2 (3)-(9b), `Reconcile`,
   A6-3, A6-4 remainder, A9-3, A9-4. (Absorbs #1059 after re-scoping.)
2. **engine render-graph publishing** — A2-4…A2-9, gated on counter evidence.
3. **engine error taxonomy** — A5-3, A5-4 (audio), A5-4 (common).
4. **app-owned entity ids** — A4-4 (1)-(5) + the recording/import design todo.
5. Single todo, not an epic: **resonance-common feature gates** (A7-3).

### Progress note — H8b (2026-09-26): ARCH-09 A9-1/A9-2, ARCH-04 A4-1/A4-2/A4-3, FU-A1c

Landed on `arch/H8b-undo-ids` (four commits). Corrections to the plan above:

* **A9-2, `same_state` cost.** The blob `Arc` change is as planned. The
  gesture-end check no longer builds a second snapshot
  (`Resonance::gesture_changed_since`: notes compared in place, extras
  against a small fresh capture, only the `ProjectFile` rebuilt), but the
  probe (`tests/timeline/undo_history.rs::snapshot_cost_probe_on_demo_project`,
  debug build, demo + 6 × 1 MiB blobs) shows what is left is not the
  copies: `build_project_file` 283 µs + two `serde_json::to_value` 167 µs
  each, against a gesture check of 692 µs (was 822 µs) and a snapshot of
  289 µs (was 425 µs). The serialized compare exists only because the
  `ProjectFile` tree has no `PartialEq`; deriving it needs ~20 one-line
  derives across `project/{model,sections}.rs`, `compose/{drumroll/*,
  generate,lane_generator}.rs`, `state/markers.rs`,
  `resonance-audio/src/types/tempo/map.rs` and
  `resonance-music-theory/src/generator/mod.rs` (`GeneratorSpec`) — tried,
  reverted as out of scope for a minimal-diff step; it is the cheap half
  of A9-3 and would make `same_state` a struct walk.
* **A4-1 found a real collision the plan called stable.** "The two-owner
  split is stable today because the ranges are disjoint" held only for
  plugins: `AddTrack`/`AddInstrumentTrack`/`AddVocalTrack`/`CreateSubTrack`,
  `AddBus` and `SetAuxSend` bumped the engine counter past *any* hint, so
  one control `track.add`, one Cmd-G group and one GUI "Add track" put a
  track on the group's id (a group is app-only; the engine cannot skip
  it). Fixed by applying the plugin rule to those paths (bump only for
  hints below the app base); the four bases now live in
  `resonance-audio/src/types/mod.rs` and `state/ids.rs` re-exports them.
  The "in-flight echo" window for busses/sends is closed by the same rule.
* **A4-3 shape.** `allocate_sub_track_id` moved from `TrackRegistry` to
  `Resonance::allocate_track_id` (it needs the group registry); 11 call
  sites (the plan counted 3 copies + group.rs; `update/track.rs` had 3).
* A4-4 remains the epic; the invariant test is the one to move each space
  against.
