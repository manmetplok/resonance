# Code review — findings & fix todo (2026-09-26)

Whole-workspace, read-only review of `master` @ `09041eee` by 10 parallel agents
(9 × Opus per area, 1 × Fable for architecture). **145 findings: 30 high,
67 medium, 48 low.** Findings were traced by reading code; almost none were
confirmed with a failing test — **the fixer's first step is always to write the
reproducing test described under *Verification*, and watch it fail.**

## Fix campaign status (high-severity pass, started 2026-09-26)

Agents work in isolated worktrees; the orchestrator merges each batch into
master and updates this table. Agents do **not** edit this file.

| Batch | Items | Model | Status | Merge |
|---|---|---|---|---|
| A1 project open/load | STATE-01/UPD-01, UPD-02, STATE-04 | opus | merged | 46c87319 |
| A2 recording+plugin undo | STATE-02, STATE-03 | opus | merged | d47a3916 |
| B compose sections | VIEW-03, VIEW-04, VIEW-05 | opus | merged | 1f1ae03d |
| C editor input | VIEW-01, VIEW-09, VIEW-02, CTL-02 | opus | merged | 302009d3 |
| D misc view | VIEW-06, VIEW-07, VIEW-08, VIEW-10 | opus | merged | bb45ccad |
| E control beat units | CTL-01 | opus | merged | 5f63160a |
| F1 playhead + render exclusivity | MIX-01, MIX-02 (=ENG-05) | fable | merged | 2c811666 |
| F2 CLAP host / recording | ENG-01, ENG-02, ENG-03 | opus | merged | eac3b10c |
| G1 drums timing | DSP-01 | opus | merged | 50dad7db |
| G2 wavetable | DSP-02, DSP-03 | opus | merged | b598d2e9 |
| G3 resampler | LIB-01 | opus | merged | ac640b82 |
| H architecture | ARCH-01, ARCH-02, ARCH-03 | fable | planned → `arch-migration-plan.md`; NOW-steps queued behind M3 (audio) and M4 (undo) to avoid conflicts | |
| M1 plugin framework (medium) | PLG-01, PLG-02, PLG-03, PLG-04 | opus | merged | e996ad33 |
| M2 DSP (medium) | DSP-04, DSP-05, DSP-06, DSP-07, DSP-08, DSP-09, DSP-10 | opus | merged | c28a8a5c |
| M3 mixer (medium) | MIX-03, MIX-05, MIX-06, MIX-07, MIX-08, MIX-09 | opus | merged | f7ad84e2 |
| M4 app state (medium) | STATE-05, -06, -07, -09, -13, CTL-03, UPD-03, UPD-04, UPD-05 | opus | merged | 7701e28a |
| M5 control API (medium+low) | CTL-04..10, CTL-12, CTL-13, UPD-11 | opus | merged | d3b41d37 |
| M6 theory + small plugins | LIB-02..LIB-09 | opus | merged | 26e1e316 |
| V1 view perf + scrolling (medium) | VIEW-11, -14, -21, -22, -23, -26, -27, -28 (+FU-D1 if time) | opus | merged | 0a8cf2fa |
| V2 compose view (medium) | VIEW-12, -13, -15, -16, -17, -19, -20, -24, -25 | opus | merged | 47e86611 |
| M7 DSP lows + follow-ups | FU-M2a, FU-M2b/DSP-12, DSP-11, -13, -14, -15, -16, FU-G2c | opus | merged | a7660033 |
| M8 plugin framework lows | PLG-05..10, ENG-10, ENG-12, FU-M1b, FU-M1c | opus | merged | 28bf0279 |
| V3 view lows + playhead follow | FU-D1/D2, VIEW-33, FU-V1a, VIEW-29/UPD-10, VIEW-30, VIEW-32, VIEW-36 | opus | merged | 57a940cb |
| M9 engine export/bounce | ENG-04, -06, -07, -08, -09, -13 | opus | merged | c7a9e298 |
| M10 socket/presets/test hygiene | CTL-11/UPD-12, STATE-14, FU-D5, STATE-15, MIX-11 | opus | merged | 32f76e9b |
| M11 vocal pipeline | UPD-08, VIEW-31, VIEW-34, VIEW-35, FU-V2d | opus | merged | 500d58ee |
| M12 autosave + undo leftovers | UPD-07, STATE-11, STATE-12, STATE-08, STATE-10, VIEW-18, UPD-09, FU-M6a | opus | merged | 213505c2 |
| H4 SDK leakage + invariant tests | ARCH-08, ARCH-10 | fable | merged | d1cdaa66 |
| H5 plan ARCH-04/05/06/07/09 | planning (read-only) | fable | done → arch-migration-plan.md Part 2 | |
| V4 import dialog + app follow-ups | VIEW-25/FU-V2a, FU-M11a, FU-M4b, FU-C2, FU-C3, FU-V2b, FU-V3a, FU-V3b | opus | merged | 6f170c1b |
| H6 logging facade | ARCH-05 A5-1 (audio/common/plugin/wayland) + A5-2 | opus | merged | 582c4fcb |
| H7 plugin dep trim | ARCH-07 A7-1 + A7-2 | opus | merged | 423ea205 |
| H8a message enums beside handlers | ARCH-06 A6-1 | opus | merged | a6a0a6e9 |
| H8b undo blobs + id allocation | ARCH-09 A9-1/2, ARCH-04 A4-1/2/3, FU-A1c | fable | merged | 4f274847 |
| L1 app logging sweep | FU-H6a, FU-H6c | opus | merged | b8952f64 |
| C1 app/control follow-ups | FU-E1, E2, M5a/b/c, B2, B3, V2c, V3c, V4a, V4b, A2c, M6b, V1b, M12c | opus | merged | ec04990b |
| A5 audio follow-ups 2 | FU-D3, D4, F2b, M3a, M3c, A4a, A4b, A4c | opus | merged | a2a53c5f |
| U1 undo follow-ups | FU-A1a, A2a, A2b, M4c, H2a, H2b, H2c | opus | merged | b611a4f5 |
| R1 autosave crash recovery | FU-M12a, FU-M12b (rest) | opus | merged | d2be52fc |
| T1 flaky tests + small items | FU-A5a, FU-A5c, FU-P1a, FU-H3a | opus | merged | 51000f97 |
| V5 vocal data loss + canvas leftovers | FU-C1a, FU-C1b, FU-V2c, FU-V3c | opus | merged | 141b3f76 |
| F1 final follow-ups | FU-G2d, FU-F1c, FU-R1a, FU-M11b | opus | merged | 58dcbbb0 |
| V6 undo audio + compose culling | FU-V5b, FU-V5a, FU-V2c (rest) | opus | merged | 06c90633 |
| P1 plugin follow-ups | FU-G2a, FU-G2b, FU-G2d, FU-M6c, FU-M6d, FU-G1, FU-M2c | opus | merged | e6d7dc93 |
| A4 audio follow-ups | FU-M4a, FU-M8b, FU-F1a, FU-F1b, FU-G3a, FU-G3b, FU-F2a, FU-M3b, FU-H6b, FU-M12b(part) | opus | merged | b9a5e1a4 |
| H1 ARCH-02 NOW steps | A2-1 per-map try_read miss counters, A2-3 off-lock compute, A2-2 deferred-drop retire queue (= MIX-04) | fable | merged | f615e46c |
| H2 ARCH-01 NOW steps | A1-1 snapshot fixed-point test, A1-2 drop redundant UndoExtras, persist chord_track | fable | merged | 9cb5803b |
| H3 ARCH-03 NOW steps | A3-4, A3-5 `test-internals` feature, A3-1 group resonance-audio tests | opus | merged | 6755fbb7 |
| refactor-intent A-1 (ARCH-01) | A1-2 (3) external_instruments + devices from ProjectFile; slow-path double restore removed | opus | merged | 37ae683a |
| refactor-intent A-2 (ARCH-01) | A1-2 (4) vocal_clip_lyrics from ProjectFile; canonical file/live lyric forms | opus | merged | cb850b0c |
| refactor-intent A-3 (ARCH-01) | A1-2 (5) automation_lanes from ProjectFile; slow-path double restore removed | opus | merged | 5479f8cf |
| refactor-intent A-4 (ARCH-01) | A1-2 (6) track_freeze from ProjectTrack.freeze; slow-path undo now deletes an undone freeze's cache | opus | merged | 8e10c24c |
| refactor-intent A-5 (ARCH-01) | A1-2 (7) reference content/monitor split; engine re-sync on both undo paths; `null` LUFS load fix | opus | merged | db57a50c |
| refactor-intent A-8 (ARCH-09) | A9-3 cheap half: `PartialEq` on ProjectFile tree; gesture check 677 → ~287 µs | sonnet | merged | cc2fd3b4 |
| refactor-intent A-10 (ARCH-06) | A6-4 exhaustive `undo_action` per enum + invariant; bounce-dialog / drum-manager UI variants Record → Skip | opus | merged | fd8a3c6e |
| refactor-intent A-12a (ARCH-06) | A6-2 batch 1: PluginCatalog, MidiDevices, Banners, InputDevices; `Resonance` 90 → 79 fields | sonnet | merged | 62086b6b |
| refactor-intent A-12b (ARCH-06) | A6-2 batch 2: PresetState (8), ModalState (7); `Resonance` 79 → 66 fields | sonnet | merged | 7c9af9de |
| refactor-intent E (ARCH-07) | A7-3 `model`/`decode` features in resonance-common; plugins set `default-features = false` (invariant) | sonnet | merged | 38d66942 |
| refactor-intent A-11 (ARCH-06) | A1-3 remainder: 11 enums moved beside handlers, `message.rs` 1084 → 416 | sonnet | merged | d8bd59b7 |
| refactor-intent C-1 (ARCH-05) | A5-3 `EngineError { kind, message }`; 39 emit sites classified (NotFound 5, Busy 2, Io 15, Plugin 4, dynamic 1, Internal 12) | sonnet | merged | 8339eecd |
| refactor-intent C-2 (ARCH-05) | A5-3 second half: BounceError carries ExportErrorKind, Track/Stem bounce errors carry EngineError; `JobStatus.error` = `{message, kind?}` | sonnet | merged | 4f3342dd |

**Campaign result (2026-09-26, full suite green: 369/369 binaries @ 06c90633):** 138/145 findings fixed; 7 open — all architecture items, each with its first steps landed (see `arch-migration-plan.md`); 89 follow-ups done, 5 open (macOS-only or needing a product decision).

**Remaining, needs a human:**
- macOS: FU-M1a, FU-M8a, FU-M8c, FU-H4a — Cocoa changes (PLG-01/03/05, async destroy) are type-checked only; run `cargo check` + the three ignored Cocoa tests on a Mac.
- Product decision: FU-B1 — section resize re-rolls generated chord/vocal lanes (hand edits lost). Keep, warn, or preserve?
- Sound change to confirm: LIB-06 compressor release now matches the knob (was ~1.5–2.5× longer) — revert `cea54427` if unwanted.
- Architecture epics → **`refactor-intent.md`** (self-contained hand-off for fresh agents): engine render-graph publishing (ARCH-02 A2-4+), state-tax / Reconcile (ARCH-01 rest + ARCH-06 A6-2..4), engine error taxonomy (ARCH-05 A5-3/4), app-owned entity ids (ARCH-04 A4-4), `Arc<Vec<MidiNote>>` + `PartialEq` on ProjectFile (ARCH-09 A9-3), feature-gate model/decode in resonance-common (ARCH-07 A7-3).

### Follow-ups found while fixing (new todos)

- [x] **FU-A1a** — fixed @317341e6; (low) `io.pending_open_path` is a single slot: two overlapping GUI opens → first load adopts the second path. Control opens are busy-guarded; GUI isn't. Fix: tag the pending path with a load token and match it in `ProjectLoaded`.
- [x] **FU-A1b** (medium, = UPD-03) — fixed @137c320a; edits during `io.loading` are still acked then wiped by replay.
- [x] **FU-A1c** — fixed @0c09001c (`Resonance::allocate_track_id` skips group ids); (low) `allocate_sub_track_id` (pool.rs, control `track.add`) is still unaware of group ids; relies on load-time counter bump. Fix: collision-check against the group registry too.
- [x] **FU-G2a** — fixed @25f5da5c (−79 dB worst bass); (low) wavetable: ~−69 dB aliasing floor below ~70 Hz at 44.1/48k from table interpolation; needs better interpolation or bigger low tables.
- [x] **FU-G2b** — fixed @20ee064c; (low) wavetable: above ~C9 the top mip still aliases (no darker table exists).
- [x] **FU-G2c** — fixed @96e5d02a; (low) wavetable golden `render_block_regression::lfo_sh_hpf` peaks at 1.2e-3 — near-silent, nearly vacuous; raise its level.
- [x] **FU-G2d** — done @28f7a5c0 + @7bccf43c (held-note stack); (low) wavetable mono legato steals + retriggers the envelope (glides, but not true non-retrigger legato).
- [ ] **FU-B1** (low, NEEDS USER DECISION — preserve hand edits vs re-roll) section resize re-rolls chord/vocal lanes from seed → hand edits to generated notes are lost (same as a chord change); vocal lanes re-render.
- [x] **FU-B2** — fixed (C1); (low) `remove_bars` leaves stale vocal-audio map entries for placements it deletes (clips themselves are removed).
- [x] **FU-B3** — fixed (C1); (low) vocal WAVs are never garbage-collected after section/placement delete.
- [x] **FU-F2a** — fixed @6f682cc6; (low) carried CLAP note events wait for the instrument's next `process()`; if the tail sub-block skips that instrument the note is late, and a carried note-on can land after a Stop panic.
- [x] **FU-F2b** — fixed @8a534f5d; (low) ENG-03 salvage rewrites the WAV header in place — may fail on CoW filesystems (btrfs) when the disk is full; then no clip.
- [x] **FU-F2c** — fixed (M8); (low) ENG-12 should reuse the new `activate_and_start()` in `clap_host/state.rs`; `restart()` still bails on inactive instances.
- [x] **FU-E1** — fixed (C1); (medium) `update/control/import_midi.rs:275` rounds imported clip length with `time_sig_num * TICKS_PER_QUARTER_NOTE` — wrong for x/8 meters; use `tempo_map.bar_len_ticks_at(bar)`.
- [x] **FU-E2** — fixed (C1); (low) `clip_split` tool description says `at` accepts `{seconds}/{samples}` but `PositionSpec` has only `sample` — fix description or add the variant.
- [x] **FU-G1** — fixed @3b9eac74; (low) drums: `render_block` alone still starts voices at frame 0; sample-accurate callers must use begin/span/end.
- [x] **FU-C1** (medium, = UPD-11) — fixed @b1357803; global `keyboard::listen()` shortcuts (Enter, B, Cmd-Z) still fire while typing; use the `any_text_input_focused` probe pattern.
- [x] **FU-C2** — fixed @0408640f; (low) timeline Delete now needs a prior click on the timeline (KeyFocus); a clip selected via control API/other widget isn't deletable by key until clicked.
- [x] **FU-C3** — fixed @71423884; (low) expanded compose editor's `+`/`-`/Escape still use hover gating, not KeyFocus.
- [x] **FU-A2a** — fixed @a7198323; (low) a take landing mid-drag: undoing the drag also drops the take (redoable).
- [x] **FU-A2b** — fixed @6c7e1ee8; (low) plugin state-blob cache isn't refreshed after param edits; quick-restore undo re-sends every non-default param (+ a PluginParamText echo each).
- [x] **FU-A2c** — fixed (C1); (medium) live MIDI recording: `close_open_recordings` sets note durations at Stop without an event → app mirror keeps zero-length held notes.
- [x] **FU-A2d** — fixed @93f57443 (= STATE-08); (low, = STATE-08 remainder) engine can still reuse clip ids after a full-reload undo.
- [x] **FU-F1a** — fixed @be70a1a9; (low) if the engine refuses Play as a backstop (e.g. external MIDI-clock master), app mirror `transport.playing` stays true until Stop (banner shows).
- [x] **FU-F1b** — fixed @e9d8bd26; (low) `measure_mix` acquires its render guard on the worker → sub-quantum window where Play lands and the transport appears to start then stall. Move acquire to the engine thread.
- [x] **FU-F1c** — fixed @fe9401a7 (progress modal + cancel); (low, UX) WAV mixdown now blocks GUI traffic like bounce-in-place but has no modal, only the master-strip label.
- [x] **FU-G3a** — fixed @115dd5f1 (freeze cache resampled at attach; rubato kept for export: 6× faster, cleaner — measured); (low) `mixer/render/frozen.rs` still linear-resamples freeze caches on rate mismatch; `bounce/resample.rs` uses rubato → two resampler implementations; consolidate on `resonance_common::resample`.
- [x] **FU-G3b** — fixed @82a7c5b0; (low) no end-to-end test of a loop-record seam at a mismatched device rate (seam flush only covered at resampler level).
- [x] **FU-D1** — fixed @68f4563d; (medium, feature gap) Arrange never followed the playhead (old `auto_follow_playhead` wrote a dead offset, now removed). Implement via scrollable id + `scrollable::scroll_to` from tick using the visible viewport width.
- [x] **FU-D2** — fixed @68f4563d; (low, cleanup) dead horizontal-scroll plumbing: `h_scrollbar_grab`, `ScrollToX`, `scroll_to_x`, `viewport.scroll_offset`.
- [x] **FU-D3** — fixed @d6680afc; (low) engine `handle_set_bpm` clamps 20..999 and passes NaN; use `sanitize_bpm` there too.
- [x] **FU-D4** — fixed @956cbb05; (low) MIDI-imported tempo points > 300 BPM are now clamped at bar-table rebuild.
- [x] **FU-D5** — fixed @5869f48f; (low, test hygiene) `track_preset_save_prompt` golden reads the real user preset dir (cf. STATE-14).
- [ ] **FU-M1a** (medium, needs macOS) Cocoa changes for PLG-01/03/05 (+ M8's async destroy) were never compiled: run `cargo check` + `editor_open_cocoa`, `cocoa-plugin-gui editor_size`, `modal_reentrancy` with `-- --ignored` on a Mac.
- [x] **FU-M1b** — fixed (M8); (low) now that `on_main_thread` runs, a plugin reporting latency while inactive triggers `restart()` on an inactive instance → spurious "failed to reactivate" error; narrow double-restart race.
- [x] **FU-M1c** — fixed (M8); (low) Wayland re-map waits ≤200 ms for configure then paints anyway (Hyprland quirk) — could be a protocol error on strict compositors; button held across hide stays pressed in egui.
- [x] **FU-M3a** — fixed @22de6f89; (low) MIX-09: >32-ch device whose capped request is rejected now fails to open input (was: crash-prone).
- [x] **FU-M3b** — fixed @c63ad91f; (low) MIX-06: every lock-contended block causes a flush on the next block → sustained notes can be cut during heavy UI edits.
- [x] **FU-M3c** — fixed @512942dc; (low) MIX-05: muted key sources keep rendering (CPU cost while muted).
- [x] **FU-F2d** (medium, upgraded) — fixed @829d19e9 (30/30 under CPU load); `bounce_plugin_lock` timing test fails 3/5 standalone — make it deterministic.
- [x] **FU-M2a** — fixed @0319fb91; (medium) DSP-10 partial: linear-phase EQ FIR design still runs on the audio thread (≤1/hop, now crossfaded). Plan: per-EQ design worker + lock-free request slot + double-buffered spectrum; fall back to inline design at hop boundary if result not ready (deterministic for bounce).
- [x] **FU-M2b** — fixed @06e2803f; (medium) mastering multiband crossover lowpass: fixed 4097-tap FIR, hard swap, allocates a Vec on the audio thread on crossover move (= DSP-12) — give it the EQ treatment.
- [x] **FU-M2c** — fixed @6d8e1fa9 (bench: HQ +24 st = 3.9% of budget; left as is); (low, perf) granular HQ sinc read ≈80 taps/grain-sample at +24 st (vs 6) — benchmark; bypassed mastering now costs full CPU.
- [x] **FU-M5a** — fixed (C1); (low) CTL-05: repeated `insert_bars` can still push content past MAX_BARS (per-call check only).
- [x] **FU-M5b** — fixed (C1); (low) CTL-12 remainder: lockstep doesn't check param / plugin-param ids in skills.
- [x] **FU-M5c** — fixed (C1); (low) control API: accept string param `key` ids for plugin params (mastering skill currently uses display names).
- [x] **FU-M6a** — fixed @4ee7b10f; (medium) `resonance-app/src/project/io.rs` has its own `atomic_write` copy with the fixed `.tmp` name + leak — make it call `resonance_common`'s.
- [x] **FU-M6b** — fixed (C1); (low) svs: unknown phonemes now fail the render (was: silent token 0); g2p paths that skip voicebank substitution will surface errors.
- [x] **FU-M6c** — fixed @2af72aee; (low) EQ band kind change restarts its stages from zero (can click on loud material); optional ~5 ms crossfade.
- [x] **FU-M6d** — fixed @6433c3ac; (low) flaky `resonance-eq` `analyzer_teardown::spectrum_workers_are_joined_on_reinitialize_and_drop` (thread count under load).
- [x] **FU-M4a** — fixed @6146d7b8; (low) UPD-04: a stale asset finishing import after a project switch still lands (unplaced) in the new project's pool — tag queued imports with a project epoch.
- [x] **FU-M4b** — fixed @12279270; (low) control `generate.*`/`harmony.*` edits on frozen tracks only mark them stale; consider refusing like GUI edits.
- [x] **FU-M4c** — no bug; documented + pinned @c75d5dad; (low) STATE-07: gestures that change only un-snapshotted state now record no undo entry.
- [x] **FU-V1a** — fixed @78487a42; (low) `snap_sample_to_grid_tempo` single-tempo shortcut uses the transport numerator (follows playhead) — wrong after a signature change with one tempo point; ruler shares the shortcut.
- [x] **FU-V1b** — not a bug (browser input ignored with no project open; pinned by test); (low, test infra) `app.update(Message::Browser(SetFilter))` didn't apply the filter in tests while `test_dispatch` did — investigate.
- [x] **FU-V1c** — fixed @68f4563d; note: FU-D1 (playhead follow) should be done together with FU-D2 (dead scroll plumbing), storing the outer Scrollable's live x offset.
- [x] **FU-V2a** — fixed @14f059e0; (medium) VIEW-25 partial: MIDI Import modal now parses off-thread + has a file chooser, but Confirm is still a no-op, Review has no Import button, TempoConflict is a placeholder (doc #158 follow-ups); needs `undo/classify.rs` to stop classifying `Message::Import(_)` as Skip.
- [x] **FU-V2b** — fixed @895ae569; (low) existing chords are not revalidated after a global signature change; control `edit_tempo_event` can still move an event past neighbours.
- [x] **FU-V2c** — done @514f0c13 + @0199bca0; (low) 100 000-bar sections are accepted but Compose views loop every bar per frame; section lengths loaded from project files aren't validated.
- [x] **FU-V2d** — fixed @accf5caf; (low) a vocal render that finishes after its placement moved into a different tempo region is placed right but rendered at the old tempo.
- [ ] **FU-M8a** (low) with async Cocoa destroy, `editor_size`/`editor_open_cocoa` teardown watchdogs pass trivially — make them wait for the main-thread teardown.
- [x] **FU-M8b** — fixed @c2fb20fc; (low) `bounce/render.rs` ignores `reset_processing()`'s bool → a plugin that stays dead after export is silent without an error.
- [ ] **FU-M8c** (low) Cocoa runtime still lacks the PLG-10 bounded `Editor::new` and the held-button release across hide.
- [x] **FU-H2a** — fixed @55c68584; (low) undo fast path rebuilds `compose.derived_clips` while slow path copies extras → a derived clip whose `MidiClipCreated` echo hasn't landed is dropped on fast-path undo (narrow race).
- [x] **FU-H2b** — fixed @a6e17050; (low) slow-path undo calls `freeze.reset()` → restored Frozen tracks lose their UPD-05 content baseline (blind spot until next freeze).
- [x] **FU-H2c** — fixed @bdc6479b; (low) `restore_performance` runs only on the slow path; `vocal_clip_lyrics` padding differs between paths (file-identical).
- [x] **FU-V3a** — fixed @dac46f8c; (low) `update/project_io/replay/mod.rs:161` resets `viewport.scroll_offset = 0` on load → can disagree with the real Scrollable until its next report.
- [x] **FU-V3b** — fixed @4907ebb1; (low, UX) playhead follow has no on/off switch; after a manual scroll it only resumes at next playback.
- [x] **FU-V3c** — done (C1 + @ecbd728b); (low) `update/compose/section.rs:508` removes clips without recomputing pool usage; `vocal_lane::view` builds a HashMap per call; `TimelineCanvas::scroll_offset` (always 0) still threaded through draw sites.
- [x] **FU-M11a** — fixed @d27ee2ab; (low) a vocal render returning `Ok(None)` (no voicebank → MIDI-only fallback) maps to `Message::Tick`: a `vocal.render` job can still hang and the `in_flight_render` entry stays.
- [x] **FU-M11b** — mitigated @f4bcdc02 (tempo-mismatch warning; per-placement render not done; MCP song.vocal doesn't report it yet); (low) a section's vocal is rendered at one tempo (first placement's); placements in other tempo regions / intra-section tempo changes aren't handled.
- [ ] **FU-H4a** (low) `resonance-gate`'s macOS-only dev-dep on `cocoa-plugin-gui` (NSApplication pump for `editor_open_cocoa`) — re-export a test_support pump from `editor_host` instead.
- [x] **FU-H4b** — done (A3-2); note: new crates need a row in `tools/arch-invariants` `allowed_internal_deps`; A3-1 (audio test grouping) should add its root list there (A3-2).
- [x] **FU-M12a** — done (R1): session marker, recovery modal, untitled recovery at startup, settings UI; control `project.open` gets `autosave_available` / `recover_autosave`; (medium) autosave crash detection + recovery prompt (#466/#467 on `ba/epic-32`, ~1000 lines) and the autosave settings UI (#471) not ported — need their own todos.
- [x] **FU-M12b** — done (A4 + R1); (low) [dir-scan part fixed @78fd7021] autosave of a never-saved project uses the real user cache dir; backup side-file folder can be orphaned by a crash mid-backup; `SetProjectDir` folder scan runs on the engine command thread.
- [x] **FU-M12c** — fixed (C1); (low) possibly flaky: `io preset_name_collisions::a_file_holding_another_preset_is_never_overwritten` failed once under full-suite load.
- [x] **FU-H3a** — done @d0f5ab01 (crate-level allow kept by design: ~150 items; doc link + counts fixed); (low) crate-level `allow(dead_code, unused_imports)` when `test-internals` is off — reviewer may prefer per-item cfg gating; broken intra-doc link to gated `RolledAudioTake` from `project/take_audio.rs`; stale binary counts in run-tests.py docstring / CLAUDE.md.
- [x] **FU-H6a** — fixed @359709b8; (medium) ARCH-05 app sweep: convert resonance-app's ~49 `eprintln!` to tracing, then remove resonance-app's exemption from `library_crates_log_through_tracing_not_stderr`.
- [x] **FU-H6b** — fixed @8bd64f1d; (low) cpal error callbacks in `engine/mod.rs` / `platform.rs` still format+log on the ALSA audio worker thread (rate-limited; outside the `mixer/` invariant) — route via atomics like the oversize latch.
- [x] **FU-H6c** — fixed @359709b8; (low) default tracing filter string duplicated in `resonance-app/src/main.rs` and `resonance-plugin/src/logging.rs`; log lines now carry `LEVEL target:` prefixes.
- [x] **FU-V4a** — fixed (C1); (low) MIDI import: "match time" rescales against the project tempo at the import point only; "use file tempo" replaces the whole tempo map from bar 1 even for playhead placement; placement controls (start at playhead, merge target) have no UI.
- [x] **FU-V4b** — fixed (C1); (low) chord revalidation runs only on signature-changing messages, not on project load.
- [x] **FU-A4a** — fixed @ab066bb5; (low) a carried note-on is dropped when a panic had to be parked (busy lock); MIDI-clip lock contention during piano-roll edits still flushes held notes.
- [x] **FU-A4b** — fixed @6dbbeeec; (low) after a project switch, the import modal rows of a stale pool batch get no final event — verify the app clears them on load.
- [x] **FU-A4c** — fixed @b40d19d9; (low) mismatched-rate freeze cache conversion runs on the engine command thread (~2.7 ms per audio second); backend stream-error text no longer logged (kind only).
- [x] **FU-P1a** — fixed @2b659ca7 (bit-exact; was compile-time powi folding); (low, test infra) EQ `dsp_golden` differs in last bits between debug and release — always bless under the debug profile the suite uses (or make the comparison tolerant).
- [x] **FU-A5a** — fixed @5461de45; (medium, flaky) `resonance-audio` `io::import_audio_to_pool::a_pool_import_outlived_by_its_project_never_lands_after_clear_all` failed once under full-suite load (passes 6/6 standalone) — make its timing deterministic.
- [x] **FU-A5b** — fixed (C1); (low) import dialog doesn't show `ImportedSmf::tempo_points_clamped` yet (small hunk in `update/import.rs`).
- [x] **FU-A5c** — fixed @22757a50; (low) FU-M3c only checks the keyed plugin's own bypass (not chain-level FX bypass / muted consumer); frozen tracks play the live chain during async cache conversion (~100s of ms).
- [x] **FU-C1a** — fixed @ec2cf6a2 (only unreferenced `vocal_*.wav` are ever unlinked); (HIGH, data loss) after a project reload the vocal-clip map points at `clip_<id>.wav`; re-rendering a vocal calls `unlink_if_exists(old_path)` in `vocal_audio_install` and deletes the file the saved project and undo snapshots reference.
- [x] **FU-C1b** — documented, intentionally not deleted @94da144c; (low) stray `vocal_*.wav` files from before the project-audio-dir fix remain in sibling `audio/` folders.
- [x] **FU-R1a** — fixed @791719d1 (untitled recovery still startup-only); (low) stale autosave files inside project dirs are never deleted; marker-less scratch dirs (autosave racing Save As) are never GC'd; untitled recovery only offered at startup; non-Linux can't tell a live other instance from a crash.
- [x] **FU-V5a** — fixed @63bc0a9a; (low) `clip_*.wav` files are never garbage-collected (removed vocal clips leave theirs) — disk only.
- [x] **FU-V5b** — fixed @561a4bff (engine persists clip WAVs at undo snapshot); (medium, pre-existing) slow-path undo reloads audio from `clip_<id>.wav`, which only exists after a save → undoing a vocal re-render (or any unsaved audio clip edit) in a never-saved-since session can come back silent. Consider writing clip WAVs eagerly to the project audio dir or keeping them in memory for undo.

## How to use this file

- Each item has an ID (`AREA-NN`), severity, confidence, `file:line` locations,
  a concrete failure scenario, a suggested fix and a verification plan.
  Line numbers are as of `09041eee` — re-locate by symbol if they drift.
- Tick `[x]` when fixed and append `— fixed @<commit>`. If a finding turns out
  wrong, tick it and append `— invalid: <reason>`.
- Follow CLAUDE.md test rules: `Resonance::new_for_test*()`, add modules to the
  existing group binaries in `resonance-app/tests/` (never a new top-level
  file), no inline `#[cfg(test)]`, re-bless goldens with `RESONANCE_BLESS=1`,
  never run crate-wide `cargo fmt`.
- VIEW-11..36 rest on sub-reviewer traces that the VIEW reviewer did not
  re-read personally — verify before fixing.

## Duplicates & shared root causes — fix these together

| Group | Items | Note |
|---|---|---|
| Failed project open repoints the current project | **STATE-01 = UPD-01** | Found independently by two reviewers → very likely real. One fix. Data-loss: do first. |
| Offline render shares live plugin instances | **MIX-02 = ENG-05** (+ UPD-06) | Need one "render in progress" gate honoured by mixer, `handle_play`, and app-side I/O. |
| Socket parent dir `chmod` through symlinks / `/tmp` fallback | **UPD-12 = CTL-11** | One fix. |
| Autosave never wired on master | **UPD-07**, STATE-11 | Stranded on `ba/epic-32`; STATE-11 lists bugs to fix *before* wiring it. |
| Shortcuts ignore text-input / editor focus | **VIEW-01, VIEW-09, UPD-11** | One focus-gating mechanism in the key subscription, not three patches. |
| Stale index after re-sort | **VIEW-02, CTL-02** | Track notes by stable id (or re-find after sort) in both drag and `notes.edit`. |
| Freeze goes stale silently | **UPD-05, ENG-08** | Missing invalidation on compose/tempo/automation edits + freeze ignores automation. |
| Stuck notes | **ENG-01, MIX-06, MIX-08, MIX-10** | Sort/clamp events, detect playhead jumps, flush voices. MIX-06 fix builds on MIX-01. |
| Plugin reset / tails in export | **ENG-04, ENG-07, DSP-05** | Call CLAP `reset`, add master tail, keep bypassed stages consistent. |
| Undo-contract drift | **STATE-02, STATE-03, STATE-07, STATE-13, CTL-03** | Consider after/with ARCH-01/ARCH-09. |
| DC blocker / SR-dependent constants | **DSP-04, DSP-06, DSP-11** | Derive coefficients/lengths from sample rate. |

## Suggested order

1. **Data loss / corruption:** STATE-01/UPD-01, STATE-02, ENG-03, VIEW-06, VIEW-04, STATE-05, STATE-06, STATE-09, MIX-02/ENG-05, STATE-04.
2. **Wrong-edit bugs users hit daily:** VIEW-01, VIEW-02/CTL-02, VIEW-03, VIEW-09, VIEW-08, STATE-03, CTL-01, MIX-01.
3. **Audio correctness:** ENG-01 + stuck-note group, DSP-01, DSP-02, DSP-03, LIB-01, ENG-02, MIX-03, MIX-05.
4. **Control-API contract:** UPD-02, UPD-03, CTL-03..07.
5. Remaining medium, then low; architecture items as incremental migrations alongside.

## High-severity index

| ID | Title |
|---|---|
| STATE-01 / UPD-01 | Failed project open repoints current project → saves overwrite the other project |
| STATE-02 | Finished recording not dirty / not in undo → take lost on close |
| STATE-03 | Undo of plugin param/preset (fast path) doesn't restore params |
| STATE-04 | Track-group id counter not advanced past loaded groups → collisions |
| UPD-02 | `project.open` job reports done before load completes |
| MIX-01 | Audio thread playhead store overwrites concurrent Seek/Stop |
| MIX-02 | Live mixer processes plugin instances during offline render |
| ENG-01 | CLAP note events unsorted/unclamped → stuck notes at loop seams |
| ENG-02 | Failed `LoadPluginState` leaves plugin permanently deactivated |
| ENG-03 | WAV write error mid-recording discards whole take silently |
| VIEW-01 | Delete while editing notes deletes the whole MIDI clip |
| VIEW-02 | Note drag past neighbour moves the neighbour |
| VIEW-03 | Drum cell click toggles wrong step with non-zero phase |
| VIEW-04 | Deleting section/placement leaves generated clips playing |
| VIEW-05 | Section resize never re-derives clips |
| VIEW-06 | "nan" BPM collapses clips and makes project unloadable |
| VIEW-07 | "Save as preset…" prompt never renders |
| VIEW-08 | Inspector BYP button frozen (lazy fingerprint misses fields) |
| VIEW-09 | Vocal-roll shortcuts fire while typing lyrics |
| VIEW-10 | Clip/fade/loop drags jump after auto-follow scroll |
| CTL-01 | Wire "beat" means meter beats out, quarter notes in |
| CTL-02 | `notes.edit` move + duration/velocity edits the wrong note |
| DSP-01 | Drums ignore note-event sample offsets |
| DSP-02 | Wavetable ignores `max_voices`, breaks legato glide |
| DSP-03 | Wavetable mip selection aliases |
| LIB-01 | All sample-rate conversion is linear interpolation (no AA filter) |
| ARCH-01 | Per-feature state tax (one field → 8+ files, shadow project file) |
| ARCH-02 | Audio callback shares RwLock'd maps with control thread |
| ARCH-03 | Test-binary sprawl in resonance-audio; internals leaked as pub |

## Counts by area

| Area | Prefix | High | Med | Low |
|---|---|---|---|---|
| App state / undo / persistence | STATE | 4 | 5 | 6 |
| App update loop / engine events / control socket | UPD | 2 | 8 | 2 |
| App view / compose | VIEW | 10 | 19 | 7 |
| Control API / MCP / agent plugin | CTL | 2 | 5 | 6 |
| Audio mixer / RT path | MIX | 2 | 7 | 2 |
| Audio engine / CLAP host / I/O | ENG | 3 | 6 | 4 |
| Plugin framework / GUI runtimes | PLG | 0 | 4 | 6 |
| DSP core + big plugins | DSP | 3 | 7 | 6 |
| Small plugins / metering / theory / svs / common | LIB | 1 | 2 | 6 |
| Architecture | ARCH | 3 | 4 | 3 |

Side notes from the reviewers (not todos): the crate DAG matches
ARCHITECTURE.md; the control API is a genuine façade over `update()`; plugin
registration is down to one place (the "3 registration spots" memory is stale);
K-weighting/BS.1770 gating, RBJ biquads, the NAM loader, the control wire
framing and the CLAP state stream were checked and found correct.

---

## App state / undo / project persistence

### [x] STATE-01 — A failed project open leaves `project_path` and the engine's project dir pointing at the project that failed, so saves, recordings and undo then write into it or read from it — fixed @6a22ff1f
- **Severity:** high
- **Confidence:** high
- **Category:** data-loss
- **Location:** `resonance-app/src/update/project_io/mod.rs:114-119` (`OpenPathSelected`), `:121-138` (`OpenRecent`), `:195-201` (`ProjectLoaded(Err)`); related: `resonance-app/src/undo/snapshot.rs:252` (`project_dir: self.io.project_path`), `resonance-audio/src/engine/clips.rs:812-900` (`handle_save_clips_to_project_dir`), `resonance-audio/src/engine/clips.rs:193-196` (imports write to `project_dir/audio/clip_{id}.wav`)
- **Problem:** `OpenPathSelected` and `OpenRecent` set `r.io.project_path = Some(path)` and send `AudioCommand::SetProjectDir(path)` *before* the async `load_project` runs. `ProjectLoaded(Err)` only sets `error_message`. It never restores the previous path or engine dir. The old project stays loaded in the app and engine, but everything that is keyed on the path now targets the project that failed to open.
- **Failure scenario:** The user has project A open. They open project B, whose `project.json` is newer than this build or has a JSON error. B could also be a folder picked by mistake ("No project.json found"). The error is shown and A is still on screen. Three things now go wrong:
  - (a) Ctrl+S runs `start_save`, which targets B. The engine copies A's clips to `B/audio/clip_{id}.wav`, overwriting B's WAVs that share those ids. A's MIDI files and plugin blobs are written over B's, and B's `project.json` is replaced. Project B is destroyed.
  - (b) Even without a save, the next recording or import writes `B/audio/clip_{next_id}.wav`, because the engine's project dir is B.
  - (c) Every undo snapshot taken after this carries `project_dir = B`. A structural (slow-path) undo then runs `LoadClipFromWav` against B's audio dir, so A's clips come back silent or play B's audio.
- **Suggested fix:** Do not change `project_path` or the engine dir until the load succeeds. Carry the path inside the load result: return `(PathBuf, LoadedProject)` from `load_project_task`, or keep a `pending_open_path`. Set `project_path` and send `SetProjectDir` in the `ProjectLoaded(Ok)` arm. In the `Err` arm, leave both untouched, or re-send `SetProjectDir(previous)` if the engine dir has to be set early. Apply the same change to the control `project.open` path, which goes through `OpenPathSelected`.
- **Verification:** Add a module to `resonance-app/tests/io` (for example `open_failure_keeps_path.rs`, included from the io group binary). Build the app with `Resonance::new_for_test_with_capture()` and set a project path P1. Dispatch `OpenPathSelected(Some(P2))`, then `ProjectLoaded(Err("x"))`. Assert that `project_path == P1` and that no `SetProjectDir(P2)` command was captured (or that a later one restores P1).

### [x] STATE-02 — A finished recording never marks the project dirty and is not in the undo history, so closing loses the take and an unrelated undo removes it — fixed @c69296db (one coalesced undo entry per recording session)
- **Severity:** high
- **Confidence:** high
- **Category:** data-loss
- **Location:** `resonance-app/src/engine_events/clips.rs:112-159` (`recording_finished`); `resonance-app/src/undo/mod.rs:56-58` (the only `dirty = true` in the crate); `resonance-app/src/undo/classify.rs` (`TransportMessage::Record`/`Stop` → `Skip`); `resonance-app/src/update.rs:157-166` (quit prompt is gated on `dirty`); also `engine_events/takes.rs` (`take_captured`) and live MIDI recording
- **Problem:** `dirty` is only set in `record_undo`, which runs only for messages classified as undoable. Recording starts and stops through `Skip` transport messages. The clip lands through the `RecordingFinished` engine event, which pushes into `r.clips` but sets neither `dirty` nor an undo entry. Cycle-record takes (`TakeCaptured`) and live MIDI recordings behave the same way.
- **Failure scenario:**
  - (1) The user opens a saved project that is not dirty, records a vocal take, and closes the window. `WindowCloseRequested` sees `dirty == false` and exits without a prompt. The take is missing from `project.json`: the WAV is orphaned on disk and gone from the project.
  - (2) The user moves a fader (undo entry S1 is recorded without the take), records a take, then presses Ctrl+Z to undo the fader move. The target snapshot has no take clip, so the restore takes the slow path and `ClearAll` drops the recording. The take is only in the redo stack now, and the next edit clears redo, so the take is gone for good. The user only asked to undo a fader move.
- **Suggested fix:** In `recording_finished`, `take_captured` and the MIDI-record completion handler, set `r.dirty = true`, bump `r.revision`, and push an undo entry. Two ways to get the pre-recording snapshot: record `UndoAction::Record` on `TransportMessage::Record` so that snapshot is the pre-take state, or capture a snapshot at record start and `undo.record()` it when the recording finishes. Either way, undoing the recording is then its own step. A cheaper minimal fix is `r.dirty = true` plus `self.undo.record(pre_record_snapshot)`.
- **Verification:** Add a module to `resonance-app/tests/timeline`. With `new_for_test()` and a project path set, deliver `AudioEvent::RecordingFinished` through the test event hook and assert `app.is_dirty()`. Record a fader edit, deliver `RecordingFinished`, send `Message::Undo`, and assert that the recorded clip id is still present, or that undo first removes only the take.

### [x] STATE-03 — Undoing a plugin parameter change or preset recall (fast path) does not restore the parameters and pushes a stale state blob — fixed @d7c6074d
- **Severity:** high
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-app/src/update/project_io/replay_diff.rs:780-800` (`push_all_plugin_states`), `:432-514` / `:602-628` (`apply_track`/`apply_bus` only copy `plugin_name` and bypass); `resonance-app/src/update/plugin.rs:123-158` (`SetPluginParam` / `LoadPluginPreset`); `resonance-app/src/undo/snapshot.rs:170-177` (the blob cache is refreshed only on plugin add, editor close and save)
- **Problem:** Changing a parameter never changes the project's structure, so its undo always takes `try_diff_replay`. That path never reads `ProjectPlugin::params` from the target snapshot:
  - It does not send `SetPluginParam`.
  - It does not update `PluginSlotState::params[..].current_value`.
  - It pushes `LoadPluginState` with the cached blob. That blob comes from `plugin_state_cache`, which is refreshed only on plugin add, editor close and save, not after a `SetPluginParam`.

  The slow path is correct here because it re-applies the snapshot params after the blob (`apply_pending_param_overrides`). The fast path has no equivalent. `handle_load_plugin_state` emits no param echo, so the GUI mirror stays stale for good.
- **Failure scenario:** The user adds an EQ, whose blob D is cached with all parameters at default. They set param A from 0 to 5 in the generic panel (entry 1), then param B from 0 to 3 (entry 2). They undo once. The engine reloads blob D, so both A and B go back to 0, although only B should. The GUI still shows A=5 and B=3. The user then saves: `build_project_file` writes params A=5, B=3 from the mirror, plus the engine's blob. On reload those params override the blob, so the undone edit comes back. `LoadPluginPreset` undo behaves the same way.
- **Suggested fix:** In `try_diff_replay`, for every slot, diff `a.params` against `b.params` from the two `ProjectFile`s. For each changed id, send `SetPluginParam` and set `slot.params[..].current_value`. A parameter missing from `b` means it is back at default, so use `default_value`. Do this *after* `push_all_plugin_states`, so the explicit values win over the stale blob, as the slow path does. Also consider refreshing `plugin_state_cache` (`SavePluginState`) when a coalesced param run ends, so the blob is not older than the snapshot.
- **Verification:** Add a module to `resonance-app/tests/plugins`. Use `new_for_test_with_capture()`, a project path, and a plugin slot with two params (use the existing `test_support/mixer_plugins.rs` helpers). Dispatch two `SetPluginParam` calls for different params, then `Message::Undo`. Assert that the mirror shows the first param still at its new value and the second back at its old value, that a `SetPluginParam` for the second param was captured, and that `test_build_project_file()` agrees.

### [x] STATE-04 — New track-group ids can collide with groups loaded from a project: the counter is never advanced past loaded group ids — fixed @69d5eec0
- **Severity:** high
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-app/src/update/group.rs:77` (`allocate_sub_track_id` for a group id); `resonance-app/src/state/tracks.rs:556-564` (checks for collisions against `tracks` only); `resonance-app/src/update/project_io/replay/mod.rs:276-286` (bumps `next_sub_track_id` past *track* ids only); `resonance-app/src/state/track_group_registry.rs:271-280` (`add_group_new` calls `insert` unconditionally and overwrites)
- **Problem:** Group ids and app-allocated track ids share the `next_sub_track_id` counter, which starts at 1_000_000_000 in a fresh session. The load path advances the counter past the ids in `project.tracks` but not past the ids in `project.track_groups`, and `allocate_sub_track_id` only skips ids used by tracks. After a fresh start and a project open, the next allocation can return an id that a loaded group already uses.
- **Failure scenario:**
  - Session 1: create two tracks, press Cmd-G to get group 1_000_000_000, save, quit.
  - Session 2: open the project. The counter stays at 1e9 because both track ids are small.
  - (a) Press Cmd-G on two other tracks. `create_group_from_selection` gets 1e9, and `add_group_new` does `groups.insert(1e9, …)`, which silently replaces the saved group (members, name, macro mute/solo/level).
  - (b) Or drop an audio file on the timeline. `pool.rs:207` or a control `track.add` allocates 1e9 for a real track, so a track id now equals a group id. Membership walks (`groups.contains_key(&m)`) then treat the track as a nested group, and `is_parent_group` and `indent_depth` return wrong results.
- **Suggested fix:** In `restore_track_groups` (both the slow path and `apply_track_groups`), advance `r.registry.next_sub_track_id` past every `tg.id`. Also make `allocate_sub_track_id` skip ids present in `r.track_groups`. That needs the registry to know the group ids, or a group-aware allocator wrapper on `Resonance`. Make `add_group_new` refuse, or debug-assert, on an existing id.
- **Verification:** Add a module to `resonance-app/tests/mixer` (grouping). Replay a `ProjectFile` that has a group with id 1_000_000_000 and two tracks, select two tracks, dispatch `GroupMessage::CreateGroupFromSelection`, and assert that the registry holds two groups and the original is unchanged.

### [x] STATE-05 — Deleting a track leaves its MIDI clips, automation lanes and group memberships behind; they are saved and reattach to a later track that reuses the id — fixed @45c8ab2d
- **Severity:** medium
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-app/src/engine_events/tracks.rs:79-141` (`removed`: prunes sends, sidechain routes, audio `clips` and sub-tracks only); `resonance-audio/src/engine/tracks.rs:173-234` (the engine `RemoveTrack` removes audio clips only, not `midi_clips`); `resonance-app/src/update/project_io/serialize.rs:313, 440, 458` (serializes `midi_clips`, `track_groups` and `automation.lanes` wholesale); `resonance-audio/src/engine/tracks.rs:465` (`ClearAll` resets `next_track_id = 1`)
- **Problem:** On `TrackRemoved` the app keeps several things that reference the deleted track id:
  - Every `MidiClipState` on that track (`r.midi_clips` is never filtered by track).
  - Every automation lane targeting `TrackGain/TrackPan/TrackMute(id)`, `DeviceParam{track: id}` or `PluginParam` of the track's plugins.
  - The id inside `track_groups` member lists.

  All of these are written to `project.json`. On reload, the engine's `next_track_id` becomes max(saved ids)+1. If the deleted track had the highest id, the next new track gets that same id and inherits the orphaned MIDI clips, automation lanes and group membership.
- **Failure scenario:** Track 5 (the highest id) has MIDI clips and volume automation. The user deletes it with confirm and saves. Invisible clips remain in `project.json` and `song_notes` over MCP still lists them. The user reopens the project and adds a new instrument track. The engine gives it id 5, and the old MIDI clips appear on it and play through its instrument. The old volume automation drives its fader. If track 5 was in a group with macro mute, the new track is muted.
- **Suggested fix:** In `engine_events::tracks::removed`, for the track and each sub-track:
  - Drop its `r.midi_clips` entries and send `DeleteMidiClip` for each, because the engine does not remove them. Also drop their `compose.vocal_audio.clip_lyrics` and `derived_clips` entries.
  - Remove the automation lanes whose target references the track or its plugin instance ids, and send `ClearAutomationLane` for each.
  - Remove the id from every group's `ordered_members`.
  - Remove its `external_instruments` entry and its freeze status.

  Serialization could also filter out entities whose `track_id` is not in the registry, as a defensive second layer.
- **Verification:** Add a module to `resonance-app/tests/timeline`. Use `new_for_test_with_capture()`, add a track with a MIDI clip and a `TrackGain` lane, and deliver `AudioEvent::TrackRemoved`. Assert that `test_build_project_file()` has no MIDI clip, lane or group member referencing that id, and that `DeleteMidiClip` and `ClearAutomationLane` were captured.

### [x] STATE-06 — Saving and reloading a MIDI clip loses overlapping same-pitch notes and notes whose velocity rounds to 0 — fixed @4c60a044 (notes stored in project.json; .mid kept as fallback)
- **Severity:** medium
- **Confidence:** high
- **Category:** data-loss
- **Location:** `resonance-app/src/project/io.rs:66-71` (save via `midi_io::encode_midi`), `:258-270` (load via `read_midi_file`); `resonance-audio/src/midi_io.rs:101-127` (`build_note_track`), `:296-352` (`notes_from_track`)
- **Problem:** MIDI clips are saved only as `.mid` files. The reader tracks one pending note-on per (channel, key):
  - When two same-pitch notes overlap (A: 0–960, B: 480–1440, both C4), the events sort to `A.on@0, B.on@480, A.off@960, B.off@1440`. B's note-on overwrites A's pending start, A's note-off closes a note 480–960, and B's note-off finds nothing pending. Two notes come back as one wrong note.
  - Separately, `(velocity * 127).round()` turns any velocity below about 0.0039 into a NoteOn with velocity 0. MIDI reads that as a note-off, so the note is dropped entirely.

  Undo snapshots keep notes in memory, so the loss only shows after a save and reopen.
- **Failure scenario:** In the piano roll, the user places C4 at beat 1 (length 2 beats) and C4 at beat 2, which overlaps. This is common after Quantize, Humanize or a MIDI import. They save and reopen, and one note is gone while the other has the wrong start and length. Vocal clips are hit harder, because `vocal_lyrics` is indexed by note position and every lyric after the lost note shifts by one.
- **Suggested fix:** Either persist notes losslessly in `ProjectMidiClip` (a notes array in JSON, since there is no back-compat concern) and keep the `.mid` file only as an interchange export, or fix the codec:
  - In `notes_from_track`, keep a FIFO `Vec` of pending starts per (ch, key) and close the oldest first.
  - In `build_note_track`, sort note-offs before coincident note-ons at the same tick.
  - Clamp the encoded velocity to at least 1.
- **Verification:** Add a round-trip test to the io group binary (`resonance-app/tests/io`): `save_project` with a clip containing overlapping same-pitch notes and one note with velocity 0.001, then `load_project`, and assert that the notes are equal field by field (use `midi_notes_equal`).

### [x] STATE-07 — Clicking a clip without dragging records an empty undo entry, clears redo, marks the project dirty and bumps the control revision — fixed @71dc5aef
- **Severity:** medium
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-app/src/undo/history.rs:207-212` (`commit` records unconditionally); `resonance-app/src/undo/classify.rs:375-383`; `resonance-app/src/view/timeline/input/pointer.rs:259-266, 663-672` (every press on a clip body emits `StartClipDrag`, every release emits `EndClipDrag`); `resonance-app/src/undo/mod.rs:56-58, 70-78`
- **Problem:** The doc comment on `commit` says "Called at gesture end when the state actually changed", but nothing checks for a change. A press and release on a clip is a `Begin` and a `Commit`. The commit calls `record()`, which clears the redo stack. `record_undo` also sets `dirty` on both the `Begin` and the `Commit`, and bumps `revision` on the `Commit`. The same happens for loop, tempo and breakpoint drags that don't move, and for trim, fade and gain handles.
- **Failure scenario:** The user undoes three edits, then clicks a clip to select it, planning to redo. The redo stack is now empty, a no-op entry sits on top of the undo stack (the next Ctrl+Z appears to do nothing), and the title bar says "unsaved". An MCP client polling `revision` sees an edit that did not happen and re-reads the song state.
- **Suggested fix:** Make `commit` compare state before recording. The cheap check is to compare `build_project_file(self)` plus extras against the pending snapshot's file: derive `PartialEq` on `ProjectFile`, or compare serde_json values. Drop the pending snapshot when they are equal. Move the `dirty` flag and the revision bump to after that check: skip them on `Begin`, and on `Commit` apply them only when something was recorded. `update_inner` would need `record_undo` to return a richer result.
- **Verification:** Add to `resonance-app/tests/timeline`: with a project path set, record one edit, dispatch `Message::Undo`, then dispatch `StartClipDrag` and `EndClipDrag` with no update in between. Assert that `can_redo()` is still true, `is_dirty()` is unchanged, and `revision()` is unchanged.

### [x] STATE-08 — After a slow-path undo, the engine hands out clip ids again; because recording does not clear redo, the redo snapshot can point at a WAV that a new take has overwritten — fixed @93f57443
- **Severity:** medium
- **Confidence:** medium
- **Category:** data-loss
- **Location:** `resonance-audio/src/engine/tracks.rs:465-467` (`ClearAll` resets `next_clip_id`/`next_track_id` to 1); `resonance-audio/src/engine/clips.rs:643, 677` (replay only bumps past the snapshot's ids); `resonance-audio/src/recording.rs:632` (a recording writes `audio/clip_{id}.wav`); `resonance-app/src/undo/snapshot.rs:293-318` (slow path via `ClearAll`)
- **Problem:** The slow-path restore resets the engine allocators and re-seeds them from the *target* snapshot. Ids of clips created after that snapshot become free again, even though redo snapshots still refer to those clips and to their `audio/clip_{id}.wav` files. Recording does not clear redo (see STATE-02), so a new take can take a reused id and overwrite the WAV that a redo snapshot depends on.
- **Failure scenario:**
  1. Add track T (entry U1).
  2. Record take 1 on T. It becomes clip 7, written to `clip_7.wav`.
  3. Undo "add track". The slow path drops T and clip 7, and `next_clip_id` becomes 7. The redo stack holds the state with clip 7.
  4. Add another track through a control call and record take 2. It becomes clip 7 and overwrites `clip_7.wav`.
  5. Redo. Clip 7 comes back with take 2's audio, and take 1 is lost.

  The same reuse after reopening a project overwrites the WAVs of deleted clips that versioned backups still reference (see STATE-12).
- **Suggested fix:** Make the app the authority for a session-wide high-water mark. After a slow-path replay, send the engine the maximum id ever issued (a new `AudioCommand::ReserveIds { next_clip_id, next_track_id }`), or have `ClearAll` for undo keep the allocators and reset them only on a disk load. After a disk load, seed the value from the ids of the `audio/clip_*.wav` files on disk. Fixing STATE-02 (recording clears redo) also closes the specific redo path.
- **Verification:** Add to `resonance-app/tests/timeline` with `new_for_test_with_capture()`: record an entry, simulate `RecordingFinished` for clip 7, undo into the slow path, and assert that the replay commands include an id-reservation command whose `next_clip_id` is at least 8. The audio side needs an engine-level test in `resonance-audio/tests`.

### [x] STATE-09 — Save completion clears `dirty` even when the user edited while the files were being written — fixed @de388a2b
- **Severity:** medium
- **Confidence:** high
- **Category:** data-loss
- **Location:** `resonance-app/src/engine_events/project_io.rs:63-110` (`build_project_file` is called when the save finishes, then an async write); `resonance-app/src/update/project_io/mod.rs:139-170` (`ProjectSaved(Ok)` sets `r.dirty = false` unconditionally)
- **Problem:** The project JSON, MIDI and blobs are captured in `try_finish_save`. The write then runs asynchronously, and each file does an fsync plus a directory fsync in `atomic_write`, which can take hundreds of ms for many clips and plugins. Any edit that lands between the capture and `ProjectSaved(Ok)` sets `dirty = true`, and the completion then clears it.
- **Failure scenario:** The user presses Ctrl+S on a large project and immediately nudges a fader or deletes a clip. The save completes, the title shows "saved", and closing asks nothing. The last edit is not in `project.json` and is lost. The same happens with `quit_after_save`.
- **Suggested fix:** Store the `revision` at capture time in the `SaveCollector`, or in the `ProjectSaved` message. On `Ok`, clear `dirty` only if `r.revision` still equals it.
- **Verification:** Add to `resonance-app/tests/io`: drive a save to `try_finish_save` (via the test hooks that the existing autosave test in `tests/io/autosave_write.rs` uses), dispatch an undoable edit, then deliver `ProjectSaved(Ok(()), false)`, and assert `is_dirty()`.

### [x] STATE-10 — An undo or redo pressed before the engine echo arrives snapshots a stale app mirror, and the late echo then undoes the undo — fixed @5d2b559e
- **Severity:** low
- **Confidence:** medium
- **Category:** concurrency
- **Location:** `resonance-app/src/undo/snapshot.rs:529-556` (`try_undo`/`try_redo` snapshot `self` as the current state); `resonance-app/src/update/clips.rs:15-20` (GUI `DeleteClip` only sends the command, and the mirror drops the clip on the `ClipDeleted` echo); `resonance-app/src/update/tick.rs:32` (the idle tick that drains engine events is 200 ms); likewise `RemoveTrack`
- **Problem:** Several GUI edits change the app mirror only when the engine echo arrives. If Undo runs before that echo is drained, the snapshot still contains the deleted entity. The structural check then sees "no change" and takes the fast path, which re-adds nothing, and the late `ClipDeleted` echo removes the clip after the undo. The control API path avoids this with read-your-own-writes (`control/clip.rs:643-648`), but the GUI path does not.
- **Failure scenario:** On an idle transport the event drain runs every 200 ms. The user selects a clip and presses Delete then Ctrl+Z quickly (under 200 ms). The undo reports success, then the echo removes the clip. The clip is gone, and the undo entry has moved to redo, so a second Ctrl+Z undoes an older edit.
- **Suggested fix:** Drain pending engine events at the start of `try_undo` and `try_redo` (call the same drain the tick uses) before taking `snapshot_for_undo`. Alternatively, apply the mirror mutation optimistically in the GUI `DeleteClip` and `RemoveTrack` handlers, as the control path does.
- **Verification:** Add to `resonance-app/tests/timeline` with a captured engine: dispatch `ClipMessage::DeleteClip(id)` and `Message::Undo` without draining, then deliver `AudioEvent::ClipDeleted`. Assert that the clip is present after the undo.

### [x] STATE-11 — Autosave, which nothing triggers yet, writes the shared MIDI and plugin files into the real project folder, and a manual save that interrupts it can finish using the autosave's results — fixed @bde466ad
- **Severity:** low
- **Confidence:** high
- **Category:** data-loss
- **Location:** `resonance-app/src/project/io.rs:30-43, 48-78` (`save_autosave` writes `midi/clip_*.mid` and `plugins/plugin_*.bin` into the canonical dir); `resonance-app/src/update/project_io/mod.rs:287-335` (`begin_save`: a manual save replaces an in-flight autosave collector); `resonance-app/src/message.rs:1286` (`ProjectIoMessage::Autosave` is never emitted anywhere in `src/`)
- **Problem:** This is latent today, because nothing dispatches `Autosave`. Once a timer emits it, two things go wrong:
  - (1) For a saved project, the "recovery snapshot" overwrites the MIDI files and plugin blobs that the canonical `project.json` points to. If the user then quits with "Don't save", reopening combines the old `project.json` with the unsaved notes and plugin states, so discarding changes does not work.
  - (2) If a manual Save As lands while an autosave of an untitled project is in flight, the new collector takes the autosave's `ClipsSavedToProjectDir`, which was written to the scratch dir. It then writes `project.json` to the new path before the second batch of clip WAVs is written. The completion reports spurious "missing audio" errors, and with `quit_after_save` the engine can shut down before the WAVs are written.
- **Suggested fix:** Write autosaves to a separate subtree, for example `autosave/midi` and `autosave/plugins`, or reference the blobs with an autosave suffix in `project.autosave.json`. Tag each engine save request with a sequence number (echoed in `ClipsSavedToProjectDir` and `AllPluginStatesSaved`) and ignore events that belong to a superseded collector.
- **Verification:** Extend `resonance-app/tests/io/autosave_write.rs`: autosave into a dir that already contains a `project.json` and `midi/clip_1.mid`, and assert that the canonical `.mid` is unchanged.

### [x] STATE-12 — Versioned backups copy only `project.json`; the MIDI and plugin files they point to are overwritten on every save — fixed @dc003c41
- **Severity:** low
- **Confidence:** high
- **Category:** data-loss
- **Location:** `resonance-app/src/project/io.rs:171-203` (`write_backup` copies only `project.json`), `:48-78` (every save overwrites `midi/clip_{id}.mid` and `plugins/plugin_{id}.bin` in place), `:230-243` (`load_project` accepts only a dir or a file named `project.json`, so a `backups/project-<ts>.json` cannot be opened)
- **Problem:** The doc comment says the blobs "are shared, not copied" and that a backup's relative paths still resolve. For audio that is true, because a clip WAV never changes. MIDI notes and plugin state do change and are keyed only by id, so every save rewrites them. Restoring a backup would give yesterday's arrangement with today's notes and today's plugin states. The files of deleted clips can also be overwritten through id reuse (STATE-08). No restore UI exists yet, so this is latent, but the retention setting and the backup files suggest a guarantee that does not hold.
- **Suggested fix:** Either inline MIDI notes and base64 plugin state into the backup JSON, or give content-addressed names to `.mid` and `.bin` files (for example a hash in `state_file` and `midi_file`) so older backups keep resolving to their own versions. Let `load_project` accept any `*.json` inside a bundle, with the project dir resolved to the bundle root.
- **Verification:** Add to `resonance-app/tests/io`: save, `write_backup`, change a MIDI clip's notes, save again, load the backup with the fixed loader, and assert that it has the original notes.

### [x] STATE-13 — `RequestRemoveTrack` records an undo entry even when it only opens the delete-confirmation dialog — fixed @ea0f017a
- **Severity:** low
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-app/src/undo/classify.rs:180-199` (the `TrackMessage::_ => Record` catch-all); `resonance-app/src/update/track.rs:142-157`
- **Problem:** When the track has clips, `RequestRemoveTrack` only sets `confirm_delete_track`, but it is classified as `Record`. Opening the dialog therefore records a snapshot, clears redo, sets `dirty` and bumps `revision`. `ConfirmRemoveTrack` then records a second entry, and Cancel leaves the first one behind as a no-op.
- **Failure scenario:** The user presses delete on a track with clips and cancels. The project is now "unsaved", redo is gone, and the first Ctrl+Z does nothing visible.
- **Suggested fix:** Classify `RequestRemoveTrack` as `Skip` when a confirm is needed. The classifier cannot see state, so either always `Skip` it and have the direct-delete branch re-dispatch `ConfirmRemoveTrack`, or gate the decision in `gates_message`.
- **Verification:** Add to `resonance-app/tests/mixer`: a track with a clip, dispatch `RequestRemoveTrack` then `CancelRemoveTrack`, and assert that the undo entry count and `is_dirty()` are unchanged.

### [x] STATE-14 — The test build of the app still writes to the user's real `recent.json` — fixed @5869f48f
- **Severity:** low
- **Confidence:** medium
- **Category:** test-coverage
- **Location:** `resonance-app/src/update/project_io/mod.rs:164, 219` (`recent::add` on `ProjectSaved(Ok)` / `ProjectLoaded(Ok)`); `resonance-app/src/recent.rs:81-100` (`persist` always targets `dirs::config_dir()`); `resonance-app/src/lib.rs:757-758` (the hermetic app skips *reading* recents but not writing them)
- **Problem:** `new_for_test()` is documented as making no reads of the user's config, but the save and load completions still write `~/.config/resonance/recent.json`. Browser tests work around this for `settings.json` by setting `XDG_CONFIG_HOME` per test, but recents have no such isolation. The developer's real `recent.json` currently contains `/tmp/.tmpur0xq2/song.rproj`, which is a tempfile-crate path and very likely came from a test (for example `tests/e2e_compose_via_control.rs` or `tests/io/autosave_write.rs`). Test binaries running concurrently also race on the fixed `recent.json.tmp` name.
- **Suggested fix:** Give `Resonance` a flag set from `Host`, for example `persist_user_state: bool`, set to false for test hosts, and have `recent::add`, `recent::remove` and `settings::persist` skip disk writes when it is false. Or route those writes through an injectable config root.
- **Verification:** Add a test in `resonance-app/tests/io` that sets `XDG_CONFIG_HOME` to a temp dir, runs `new_for_test()` plus `ProjectLoaded(Ok(..))`, and asserts that no `resonance/recent.json` exists there.

### [x] STATE-15 — Different preset names can map to the same file, so saving one overwrites another and deleting one removes the other — fixed @2623c57f
- **Severity:** low
- **Confidence:** high
- **Category:** data-loss
- **Location:** `resonance-app/src/presets.rs:238-271` (`save_user_preset`, `delete_user_preset`, `sanitize_filename`), `:184-190` (`user_preset_exists`)
- **Problem:** `sanitize_filename` maps every character that is not alphanumeric, `-` or `_` to `_`. "Lead Vox", "Lead.Vox" and "Lead_Vox" all become `Lead_Vox.json`. A save overwrites the other preset without warning (`user_preset_exists` reports a clash under a name the user never typed), and deleting "Lead.Vox" removes "Lead Vox". An empty or all-symbol name produces a hidden `.json` / `___.json`.
- **Suggested fix:** Before writing, load the target file and refuse or prompt when its stored `name` differs from the preset being saved. For delete, match on the stored `name`, not only on the sanitized filename. Alternatively, use a reversible encoding such as percent-encoding, and reject empty names.
- **Verification:** A test in `resonance-app/tests/io` with `RESONANCE_PRESET_DIR` pointed at a temp dir: save "A B" and then "A.B", and assert that both still load.

---

## App update loop / engine events / control socket

### [x] UPD-01 — A failed project open repoints the *current* project at the failed path (save/undo/recording then write into the wrong folder) — fixed @6a22ff1f (same as STATE-01)
- **Severity:** high
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-app/src/update/project_io/mod.rs:114-120` (`OpenPathSelected`), `:127-137` (`OpenRecent`), `:219-225` (`ProjectLoaded(Err)`); consumers `update/project_io/mod.rs:292-326` (`begin_save`), `undo/snapshot.rs:253` (`project_dir: self.io.project_path`), `update/project_io/replay/mod.rs:518`
- **Problem:** `OpenPathSelected` / `OpenRecent` write `r.io.project_path = Some(new_path)` and send `AudioCommand::SetProjectDir(new_path)` *before* the async `load_project_task` runs. If the load fails, `ProjectLoaded(Err)` only sets an error banner. It does not restore the previous `project_path` or the engine's project dir. The project that is still open (A) is now tied to the directory of the project that failed to load (B).
- **Failure scenario:** With project A open, the user opens B, and the load fails. B might be a folder picked by mistake that has no `project.json` (the dialog is `pick_folder`), a corrupt project, or one written by a newer version. The banner says "Load failed" and A is still on screen. (1) The user presses Ctrl+S. `start_save` targets B's directory, the engine writes A's `audio/clip_<id>.wav` into `B/audio/` over B's clips with the same ids, and `project.json` overwrites B's. A's own file is never updated. (2) Without saving: new recordings and imports go to B's dir, because the engine's project dir is B. A structural undo builds its snapshot with `project_dir = B`, and `replay_audio_clips` then loads every A clip from `B/audio/...`. They all come back missing or wrong. The same applies to control `project.open`: the job fails, but the path has already been changed.
- **Suggested fix:** Don't change `project_path` or the engine dir until the load succeeds. Carry the path in the task, for example `ProjectLoaded(Result<..>, PathBuf)`, or stash it in `io.pending_open_path`. On `Ok`, assign it and send `SetProjectDir`. On `Err`, leave both alone. Pitfall: `all_cleared` currently reads `project_path` to restore it after `replay_loaded_project` nulls it, so assign it in the `Ok` arm before sending `ClearAll`.
- **Verification:** New module in `tests/io/`: `Resonance::new_for_test_with_capture()` with a saved project at path A. Dispatch `OpenPathSelected(Some(tmp_dir_without_project_json))` and then feed `ProjectLoaded(Err(..))`. Assert `project_path()` is still A and that no `SetProjectDir(B)` command was captured.

### [x] UPD-02 — `project.open` job reports `done` before the project has loaded; readbacks and edits right after `job.wait` hit the OLD project — fixed @e344d9c6
- **Severity:** high
- **Confidence:** high
- **Category:** concurrency
- **Location:** `resonance-app/src/update/project_io/mod.rs:190-217` (`ProjectLoaded(Ok)` → `complete_token(ProjectLoad)` before `ClearAll` is even sent); `engine_events/project_io.rs:179-254` (`all_cleared`, where the replay actually happens); `update/tick.rs:62-78,112-118` (events are drained only on Tick, at up to 200 ms while idle)
- **Problem:** The `ProjectLoad` job is completed at the top of the `ProjectLoaded(Ok)` arm. That is before `AudioCommand::ClearAll` is sent, and before `AllCleared` → `replay_loaded_project` rebuilds the registry. The comment says the replay "is synchronous within this dispatch's task chain", but it isn't: it waits for an engine event that is drained on a later Tick. `needs_fast_tick` ignores `io.loading`, so that can be 200 ms later. `job.wait` runs on the socket reader thread and is woken by `notify_all` immediately. The reported `revision` is also the pre-load one.
- **Failure scenario:** The MCP client calls `project_open` → `job_wait` and gets `done` within a few ms. It then calls `song_summary` or `song_tracks`, which runs on the update loop before the next Tick and returns the previous project's tracks. A mutation such as `mixer.set_volume` or `track.rename` passes `mutation_gate_error` (`has_active_project` is already true) and edits the old registry. `replay_loaded_project` then wipes it, so the edit is lost even though it was acknowledged with a bumped revision. The agent's "verify by reading back" loop sees stale data.
- **Suggested fix:** Resolve the `ProjectLoad` token in `engine_events::project_io::all_cleared` for disk loads, next to the existing `ProjectNew` completion. Keep `fail_token` in `ProjectLoaded(Err)`. Build the result's `revision` after the replay. Pair this with UPD-03 so nothing can mutate during the window.
- **Verification:** `tests/control/` module: `new_for_test_with_capture()`, run `project.open` via `update::control::execute`, feed `ProjectLoaded(Ok)`, and assert `control_jobs().status(id).state` is not terminal. Then feed `AllCleared` through the engine-event path and assert it is `Done` and that `song.tracks` shows the loaded project.

### [x] UPD-03 — The mutation gate ignores `io.loading`; control edits during a load or a slow-path undo replay are acknowledged and then silently wiped — fixed @137c320a
- **Severity:** medium
- **Confidence:** high
- **Category:** concurrency
- **Location:** `resonance-app/src/update/control/mod.rs:481-508` (`mutation_gate_error`); `undo/snapshot.rs:340-346` (slow-path restore sets `io.loading` and sends `ClearAll`); `update/control/edit.rs:64-92`
- **Problem:** `mutation_gate_error` checks only `has_active_project` and bounce/freeze. Between `ClearAll` and `AllCleared`, which covers project load, template instantiation and a structural undo/redo, `io.loading` is true and a replay is pending. `can_record_undo()` is false, so the edit is not even recorded in history. `edit.undo` returns success with the post-undo `status`, but the GUI state is still pre-undo until the replay runs.
- **Failure scenario:** An agent calls `edit_undo`, which undoes a track add, so the slow path runs. It then immediately calls `mixer_set_volume` on another track. The handler mutates `registry` and sends `SetVolume`, and the revision is bumped. The next Tick drains `AllCleared`, and `replay_loaded_project` rebuilds everything from the snapshot, so the volume change disappears. A `track.add` in the same window is worse: the engine `TrackAdded` echo arrives after the replay and `tracks::added` pushes a track that no undo snapshot knows about.
- **Suggested fix:** In `mutation_gate_error`, return `RpcError::busy("a project load / undo replay is in progress")` when `app.io.loading || app.io.pending_load.is_some()`. Also make the control `edit.undo` response say whether the restore is deferred, or delay the reply. Consider holding the fast tick while `io.loading` so the window stays short.
- **Verification:** `tests/control/control_mutation_gate.rs` (existing module): set up a structural undo through `new_for_test_with_capture()` and call `edit.undo`. Then call `mixer.set_volume` via `execute` and assert the reply is `busy`.

### [x] UPD-04 — Queued pool-import placements are never cleared: undo, track delete or project switch still place the clip later — fixed @eb6ab142
- **Severity:** medium
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-app/src/update/pool.rs:226-263` (queues `PendingImport`); `engine_events/pool.rs:35-79,176-251` (`asset_imported` → `place_clip_with_id`, with no track-existence check); `state/pool_import.rs:161` (`clear()` has no callers); `update/project_io/mod.rs:190-217` (load doesn't clear it)
- **Problem:** The engine decodes and transcodes imports off-thread. The placement target (`track_id`, `start_sample`) sits in `r.pool_import` until `AssetImported` arrives, and nothing ever clears it or checks it against the current state. `place_clip_with_id` pushes a `ClipState` and sends `LoadClipFromWav` for whatever `track_id` was queued, even if that track no longer exists. The module doc says one undo of the pre-import snapshot "removes the pool asset, the placed clip, and any track spawned", but that only holds if the import finished before the undo.
- **Failure scenario:** The user drops a large FLAC onto the arrangement (`WindowAudioDrop` → a new track plus a queued placement) and presses Ctrl+Z straight away. The snapshot restore removes the new track. Seconds later `AssetImported` lands and the asset is re-added to the pool. A clip is pushed onto the deleted track id: it is saved into the project but belongs to no visible track. It is also outside the undo history, so redo/undo can't remove it cleanly. The same happens if the user opens project B during the import: the clip is placed onto whichever of B's tracks has the old id, with a WAV path under B's dir that doesn't exist. A `clip.place` control job then reports `done` for that orphan clip.
- **Suggested fix:** Clear `pool_import` (and fail matching `PoolImport` jobs) on `ProjectLoaded(Ok)`, template instantiation, and slow-path undo restore. In `asset_imported`, refuse to place when the target track is not in `registry.tracks`: drop the placement and fail the `clip.place` job with the existing "target track deleted" message. Better still, stamp each `PendingImport` with a project/replay generation counter and ignore events from an older generation.
- **Verification:** `tests/io/` (or the pool module in `timeline`): `new_for_test_with_capture()`, queue `ImportAndPlaceExact` onto track T, delete T (or run `ProjectLoaded(Ok)` + `AllCleared`), then feed `AudioEvent::AssetImported`. Assert no `ClipState` exists with `track_id == T` and no `LoadClipFromWav` was captured.

### [x] UPD-05 — Frozen tracks are not invalidated by compose, arrangement or tempo edits; playback keeps the stale frozen render — fixed @faca7544 (app-side content fingerprint)
- **Severity:** medium
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-app/src/update/gates.rs:420-450` (`frozen_input_edit_target` covers only `MidiEditor`, `MidiClip::DeleteMidiClip`, `Plugin`, and `ToggleTrackFxBypass`); `update/compose/regenerate.rs` and `update/compose/mod.rs:133` (compose writes and creates MIDI clips with no freeze handling); `update/freeze.rs:602` (`revalidate_frozen_track` is only called from tests)
- **Problem:** The only freeze invalidation is the pre-dispatch classifier. Compose regeneration (section chord edits, `harmony.*`, `generate.*`, lane-inspector re-rolls, motif propagation), `CreateMidiClipInSection`, `ArrangementMessage::InsertBars/RemoveBars` (they shift MIDI clip starts, which the freeze fingerprint includes), vocal lyric edits through compose, and tempo-map edits all change what a frozen track would render. None of them flips the track to `Stale`. The fingerprint-based `revalidate_frozen_track` exists but nothing in the app calls it.
- **Failure scenario:** The user freezes an instrument track whose part is generated by a section, then edits a chord in that section. `regenerate_lane` rewrites the track's MIDI clip, but the engine keeps playing the frozen cache (the old chords), and the UI shows no stale/refreeze affordance. The same happens after inserting bars before the part or changing tempo: the frozen audio drifts out of sync with the arrangement.
- **Suggested fix:** After dispatch in `update_inner` (or at the end of each compose/arrangement/global-track handler), call `revalidate_frozen_track` for every frozen track. The fingerprint makes this safe for no-ops. Also add tempo/signature events to the fingerprint, or invalidate every frozen track on a tempo-map change. Control `generate.*` / `harmony.*` should also use a `frozen_reject` like `notes.*`, or at least report that the track went stale.
- **Verification:** `tests/plugins/freeze_readonly.rs` (existing module): freeze a track that is the lane of a section with chords, dispatch a `ComposeMessage` chord edit, and assert `freeze.status(track).is_stale()`.

### [x] UPD-06 — A WAV mixdown (`io.bouncing`) gates nothing, and the GUI can swap the project out under any offline render — fixed @e92649ee
- **Severity:** medium
- **Confidence:** medium
- **Category:** concurrency
- **Location:** `resonance-app/src/update/gates.rs:447-462` (`gates_message` checks `bounce_in_progress` and `freeze` only, never `io.bouncing`); `gates.rs:149,211` (`ProjectIo(_)` is whitelisted during bounce-in-place and freeze); `update/control/mod.rs:497-508` (`offline_render_busy_error` omits `io.bouncing`); `update/project_io/mod.rs:114-137` (Open/OpenRecent have no render check); `update.rs:190` (Ctrl+O comes from the raw key subscription, so a modal can't block it)
- **Problem:** The WAV bounce (`BounceToWav` → `bounce::run_export` on a worker thread) drives the live plugin instances and track/clip locks, the same as freeze and bounce-in-place. Those two gate every GUI message, and the control `render_guard` refuses `project.new/open` while `io.bouncing`, with the comment "swapping the project out mid-render pulls the song out from under the offline renderer". But (a) during a WAV mixdown no GUI message and no control mutation is gated. `transport.play`, plugin add/remove, and track delete all go through, and the engine only checks `playing` once, when the export starts. (b) During any offline render, `ProjectIo` passes, so Ctrl+O → `OpenPathSelected` → `ProjectLoaded` → `ClearAll` drains `ctx.plugins`/tracks while the renderer is running.
- **Failure scenario:** The agent calls `render_mixdown`, which returns still-running, and then `transport_play` or `track_delete`. Both are accepted. The live callback and the offline renderer now interleave `process()` on the same CLAP instances, which is the corruption the engine's own comment in `run_export` warns about, or the render loses a track partway through. Or the user presses Ctrl+O during a freeze-all and opens another project, which clears the engine under the renderer.
- **Suggested fix:** Treat `io.bouncing` like `bounce_in_progress` in `gates_message` (block everything except `ProjectIo::Save*`, Tick, Control and close), and add it to `offline_render_busy_error`. In the `ProjectIo` handlers, refuse `OpenProject/OpenPathSelected/OpenRecent/TemplateLoaded` and `Ui::StartNewProject` while any offline render (`io.bouncing`, `bounce_in_progress`, freeze in flight, offline measure) is running.
- **Verification:** `tests/control/control_mutation_gate.rs`: set `io.bouncing` (via `BouncePathSelected`) on `new_for_test_with_capture()`, then call `transport.play` and assert `busy`, with no `AudioCommand::Play` captured. A GUI test should assert that `OpenPathSelected` during a freeze sends no `SetProjectDir`/load.

### [x] UPD-07 — Autosave is dead on master: nothing ever emits `ProjectIoMessage::Autosave` — fixed @8c0ad2a0 (trigger; crash recovery + settings UI not ported)
- **Severity:** medium
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-app/src/message.rs:1286`, `update/project_io/mod.rs:73-75,275-280` (write path), `settings.rs:38-46` (defaults `enabled: true, interval_secs: 30`); the trigger commit `f196f59d` (ba todo #465), the settings UI `0cb61f0d` (#471), and crash detection (#466) are only on `remotes/main/ba/epic-32`, not on master
- **Problem:** The autosave write path (#463) is merged. The periodic change-gated trigger that is supposed to call it from the tick is not: `git merge-base --is-ancestor f196f59d master` is false, and `grep` finds no producer of `ProjectIoMessage::Autosave` anywhere in `resonance-app/src`. The persisted settings still say autosave is on every 30 s.
- **Failure scenario:** The app or engine crashes, or a plugin aborts the process (a known historical failure mode). Every change since the last manual Ctrl+S is lost, even though the settings say autosave is on. The code paths look covered because the tests exercise `start_autosave` directly.
- **Suggested fix:** Land #465 (and, if still wanted, #466/#471) from `ba/epic-32`, rebased onto the current `tick.rs`. That means calling a pure `should_autosave(gate)` from `handle_tick` that requires dirty, an active project, `!io.loading`, no `save_state`, and the interval elapsed. Also add `io.loading` and `bounce/freeze` to its gate (see UPD-06), and require `project_path` to be trustworthy (see UPD-01).
- **Verification:** `tests/io/` module (`autosave_write.rs` sits alongside): `new_for_test()` with a dirty saved project, advance the clock past `interval_secs`, dispatch `Message::Tick`, and assert `saving()` becomes true or an autosave engine command is captured.

### [x] UPD-08 — The vocal render epoch map is wiped on every replay: jobs can hang forever and stale renders can alias fresh ones — fixed @8cfe8cd2
- **Severity:** medium
- **Confidence:** medium
- **Category:** concurrency
- **Location:** `resonance-app/src/update/project_io/replay/mod.rs:53-59` (`compose.vocal_audio.clear()`); `compose/state.rs:96` (`clear()` resets `render_epoch`); `update/compose/vocal_render_plan.rs:120` (epochs restart at 1); `update/compose/mod.rs:377-418`; `control_jobs.rs:409-477`
- **Problem:** Epochs are per `(definition_id, track_id)` and go back to 0 whenever a project load or slow-path undo replays. (a) An in-flight render's `VocalAudioReady`/`VocalAudioFailed` then fails the epoch check and is dropped, so a `vocal.render` job covering that lane is never resolved. `prune` never evicts live jobs, and each `job.wait` just times out after `MAX_WAIT`. (b) If the lane is re-rendered after the replay, it gets epoch 1 again. The pre-replay render, which also carried epoch 1, is then accepted as current if it finishes first, installing audio rendered from the old notes or project.
- **Failure scenario:** An agent calls `vocal_render` on a track (SVS takes seconds), then `edit_undo` of a structural edit. The job stays `running` forever. Or the user regenerates a vocal lane, immediately undoes a track add (slow path), and regenerates again. The first render finishes first, is accepted under epoch 1, and its clip is installed; the second is then treated as current too and stacks a second clip set.
- **Suggested fix:** Make epochs globally monotonic (one `u64` counter on `Resonance` that `clear()` never resets), so a pre-replay epoch can never equal a post-replay one. On replay, fail every live `JobToken::VocalRender` job ("project state was replaced mid-render").
- **Verification:** `tests/vocal/` module: `new_for_test()`, start `vocal.render` via `execute`, run a slow-path undo (`ProjectLoaded`/`AllCleared`), and feed the stale `VocalAudioReady`. Assert the job is terminal (`Error`), and that a new render's epoch differs from the stale one.

### [x] UPD-09 — A queued `ImportClip` finishing after `ClearAll` injects a stale clip into the new project — fixed @c1d74004
- **Severity:** medium
- **Confidence:** medium
- **Category:** concurrency
- **Location:** `resonance-app/src/engine_events/clips.rs:8-45` (`imported`: when the id matches an existing clip it overwrites that clip's `waveform_peaks`/`total_frames`, otherwise it pushes a new `ClipState` with no track check); engine side `resonance-audio/src/engine/clips.rs:159-233` (the `ImportQueue` worker pushes into `ctx.clips` regardless) and `engine/tracks.rs:465-469` (`ClearAll` resets `next_clip_id`/`next_track_id` to 1 without fencing in-flight imports)
- **Problem:** `ImportClip` decodes on the `ImportQueue` worker with a `clip_id` allocated before `ClearAll`. `ClearAll` resets the id counters and doesn't cancel the queue. When the worker finishes, it pushes an `AudioClip` with that old id and old `track_id` into the new project's clip list and emits `ClipImported`. The app handler can't tell the event is stale.
- **Failure scenario:** The user imports a long file, then opens project B (or a slow-path undo runs) before the decode finishes. B's clip 3 (restored from file) gets its waveform and length overwritten by the stale import's. If B has no clip 3, a phantom clip appears on B's track with the old id and is saved with the project.
- **Suggested fix:** Engine: give each import job a generation that `ClearAll` bumps, and drop results from older generations; this is the owning dev's call. App: in `clips::imported`, ignore `ClipImported` while `io.loading`, and when the `track_id` isn't in `registry.tracks`.
- **Verification:** `tests/timeline/` module: `new_for_test()` with a loaded project that has clip 3. Feed `AudioEvent::ClipImported { clip_id: 3, track_id: 99, .. }` and assert clip 3's `total_frames` is unchanged and no clip on track 99 exists.

### [x] UPD-10 — "Search folder" relink walks the whole directory tree synchronously in `update()` — fixed @a19e6cad
- **Severity:** medium
- **Confidence:** high
- **Category:** performance
- **Location:** `resonance-app/src/update/relink.rs:140-176` (`start_batch_relink` → `scan_folder_for_names`), `:296-333`
- **Problem:** `scan_folder_for_names` is a recursive `read_dir` DFS (up to depth 24, over every file) that runs on the UI thread inside the update handler. Only the per-file transcode that follows is moved to `spawn_blocking`.
- **Failure scenario:** A project reports missing files, and the user points "Search folder" at `~` or a sample-library root (hundreds of thousands of files, possibly on NFS or a spun-down HDD). The window freezes for seconds to minutes. Control-socket requests queue behind it, so MCP calls time out and the client drops the connection.
- **Suggested fix:** Run the scan in `Task::perform(spawn_blocking(scan_folder_for_names(..)))` and return a new `RelinkMessage::ScanFinished(found)` that starts the imports. Mark the wanted assets in flight before spawning so a second click doesn't start a duplicate scan.
- **Verification:** Add a unit-level test in the existing relink test module that `start_batch_relink` returns without having called `read_dir`, for example by asserting no assets were started before the `ScanFinished` message is fed.

### [x] UPD-11 — Global shortcuts that aren't focus-gated fire while typing (Enter, B, Cmd-Z) — fixed @b1357803
- **Severity:** low
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-app/src/update.rs:227-229` (Enter → `OpenSelectedMidiClip`), `:193-199` (Cmd-Z/Y → project undo/redo), `:345-361` (`B` momentary audition); compare `F` and `.`/`,`, which go through `crate::focus::any_text_input_focused`
- **Problem:** `keyboard::listen()` receives key presses that a focused `text_input`/`text_editor` has already consumed (as `focus.rs` documents). Only `F`, `.` and `,` are probed.
- **Failure scenario:** With a MIDI clip selected, the user renames a track and presses Enter to submit, and the piano roll opens. While typing "Bass" into a text field in the Mixer view with the reference rail open, pressing B switches monitoring to the reference and back. Cmd-Z inside a text field (iced has no text undo) runs a project-level undo, possibly a slow-path replay.
- **Suggested fix:** Route Enter and B through the same `Request*` → `any_text_input_focused()` → `*Resolved` pattern as `RequestPerformanceToggle`. For Cmd-Z/Y, either do the same or accept it and document it.
- **Verification:** `tests/control/` or `tests/timeline/` module that already exercises `RequestPerformanceToggle`: add equivalent cases for a `RequestOpenSelectedMidiClip` resolved with `focused = true` and assert `editing_midi_clip` stays `None`.

### [x] UPD-12 — Socket parent-dir hardening `chmod`s whatever the path resolves to, following symlinks — fixed @9ef9201a (same as CTL-11)
- **Severity:** low
- **Confidence:** medium
- **Category:** security
- **Location:** `resonance-app/src/control_socket.rs:122-131` (`prepare_parent_dir`)
- **Problem:** `set_permissions(dir, 0o700)` follows symlinks, runs on any existing directory, and never checks ownership. With `RESONANCE_CONTROL_SOCKET=/home/u/proj/ctl.sock` it silently makes the project dir 0700. In the `/tmp/resonance-<uid>` fallback (no `XDG_RUNTIME_DIR`, e.g. under `sudo -u` or on non-systemd sessions), another local user can pre-create that name as a symlink to a directory the victim owns. The chmod then applies to that target and the socket is bound inside it. A pre-created directory owned by someone else only causes a startup failure (EPERM), which is fine. Access control therefore rests entirely on the parent dir, with no `SO_PEERCRED` check.
- **Failure scenario:** Low impact: a surprise permission change on a user directory, or a socket published in an unexpected place. Not remotely exploitable.
- **Suggested fix:** Use `symlink_metadata` and refuse when the parent is a symlink or `uid != geteuid()`. Only chmod a directory the server itself created (or the fixed `resonance/` subdir). Optionally verify `SO_PEERCRED` uid == own uid on accept.
- **Verification:** Unit test in `resonance-control` or an app `tests/control/` module: point `RESONANCE_CONTROL_SOCKET` into a temp dir whose parent is a symlink and assert `spawn` returns an error, leaving the target's mode unchanged.

---

## App view layer / compose

Paths are relative to `resonance-app/src/` unless stated otherwise. Every finding was traced through view → message → update handler. Items marked "spot-checked" were re-read at the cited lines during consolidation.

### [x] VIEW-01 — Delete/Backspace while editing notes deletes the whole MIDI clip being edited — fixed @2c9eddcc (KeyFocus click-to-own)
- **Severity:** high
- **Confidence:** high (spot-checked)
- **Category:** correctness
- **Location:** `view/timeline/input/keyboard.rs:20-45`, `view/timeline/mod.rs:883-885`, `view/midi_editor/input.rs:350-368`, `update/clips.rs:668` (`start_midi_clip_drag` sets `selected_midi_clip`), `view/mod.rs` (`view_main_area`: `column![timeline, editor]`)
- **Problem:** The timeline canvas handles every `KeyPressed` event, whatever holds focus and wherever the pointer is. On Delete/Backspace it publishes `DeleteMidiClip(selected_midi_clip)`. iced 0.14 `Column`/`Scrollable` deliver keyboard events to every child, and canvas `update` ignores `is_event_captured`. Double-clicking a MIDI clip to open it leaves `selected_midi_clip` set, because the first click starts a clip drag and selects the clip, and nothing clears it while `editing_midi_clip` is open.
- **Failure scenario:** Double-click a MIDI clip, click one note in the piano roll, press Delete. The timeline sits earlier in the tree, so it publishes `DeleteMidiClip` first and the whole clip is gone; the piano roll's `RemoveSelectedNotes` then runs against a clip that no longer exists. Backspace typed into the quantize panel's "Groove name" `text_input` (`view/midi_quantize.rs:189`) deletes the clip the same way.
- **Suggested fix:** Pass `editing_midi_clip.is_some()` into `TimelineCanvas` and skip clip deletion while it is set. More generally, act on Delete only when `cursor.is_over(bounds)` or behind `crate::focus::any_text_input_focused()` returning false. The piano-roll canvas needs the same guard. The simplest structure is one app-level Delete router.
- **Verification:** Add a module to the `timeline` group binary: `Resonance::new_for_test_with_capture()`, open a MIDI clip in the editor, select a note, send `keyboard::Event::KeyPressed(Named::Delete)` through the view, then assert the captured commands contain a note removal and no `DeleteMidiClip`. No current test sends Delete or Backspace.

### [x] VIEW-02 — Dragging a note past a neighbour starts moving the neighbour (stale `note_index` after the re-sort) — fixed @91a200af
- **Severity:** high
- **Confidence:** high (spot-checked)
- **Category:** correctness
- **Location:** `view/midi_editor/input.rs:180-190,251-270`, `view/compose/vocal_roll/canvas_program.rs:96-99,147-162`, `view/compose/expanded_editor/input.rs:48-56,124-143`, `update/midi_editor.rs:61-72`, `resonance-audio/src/engine/midi/clips.rs:270-274`, `engine_events/midi.rs:298-315`
- **Problem:** `DragMode::MoveNote { note_index }` is captured when the mouse goes down. Every `CursorMoved` sends `MoveNote { note_index }`, and both the engine and the app mirror `sort_by_key(start_tick)` after each move. Once the dragged note crosses a neighbour, the stored index points at the neighbour. The index-based `selected_notes` and the vocal lyric side-table follow the wrong note too.
- **Failure scenario:** A clip has note A at tick 0 (index 0) and note B at tick 480. Drag A right past tick 480. After the sort the order is [B, A], so the next mouse move sends `MoveNote(0)`: B jumps to the cursor while A stays behind. Both notes end up edited. In the vocal roll, the lyric moves with the wrong note.
- **Suggested fix:** Pick one:
  - Keep the drag as a view-local preview and commit a single `MoveNote` on release.
  - Have the move handler or echo return the note's new index and update `DragMode`.
  - Give notes stable ids.

  Apply the fix to all three editors.
- **Verification:** Add a `timeline` or `compose` group-binary module with a two-note clip. Send `MoveNote(0 → tick 600)` and then `MoveNote(0 → tick 700)` the way the canvas does, and assert that B still sits at tick 480. Also drive the canvas `update` with synthetic CursorMoved events.

### [x] VIEW-03 — Clicking a drum cell toggles the wrong step when the group's phase is non-zero (phase applied twice) — fixed @aa57955c
- **Severity:** high
- **Confidence:** high (spot-checked)
- **Category:** correctness
- **Location:** `view/compose/drumroll/canvas.rs:588-590`, `update/compose/drum_groups.rs:354-366`, `compose/messages.rs:732-740`
- **Problem:** The canvas sends `step = (global_step + group.phase) % cycle`, which is already a pattern index, and uses the *primary* group's phase to do it. The handler treats `step` as the visible step (as the message doc says) and adds the *resolved* group's phase again: `pattern_idx = (step + phase) % cycle`. The cell that gets flipped is `(global + 2·phase) % cycle`, while the cell drawn is `(global + phase) % cycle`.
- **Failure scenario:** Set a group to cycle 16 and phase 4. Click the first cell of bar 1. The handler flips pattern index 8, so a cell four steps to the right toggles, the clicked cell stays unchanged, and `materialize_drum_clips` bakes the wrong hit into playback.
- **Suggested fix:** Have the canvas send the raw `global_step` (per the message contract) and let the handler apply the resolved group's phase. Keep the cycle modulo in the handler only.
- **Verification:** Add a module to the `compose` group binary: a group with phase=4, cycle=16. Send the `TogglePadStep` the canvas would emit for a click at step 0 (build it through the canvas `update` with a synthetic press), and assert that `pad.pattern[4]` flipped.

### [x] VIEW-04 — Deleting a placement or section leaves its generated MIDI and vocal clips playing — fixed @493bdcb9
- **Severity:** high
- **Confidence:** high (spot-checked)
- **Category:** correctness
- **Location:** `update/compose/section.rs:362-385` (`handle_delete_with_placements`), `:432-438` (`handle_delete_placement`), cf. `update/arrangement.rs:196-201` (`remove_bars` is the only cleanup), `update/control/section.rs:181-186`
- **Problem:** Both handlers only `retain()` placements and definitions. `compose.derived_clips`, `vocal_audio.clips`, the engine's MIDI and audio clips, and per-definition side tables (`render_epoch`, `render_cache`, `vocal_bulk_lyrics`, `expression_curves`) are never cleaned up. No code path removes `derived_clips` entries except `clear()` on rebuild. The control API's confirm text promises that deleting a section removes "every generated lane for it", which it does not.
- **Failure scenario:** Place Verse at bar 1 with a bass generator, so a bass clip exists. Delete the placement: the section block disappears but the bass clip keeps playing and is saved. Re-place Verse at bar 1 and regenerate: two basses now play. After a reload, `rebuild_derived_clips` claims only one of them, so the duplicate is permanent.
- **Suggested fix:** Factor the `remove_bars` cleanup into a `purge_placement_outputs(r, placement_id)` helper that removes the derived and vocal entries, sends `DeleteMidiClip`/`DeleteClip`, retains `r.midi_clips`/`r.clips`, and unlinks the vocal WAV. Call it from both handlers. For a definition delete, also drop its side tables.
- **Verification:** Add a module to the `compose` group binary using `new_for_test_with_capture()`: generate a lane, delete the placement, then assert a `DeleteMidiClip` was captured and `r.midi_clips` holds no clip for that key.

### [x] VIEW-05 — Resizing a section never re-derives its clips: shrinking leaves overlapping clips, growing leaves silent drums — fixed @0b84e135
- **Severity:** high
- **Confidence:** high
- **Category:** correctness
- **Location:** `update/compose/section.rs:300-345` (`handle_resize`), `compose/section.rs:139-148` (`set_primary_pattern` writes `Bars(length_bars)`), `update/compose/drum_groups.rs:872-888` (gaps render silent)
- **Problem:** `handle_resize` only sets `def.length_bars`. Clip durations were fixed from the old length when the clips were generated. After a shrink, `placement_overlaps` lets another section occupy the freed bars while the old full-length clips remain. After a grow, a single-entry `Bars(old_len)` drum arrangement leaves a trailing gap, and gaps are deliberately rendered silent.
- **Failure scenario:**
  - Shrink: an 8-bar Verse with bass and drums is resized to 4 bars. Place Chorus at bar 4 and generate. Bars 4-7 now play both sections' parts.
  - Grow: `generate.drums` on an 8-bar section, then resize it to 16 bars. Bars 9-16 have no drums.
- **Suggested fix:** After a successful resize, regenerate every lane generator, the drums (`materialize_drum_clips`) and the vocal for all placements of the definition. Rewrite a single-entry `Bars(old_len)` arrangement to the new length, or run `trim_to_fit`/`fill_to_end`.
- **Verification:** Add a `compose` group-binary module: generate, resize, then assert each derived clip's `duration_ticks` matches the new length and the drum clip covers every bar.

### [x] VIEW-06 — Typing "nan" in the BPM field collapses every clip to sample 0 and makes the saved project unloadable — fixed @f0208269 (sanitize_bpm 20..=300; non-finite rejected; loader falls back)
- **Severity:** high
- **Confidence:** high for the parse path (spot-checked), medium for the exact downstream effects
- **Category:** error-handling
- **Location:** `view/transport.rs:306-307`, `update/transport.rs:75-97`, `resonance-audio/src/types/tempo/map.rs:81` (`rebuild_bar_table`), `project/model.rs:31`
- **Problem:** `"nan".parse::<f32>()` returns `Ok(NaN)`, and `NaN.clamp(20.0, 300.0)` is still NaN. That NaN is written to `transport.bpm` and `tempo_events[0].bpm` and sent to the engine as `SetBpm`. Because `NaN != old` is true, `musical_anchors`/`reanchor_to_tempo` then run on a NaN bar table. `"inf"` is clamped to 300 and is harmless.
- **Failure scenario:** Type `nan` in the BPM box and press Enter. Every bar's sample position becomes NaN, which casts to 0, so the clips, markers and automation are all re-anchored to sample 0. Saving writes `"bpm": null`, and loading rejects it because `bpm: f32` cannot be null.
- **Suggested fix:** Check `parsed.is_finite()` before the clamp. Put the check in a shared `validate_bpm` also used by the tempo-lane edits and the control API.
- **Verification:** Add a module to the `control` or `timeline` group binary: send `SetBpmText("nan")` and `CommitBpm`, then assert `transport.bpm` is unchanged and no `SetBpm` command was captured.

### [x] VIEW-07 — The "Save as preset…" name prompt never renders, then shows up in place of a later context menu — fixed @aaa06185
- **Severity:** high
- **Confidence:** high (spot-checked)
- **Category:** ux
- **Location:** `update/track.rs:394-411`, `view/mod.rs:246-250`, `view/menus.rs:429-433`
- **Problem:** `OpenSavePresetPrompt` sets `preset_save = Some` and `track_menu = None`. `view_main_area` stacks `view_track_menu_overlay` only when `track_menu.is_some()`, and that overlay is the only place `preset_save_overlay` is drawn.
- **Failure scenario:**
  1. Right-click a track and choose "Save as preset…". The menu closes and nothing appears.
  2. Later, right-click any track. Instead of the menu, the stale name prompt for the first track appears.
- **Suggested fix:** Gate the stack on `track_menu.is_some() || preset_save.is_some()`, or move the prompt into the root overlay chain.
- **Verification:** Add a golden snapshot in the `control` or `mixer` group binary: `new_for_test_on(Arrange)`, send `OpenSavePresetPrompt(track)`, and render. The golden must show the prompt. Bless with `RESONANCE_BLESS=1`.

### [x] VIEW-08 — Inspector BYP button (and several other inspector fields) freeze because the lazy fingerprint doesn't hash them — fixed @fe17908b
- **Severity:** high
- **Confidence:** high (spot-checked)
- **Category:** correctness
- **Location:** `view/mixer/inspector/mod.rs:235-238` (hashes only `instance_id` and `plugin_name`), `view/mixer/inspector/chain.rs:57-63`, `view/mixer/inspector/bus.rs:108-111` vs `:333`, `view/mixer/inspector/external_instrument.rs:571-572,629,672`, `bus.rs:234-253`
- **Problem:** The chain rows render `plugin.bypassed`, and the button's `on_press` is built from that cached value (`bypassed: !bypassed`). `bypassed` is not in the fingerprint, so after the `PluginBypassChanged` echo `lazy` keeps the old subtree. The same gap affects:
  - `track.playback_source`
  - `ext.latency_detect_in_progress` and `transport.playing`
  - `ext.latency_detect_error`
  - the send-source names in the bus SENDS IN list
- **Failure scenario:** In the Mixer inspector, click BYP on an FX. The engine bypasses it and the strip pill updates, but the inspector's BYP stays unlit and its press still sends `bypassed: true`. The second click does nothing, so the user cannot un-bypass from the inspector.
- **Suggested fix:** Hash `p.bypassed` in both the track and bus fingerprints, along with the other fields listed. Extend the table-driven fingerprint tests with these facets.
- **Verification:** The fingerprint tests live in the `mixer` group binary. Add a case asserting that flipping `bypassed` changes the fingerprint, plus a golden of the inspector with a bypassed plugin.

### [x] VIEW-09 — Vocal-roll shortcuts fire while typing in right-rail text fields ("s" toggles a slur, Backspace deletes a note) — fixed @2c9eddcc
- **Severity:** high
- **Confidence:** high
- **Category:** correctness
- **Location:** `view/compose/vocal_roll/canvas_program.rs:192-226`, `view/compose/page.rs:175-214`, `view/compose/lane_inspector/vocal/draft_group.rs:166`, `lyrics_group.rs:35`
- **Problem:** The canvas acts on `Delete`/`Backspace` (`RemoveNote` for the selected note) and on the text `s`/`+` (`ToggleSlur`) without checking focus or pointer position. The lyric `text_input`s share the page, and iced routes keyboard events to every child. `crate::focus::any_text_input_focused` already exists for this case.
- **Failure scenario:** Select a note in the vocal roll, then type "sun" into the draft-lyric field. The "s" toggles a slur on the note. Backspace to fix a typo deletes the note.
- **Suggested fix:** Handle these keys only when `cursor.is_over(bounds)` (the expanded editor already does this for +/-), or when no text input is focused.
- **Verification:** Add a `compose` group-binary test: focus a lyric `text_input` via `iced_test`, type "s", and assert the note's slur flag is unchanged.

### [x] VIEW-10 — Clip, fade and loop drags jump sideways after playback auto-follow wrote a stale `viewport.scroll_offset` — fixed @8275c04b (removed dead auto_follow_playhead)
- **Severity:** high
- **Confidence:** medium (the write path and the reducers are spot-checked; the size of the offset depends on `viewport_width` being the full content width)
- **Category:** correctness
- **Location:** `update/tick.rs:350-363` (`auto_follow_playhead`), `update/clips.rs:266,432,695`, `update/transport.rs:191`, `view/timeline_panel.rs:53` (canvas `scroll_offset: 0.0`), `view/timeline/input/hover.rs:227-231`
- **Problem:** The canvas now works in content coordinates: an outer `Scrollable` owns horizontal scroll and the canvas's own offset is pinned to 0. The drag reducers still compute `seconds = (x - grab + r.viewport.scroll_offset) / zoom`, and `auto_follow_playhead` still writes `r.viewport.scroll_offset` during playback. `viewport_width` is reported from the canvas `bounds.width`, which is the full content width. `timeline_content_size` then clamps the offset only to `content_w - viewport_w`.
- **Failure scenario:** Play past about 80% of the song, then stop. Grab a clip and nudge it: it jumps right by `scroll_offset` pixels, many bars. Loop-marker and fade drags are offset by the same amount until playback resets it.
- **Suggested fix:** Remove `r.viewport.scroll_offset` from every pointer-to-sample conversion, since pointer x is already in content space. Either delete `auto_follow_playhead` or re-implement it with `scrollable::scroll_to` on the outer scrollable's id.
- **Verification:** Add a module to the `timeline` group binary: set `r.viewport.scroll_offset = 500.0`, send `StartClipDrag`/`UpdateClipDrag` with an x delta of 0, and assert the clip's start is unchanged.

### [x] VIEW-11 — Compose instrument-track canvas is shifted by the Arrange timeline's vertical scroll — fixed @b8ebe088
- **Severity:** medium
- **Confidence:** high (spot-checked)
- **Category:** correctness
- **Location:** `view/compose/tracks/mod.rs:131`, `view/compose/tracks/draw.rs:33`, `update/viewport.rs:24-38`
- **Problem:** `scroll_offset_y: app.viewport.scroll_offset_y` is the Arrange timeline's vertical scroll. The Compose tracks canvas has a fixed height, sits inside Compose's own `scrollable`, and has no wheel handler, yet it subtracts that value from every row's y.
- **Failure scenario:** Scroll Arrange down 300 px, then switch to Compose. The first 2-3 instrument lanes are drawn above y=0 and clipped, and the bottom is empty. Those lanes can be neither seen nor clicked from Compose.
- **Suggested fix:** Pass `0.0`, since the outer scrollable already handles overflow.
- **Verification:** Add a golden in the `compose` group binary: `new_for_test_on(Compose)` with `viewport.scroll_offset_y = 300.0`, rendered and compared to the unscrolled golden.

### [x] VIEW-12 — Deleting a track leaves its lane generators behind, and the next chord edit creates a ghost clip on the dead track — fixed @0110881f
- **Severity:** medium
- **Confidence:** high
- **Category:** correctness
- **Location:** `engine_events/tracks.rs:79-143`, `update/compose/regenerate.rs:96-126,233-239`, `update/compose/mod.rs:96-146`, `resonance-audio/src/engine/midi/clips.rs:105-139`
- **Problem:** Track removal prunes `r.clips` but not `definitions[*].lane_generators`, `derived_clips`, `vocal_audio.*`, `vocal_bulk_lyrics` or `expression_curves`. `propagate_chord_change` then regenerates lanes for the removed `TrackId`; the missing track falls back to the name "Track". The engine accepts any track id.
- **Failure scenario:** Give Bass a generator in Verse, delete the Bass track, then edit a chord in Verse. `LoadMidiClipDirect` creates a clip on the nonexistent track, and that clip is saved. For a vocal lane, a multi-second SVS render is also queued, followed by `LoadClipFromWav` onto the deleted track.
- **Suggested fix:** Purge every compose map keyed by the removed track id on removal. Make `regenerate_lane`/`roll_vocal_melody` return early when the track is not in `registry.tracks`.
- **Verification:** Add a `compose` group-binary test: generator → delete track → `EditChord`. Assert no MIDI clip exists whose `track_id` is missing from the registry.

### [x] VIEW-13 — Compose uses the time signature and BPM under the playhead, not at the section — fixed @7558afff
- **Severity:** medium
- **Confidence:** high
- **Category:** correctness
- **Location:** `update/compose/mod.rs:148`, `update/compose/regenerate.rs:211-213,230-231,282`, `update/compose/vocal_render.rs:106,134-135,247`, `update/compose/drum_groups.rs:805`, `update/compose/chord_inspector.rs:349`, `update/tick.rs:310-340` (`sync_tempo_at_playhead`)
- **Problem:** These all read `r.transport.time_sig_num` and `r.transport.bpm`:
  - clip `duration_ticks`
  - `chord_fits_in_section` and resize validation
  - drum steps per bar
  - the SVS render's seconds-per-tick

  During playback those two values follow the playhead, while clip positions come from the tempo map. A global signature change also never revalidates existing chords.
- **Failure scenario:** The song is 4/4 at 100 BPM with a 3/4, 140 BPM event at bar 17.
  - Regenerate a bar-1 section while playing past bar 17: its clips are built with 3-beat bars and are too short.
  - Render a vocal for a bar-17 section with the playhead at bar 1: the audio uses 100 BPM timing and drifts against the MIDI.
  - Whether a chord at beat 13 is accepted depends on where the playhead is.
- **Suggested fix:** Resolve the signature and tempo from `tempo_map` at each placement's `start_bar`. Never read `r.transport.*` in compose code.
- **Verification:** Add a `compose` group-binary test with a two-signature tempo map: move the playhead past the change and regenerate the bar-1 section. Assert `duration_ticks == length_bars * 4 * TPQ`.

### [x] VIEW-14 — Media-browser drop lands on the wrong track when rows have different heights — fixed @8ea80ee9 (drops onto non-track rows now refused)
- **Severity:** medium
- **Confidence:** high
- **Category:** correctness
- **Location:** `view/timeline/placement.rs:57-72` (`lane_at`), `view/timeline/mod.rs:198`, `view/timeline/draw/drag.rs:31-107`
- **Problem:** `lane_at` computes `floor(rel / TRACK_HEIGHT)` over every non-sub track, including collapsed ones. Every other hit path uses `ArrangeRowLayout`, which accounts for group headers, automation and take rows, and hidden members.
- **Failure scenario:** Track 1 has three automation lanes expanded. Drag a sample onto Track 2: it lands on Track 3, and the ghost and tooltip are drawn on that wrong row too.
- **Suggested fix:** Resolve the row with `ArrangeRowLayout::row_at_y` (reuse `row_at_canvas_y`), accept only `Track` rows, and draw the ghost from `track_row_rect`.
- **Verification:** Add a `timeline` group-binary test: expand automation on track 1, drop at track 2's layout y, and assert the new clip's track id.

### [x] VIEW-15 — After dragging a tempo point, Delete removes a different event; a drag can also stack two events on one bar — fixed @c830e8fd
- **Severity:** medium
- **Confidence:** high
- **Category:** correctness
- **Location:** `view/timeline/input/global_tracks.rs:80-89`; `update/global_track.rs` (`StartTempoDrag` / `UpdateTempoEvent` / `EndTempoDrag`)
- **Problem:** The selection stores an index. `EndTempoDrag` sorts `tempo_events` without remapping that index. During the drag the list is unsorted, so the tempo preview is wrong. Dropping onto an occupied bar produces a duplicate, the same bug #1382 fixed for `AddTempoEvent`.
- **Failure scenario:** Tempo events sit at bars 0, 4 and 8. Drag the bar-4 event to bar 12 and press Delete: the bar-8 event is removed.
- **Suggested fix:** After the sort, locate the dragged event and update `selected_global_event`. Clamp the drag between its neighbours, or upsert by bar.
- **Verification:** Add a `timeline` group-binary test driving the drag messages followed by `DeleteSelectedEvent`, asserting the remaining bars are [0, 8].

### [x] VIEW-16 — The expanded Compose editor treats clip-relative note ticks as section-relative — fixed @6ac373c0
- **Severity:** medium
- **Confidence:** high
- **Category:** correctness
- **Location:** `view/compose/expanded_editor/mod.rs:472`, `view/compose/expanded_editor/input.rs:66-86,131-141`, compare `view/compose/tracks/draw.rs` (which adds `sample_to_section_tick(clip.start_sample)`)
- **Problem:** `note_rect`, hit-testing, AddNote and MoveNote use `note.start_tick` directly, even though the editor draws every clip that intersects the section.
- **Failure scenario:** A clip starts at song bar 1 and the section is placed at bar 9. Its notes are drawn at section bar 1. Clicking there adds a note at clip tick 0, which is song bar 1 and outside the section. Two clips in one section are drawn on top of each other.
- **Suggested fix:** Add the clip offset when drawing and hit-testing, and subtract it when converting back to clip ticks, as `tracks/draw.rs` does.
- **Verification:** Add a golden in the `compose` group binary with an offset clip, plus an AddNote test asserting the clip-relative tick.

### [x] VIEW-17 — Section lengths and chord or placement positions are unbounded; the invariants overflow (UI hang, debug panic, release bypass) — fixed @bea9237c
- **Severity:** medium
- **Confidence:** high
- **Category:** error-handling
- **Location:** `update/compose/section.rs:54-71` (`first_free_bar`), `:116,129-135,182,195-201` (only `> 0` is checked); `compose/invariants.rs:13,21,39,59-60`; `view/compose/layout.rs:21-25`; `view/compose/drumroll/canvas.rs:254,389-414`; `view/compose/vocal_roll/mod.rs:144`
- **Problem:**
  - Any `u32` is accepted as a section length. The views then loop `0..length_bars` several times per frame and the drum canvas tessellates bars × cells × pads rectangles.
  - The invariants use plain `+` and `*`, which panic in debug and wrap in release: `start_bar + length_bars`, `start_beat + duration_beats`, `length_bars * time_sig_num`.
  - `first_free_bar` returns `10_001` even when that bar overlaps.
- **Failure scenario:**
  - Typing `100000000` bars in New Section hangs the UI thread.
  - `harmony_add_chord` with `start_beat = u32::MAX` wraps past the fit check in release and panics in debug.
  - `section_place` with `start_bar = 4294967294` escapes the overlap check.
- **Suggested fix:** Clamp or reject lengths above a maximum (for example 1024 bars) in the dialog confirm handlers and in `handle_create`/`handle_resize`. Use `checked_add`/`checked_mul` in the invariants and treat overflow as "does not fit". Make `first_free_bar` return `Option`.
- **Verification:** Add `compose` group-binary tests: create a section of `u32::MAX` bars (expect rejection), and add a chord with `start_beat = u32::MAX` (expect rejection, no panic).

### [x] VIEW-18 — Ribbon selection, expression tool state and async vocal completions record undo entries and wipe redo — fixed @5423f84f
- **Severity:** medium
- **Confidence:** high (spot-checked)
- **Category:** correctness
- **Location:** `undo/classify.rs:480-509`, `update/compose/mod.rs:154-158`, `view/compose/drumroll/ribbon.rs:348,357`, `undo/history.rs:86-93`
- **Problem:** Only `Arrangement(SelectEntry)` and a fixed UI list are classified `Skip`. `ComposeMessage::SelectArrangementEntry`, which the handler itself calls "pure view state… no undo", falls through to `Record`. So do `Expression{SelectCurve, SetPenMode, SetSnap}`, `VocalAudioFailed`, and `VocalAudioReady`, which arrives seconds after the edit. `record()` clears redo.
- **Failure scenario:** Press Ctrl+Z, then click a span in the drum ribbon. Redo is lost, the project is marked dirty, and the control-API `revision` is bumped. Likewise, pressing Ctrl+Z while a vocal render is running loses redo when the render completes.
- **Suggested fix:** Classify those messages as `Skip`. Give `VocalAudioReady` a non-recording path that amends the queuing entry or relies on the render epoch.
- **Verification:** Add a `control` group-binary test: edit, undo, send `SelectArrangementEntry(Some(0))`, then assert `edit_status().can_redo`.

### [x] VIEW-19 — An in-flight vocal render installs audio at placements that were deleted or moved while it ran — fixed @9fc073b3
- **Severity:** medium
- **Confidence:** high
- **Category:** correctness
- **Location:** `update/compose/vocal_audio_install.rs:22-71`, `update/compose/vocal_render.rs:152-162`
- **Problem:** The `(placement_id, start_sample)` list is captured when the render is queued, and the only guard on completion is the lane epoch. Deleting or moving a placement, or inserting bars, does not bump the epoch. Full replay resets epochs to 0, so a pre-undo render can also collide with a later one.
- **Failure scenario:** Generate a vocal, then delete its placement or insert bars before it while the ONNX render runs. `LoadClipFromWav` places the audio at the old position, or for the deleted placement.
- **Suggested fix:** On completion, re-resolve each `placement_id` against current placements: skip missing ones and recompute the start from the current `start_bar`. Take epochs from a global monotonic counter.
- **Verification:** Add a `compose` group-binary test: queue the render, delete the placement, deliver a synthetic `VocalAudioReady`, and assert no clip was loaded.

### [x] VIEW-20 — Chord-lane drag preview doesn't move during the drag — fixed @7f698488
- **Severity:** medium
- **Confidence:** high (spot-checked)
- **Category:** ux
- **Location:** `view/compose/chord_lane/mod.rs:109-124`, `view/compose/chord_lane/input.rs:112-140`
- **Problem:** `CursorMoved` updates `pending_start_beat`/`pending_duration_beats`, and `draw_into` renders them through `apply_drag_preview`. The cache fingerprint only has `drag_active: bool`, so after the first frame of the drag the cached geometry is reused.
- **Failure scenario:** Drag a chord 8 beats. It stays drawn at its old position until mouse-up, then jumps. Resize behaves the same way.
- **Suggested fix:** Add `(chord_id, pending_start_beat, pending_duration_beats)` to `ChordLaneFingerprint`, or draw the dragged chord in an uncached overlay layer.
- **Verification:** Add a unit test in the `compose` group binary: the fingerprint must change as `pending_start_beat` changes.

### [x] VIEW-21 — The bottom rows can't be reached when automation or take lanes are expanded (vertical scroll clamp) — fixed @9c12db23
- **Severity:** medium
- **Confidence:** high (spot-checked)
- **Category:** ux
- **Location:** `update/viewport.rs:24-38`
- **Problem:** `scroll_y_delta` and `scroll_to_y` cap at `tracks.len() * TRACK_HEIGHT`, ignoring the automation, take and group-header rows that `ArrangeRowLayout::total_height()` includes.
- **Failure scenario:** Two tracks with six automation lanes each give 720 px of content in a 400 px viewport, which needs 320 px of scroll. The cap is 192, so the last three lanes can't be reached.
- **Suggested fix:** Clamp to `timeline_content_height - viewport_height`.
- **Verification:** Add a `timeline` group-binary test: expand lanes, send `ScrollY(1e6)`, and assert `scroll_offset_y == content_h - viewport_h`.

### [x] VIEW-22 — Clip drag snaps to a flat grid that ignores the tempo map — fixed @6072e123
- **Severity:** medium
- **Confidence:** high
- **Category:** correctness
- **Location:** `update/clips.rs:276,325,705,761`, `view/timeline/snap.rs:16-24`
- **Problem:** `snap_sample_to_grid` builds `TempoMap::default()` (denominator 4, no tempo points) and uses the tempo at the playhead. The ruler, markers, loop and placement all use `snap_sample_to_grid_tempo(&r.tempo_map)`.
- **Failure scenario:**
  - In a 6/8 song, clips snap to every other drawn bar.
  - With a tempo change at bar 9, clips dragged past bar 9 snap off the drawn grid, and the grid shifts with the playhead position.
- **Suggested fix:** Use `snap_sample_to_grid_tempo(..., &r.tempo_map)` in all four reducers.
- **Verification:** Add a `timeline` group-binary test with a 6/8 map: drag a clip to near bar 2 and assert the snapped sample equals `bar_to_sample(2)`.

### [x] VIEW-23 — Pan-knob drag stops at the knob's 28 px edge, so full pan takes several gestures — fixed @a32e9cc3
- **Severity:** medium
- **Confidence:** high (spot-checked)
- **Category:** ux
- **Location:** `view/knob.rs:225-233`
- **Problem:** `CursorMoved` uses `cursor.position_in(bounds)?`, which returns `None` once the cursor leaves the 28 px widget, while the drag math expects 140 px of travel.
- **Failure scenario:** Grab a centred pan knob and drag up 100 px. The value stops at about R20. Reaching L100 or R100 takes 3-5 separate drags.
- **Suggested fix:** Store the absolute y at press and use `cursor.position()` while `drag_anchor_y.is_some()`.
- **Verification:** Add a `mixer` group-binary test: drive the knob program's `update` with a press and then a move outside the bounds, and assert it publishes `on_change(1.0)`.

### [x] VIEW-24 — Drift in grid-7 drum groups accumulates across the section — fixed @c7dfef0f
- **Severity:** medium
- **Confidence:** high (spot-checked)
- **Category:** correctness
- **Location:** `update/compose/drum_groups.rs:974-989` (`emit_span_notes`)
- **Problem:** `step_ticks = 480 / 7 = 68`, and `start_tick = absolute_step * 68`, so each beat loses 4 ticks and the error accumulates across the whole section. The UI allows grid 7.
- **Failure scenario:** In an 8-bar 4/4 section with a grid-7 group, the last hit lands 128 ticks (about 67 ms at 120 BPM) early. Each bar is audibly further ahead.
- **Suggested fix:** Compute `start_tick = step * TPQ / grid`, and make each duration the difference to the next onset.
- **Verification:** Add a `compose` group-binary test asserting that the first step of the last bar lands exactly on `bar * 4 * TPQ`.

### [x] VIEW-25 — MIDI Import modal is a dead end: a dropped file stays on "Parsing…" and there is no file chooser — fixed @14f059e0 (Confirm imports as one undo; tempo-conflict choices)
- **Severity:** medium
- **Confidence:** high
- **Category:** ux
- **Location:** `update/import.rs:63-69,141`, `view/import_dialog.rs:105-118`, `update.rs:278-283`, `view/transport.rs:106`
- **Problem:** `FileDropped` sets `stage = Parsing` and returns `Task::none()`. Nothing in the crate ever produces `ParseCompleted`, and `Confirm` is a no-op. The dialog text offers "choose one" but has no button to do so.
- **Failure scenario:** Drop a `.mid` file on the window. The modal sits on "Parsing…" forever and only Cancel exits.
- **Suggested fix:** Spawn the parse task (with a stale-path guard) and add a chooser, or hide both entry points until the pipeline exists.
- **Verification:** Add a `control` group-binary test: `FileDropped` must eventually yield `ParseCompleted`, or the entry points must be absent from the rendered golden.

### [x] VIEW-26 — Plugin parameter panel clones every parameter on every frame — fixed @6e62a839
- **Severity:** medium
- **Confidence:** high
- **Category:** performance
- **Location:** `view/mixer/plugin_panel.rs:31-48`
- **Problem:** There is no `lazy` and no cache. The panel builds a `Vec<UiParam>` with two `String` clones per parameter and a widget per parameter, and the Mixer re-renders at the fast tick while meters are live (`update/tick.rs:62`). This violates ui-work §11 (lazy-wrap non-live regions).
- **Failure scenario:** Select a synth with about 1,000 parameters during playback. Every frame rebuilds roughly 2,000 Strings and 1,000 widgets.
- **Suggested fix:** Wrap the panel in `lazy`, keyed on `instance_id`, a hash of the parameter values and text, `editor_open`, `has_gui` and availability.
- **Verification:** Existing mixer goldens must still pass (`RESONANCE_BLESS=1` only if the pixels legitimately change). Add a fingerprint unit test in the `mixer` binary.

### [x] VIEW-27 — Browser Files tab rebuilds the whole folder listing every frame, with uncached thumbnails — fixed @c1c0dfbe
- **Severity:** medium
- **Confidence:** high
- **Category:** performance
- **Location:** `view/browser/mod.rs:75`, `view/browser/files_tab.rs:305-320,356-407`, `view/browser/style.rs:29-73` (`WaveThumbnail` has `State = ()`), `view/browser/pool_tab.rs:228`
- **Problem:** Each frame builds O(files) rows, each with display-name, duration and lowercase Strings plus a HashMap lookup. `WaveThumbnail` tessellates a new `Frame` on every draw. The audition transport inside the panel is live, so this runs at 60 Hz while previewing.
- **Failure scenario:** Open a folder of 3,000 samples and audition one. Every tick allocates about 9,000 Strings and re-tessellates every visible thumbnail.
- **Suggested fix:** Put the listing in `lazy`, keyed on scan identity, filter and selected/playing path, and keep the transport outside it. Precompute the display strings once per scan. Give `WaveThumbnail` a `canvas::Cache`.
- **Verification:** Existing browser goldens (`control` binary) must stay unchanged. Add a unit test of the fingerprint key.

### [x] VIEW-28 — Arrange redraw tessellates every waveform and MIDI column of a partly visible clip — fixed @2fc9eaf0
- **Severity:** medium
- **Confidence:** high
- **Category:** performance
- **Location:** `view/timeline/draw/clip.rs:186-208,218`, `view/timeline/draw/midi_notes.rs:152-178`
- **Problem:** The cull window skips only clips that are entirely off-screen. The column loop runs over the clip's full width. The frozen-MIDI silhouette calls `notes.iter().any()` for every column. `draw_crossfades` is O(n²) over all clips. All of this reruns on every cache miss: every edit, and every scroll of half a viewport.
- **Failure scenario:** A 5-minute clip at 100 px/s tessellates about 30k rectangles per invalidation. A frozen 2,000-note clip at 200 px/s takes about 48M note checks.
- **Suggested fix:** Clamp the column loop to the visible window. Walk the sorted notes with a moving cursor instead of `any()`. Group clips by track before pairing crossfades.
- **Verification:** The timeline goldens must be pixel-identical (no re-bless). Optionally add a timing smoke test in the `timeline` binary.

### [x] VIEW-29 — Batch relink walks the chosen folder tree on the UI thread — fixed @a19e6cad (async folder scan)
- **Severity:** medium
- **Confidence:** high
- **Category:** performance
- **Location:** `update/relink.rs:141-178` (`scan_folder_for_names`, recursive to depth 24, called inside `update()`)
- **Problem:** A recursive filesystem walk runs synchronously in the reducer.
- **Failure scenario:** In the Relink modal, choose "Search folder" and pick `~` or a large sample drive. The whole app, meters and transport included, freezes until the walk finishes.
- **Suggested fix:** Run the walk in `Task::perform` with `spawn_blocking`, and start the imports from the result message. Guard against a modal that was closed in the meantime.
- **Verification:** Add a `control` group-binary test asserting that `start_batch_relink` returns a non-empty `Task` and does not mutate state until the result message arrives.

### [x] VIEW-30 — Pool "used ×N" badges and `pool_list.usage_count` go stale after clip delete, split or track removal — fixed @4020e52b
- **Severity:** low
- **Confidence:** high
- **Category:** correctness
- **Location:** `state/pool.rs:278-297` (`recompute_pool_usage`), which is not called from `engine_events/clips.rs:47`, `engine_events/tracks.rs:133`, `update/clips.rs:192` or `update/arrangement.rs:188`
- **Problem:** Usage counts are recomputed only on pool add/remove, relink, load and undo replay.
- **Failure scenario:** Delete the only clip that uses an asset: the badge still reads "used ×1". Split a clip: the badge stays at ×1 instead of ×2. MCP `pool_list` reports the same stale values.
- **Suggested fix:** Recompute after every clip add/remove/split and after track removal, or derive the counts from `clips` when they are read.
- **Verification:** Add a `control` group-binary test: place, delete, then assert `usage_count == 0`.

### [x] VIEW-31 — Vocal-roll cache fingerprint misses chord quality/bass, voicebank and voice — fixed @633cdad6
- **Severity:** low
- **Confidence:** high
- **Category:** correctness
- **Location:** `view/compose/vocal_roll/draw.rs:31-35,43-66`, `grid.rs:133,201`, `draw.rs:94`
- **Problem:** Only the chord `root` is hashed, but the label draws root plus quality. The voicebank changes the phoneme mapping and `voice` is painted as the voice label, yet neither is hashed.
- **Failure scenario:** With the vocal roll open, change C to Cm: the strip still shows "C". Switching voicebank leaves the old phoneme symbols on screen.
- **Suggested fix:** Hash the chord's `Display` string, `params.voicebank` and `params.voice`.
- **Verification:** Add a fingerprint unit test in the `compose` binary.

### [x] VIEW-32 — Vocal lane and drum ribbon canvases have no geometry cache and re-tessellate on every 16 ms tick — fixed @5ea94049
- **Severity:** low
- **Confidence:** high
- **Category:** performance
- **Location:** `view/compose/vocal_lane/mod.rs:110-118,175-231`, `view/compose/drumroll/ribbon.rs:247-330`
- **Problem:** Both create a new `Frame` on every draw: staff lines, lyric layout and every derived note, or the hatch loops. `vocal_lane::view` also rebuilds a HashMap over all `derived_clips` every frame. Their sibling canvases were moved to fingerprinted `canvas::Cache`; these two were not (ui-work §11).
- **Failure scenario:** During playback in Compose these two redo all their geometry at about 60 fps. The cost grows with section length and note count.
- **Suggested fix:** Add a `Cache` plus a fingerprint, following `ComposeDrumCanvasState`, and draw the hover wash in an uncached overlay.
- **Verification:** The compose goldens must be pixel-identical.

### [x] VIEW-33 — The timeline's in-canvas vertical scrollbar is drawn at the far right end of the song — fixed @21e69aa3
- **Severity:** low
- **Confidence:** high
- **Category:** ux
- **Location:** `view/timeline/scrollbar.rs:37`, `view/timeline/input/hit_test.rs:20-40`
- **Problem:** The bar sits at `x = bounds.width - THICKNESS`, but the canvas is `Fixed(content_w)` inside a horizontal Scrollable, so `bounds.width` is the full song width.
- **Failure scenario:** In any song wider than the window the vertical scrollbar can't be seen or clicked unless you scroll horizontally to the very end.
- **Suggested fix:** Position the bar from the visible-viewport probe, or move it out of the canvas.
- **Verification:** Add a golden in the `timeline` binary with a long song. The scrollbar must be visible at the right edge of the window.

### [x] VIEW-34 — Stacked or very short vocal notes add time that doesn't exist on the timeline, so the vocal drifts behind the MIDI — fixed @7afffe1f
- **Severity:** low
- **Confidence:** high
- **Category:** correctness
- **Location:** `compose/vocal_svs/segment/duration.rs:293-309`, `render_cache.rs:108-116`
- **Problem:** `slot_sec = (slot_ticks * spt).max(0.05)`. Two notes on the same tick, or any note shorter than 50 ms, get extra time. Every later phoneme in the render unit shifts, and the drift accumulates until the next silence split.
- **Failure scenario:** Draw a two-note chord in the vocal roll, or a 1/32 run at 160 BPM. The rest of the phrase sings late, with no error reported.
- **Suggested fix:** Merge or reject overlapping and zero-slot notes before building, or take the 50 ms floor out of the neighbouring slot so the segment total equals the tick span.
- **Verification:** Add a `compose` binary test: the summed segment durations must equal the tick span converted to seconds.

### [x] VIEW-35 — `vocal.generate` with an explicit seed and `lyrics=true` actually uses seed + 1 — fixed @df19db92
- **Severity:** low
- **Confidence:** high
- **Category:** correctness
- **Location:** `update/compose/mod.rs:494-503`, `util.rs:75` (`bump_seed(seed, 0) == seed + 1`), `vocal_render.rs:62`
- **Problem:** The comment says "mix 0 leaves it be", but `bump_seed` always increments. With `seed=S, lyrics=true` the lyrics and melody come from S+1, and S+1 is persisted. With `lyrics=false` the melody uses S. The existing test compares `lyrics=true` only against itself, so it passes.
- **Failure scenario:** `vocal_generate(seed=1234, lyrics=true)` followed by `vocal_generate(seed=1234, lyrics=false)` gives a different melody, and the stored seed is 1235.
- **Suggested fix:** When the seed is explicit, use it directly without bumping.
- **Verification:** Extend `tests/control/control_vocal_generate.rs`: the melodies from the two calls must be equal and the stored seed must be 1234.

### [x] VIEW-36 — Audition scrub strip keeps showing the previous file's waveform — fixed @ca56cbc8
- **Severity:** low
- **Confidence:** high
- **Category:** correctness
- **Location:** `view/browser/pool_tab.rs:381-393`, `update/browser.rs:210`
- **Problem:** The fingerprint mixes `peaks.len()`, position, total frames and playing state, but not the peak data. The length is always `THUMBNAIL_BUCKETS`.
- **Failure scenario:** With auto-play off, select loop A and then loop B of the same length. The strip still draws A.
- **Suggested fix:** Mix the selected path, or the peaks' data pointer, into the fingerprint.
- **Verification:** Add a fingerprint unit test in the `control` binary.

---

## Control API / MCP server / agent plugin

### [x] CTL-01 — Wire "beat" means two different units: views report meter beats, `PositionSpec` resolves quarter notes — fixed @102ec1da (meter beats in and out; beat past bar → invalid_params)
- **Severity:** high
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-app/src/update/control/transport.rs:202-231` (`resolve_position`: `ticks = (beat - 1.0) * TICKS_PER_QUARTER_NOTE`), `resonance-app/src/update/control/view_model/position.rs:12-19` (`song_position` → `TempoMap::position_to_bars`), `resonance-audio/src/types/tempo/bars.rs:121-165` (beats counted in signature-beat units: `numerator` beats per bar, `samples_per_signature_beat(.., denominator, ..)`), `resonance-control/src/common.rs:9-53` (docs say only "1-based beat within the bar")
- **Problem:** Every `SongPosition` the server emits (`song.summary.playhead`, clip/section/meter positions) counts beats in the time signature's beat unit (eighths in 6/8 and 7/8, halves in 2/2). Every `PositionSpec` the server accepts is converted as if `beat` were a quarter note. The two agree only in x/4. A position read from the app therefore does not round-trip. `resolve_position` backs `transport.seek`, `transport.loop_set`, `clip.place`, `clip.move`, `clip.split`, `clip.trim` and the `meter.*` range windows, so all of them land in the wrong place in any non-x/4 bar. Nothing in the protocol docs, the tool descriptions or the skills says which unit "beat" is. The same word also means "quarter note" in the clip- and section-relative `start_beat` / `duration_beats` of `notes.*` and `song.notes` (`song.rs:161-172`, `start_tick / TPQ`), so an agent writing a 6/8 part has no way to tell.
- **Failure scenario:** Song in 6/8 (a bar is 3 quarters, or 6 eighth-note beats). `song.summary` reports the playhead at `{bar: 2, beat: 4}`, the 4th eighth of bar 2. The agent calls `transport_seek {bar: 2, beat: 4}`. `resolve_position` computes `(4-1) * 480 = 1440` ticks, which is 3 quarters and so a whole 6/8 bar. The playhead lands on bar 3 beat 1. `clip_split {at: {bar: 5, beat: 4}}` splits at the bar-6 downbeat instead of mid-bar. In 7/8, `beat: 7` resolves past the bar line, and nothing rejects it.
- **Suggested fix:** Pick one unit. Signature beats are the right choice for `PositionSpec`, because that is what `SongPosition` already reports. In `resolve_position`, convert `beat - 1` to ticks with `beat_len_ticks(denominator of the bar)` from the tempo map (the same helper `samples_per_signature_beat` uses). Also reject `beat >= numerator + 1` instead of letting it roll into the next bar. Document in `common.rs` (`SongPosition` / `PositionSpec`), in the server `INSTRUCTIONS` string (`resonance-mcp/src/server.rs:31`) and in the `transport_seek` / `clip_*` tool descriptions that bar-level beats are meter beats while clip- and section-relative `*_beat` fields are quarter notes. Mirror that in the drumming, arranging and song-structure skills. This changes one resolver plus docs, so it lands as one slice (it touches no tool or wire shape).
- **Verification:** Add to the `control` group binary (`resonance-app/tests/control/control_transport.rs`): set 6/8, `transport.seek {bar:2, beat:4}`, then assert `song.summary.playhead == {bar:2, beat:4}` and that the sample equals `bar_to_sample(1) + 3 eighths`. Add the same round trip for `clip.split` and a `meter.measure` range.

### [x] CTL-02 — `notes.edit` that moves a note earlier/later AND changes duration/velocity edits the wrong note — fixed @91a200af
- **Severity:** high
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-app/src/update/control/notes.rs:252-298` (fan-out: `MoveNote` first, then `ResizeNote` / `SetNoteVelocity` with the ORIGINAL `index`), `resonance-app/src/engine_events/midi.rs:298-323` (`apply_note_move` re-sorts `clip.notes`), `resonance-audio/src/engine/midi/clips.rs:261-301` (engine `handle_move_midi_note` re-sorts; `handle_resize_midi_note` indexes the re-sorted vec)
- **Problem:** Both the app mirror and the engine re-sort the clip's notes by `start_tick` after a move. The follow-up resize and velocity sub-edits still address `note_index: index`. After a reordering move, that index points at a different note. The mirror and the engine agree with each other, so nothing flags it, and the wrong note is silently changed. The in-code comment ("address the note by its original index, exactly as the engine does") records the behaviour but not its consequence.
- **Failure scenario:** The clip holds C@0, D@1, E@2 (beats). `notes_edit {clip_id, index: 0, start_beat: 3, duration_beats: 0.25, velocity: 30}`. The move re-sorts the notes to [D@1, E@2, C@3]. The resize and velocity edits then hit index 0, which is now D. Result: D is shortened and quiet, and C keeps its old duration and velocity. The reply is a success `{revision}`. The existing test (`control_notes.rs:144`) uses a one-note clip, so it cannot catch this.
- **Suggested fix:** In `notes.rs::edit`, dispatch `ResizeNote` and `SetNoteVelocity` BEFORE `MoveNote`, while `index` is still valid, and keep the move last. Alternatively, recompute the post-sort index (partition point of `(want_start, …)` with stable-sort tie handling) and use it for the later sub-edits. Keep everything inside the existing `with_compound_undo`. The fix is handler-only, with no wire or tool change.
- **Verification:** `control` group, `control_notes.rs`: a three-note clip, a `notes.edit` that moves index 0 past the others and also sets duration and velocity. Assert via `song.notes` and via the captured `AudioCommand`s that the moved note carries the new duration and velocity and the others are untouched.

### [x] CTL-03 — The "one revision bump / one edit_undo per call" contract is broken in three places — fixed @3a4a8ce0 (one compound undo per control call)
- **Severity:** medium
- **Confidence:** high
- **Category:** api-design
- **Location:** contract stated in `resonance-mcp/src/server.rs:37-41` (INSTRUCTIONS: "bumps exactly once per mutating call … One edit_undo takes back one whole call"). Violations:
  - `resonance-app/src/update/control/section.rs:75-108`: `section.create` with `scale` dispatches `CreateSection` then `SetSectionScale`. Both classify as `Record` (`undo/classify.rs:508`), outside `with_compound_undo`, so the call makes 2 bumps and 2 undo entries.
  - `resonance-app/src/update/control/track/lifecycle.rs:115-124`: `track.delete` on a track WITH clips dispatches `RequestRemoveTrack` then `ConfirmRemoveTrack`. Both are `Record` (`classify.rs:231`), so the call makes 2 bumps and 2 undo entries. The first entry snapshots an unchanged project, so a second `edit_undo` is a silent no-op. The test (`control_track_mixer.rs:158`) only deletes an empty track.
  - Coalescing: `mixer.set_volume(_db)`, `mixer.set_pan`, `master.set_volume` and `*.set_plugin_param` classify as `RecordCoalesced` with no time window (`undo/history.rs:116`). Two separate calls on the same control, even with a `meter_measure` between them, merge into one undo entry.
- **Problem:** Agents are told to detect concurrent user edits from revision deltas and to back out single decisions with `edit_undo`. The mixing skill states: "edit_undo reverses them one at a time". Both mechanisms give wrong answers in the cases above.
- **Failure scenario:** (a) `section_create {scale:…}` returns a revision 2 higher than before. The agent concludes the user is editing concurrently and re-reads or stops. `edit_undo` then removes only the scale and leaves the section in place. (b) Mixing loop: `mixer_set_volume_db {t, -6}` → `meter_stems` → `mixer_set_volume_db {t, -4}` → the agent decides −4 was wrong and calls `edit_undo`. The fader goes back to its pre-−6 value, not to −6.
- **Suggested fix:** Wrap every multi-dispatch handler in `app.with_compound_undo`: at least `section.create`, `track.delete` and `transport.set_tempo`, and audit the others listed by `grep run_via_update` with more than one call per function. For coalescing, either break the coalesce run at the start of each control request (e.g. `app.undo.break_coalesce()` in `control::execute` before dispatching a mutating method) or run each control mutation in a compound group, which already records its opening edit plain (`undo/mod.rs:96-103`). Update the `set_plugin_param` doc comment (`track/params.rs:17`), which currently documents the coalescing as intended. Handler-only change.
- **Verification:** `control` group (`control_edit_undo.rs`): assert `revision == before + 1` and a single undo entry for `section.create` with scale and for `track.delete` of a track with a MIDI clip. Assert that two `mixer.set_volume_db` calls produce two undo entries and that one `edit.undo` restores the first call's value.

### [x] CTL-04 — `track_add_instrument` is described as "set" and marked idempotent, but it appends a second instrument — fixed @2fc4faf5 (set semantics, swap in place)
- **Severity:** medium
- **Confidence:** high
- **Category:** api-design
- **Location:** `resonance-mcp/src/tools/trackmix.rs:52-75` (`"Set a track's instrument…"`, `idempotent_hint = true`), `resonance-control/src/methods/track.rs:23-25` ("set a built-in instrument"), `resonance-app/src/update/control/track/lifecycle.rs:147-260` (no check for an existing instrument), `resonance-app/src/update/plugin.rs:33-59` (`AddPluginToTrackWithId` pushes onto the chain). The team already knows about the append: `resonance-app/tests/control/control_track_remove_effect.rs:177-181`.
- **Problem:** Every call appends another instrument slot. The wording and the idempotent hint both tell the model that re-calling it, or calling it to switch sound, is safe.
- **Failure scenario:** The track already has `com.resonance.wavetable`. The agent calls `track_add_instrument {plugin_id: "com.resonance.drums"}` to switch the sound, or retries after a timeout. The track now carries two instruments, and slot 1 overwrites slot 0's output. A later `track_set_plugin_param` with no `plugin_id` resolves "the track's instrument" to the FIRST instrument entry (`presets.rs:61-64`, `params.rs` likewise), so it tweaks the synth that is no longer heard. CPU cost doubles, and each retry adds another instance.
- **Suggested fix:** In `add_plugin` with `PluginRole::Instrument`, reject with `invalid_params` when the track already has an instrument, and point the error at `track.replace_effect {slot: 0}`. Or make `add_instrument` replace slot 0 through the `replace` path. Update the wire doc and the tool description to match, and drop `idempotent_hint` unless the call becomes a true set. One vertical slice: handler, doc and tool together.
- **Verification:** `control` group (`control_track_add_plugin_result.rs`): a second `track.add_instrument` on a track that has one either fails with the pointer to `replace_effect` or leaves exactly one instrument, depending on the chosen semantics.

### [x] CTL-05 — Unbounded bar counts overflow u32: debug panic in the update loop, or a release-mode wrap that bypasses `remove_bars`' confirm — fixed @30939036 (MAX_BARS=100000)
- **Severity:** medium
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-app/src/update/control/arrangement.rs:36-46` (`check_range` only rejects 0) and `:122` (`params.at_bar + params.count - 1` in the confirm message); `resonance-app/src/update/arrangement.rs:117` (`at_bar.saturating_sub(1) + count`), `:146`, `:212`, `:246-247`; `resonance-app/src/update/control/section.rs:62,147,205-215` (no upper bound on `length_bars` / `start_bar`); `resonance-app/src/compose/invariants.rs:13,21` (`start_bar + length_bars`); `resonance-app/src/update/control/view_model/position.rs:52` (`p.start_bar + def.length_bars` on every `song.summary`)
- **Problem:** `notes.*` and `clip.*` are capped (`MAX_BEATS`, `MAX_SECONDS`), but bar-valued params are raw `u32` with no ceiling.
- **Failure scenario:** `arrangement_remove_bars {at_bar: 2, count: 4294967295}` on a project with content. Debug build: `cut_span` overflows inside `removal_casualties`, before the confirm check, and the panic in `update()` takes the app down. Release build: `1 + count` wraps to 0, the cut span becomes empty, `casualties` is empty, so no confirm is required, and `remove_bars` then runs with wrapped `from_bar` / `bar - count` arithmetic and corrupts markers and events. Likewise `section_place {start_bar: 4294967295}`: the debug build panics in `invariants.rs:13`. In release, the placement is stored and every later `song.summary` computes `start_bar + length_bars` wrapping.
- **Suggested fix:** Add a `MAX_BARS` constant in `resonance-control` (e.g. 100_000, next to `MAX_BEATS`) and enforce it in `arrangement::check_range` (`at_bar`, `count` and `at_bar + count`), `section.create` / `resize` (`length_bars`), `section.place` (`start_bar`), `notes.create_clip.start_bar` and `global.*` `bar` / `new_bar`. Use checked arithmetic in the confirm message. Document the bound on the params structs so the MCP schema shows it. One slice: constant, handler checks and schema doc.
- **Verification:** `control` group (`control_arrangement_bars.rs`, `control_section_harmony.rs`): huge `count` / `start_bar` / `length_bars` values return `invalid_params`, and the project is unchanged.

### [x] CTL-06 — Two skills tell the agent to "repair" tempo/meter events after insert/remove bars; that defect was fixed, so following them double-shifts the global tracks — fixed @f77148d3
- **Severity:** medium
- **Confidence:** high
- **Category:** docs
- **Location:** `resonance-agent-plugin/skills/song-structure/SKILL.md:159-169` and `:212-214`; `resonance-agent-plugin/skills/drumming/SKILL.md:158-164`. Fixed in commit `835a9299` ("Shift tempo & signature events with insert_bars/remove_bars (ba todo #1388)"); see `resonance-app/src/update/arrangement.rs:22-50` and `ShiftResult.tempo_events_moved` / `signature_events_moved` in `resonance-control/src/methods/arrangement.rs`.
- **Problem:** The skills still say "Confirmed defect, not yet fixed — ba todo #1388 … put the affected events back where the music went … tell the user you had to". Insert/remove now move events (and clamp events inside a removed span).
- **Failure scenario:** 7/8 event at bar 33, bridge placement at bar 33. The agent calls `arrangement_insert_bars {at_bar: 20, count: 8}`, which moves both to bar 41. Following the skill's worked example ("strands the 7/8 event at bar 33 while the bridge moves to bar 41"), the agent reads `global_list_events`, sees 7/8 at 41 and assumes it still has to act. It adds 7/8 at "where the music went" again, or treats bar 41 as the stale copy and removes or moves it. It also reports a nonexistent defect to the user. The lockstep test cannot catch this; see CTL-12.
- **Suggested fix:** Replace both passages with the current behaviour: events move by `±count`, bar 1 stays pinned, events inside a removed span clamp to the cut, and the later event wins on collision. Tell the agent to verify with `ShiftResult.tempo_events_moved` / `signature_events_*` and `global_list_events` rather than repair by hand. Bump the plugin `version`.
- **Verification:** Covered by CTL-12. Until then, grep `resonance-agent-plugin` for `1388` in review.

### [x] CTL-07 — The MCP client holds its single connection mutex for the whole `job.wait`, so every other tool call stalls for up to 5–10 minutes — fixed @515b6fde (250 ms job.wait slices)
- **Severity:** medium
- **Confidence:** high
- **Category:** api-design
- **Location:** `resonance-mcp/src/client.rs:227-246` (the lock is held across `round_trip`), `:337-341` (630 s read deadline for `job.wait`), `resonance-mcp/src/server.rs:127-165` (`invoke_job` waits `wait_ms` for up to 300 s, used by `render_mixdown`, `meter_*`, `vocal_render`, `pool_import`, `clip_place`), `resonance-mcp/src/tools/render.rs:63-75` (`job_wait`: "omit timeout_ms to wait indefinitely"), `resonance-app/src/control_socket.rs:331-337` (the server also serializes the connection behind the wait)
- **Problem:** There is one socket per MCP server process, and `call_blocking` keeps the `Mutex<Option<Connection>>` locked for the whole round trip. A `meter_stems` or `render_mixdown` holds it for up to 300 s, and a bare `job_wait` for up to 600 s. Any concurrent tool call from the same session (MCP clients do issue parallel calls) blocks on the mutex. That wait is not covered by `CallTimeouts`, so it never becomes `Unresponsive`. If the MCP client cancels the tool, the `spawn_blocking` task keeps both the lock and the wait. The `job_wait` description ("wait indefinitely") also contradicts the server's 600 s cap (`control_jobs.rs:39`).
- **Failure scenario:** The model issues `meter_stems` (a 4-minute render on a 20-track project) in parallel with `song_summary`. `song_summary` returns only after the meter job finishes or the 300 s wait expires. A user-visible "cancel" frees nothing.
- **Suggested fix:** Keep the blocking wait off the shared connection. Either open a dedicated short-lived connection for each `job.wait` in `invoke_job` / `job_wait` (job ids are global, and `JobBoard` checks no ownership), or poll `job.status` with a short sleep outside the lock. Clamp the tool's `timeout_ms` to 600 000 and say "at most 10 minutes" in the description. MCP-side only.
- **Verification:** `resonance-mcp/tests/client_timeout.rs`: against a fake server whose `job.wait` blocks, a concurrent `song.summary` call returns promptly.

### [x] CTL-08 — The `ii-V-I` progression preset can never match (the lookup lowercases, the table key is mixed-case) — fixed @eeaf49a3
- **Severity:** low
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-app/src/update/control/harmony.rs:271` (`("ii-V-I", …)`) vs `:311-313` (`wanted = preset.trim().to_ascii_lowercase(); … *name == wanted`)
- **Problem:** `"ii-V-I"` is compared as `"ii-v-i"` against the key `"ii-V-I"`, so it never matches. The rejection lists `ii-V-I` as valid, which invites a retry that fails the same way. The arranging skill documents this as a known bug ("The advertised `ii-V-I` preset is unreachable") instead of the bug being fixed.
- **Failure scenario:** `harmony_apply_progression {key:{tonic:"C",scale:"major"}, preset:"ii-V-I"}` returns `invalid_params: unknown progression preset "ii-V-I" (one of: …, ii-V-I, …)`.
- **Suggested fix:** Compare with `name.eq_ignore_ascii_case(&wanted)`, or lowercase the table key. Then drop the workaround line from `skills/arranging/SKILL.md`.
- **Verification:** `control` group (`control_section_harmony.rs`): apply `preset: "ii-V-I"` and `"II-v-i"`, and assert three chords ii, V, I.

### [x] CTL-09 — The mastering skill calls string parameter ids (`glue_on`, `lim_ceiling`, …) addressable, but `*_set_plugin_param` resolves only display names or numeric ids — fixed @bfc5ac27 (docs: display names)
- **Severity:** low
- **Confidence:** high
- **Category:** docs
- **Location:** `resonance-agent-plugin/skills/mastering/SKILL.md` §2 ("parameter ids … `glue_on`, `sat_on`, `mb_on`, `img_on`, `lim_on`, `dith_on` … `lim_ceiling` (dBTP) and `lim_release`") and §5 (`dith_on`); resolver at `resonance-app/src/update/control/master.rs:565-583` and `track/params.rs:89-110` (`p.name.eq_ignore_ascii_case(wanted)` or `parse::<u32>()`); `PluginParamView` (`resonance-control/src/methods/track.rs:618-648`) exposes `id: u32` and `name` ("Glue On") but no string key
- **Problem:** The string ids from `plugins/resonance-mastering/src/params/*.rs` appear nowhere in the API. An agent that follows the skill sends `param: "lim_ceiling"` and gets `not_found`.
- **Failure scenario:** `master_set_plugin_param {plugin_id: "com.resonance.mastering", param: "lim_on", value: 1}` returns `not_found: plugin … has no parameter "lim_on" (has: [Bypass, …, Limiter On, …])`. It can be recovered, but every stage switch costs an extra round trip.
- **Suggested fix:** Either rewrite the skill in display names ("Glue On", "Limiter On", "Ceiling"…) or, better, add a `key` (string id) to `PluginParamView` and accept it in the `set_plugin_param` resolvers. The app already maps string id to CLAP id with `resonance_plugin::stable_hash` (`plugin_presets.rs:176`). As a wire change that is one slice across all three surfaces.
- **Verification:** `control` group (`control_master_params.rs`): `param: "lim_on"` resolves, or the skill passes a new lockstep check on parameter names.

### [x] CTL-10 — Overwriting a user plugin preset is refused as `invalid_params`, not `needs_confirmation` — fixed @75e42520
- **Severity:** low
- **Confidence:** high
- **Category:** error-handling
- **Location:** `resonance-app/src/update/control/plugin_presets.rs:222-242` (`check_save`), used by `track/bus/master.save_plugin_preset` (`chain_presets.rs:189`)
- **Problem:** Every other overwrite guard (`track.save_preset`, `render.mixdown`, `project.save_as`) answers with `ErrorKind::NeedsConfirmation`. Clients branch on the kind, and the MCP client adds its "retry with confirm/overwrite" hint only for that kind (`resonance-mcp/src/client.rs:126-129`).
- **Failure scenario:** `track_save_plugin_preset {name: "Warm"}` over an existing preset returns `invalid_params`. A client that treats `invalid_params` as "my arguments are wrong" never retries with `overwrite: true`.
- **Suggested fix:** Return `RpcError::needs_confirmation(...)` in `check_save` (handler only).
- **Verification:** `control` group (`control_plugin_presets.rs`): assert `ErrorKind::NeedsConfirmation` on a repeated save without `overwrite`.

### [x] CTL-11 — The socket setup chmods whatever directory the socket path's parent is, and trusts a pre-existing `/tmp/resonance-<uid>` — fixed @9ef9201a
- **Severity:** low
- **Confidence:** medium
- **Category:** security
- **Location:** `resonance-app/src/control_socket.rs:120-130` (`prepare_parent_dir`: unconditional `set_permissions(dir, 0o700)`), `resonance-control/src/socket.rs:37-61` (`/tmp/resonance-<uid>` fallback), `resonance-mcp/src/client.rs:262-266` (connects with no peer check)
- **Problem:** (1) With `RESONANCE_CONTROL_SOCKET=$HOME/x.sock` or `…/project/control.sock`, startup silently chmods `$HOME` or the project directory to 0700. With `/tmp/x.sock`, the chmod fails with EPERM and the whole endpoint is disabled. (2) Without `XDG_RUNTIME_DIR` (some macOS or headless setups), another local user can pre-create `/tmp/resonance-<uid>` as their own directory. The app's chmod then fails and the endpoint never starts (DoS). The attacker can also listen on `control.sock` inside that directory, and `resonance-mcp` connects to it with no ownership or peer-credential check, then feeds attacker-chosen "tool results" to the model.
- **Failure scenario:** On a shared host without `XDG_RUNTIME_DIR`, user B runs `mkdir -m 777 /tmp/resonance-1000` and starts a fake server. User A's resonance fails to start its endpoint, and A's `resonance-mcp` talks to B's fake server.
- **Suggested fix:** Only chmod a directory the app created itself. For an existing directory, verify it is owned by the current uid and is not group- or world-writable, and otherwise refuse with a clear error. On the client, check `SO_PEERCRED` / `getpeereid` uid before sending the handshake, or check the socket's parent ownership.
- **Verification:** A `resonance-control/tests/socket_path.rs`-style unit test for an ownership-check helper, plus an app test (control group) that a pre-existing foreign-mode parent directory is refused rather than chmodded.

### [x] CTL-12 — The lockstep test checks only `mcp__resonance__`-prefixed tool names; bare tool names, parameter names and behavioural claims in skills are unchecked — partial @add1fb65 (bare tool names checked; param ids not)
- **Severity:** low
- **Confidence:** high
- **Category:** test-coverage
- **Location:** `resonance-mcp/tests/agent_plugin_lockstep.rs:60-76` (`referenced_tools` scans only the `mcp__resonance__` prefix), `:106-126`
- **Problem:** The skills mostly name tools bare (`section_create`, `generate_part`, `edit_undo`, `master_move_effect`, …) and name parameters and plugin param ids (`place`, `beats_per_chord`, `glue_on`, `lim_ceiling`). A renamed tool or field in bare form passes the suite. Stale behavioural claims (CTL-06, CTL-09) are also invisible to it. The test's own rationale ("a renamed tool … leaves the skills quietly describing a surface that no longer exists") applies equally to those forms.
- **Failure scenario:** Renaming `section_place` to `section_add_placement` keeps the suite green, because `song-structure/SKILL.md` names it bare in 6 places.
- **Suggested fix:** Also extract backticked `` `[a-z]+_[a-z_]+` `` tokens whose prefix is a known namespace (`track_`, `section_`, `notes_`, …) and require them to be published tools. Optionally add a small allow-list file for known wire field names and plugin param ids, checked against the schemars schema and the plugins' `::new("id", …)` declarations. Add a `ba`-referenced "known defect" marker convention (e.g. `<!-- defect: #1388 -->`) that the test cross-checks against a list of still-open todos, so fixed-defect prose fails the suite.
- **Verification:** Temporarily rename a bare-referenced tool locally and confirm the new check fails.

### [x] CTL-13 — `generate.part` accepts `chord_count` / `beats_per_chord` / `sevenths` and silently ignores them — fixed @d3404b84 (rejected with pointer)
- **Severity:** low
- **Confidence:** high
- **Category:** api-design
- **Location:** `resonance-control/src/methods/generate.rs:30-48` (fields documented as working: "Number of chords to generate over", "Include sevenths in generated voicings"), `resonance-app/src/update/control/generate.rs:56-113` (never read), `resonance-mcp/src/tools/compose.rs:22-25` (the tool description admits "the app IGNORES them")
- **Problem:** The published input schema advertises three knobs that do nothing. The wire doc comments contradict the tool description. Silently accepting ignored fields is the "no-op that claims success" pattern the control layer otherwise works to avoid.
- **Failure scenario:** `generate_part {role: "pad", sevenths: true}` returns success with triads. Only a model that happened to read the long description knows why.
- **Suggested fix:** Reject any of the three when set, with `invalid_params` pointing to `harmony_apply_progression`, and fix the struct docs. Better, remove them from `PartParams`: there are no real users, and this is a breaking wire change, so check the `PROTOCOL_VERSION` policy in `lib.rs:22-40` first. One slice.
- **Verification:** `control` group (`control_generate.rs`): `generate.part` with `sevenths: true` returns `invalid_params`.

---

## Audio mixer / RT path

### [x] MIX-01 — Audio thread's playhead store overwrites concurrent Seek / Stop (lost update) — fixed @07efbf58 (CAS publish; commit_playhead false = MIX-06 hook)
- **Severity:** high
- **Confidence:** high
- **Category:** concurrency
- **Location:** `resonance-audio/src/mixer/callback/play.rs:22` (load) and `:113` (store), `:43` (lock-contended path); `resonance-audio/src/mixer/callback/reference.rs:58`; writers racing it: `resonance-audio/src/engine/transport.rs:445` (`handle_seek_to`), `:367` (`handle_stop`), `resonance-audio/src/engine/midi/clock.rs:135,182` (MIDI-clock SPP), `resonance-audio/src/engine/tracks.rs:403`
- **Problem:** `render_playing_block` does `playhead = shared.playhead.load()` at the start of the block, renders the whole arrangement + master pass, and then unconditionally does `shared.playhead.store(new_playhead)` where `new_playhead` is derived from the stale load. Any store the engine thread made in between (seek, stop-to-zero, MIDI-clock relocate) is silently overwritten. The window is the entire render time of the block (typically 10–60 % of the period), so this is not a rare race. Note also that `mix_audio` loads the playhead once for `BlockTiming` (`callback/mod.rs:72`) and `play.rs:22` loads it again — two different values can be used in one block.
- **Failure scenario:** Transport playing, user clicks the ruler at bar 40 while at bar 3. Engine thread stores 40-bar position; audio thread (mid-render) finishes and stores `bar3 + frames`. Playback keeps going from bar 3; the seek is lost (≈ render_time/period probability per seek). Same for Stop: `handle_stop` stores `playing=false, playhead=0`, the in-flight block stores `old+frames`, and the transport is left parked at the old position instead of 0. A slaved MIDI-clock Song Position Pointer can be dropped the same way.
- **Suggested fix:** Make the audio thread's publish conditional: `let _ = shared.playhead.compare_exchange(playhead, new_playhead, AcqRel, Relaxed);` — if it fails, somebody else moved the playhead and theirs wins. Do this in all three audio-thread writers (`play.rs` both paths, `reference.rs`). Use the single playhead value loaded in `mix_audio` (pass `playhead_now` into `render_playing_block` instead of re-loading at `play.rs:22`) so timing, render and CAS all agree. Alternatively route seeks through a `pending_seek: AtomicU64` (sentinel `u64::MAX`) that the callback `swap`s at the top of `mix_audio`; CAS is the smaller change. Pair with MIX-06 (panic on discontinuity) so the seek also flushes voices.
- **Verification:** Add `resonance-audio/tests/playhead_seek_race.rs` (resonance-audio tests are per-file; the no-new-file rule only applies to resonance-app/tests). Deterministic repro: a test CLAP-less hook is hard, so factor the publish into `fn commit_playhead(shared, observed, new)` in `mixer/common.rs`, `pub` via the existing test re-exports, and assert that after `shared.playhead.store(X)` between observe and commit, the value stays X. Also a `MixAudioHarness` test: set playhead P, run one block, store Q, run a block and assert the playhead continues from Q.

### [x] MIX-02 — Live mixer keeps processing the shared plugin instances while an offline render is running — fixed @de0b2a42
- **Severity:** high
- **Confidence:** high
- **Category:** concurrency
- **Location:** `resonance-audio/src/mixer/callback/stopped.rs:20-44`, `resonance-audio/src/mixer/callback/count_in.rs:35-53`, `resonance-audio/src/mixer/monitor.rs:220-233` (live monitor `process()`), `resonance-audio/src/mixer/callback/play.rs` (whole branch); guards that only check at render start: `engine/bounce/wav.rs:321`, `engine/bounce/stem.rs:511`, `engine/bounce/freeze.rs:87`, `engine/bounce/clip.rs:67`, `engine/bounce/measure.rs:136`; `engine/transport.rs:16` (`handle_play` has no offline-render check); counter `SharedState::offline_render_count` (`engine/mod.rs:194`) is never read by the mixer.
- **Problem:** Every offline renderer drives the *same* CLAP instances as the live mixer (documented at `engine/bounce/mod.rs:63`). The only protection is "refuse to start if `playing`". Nothing stops (a) the stopped/count-in branches from processing monitor-enabled tracks' plugin chains through `mix_monitor_passthrough` during an export, (b) `handle_play` (UI or MCP `transport_play`) from starting playback mid-export, or (c) `stem.rs`'s `reset_plugins` from resetting instances the live monitor is using. The bounce's `try_lock_with_backoff` and the live `try_lock` simply alternate on the mutex, interleaving two unrelated audio streams through one stateful plugin.
- **Failure scenario:** Guitar track is monitor-enabled with an amp sim + delay; user exports the mix. The live callback keeps feeding the live guitar input through the amp sim/delay every quantum while the export worker feeds the recorded take in large chunks: the exported file contains delay/reverb tails of the live input and filter state jumps; the live monitor glitches. Or: an MCP client calls `transport_play` while `render_mixdown` runs — the live timeline and the export both advance voice/delay state of every plugin; both outputs are corrupted and non-deterministic.
- **Suggested fix:** In `mix_audio` (`callback/mod.rs`), right after `scratch.data.fill(0.0)`, if `shared.offline_render_count.load(Acquire) > 0` output silence (still run `mix_audition_overlay` if desired, which touches no plugins) and return before any branch that locks a plugin; also skip `pickup_live_midi`'s delivery (leave events queued) in that state. Make `handle_play` / `handle_record` refuse (emit an error event) while `offline_render_count > 0`, mirroring the existing "Stop transport before bouncing" guard in the other direction. Keep the check a single relaxed/acquire atomic load — no locking.
- **Verification:** `resonance-audio/tests/bounce_transport_guard.rs` already covers the start-guard; add cases there: with `offline_render_count` bumped (via `OfflineRenderGuard::mark` through a test hook or by setting the atomic), a `MixAudioHarness` block with a monitor-enabled track must not call the plugin (use a counting test plugin as in `plugin_bypass.rs`) and must output zeros; `handle_play` must leave `playing == false`.

### [x] MIX-03 — Plugin-delay-compensation lines silence `delay` samples at every loop seam (compensated tracks drop out, latent track does not) — fixed @f107e1c4
- **Severity:** medium
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-audio/src/mixer/callback/seam.rs:115-134` (tail sub-block rendered with `playhead: loop_in`); `resonance-audio/src/latency.rs:299-327` (`apply_comp`), `:483-516` (`apply_dry`); callers `mixer/render/track_pass.rs:115`, `mixer/render/sub_track.rs:573`, `mixer/render/bus_pass.rs:108`, `mixer/render_core.rs:83`; test that encodes the behaviour: `resonance-audio/tests/latency_comp.rs:384` (`loop_wrap_discontinuity_never_replays_stale_audio`)
- **Problem:** Each delay line invalidates itself when `playhead != next_playhead`. A loop wrap is such a mismatch, so on every pass each compensated track/bus/dry line outputs `delay` samples of hard zeros (`warmup`) and discards the last `delay` samples it received before `loop_out`. But the output timeline is *continuous* across a seam: the audio in the line is exactly what should be heard next (everything reaches master `max_latency` late). The un-delayed (most latent) track's plugin keeps its internal buffer and *does* play its pre-seam tail, so at each wrap the latent track plays while every compensated track drops to silence for `delay` samples, with a hard step to zero (click) and back.
- **Failure scenario:** Loop bars 1–5 with a 20 ms lookahead limiter (≈960 samples) on the vocal track. Every other track (delay 960) goes silent for 20 ms at every loop wrap with a click at each edge, while the vocal continues. With a linear-phase EQ (~80 ms) the gap is very audible. Offline bounce never loops, so live and bounce differ.
- **Suggested fix:** Tell the delay lines the tail sub-block is continuous. Add a field to `BlockInputs` (e.g. `wrapped_from: Option<u64>` = `Some(loop_out)` for the seam's tail sub-block, `None` elsewhere, including the bounce) and thread it into `LatencyComp::apply/apply_bus/apply_dry` so the continuity check becomes `st.next_playhead == Some(playhead) || (wrapped_from.is_some() && st.next_playhead == wrapped_from)`. Keep invalidation for real seeks. Update `latency_comp.rs:384` to assert tail continuity across a wrap instead of silence. Pitfall: the `tail_frames == 0` aligned case renders a zero-length tail block which also sets `next_playhead = loop_in`; it must carry `wrapped_from` too so the next buffer (starting at `loop_in`) is not treated as a discontinuity.
- **Verification:** In `resonance-audio/tests/latency_comp.rs`, add a `MixAudioHarness` (or `RenderBenchHarness`) test: two DC clips on two tracks, `LatencyComp` giving one track a delay of e.g. 64, loop enabled with a seam falling mid-buffer and one aligned; assert no zero samples in the output across the seam and that both tracks' sums stay constant.

### [x] MIX-04 — Freeing ArcSwap snapshots (LatencyComp / automation / tempo map / frozen cache) can happen on the audio thread — fixed @f3d90760 (retire queue swept in engine loop)
- **Severity:** medium
- **Confidence:** high
- **Category:** rt-safety
- **Location:** guards held for the whole block in `resonance-audio/src/mixer/callback/play.rs:53,57,63,69,75` and `callback/mod.rs:73`; `resonance-audio/src/mixer/render/track_pass.rs:192` (`frozen_source.load_full()`); publishers that drop the old value immediately: `engine/thread/mod.rs:321` (automation, every lane edit), `engine/thread/mod.rs:542` + `engine/transport.rs:450ff` + `engine/midi/clock.rs:165` (tempo via `rcu_tempo`), latency-comp republish on topology/bypass changes, `engine/takes.rs:671`, `engine/busses.rs:249`, `engine/tracks.rs:85,94` (freeze/unfreeze).
- **Problem:** With `arc_swap`, a `Guard`/`load_full()` held by the reader keeps the old `Arc` alive; when the writer `store`s a replacement, the writer drops its reference and the reader's drop at the end of the block becomes the *last* reference, so the destructor (and `free`/`munmap`) runs on the realtime thread. There is no deferred-reclamation ("graveyard") anywhere in the crate. `LatencyComp` owns `DelayLine`s of up to `MAX_COMP_LATENCY` (960 000) floats each — well above glibc's mmap threshold, so freeing it is a `munmap` syscall. A frozen track's cache can be tens of MB.
- **Failure scenario:** Playing back, user toggles bypass on a latency-carrying plugin (republishes the comp table) or unfreezes a track, or drags an automation point (republishes on each edit): with probability ≈ render_time/period per publish, the callback ends by freeing the old table/cache — `munmap` of MBs inside the deadline → xrun/click. With a MIDI-clock slave the tempo map is republished continuously.
- **Suggested fix:** Keep the previous snapshot alive on the publishing (engine) thread until the audio thread can no longer hold it: e.g. a small `Retired<T>` queue on the engine thread that holds `Arc`s just replaced and drops them only once `Arc::strong_count == 1` (checked on the next engine tick), or use `arc_swap`'s pattern of `swap()` returning the old `Arc` and pushing it to such a list. Apply to `automation`, `tempo_map`, `latency_comp`, `take_comp`, `aux_sends`, `sidechain_routes`, `frozen_source` and `Track::plugin_chain`. Do NOT try to "fix" it by dropping on a background thread from the audio side (that needs a channel send which may allocate).
- **Verification:** Add a test in `resonance-audio/tests/` that holds `automation.load()` (simulating the callback), publishes a new snapshot through the engine helper, drops the guard, and asserts via a `Drop`-counting wrapper or `Arc::strong_count` on a clone kept by the retire list that the old value was not destroyed on the reading thread (the retire list still owns it). Run under the existing `assert_no_alloc`-style harness if one exists; otherwise document with a unit test on the retire list.

### [x] MIX-05 — Muting / solo-suppressing a sidechain key source silently switches the keyed plugin to self-keying — fixed @0c2e53c7
- **Severity:** medium
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-audio/src/mixer/render/track_pass.rs:72-75` and `:82-88` (return before the capture at `:94-101`), `resonance-audio/src/mixer/render/strategy.rs:629-652` (`discard_after_instrument`), `:667-668` (bounce), `resonance-audio/src/mixer/render/sub_track.rs:520-529` (sub-track capture after disposition), `resonance-audio/src/mixer/render/bus_pass.rs:58-62`; semantics of a missing key: `resonance-audio/src/types/sidechain.rs:231-242`
- **Problem:** Key capture happens *after* the mute/solo disposition. Once a muted (or solo-suppressed) key-source track has faded out, `track_disposition` returns `None` (audio track) or `render_track_source` returns `None` (instrument track), so `capture` never runs, `key()` returns `None`, and the keyed compressor falls back to its own input. The same applies to sub-track taps and muted busses. Key taps are documented as post-FX / pre-fader, so they should not depend on the source's mute state.
- **Failure scenario:** The classic "ghost kick" — a muted kick track used only to key a bass compressor. As soon as the kick is muted (or another track is soloed), the bass compressor stops ducking to the kick and instead compresses the bass by its own level. Same in the mixdown export (`respect_mute_solo` drops it identically), so the export sounds wrong too.
- **Suggested fix:** When the track/bus is tapped (`scratch.sidechain.is_tapped(...)`), still render its source + FX chain and capture the key, then stop before PDC/fader/routing — exactly the existing `is_key_only` early-return path. Concretely: in `render_one_track`, if disposition is `None` (or `discard_after_instrument`) but the track is tapped, run the source stage with a "key-only" flag, capture, and return. Do the same for sub-tracks (`render_sub_track_tap`) and busses. Pitfall: for the live strategy keep updating `last_gains` to 0 so un-muting still ramps in; for bounce keep the key-only track out of the mix.
- **Verification:** `resonance-audio/tests/sidechain_key_delivery.rs`: add live and bounce cases where the source track is muted (and one where another track is soloed) and assert the keyed test plugin still receives a non-`None` key equal to the source's post-FX signal, while the muted source contributes nothing to the output.

### [x] MIX-06 — Seek during playback and A/B-reference monitoring leave stuck notes (mixer never panics on a playhead discontinuity) — fixed @fb058b81
- **Severity:** medium
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-audio/src/mixer/callback/play.rs` (no discontinuity check), `resonance-audio/src/mixer/callback/reference.rs:23-61` (mix not rendered while playing, no panic on entry/exit), `resonance-audio/src/engine/transport.rs:418-433` (`panic_all_instrument_plugins` uses `try_lock` and silently skips on contention), `:436-446` (`handle_seek_to`), `resonance-audio/src/engine/reference.rs:750-757` (`handle_set_ab_source`, no panic)
- **Problem:** Voices are only flushed at the loop seam (`seam.rs:111`) and by engine-thread `panic_all_instrument_plugins`. The latter `try_lock`s each instrument and *skips it* if the audio thread holds the lock (its comment claims the next seam "will run the queued NoteOffs", but nothing was queued). The mixer itself never detects that the playhead jumped. Separately, while the A/B source is the reference, the playing branch returns before rendering the mix: timeline NoteOffs that fall during that time are never collected, and switching back resumes rendering with those voices still held.
- **Failure scenario:** (1) Heavy synth playing a sustained pad, user seeks while playing: with probability ≈ fraction of the period the audio thread holds that instrument's mutex, the panic is skipped and the pad note sustains forever after the jump. (2) User toggles to the reference during a held chord and back two bars later: the chord's NoteOffs were never delivered, so the notes hang (until the next loop seam or Stop).
- **Suggested fix:** Detect discontinuities on the audio thread, which owns the `MidiStash` and so never loses a panic: keep `expected_playhead: Option<u64>` in `CallbackScratch` (audio-thread owned), set it to `new_playhead` at the end of every rendered playing block; at the start of `render_playing_block`, if `Some(e)` and `e != playhead`, call `panic_instrument_tracks(tracks, plugins, midi_stash)` (it stashes a panic on contention) and `scratch.sidechain.clear()`. In the reference branch and the lock-contended fallback, do not update `expected_playhead` (so the next rendered block sees a mismatch and panics). Reset to `None` in the stopped branch (Stop already panics). Combine with MIX-01's single playhead load.
- **Verification:** `MixAudioHarness` test (add to `resonance-audio/tests/midi_stash.rs` or `clap_all_notes_off.rs`): with a recording `NoteSink`-style test instrument, play a long note, move `shared.playhead` between blocks, assert an all-notes-off arrives in the next block. Second case: set the reference monitor active for N blocks while playing, then back, and assert all-notes-off before any new event.

### [x] MIX-07 — Soloing a multi-output parent drops its sub-tracks from the mixdown export (live vs bounce divergence) — fixed @f67f9df4
- **Severity:** medium
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-audio/src/mixer/render/strategy.rs:739-747` (Bounce arm of `sub_track_disposition`) vs `:714-723` (Live arm); `any_top_level_solo` in `resonance-audio/src/types/track.rs:503`; mixdown passes `respect_mute_solo = true` at `engine/bounce/wav.rs:252-265`, master stem at `engine/bounce/stem.rs:539`
- **Problem:** Solo semantics say sub-tracks follow their parent (`any_top_level_solo` ignores sub-track solo; `tests/solo_predicate.rs:29`). Live honours that: a sub-track is silenced only by its own mute or `parent_silenced`. The bounce arm instead requires the *sub-track's own* solo flag: `respect_mute_solo && (muted || (any_solo && !sub_track.soloed()))`. With the parent soloed, `any_solo` is true and the (un-soloed) sub-tracks are dropped.
- **Failure scenario:** Drum kit on resonance-drums with kick/snare/OH taps routed as sub-tracks; user solos the kit and exports (or MCP `render_mixdown`). Playback has the whole kit; the WAV contains only the parent's port-0 output — kick/snare/overheads missing.
- **Suggested fix:** In the Bounce arm, drop the solo clause for sub-tracks: the parent's `track_disposition` already returned `None` when the parent is solo-suppressed, so the fan-out never runs for it. I.e. `if *respect_mute_solo && muted { return None; }`. Keep the `in_filter` check.
- **Verification:** Add to `resonance-audio/tests/render_block_parity.rs` (or `stem_sub_track_render.rs`): project with a multi-output parent (use the `tests/multi_out_harness` plugin) and sub-tracks; parent soloed; render one block Live and Bounce(`respect_mute_solo: true`) and assert identical sub-track contribution (non-zero).

### [x] MIX-08 — Instrument plugins are never processed while the transport is stopped unless the track is input-monitored with live input; queued notes pile up and burst on Play — fixed @489d2a8f
- **Severity:** medium
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-audio/src/mixer/callback/stopped.rs:20-22` (`monitor.frames == 0 || !monitoring` → return), `resonance-audio/src/mixer/monitor.rs:263-266` (only `monitor_enabled` tracks), `resonance-audio/src/mixer/live_midi.rs:245-263` (still queues into the plugin), `resonance-audio/src/engine/midi/live.rs:96-126` (piano-roll `SendNoteOn`), `resonance-audio/src/clap_host/instance.rs:454-466` (`MAX_PENDING_NOTES` cap drops note-offs too)
- **Problem:** When stopped, only tracks with `monitor_enabled` *and* a delivering input stream are processed. Live hardware MIDI (`pickup_live_midi`) and piano-roll preview notes (`MidiEditorMessage::PreviewNote` → `SendNoteOn`) are still queued into the instrument's `pending_notes`, but `process()` never runs, so they are silent. They accumulate; once 256 are queued, further events — including note-offs — are dropped. On the next Play the first block fires the whole backlog at once (stale offsets), and a note-on whose note-off was dropped at the cap sticks.
- **Failure scenario:** Instrument track without input monitoring (or no audio input device, so `monitor.frames == 0`): clicking piano-roll keys or playing a MIDI controller while stopped produces no sound. After ~128 key presses the queue is full; pressing Play blasts a chord of every queued note and some notes hang.
- **Suggested fix:** In the stopped branch, also render MIDI-accepting tracks whose instrument has pending events or recent live input: simplest is to process every instrument track (the first plugin + its FX chain) with a zeroed input buffer when `!playing`, summing through fader/pan like `mix_monitor_passthrough` does, independent of `monitor.frames` (use `frames`, not `monitor.frames`). Cost can be bounded by only processing instruments that received events in the last N seconds (tracked in a small audio-thread-owned array). Also make `queue_note_off` evict a queued note-on instead of dropping at the cap (mirror `push_capped` in `midi_events.rs:387`).
- **Verification:** `MixAudioHarness` test (e.g. in `resonance-audio/tests/live_note_retry_order.rs` or `audition_preview.rs`): stopped transport, no monitor, send a live note-on to an instrument test plugin and assert it is processed (non-zero output / plugin sees the event) within one block.

### [x] MIX-09 — Input device with more than 32 channels panics the audio callback (monitor scratch overrun) — fixed @3affbcad
- **Severity:** medium
- **Confidence:** medium
- **Category:** rt-safety
- **Location:** `resonance-audio/src/mixer/callback/monitor_input.rs:24-48` (`&mut scratch.monitor_temp[..to_read]`), scratch sizing `resonance-audio/src/engine/mod.rs:692-693` (`audio_buf_frames * MAX_INPUT_CHANNELS`, ring `quantum * MAX_INPUT_CHANNELS * 4`), channel count never clamped: `resonance-audio/src/platform.rs:934` (`desired_channels.max(default_channels)`), `resonance-audio/src/engine/tracks.rs:337-380`, `resonance-audio/src/engine/transport.rs:291`, `resonance-audio/src/input_pipewire.rs:274`
- **Problem:** `MAX_INPUT_CHANNELS` (32) is documented as the limit the monitor scratch is sized for, but the negotiated input channel count is stored into `shared.input_channels` unclamped (cpal path takes the device's default channel count, the PipeWire path takes whatever the format negotiates / `input_port + 2`). `needed = frames * input_channels` then can exceed `monitor_temp.len()` and the slice index panics on the realtime thread. Reachable whenever `frames * C > buf_frames * 32` with enough ring occupancy (e.g. `C > 64` at a normal quantum, or `C > 32` when the graph quantum rises to `buf_frames` and `buf_frames < 4 * quantum`).
- **Failure scenario:** 64+ channel interface (MADI/Dante/large RME) opened via the cpal path, or a user picks input port 40 on a big interface; monitoring enabled → `index out of range` panic inside the output callback → stream dead / process abort.
- **Suggested fix:** Clamp at the source: refuse or cap stream channel counts to `MAX_INPUT_CHANNELS` in `build_input_stream` (both backends) and clamp track `input_port` accordingly; and defensively clamp in `read_monitor_input`: `let to_read = to_read.min(scratch.monitor_temp.len() / frame_stride * frame_stride);` (and bail out if `frame_stride > MAX_INPUT_CHANNELS`, skipping the backlog so the ring does not fill).
- **Verification:** `resonance-audio/tests/monitor_ring_alignment.rs`: drive `MixAudioHarness` with `shared.input_channels = 40` and a full ring; assert no panic and a whole-frame read. Unit-test the clamp in `monitor_read_len`'s neighbourhood.

### [x] MIX-10 — Zero-length notes are emitted Off-before-On in the same block (stuck note) — fixed @1f925f2c (with F2)
- **Severity:** low
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-audio/src/mixer/midi_events.rs:338-370` (sort key `(sample_offset, is_note_on)`); zero-duration sources: `resonance-audio/src/midi_io.rs:713-719` (SMF note-on/off on the same tick), `resonance-audio/src/engine/midi/live.rs:364-366` (open notes during MIDI recording/loop-record)
- **Problem:** For a note with `duration_ticks == 0` both events land at the same `sample_offset`; the sort puts the NoteOff first (intended for different-pitch retriggers), so the plugin receives Off then On and the voice never gets released.
- **Failure scenario:** Import an SMF drum/arp file containing zero-length notes onto a synth (or play back a loop-recorded MIDI pass that still holds an open note) → the note hangs until the next loop seam or Stop.
- **Suggested fix:** In `collect_midi_events`, skip notes whose `note_abs_end <= note_abs_start` (or force `note_abs_end = note_abs_start + 1` so the off lands one sample later); optionally sanitise to a 1-tick minimum at SMF import.
- **Verification:** `resonance-audio/tests/midi_event_window.rs`: a clip with a zero-duration note; assert either no events or On strictly before Off.

### [x] MIX-11 — Silent playhead advance at a loop seam snaps to `loop_in` and drops the overshoot — fixed @4338f909
- **Severity:** low
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-audio/src/mixer/common.rs:320-337` (`advance_playhead_silent`), used by `callback/play.rs:42` (lock-contended block) and `callback/reference.rs:57` (every block while A/B = reference)
- **Problem:** On a seam it sets `new_playhead = lo` instead of `lo + (playhead + frames - hi)`, unlike the rendering path (`seam.rs:136`, `loop_in + tail_frames`). Each wrap therefore loses up to one buffer of timeline.
- **Failure scenario:** Loop-to-mix reference A/B: each loop pass the reference and the (hidden) mix position fall behind wall-clock by the overshoot; after switching back the mix resumes up to one buffer early relative to where it would have been. Lock-contended blocks at a seam likewise shift the loop by up to a buffer.
- **Suggested fix:** `new_playhead = lo + (new_playhead - hi)` (clamped to stay `< hi` for pathological loops shorter than a buffer).
- **Verification:** Unit test via `MixAudioHarness` in `resonance-audio/tests/reference_monitor.rs`: reference active, loop with seam mid-buffer; assert the playhead after the wrap equals `loop_in + tail`.

---

## Audio engine / CLAP host / I/O

### [x] ENG-01 — CLAP note events are neither sorted nor clamped to `frames_count`; loop-seam sub-blocks drop live notes (stuck notes) — fixed @214f64f1
- **Severity:** high
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-audio/src/clap_host/process.rs:239-260` (note event build), `resonance-audio/src/clap_host/instance.rs:840-852` (`queue_note_on/off`), producers: `resonance-audio/src/mixer/live_midi.rs:77-92` + `resonance-audio/src/engine/midi/live.rs:29-42` (offset computed against the *whole* callback `frames`), `resonance-audio/src/mixer/render/track_pass.rs:258-265` (timeline notes appended after), `resonance-audio/src/mixer/callback/seam.rs:52-55` (head/tail sub-blocks)
- **Problem:** `process_multi_with_key` drains `pending_notes` into the CLAP input list in insertion order (the doc comment says "sorted by time" but nothing sorts) and never checks `time < frames_count`. CLAP requires input events sorted by `header.time` and inside the block. Two producers violate that:
  1. `pickup_live_midi` queues live notes at the start of the callback with offsets up to `frames-1` of the *full* callback buffer; when the buffer crosses a loop seam the mixer renders it as two sub-blocks, and the head sub-block's `process()` (with `frames_count = head_frames`) drains ALL pending notes, including ones with `time >= head_frames`.
  2. Timeline notes (`collect_midi_events`, sorted among themselves) are appended *after* live notes already queued, so a live note at offset 110 followed by a timeline note at offset 5 is delivered as `[110, 5]`.
- **Failure scenario:** Loop playback on an instrument track using the first-party wavetable synth while playing along on a MIDI keyboard. A NoteOff arrives in the callback that contains the loop wrap, landing at offset 100 with `head_frames = 20`. The head sub-block's process gets an event with `time=100 >= frames_count=20`; `resonance-wavetable`'s `render_block` (`plugins/resonance-wavetable/src/dsp/render/mod.rs:77-83`, `drain_events` breaks on `timing > sample_id`) never reaches it and drops it — the host already drained the queue — so the note sticks until panic/stop. Unsorted case: an out-of-range first event also blocks every later event in the same list from being drained.
- **Suggested fix:** In `process_multi_with_key`, after building `note_event_buf`: (a) clamp each `header.time` to `frames - 1` (or better, keep events with `time >= frames` in `pending_notes` re-based by `-frames` for the next call — keep a fixed-capacity carry so it stays allocation-free); (b) `sort_unstable_by_key(|e| (e.header.time, e.header.type_ == CLAP_EVENT_NOTE_ON))` (in-place, no allocation; note-offs before note-ons at equal time, matching `collect_midi_events`). Params stay at time 0 first. Fix the misleading doc comment on `MixedEventListCtx`.
- **Verification:** New test module in an existing `resonance-audio/tests/` binary (e.g. extend `tests/clap_all_notes_off.rs` style) using `__instance_from_raw_for_test` with a fake plugin whose `process` records `(time, type)` of each input event: queue `note_on(key, v, 100)`, then `note_on(key2, v, 5)`, call `process(.., frames=20)`; assert events arrive sorted and every `time < 20` (or the out-of-range one arrives in the next call).

### [x] ENG-02 — A failed `LoadPluginState` leaves the plugin permanently deactivated (silent) with no error; later loads never reactivate it — fixed @3001d761
- **Severity:** high
- **Confidence:** high
- **Category:** error-handling
- **Location:** `resonance-audio/src/clap_host/state.rs:171-176` (`reload_with_state`), `:196-242` (`cycle_activation`), caller `resonance-audio/src/engine/plugins.rs:668-683` (`handle_load_plugin_state` ignores the bool)
- **Problem:** `cycle_activation` stops + deactivates the plugin, sets `active = false`, then runs `load_state`; if that returns false it returns early *without reactivating*. `handle_load_plugin_state` discards the result and emits nothing. From then on `process_multi_with_key` returns immediately (`!self.active`) so the slot outputs whatever was in the buffer (effect: dry pass-through; instrument: silence). A subsequent good `LoadPluginState` hits `if !self.active { return self.load_state(data) }` and never reactivates either. Only removing/re-adding the plugin (or reopening the project) recovers.
- **Failure scenario:** User loads a preset saved by an older/newer plugin version, or a corrupt preset file; first-party `resonance-plugin` `state::load` returns false on a JSON parse error (`resonance-plugin/src/clap_bridge/state.rs:95`). The synth goes silent (or an EQ stops processing) with no banner, and loading a valid preset afterwards does not fix it.
- **Suggested fix:** In `cycle_activation`, always attempt reactivation after `while_deactivated` regardless of its result (the plugin's previous state is still valid when `load` fails), and return `load_ok && reactivated`. In `reload_with_state`, when `!self.active` (a previously failed instance) try the full activate → requery_latency → start sequence after loading. Have `handle_load_plugin_state` emit `AudioEvent::Error` (naming the instance) when the load or reactivation fails; mirror the wording used in `poll_plugin_host_requests`.
- **Verification:** Test via `__instance_from_raw_for_test` with a fake plugin whose `state.load` returns false and which counts activate/deactivate calls: after `reload_with_state(bad)` assert the instance is still active and `process()` reaches the plugin; after a subsequent good load assert it is active. Put it in an existing clap test binary (e.g. alongside `tests/clap_latency_tracking.rs`).

### [x] ENG-03 — A WAV write error mid-recording silently discards the whole take — fixed @d64cecc3
- **Severity:** high
- **Confidence:** high
- **Category:** error-handling
- **Location:** `resonance-audio/src/recording.rs:308-317` (drain sets `writer = None` on error), `:345-348` (`finalize_recording` → `finalize_wav_file` returns `Err("writer already closed")` → `continue`), `:683-686`, also `:475-478` (trailing cycle-record pass)
- **Problem:** When `write_sample` fails (disk full / quota exceeded — which has happened on this machine — or a >4 GiB RIFF limit on a very long take), the drain drops the writer (hound's `Drop` best-effort finalizes the header, so the frames already written are on disk and readable). But `finalize_wav_file` then treats "writer already None" as a corrupt file, and `finalize_recording` `continue`s: no clip is created, no `RecordingFinished`, and the only trace is `eprintln!`. The comment at `:313-315` claims "finalize will surface a short clip", which is not what happens. The WAV file is orphaned in `audio/`.
- **Failure scenario:** A 20-minute take hits "Disk quota exceeded" at minute 19. The user presses Stop; nothing appears on the timeline and no error is shown — 19 minutes of captured audio are unreachable from the project.
- **Suggested fix:** Track a `write_failed: Option<String>` on `TrackRecordingBuf` instead of relying on `writer == None`. On write error: take the writer and call `finalize()` explicitly (report its result), keep `frames_written` as the count of frames known good, and set the flag. In `finalize_wav_file`, treat an already-finalized writer as success when `frames_written > 0`, so the short clip is mapped and emitted. Emit a user-visible `AudioEvent::Error` (or a dedicated recording-error event) from the engine loop the first time a write fails, similar to `poll_overflow`. Same treatment in `roll_audio_pass`.
- **Verification:** Test in an existing recording test binary: build a `TrackRecordingBuf` whose writer targets a path on a tiny tmpfs or a writer wrapper that fails after N bytes (e.g. make `write_samples_and_peaks` testable over a failing `Write`); drain some frames, force failure, call `finalize_recording`; assert one clip with the pre-failure frames and an error event.

### [x] ENG-04 — `reset_plugins` never calls `clap_plugin.reset`, so offline renders start from live-playback plugin state — fixed @85c4dd0e
- **Severity:** medium
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-audio/src/clap_host/state.rs:246-256` (`reset_processing` = stop + start processing), `resonance-audio/src/engine/bounce/render.rs:252-260` (`reset_plugins`), used by export (`engine/bounce/wav.rs` both normalize passes), freeze (`engine/bounce/freeze.rs:155`), clip bounce, stems, measure
- **Problem:** CLAP's `clap_plugin.reset()` ("clears all buffers, perform a full reset of the processing state… and kills all voices") is never called anywhere in the host. `stop_processing`/`start_processing` carry no reset semantics; first-party plugins are built on clack, whose default `start_processing` is a no-op and whose `reset` maps to `Plugin::reset` (`resonance-plugin/src/clap_bridge/process.rs:582-584`) — i.e. the bridge only clears state on `reset`, which the host never sends. So the "clean deterministic state" the bounce code relies on does not exist.
- **Failure scenario:** (1) Play a song ending in a long reverb/delay, stop, export immediately: the first second of the file contains the reverb/delay tail from live playback. (2) Normalized export: pass 1 ends with the song's tail in every reverb/delay; pass 2 starts with that tail audible, so pass 2 ≠ the pass that was measured (the comment at `wav.rs` "reset_plugins before the pass makes the render deterministic" is false), and held synth voices can bleed in.
- **Suggested fix:** In `reset_processing`, call `(*self.plugin).reset` when present (the caller holds the instance mutex, so it is not concurrent with `process()` — same argument as `flush_pending_params`), and only fall back to stop/start when `reset` is absent. Also queue/flush `all_notes_off()` so voice-holding plugins without a good reset release. Update the doc comments that claim the stop/start cycle clears tails.
- **Verification:** Fake plugin test (`__instance_from_raw_for_test`) asserting `reset` is invoked by `reset_processing`; and an offline-render test with the first-party delay/reverb: render a burst, then run the export path and assert the first N frames of the export are silent when the timeline is silent there.

### [x] ENG-05 — Nothing in the engine stops live playback from running on the same plugin instances as an in-flight offline render — fixed @de0b2a42 (same as MIX-02)
- **Severity:** medium
- **Confidence:** high
- **Category:** concurrency
- **Location:** `resonance-audio/src/engine/transport.rs:16-33` (`handle_play` has no guard), `:35-` (`handle_record`), check-once guards at `engine/bounce/wav.rs:320-328`, `engine/bounce/freeze.rs:87-89`, `engine/bounce/clip.rs:~69`; app side `resonance-app/src/update/transport.rs:11-37` sends `AudioCommand::Play` unconditionally; also `mixer/callback/stopped.rs:13-44` (monitoring runs armed-track FX chains while stopped)
- **Problem:** Every offline render drives the live `ClapInstance`s (`bounce/mod.rs:61-67` says so). The only protection is "transport stopped at start" — checked once. `Play`/`Record` (from the UI or the MCP `transport_play` tool) during an export/freeze/stem render is accepted, after which the audio callback and the bounce thread interleave `process()` calls on the same instances at unrelated timeline positions, and `reset_plugins` for pass 2 can run mid-playback. Input monitoring while stopped does the same for armed tracks' FX chains.
- **Failure scenario:** Export with normalization (two full passes, minutes on a large project); the user or an agent hits Play to check something. The exported file contains garbled reverb/delay/compressor state from the live pass, and live playback glitches; nothing reports it.
- **Suggested fix:** Make the engine the authority: in `handle_play` / `handle_record` (and the monitoring enable path), refuse with an `AudioEvent::Error` when `shared.offline_render_count > 0`; or, alternatively, have the audio callback skip plugin processing (output silence) while `offline_render_count > 0`. Keep the app-side gate as UX, not as the guard.
- **Verification:** Engine test via `engine/thread/test_support.rs` harness: bump `offline_render_count` (hold an `OfflineRenderGuard::mark`), dispatch `AudioCommand::Play`, assert `shared.playing` stays false and an error event is emitted.

### [x] ENG-06 — Normalized export hard-clips at 0 dBFS *before* the normalization gain and true-peak limiter — fixed @13c63892
- **Severity:** medium
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-audio/src/engine/bounce/render.rs:418-445` (`clamp(-1.0, 1.0)` when `include_master_fx`), `resonance-audio/src/engine/bounce/wav.rs:420-470` (pass 1 measures, pass 2 applies `gain_db` + `TruePeakLimiter` to the already-clamped chunk)
- **Problem:** `render_chunk` applies master volume and a hard clip to every chunk. The normalize path then measures that clipped signal and trims/limits it. Any master overs are baked in as hard-clip distortion even when the normalization gain is negative and the limiter would have handled the peaks cleanly — which defeats the purpose of the true-peak limiter.
- **Failure scenario:** A mix peaking at +4 dBFS (master FX off), exported with normalize to −14 LUFS / −1 dBTP. The gain is about −8 dB, so the file has headroom, yet every peak above 0 dBFS is flat-topped. LUFS is measured on the distorted signal.
- **Suggested fix:** Add a parameter to `render_chunk` (or a `ChunkCtx` flag) that skips the clamp while keeping master volume/automation, and use it for both normalize passes. The limiter already enforces the ceiling; keep the clamp for the non-normalized path so it matches live playback.
- **Verification:** Test with `encode_buffer_for_test` / `normalize_buffer_for_test`, or an export test on a project with one clip at +6 dBFS: with normalization on, assert the output waveform is a scaled copy of the source (correlation ≈ 1, no flat tops) and the peak is ≤ ceiling.

### [x] ENG-07 — Master export stops exactly at the last clip end: reverb/delay/synth-release tails are cut, unlike stems (2 s tail) and bounce-in-place — fixed @f517ee53 (one 2 s FX-tail policy)
- **Severity:** medium
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-audio/src/engine/bounce/wav.rs:336-356, 400-401` (`render_stop = render_end + comp_latency`), compare `engine/bounce/stem_export.rs:42,100` (`FX_TAIL_SECONDS = 2`) and `engine/bounce_common.rs:9,25` (`BOUNCE_TAIL_SECONDS`)
- **Problem:** `run_export` renders `[earliest clip, latest clip/MIDI end + PDC latency)`. Nothing is rendered after the last note-off or clip end, so the final reverb/delay decay and instrument release are truncated by a hard cut. Stem export of the same project is 2 s longer than the master mix.
- **Failure scenario:** A song ending on a piano chord into a hall reverb: the exported WAV/FLAC ends abruptly with an audible click at the last MIDI clip end. Stems from the same project line up at the start but run 2 s longer than the master.
- **Suggested fix:** Add the same tail to `render_stop` (share one constant with stems/bounce; optionally make it an `ExportSettings` field, or render until the master output drops below a threshold for N ms with a hard cap). Keep the PDC trim unchanged. Check that `measure_mix` uses the same range so the reported loudness matches the file.
- **Verification:** Test in the bounce test binary (`tests/bounce_tail_and_master_latency.rs` exists): a MIDI clip ending at T through a delay/reverb plugin; assert the exported frame count ≥ T + tail and non-silent content after T.

### [x] ENG-08 — Freeze renders with an empty automation snapshot, so frozen audio diverges from live playback — fixed @3fff5f58
- **Severity:** medium
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-audio/src/engine/bounce/freeze.rs:164-168` (`AutomationSnapshot::default()`), fingerprint `compute_track_fingerprint` (same file) excludes automation
- **Problem:** The freeze cache is rendered without any plugin-parameter automation, although live playback, export and bounce all apply it. Once frozen, the track plays the cache instead of its plugins, so its automation stops having any effect. Automation is not in the freeze fingerprint, so editing a lane never marks the cache stale either.
- **Failure scenario:** A synth track with an automated filter-cutoff sweep. Freeze it: the frozen playback has no sweep. The export of the frozen project also has no sweep, while the unfrozen export does.
- **Suggested fix:** Pass the current `AutomationSnapshot` (`ctx.automation.load_full()`) through the `FreezeTrack` spawn path, the way `run_export` receives it, and include the source track's plugin automation lanes in the fingerprint the app compares. Decide whether gain/pan lanes belong in the cache: `freeze_raw` renders pre-fader, so only plugin-param lanes should be baked.
- **Verification:** Freeze test: a fake or first-party gain plugin with a param lane ramping 0→1; freeze; assert the cache amplitude ramps. Fingerprint test: changing a lane changes the fingerprint.

### [x] ENG-09 — The pool-import worker thread is not panic-supervised; a decoder panic leaves import rows stuck at "Working" — fixed @0e7a33b7
- **Severity:** medium
- **Confidence:** medium
- **Category:** error-handling
- **Location:** `resonance-audio/src/engine/import_pool.rs:201-210` (bare `std::thread::Builder::spawn` running `run_pool_import`)
- **Problem:** The recent supervision work (`supervise::run_supervised`, commit e4f72763) covers offline renders and `ImportQueue` jobs, but `handle_import_audio_to_pool` spawns its own thread outside both. A panic inside symphonia decode/probe of a malformed file, or in resampling/peaks, kills the thread after `ImportProgress::Working` was sent. No `ImportFailed` is emitted for that file, and the rest of the batch never runs.
- **Failure scenario:** Dragging 10 files into the pool, where file 3 is a truncated/crafted MP3 that trips a panic in the decoder. The modal shows file 3 "Working" forever and files 4–10 "Queued" forever; an agent's `pool_import` never resolves.
- **Suggested fix:** Wrap each per-file `import_one_to_pool` call in `run_pool_import` with `std::panic::catch_unwind` (or `run_supervised`) and turn a panic into `ImportFailed { reason: "decoder panicked: …" }`, so the batch continues. Optionally wrap the whole thread body too.
- **Verification:** `run_pool_import` is generic over `emit`, but `import_one_to_pool` is not injectable. Add a seam (a closure parameter, or a test hook) so a test can make one job panic, then assert the events are `Queued×N`, `ImportFailed` for the panicking job, and `Done` for the others.

### [x] ENG-10 — Plugins are activated with `min_frames_count = 32`, but loop-seam sub-blocks call `process()` with 1–31 frames — fixed
- **Severity:** low
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-audio/src/clap_host/bundle.rs:~315` and `clap_host/state.rs:~210` (`activate(plugin, sr, 32, 8192)`), `resonance-audio/src/mixer/callback/seam.rs:52-55` (head/tail split with arbitrary sizes), `mixer/track_block.rs:6-7`
- **Problem:** The offline path pads chunks to `MIN_CLAP_FRAMES` because of this contract (`bounce/render.rs:24-27`), but the live callback renders a buffer that crosses a loop boundary as head + tail sub-blocks of any length, e.g. `head_frames = 3`. CLAP requires `min_frames_count <= frames_count`. Plugins that size internal sub-blocking or FFT hops from `min_frames_count` may misbehave.
- **Failure scenario:** Loop playback where the loop end falls 3 frames into a callback: every plugin gets `process(frames_count=3)` after promising ≥ 32. Well-behaved plugins cope; strict ones (block-based convolution/look-ahead) can glitch or assert.
- **Suggested fix:** Activate with `min_frames_count = 1` in both `build_instance` and `cycle_activation` (that is what the host actually does), then drop the `MIN_CLAP_FRAMES` padding in `chunk_span`, or keep it as harmless. Change both call sites together through one shared constant.
- **Verification:** Fake-plugin test asserting the `activate` args; existing `chunk_span` tests updated if the padding is removed.

### [x] ENG-11 — `ensure_tuning_caches` holds the clips write lock across a full FFT retune of every tuned clip, on every offline render — fixed @a56c4e4c (tuning caches built off-lock)
- **Severity:** low
- **Confidence:** high
- **Category:** performance
- **Location:** `resonance-audio/src/engine/vocal_render.rs:71-99`, callers `engine/bounce/wav.rs:332`, `bounce/freeze.rs:93`, `bounce/clip.rs:127`, `bounce/stem.rs:520`
- **Problem:** `clips.write()` is taken first and held while `retune_clip` resynthesises every tuned clip (formant shifter, O(clip length)), even if the cache is already current. It re-runs unconditionally on every export/freeze/stem/bounce. While it runs, the engine thread blocks on any clip read or write, and the audio callback's `clips.try_read()` fails, so the block is silent — relevant because ENG-05 lets playback run during a render, and because monitoring/preview can be live.
- **Failure scenario:** A project with several minutes of tuned vocals: each export stalls the engine command loop for seconds, which delays every UI command and the recording drain.
- **Suggested fix:** Snapshot `(clip_id, Arc/clone of source, tuning)` under a read lock, compute the retunes with no lock held, then take the write lock briefly to install results whose tuning is unchanged. Skip clips whose cache is already valid, e.g. with a tuning-generation/hash field on `AudioClip`.
- **Verification:** Unit test: after `ensure_tuning_caches`, calling it again performs no rebuild (return count 0 when a validity key is added); a concurrency test asserting `clips.try_read()` succeeds while the retune runs (with a long synthetic clip).

### [x] ENG-12 — `reset_processing` ignores a failed `start_processing`, after which `process()` runs on a non-processing plugin — fixed
- **Severity:** low
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-audio/src/clap_host/state.rs:246-256`; related `engine/plugins.rs:219-230`
- **Problem:** `start(self.plugin)`'s bool is discarded; `active` stays true, so the next `process_multi_with_key` calls `process` on a plugin in the activated-but-not-processing state, which CLAP forbids. Separately, a plugin left deactivated by a failed `restart()` that keeps calling `request_restart()` gets the "failed to reactivate" `AudioEvent::Error` re-sent on every call, because `restart()` returns false immediately when `!active`.
- **Failure scenario:** A third-party plugin refuses `start_processing` right after `stop_processing` (resource contention). The export then calls `process` on it anyway, which is undefined per spec (crash or garbage).
- **Suggested fix:** Track a `processing: bool` next to `active`; skip `process` when it is false. In `reset_processing`, on start failure, deactivate and set `active = false` like `cycle_activation` does, and report it. In `poll_plugin_host_requests`, only report a restart failure once per instance.
- **Verification:** Fake plugin whose `start_processing` returns false on the 2nd call: after `reset_processing`, assert `process` is not invoked.

### [x] ENG-13 — A failed export leaves a truncated, invalid file at the target path and has already destroyed the previous file there — fixed @224fc361
- **Severity:** low
- **Confidence:** high
- **Category:** error-handling
- **Location:** `resonance-audio/src/engine/bounce/wav.rs:376-384` (`build_sink` creates/truncates the target path up front), `:440-447, 470-487, 508-527, 540-551` (WriteError / finalize-error paths `return` without cleanup; only `cancel_cleanup` at `:284-293` removes the file)
- **Problem:** The encoder writes straight to the user's chosen path. On a write or finalize error (disk full, removable drive pulled), the error is reported but the half-written file is left behind: a WAV with placeholder RIFF sizes or a FLAC without a valid final frame. Any previous good export at that path was already truncated when the sink was built.
- **Failure scenario:** Re-exporting `mix.wav` over a good previous version and running out of disk at 80%: the user now has neither the old file nor a playable new one.
- **Suggested fix:** Write to a sibling temp file (`<path>.partial`) and `rename` it into place only after `sink.finalize` succeeds. On any error or cancel, remove the temp file. Apply the same pattern to stems (`bounce/stem.rs:652`) and freeze caches.
- **Verification:** Export test with a sink that fails after N frames (inject via a test-only `EncoderSink`): assert the target path is untouched (pre-existing content preserved) and no `.partial` remains.

---

## Plugin framework / GUI runtimes

### [x] PLG-01 — First-party plugins never send `clap_host_gui.closed()`, so the host's #1347 handling never runs and the app keeps a closed editor marked "open" — fixed @0cfc9979 (host request_callback now runs on_main_thread)
- **Severity:** medium
- **Confidence:** high
- **Category:** correctness
- **Location:** `plugin-gui-core/src/app.rs:17` (default no-op `on_close`), `wayland-plugin-gui/src/window_thread/event_loop.rs:285-289`, `cocoa-plugin-gui/src/window_main_thread/view.rs:349-363,379-385,401-409`, `resonance-plugin/src/clap_bridge/gui.rs` (no `closed` call anywhere), `resonance-plugin/src/host.rs` (HostHandle exposes no gui channel); host side that waits for it: `resonance-audio/src/clap_host/mod.rs:171-177`, `resonance-audio/src/clap_host/gui.rs:49-55,153-179`, `resonance-audio/src/engine/plugins.rs:207-218`
- **Problem:** When the user closes the window with its own close button (Wayland CSD button or `xdg_toplevel.close`, Cocoa titlebar), each runtime calls `EditorApp::on_close()` and tears the window down. The only thing that could tell the host is `on_close`. No plugin implements it (grep: only the `hello` examples and one test do), and neither `resonance-plugin`'s bridge nor `HostHandle` gives a plugin any way to reach `clap_host_gui.closed`. `ClapMainThread.editor` stays `Some(RuntimeEditorHandle)` with a dead runtime behind it. The host-side `take_gui_closed` / `PluginEditorState{open:false}` path from #1347 cannot fire for any first-party plugin.
- **Failure scenario:** Open the EQ editor from the mixer, then close it with the window's X. `ClapInstance::gui_open` is still `true`, so the plugin panel still shows "Close Editor". Clicking it sends `ClosePluginEditor`, which runs `hide` and `destroy` on the dead handle. Only a second click reopens the window. Anything that sends `OpenPluginEditor` directly gets `Ok(())` from `open_gui`'s `if self.gui_open { return Ok(()) }` guard, and no window appears.
- **Suggested fix:** Wire the notification into the shared bridge, not into each of the 11 plugins. (1) Keep a `HostSharedHandle`-backed "gui closed" callback in `HostHandle`, retired the same way as the existing callbacks, and add `HostHandle::gui_closed(was_destroyed)` (or a bridge-private equivalent). (2) Have `RuntimeEditorHandle` / the runtime `Editor` take a close callback, e.g. an `EditorOptions::on_closed: Option<Arc<dyn Fn() + Send + Sync>>` that the runtime calls after `app.on_close()`, so every factory gets it through `editor_host`. (3) CLAP marks `closed` as `[main-thread]`. So either latch it and call it from `on_main_thread` after a `request_callback`, or document that the Resonance host tolerates any thread (it only stores atomics). Note that Resonance's `host_request_callback` is currently a no-op, which the latch route would have to fix too.
- **Verification:** Add a headless test in `resonance-plugin/tests/` (next to `clap_bridge_params_state.rs`) that drives `RuntimeEditorHandle`'s close callback through a fake `PluginEditor` and asserts the host's `closed` fires once. Then by hand: `WPG_TEST_CLOSE_AT=30 cargo test -p resonance-gate --test editor_open -- --ignored --nocapture`, extended to assert the closed callback fired. `CPG_TEST_CLOSE_AT` does the same with `editor_open_cocoa` on macOS.

### [x] PLG-02 — Wayland runtime never calls `eglTerminate` on its per-connection EGLDisplay: every editor open leaks a DRI screen and DRM fd, and a reopen can pick up stale driver state — fixed @fbce9c16
- **Severity:** medium
- **Confidence:** medium (the leak is certain; the stale-state reuse depends on the allocator returning the freed `wl_display` address)
- **Category:** resource-leak
- **Location:** `wayland-plugin-gui/src/egl_context.rs:384-398` (display created per `Connection`), `:540-553` (Drop skips `eglTerminate`); `wayland-plugin-gui/src/window_thread/event_loop.rs:64,314-317` (the `Connection` drops right after, and wayland-backend's `owns_display` Drop calls `wl_display_disconnect`)
- **Problem:** The Drop comment says the EGL display is process-wide state shared with other editors and the host. It isn't: every editor thread opens its own `Connection`, and `eglGetPlatformDisplay(EGL_PLATFORM_WAYLAND_KHR, display_ptr)` is keyed on that private `wl_display*`. So each editor gets its own EGLDisplay. It is `eglInitialize`d (Mesa opens a render-node fd, loads the DRI screen and creates a `wl_event_queue` plus registry/dmabuf proxies on that `wl_display`), and it is never terminated. The `wl_display` is then disconnected under it. Mesa keeps the `_EGLDisplay` in its global list, keyed by the now-dangling pointer and still marked Initialized.
- **Failure scenario:** (a) Open and close a plugin editor N times in one session. Each cycle leaks one DRM fd plus the driver screen allocations (several MB on Mesa), so `ls /proc/<pid>/fd | grep dri` grows by one per open. (b) If glibc hands the next editor's `wl_display` the same address as the freed one (same-size allocation, likely), `eglGetPlatformDisplay` returns the stale, already-initialized display and `eglInitialize` returns early. `eglCreateWindowSurface` then uses proxies and an event queue that belonged to the disconnected connection. The result is use-after-free or a Wayland protocol error (invalid object), which kills the new editor, or worse.
- **Suggested fix:** Call `self.egl.terminate(self.display)` in `EglContext::drop` after destroying the surface and context. It is safe because nothing else shares this display: it is keyed on the editor's private connection. Make sure it runs before the `Connection` drops; it does today, because `egl_ctx` is dropped explicitly before `state`/`conn` in `run_inner`. Correct the comment. As a belt-and-braces option, pass the display attribute list with `EGL_TRACK_REFERENCES_KHR` where available.
- **Verification:** By hand, from a Wayland session: extend `wayland-plugin-gui/tests/editor_size.rs` (ignored) with a loop that opens and destroys an `Editor` 20 times and asserts that the count of `/proc/self/fd` entries pointing at `/dev/dri/*` does not grow, and that the 20th open still paints (size settles). Run it with `cargo test -p wayland-plugin-gui --test editor_size -- --ignored --nocapture` and re-run `editor_open`.

### [x] PLG-03 — Cocoa: a panic in a plugin's `ui()` unwinds out of `drawRect:` and takes down the whole host (Wayland only loses the editor thread) — fixed @fd420c5f (Cocoa code NOT compiled — needs macOS check)
- **Severity:** medium
- **Confidence:** medium
- **Category:** unsafe/ffi
- **Location:** `cocoa-plugin-gui/src/window_main_thread/view.rs:100-103` (`drawRect:` → `paint`), `:554-557` (`app.ui(ui)` with no `catch_unwind`); likewise the input methods (`with_input`), `tick` → `fire_on_close` → `app.on_close()`, and `EditorMain::create`'s `.expect(...)` at `cocoa-plugin-gui/src/window_main_thread/mod.rs:325`
- **Problem:** On macOS the plugin's egui code runs inside Objective-C method implementations and GCD blocks on the host's main thread. A Rust panic there has to unwind through AppKit or libdispatch frames. That is either an abort (non-unwind ABI) or an uncaught foreign exception that terminates the process. Nothing catches it. On Wayland the same panic only ends the editor thread; the host survives and the handle degrades to `ChannelClosed`. The `in_paint` flag also stays `true` forever if unwinding is somehow caught higher up.
- **Failure scenario:** A plugin editor hits an `unwrap`, an index out of bounds or a `debug_assert_eq!` (e.g. `editor_widgets::float_knob`'s `KNOB_CELL` assert in a debug build) during a frame. The Resonance app, which is the process hosting the main thread, aborts and loses unsaved project work. On Linux the same bug only closes the editor.
- **Suggested fix:** Wrap every entry into plugin code in the view (`app.ui`, `app.on_close`) in `std::panic::catch_unwind(AssertUnwindSafe(...))`. On a panic, log it, mark the editor dead (`alive=false`), reset `in_paint`, stop the timer, and close the window through the same path as a user close, so the host is notified once PLG-01 lands. Replace the `expect` in `EditorMain::create` with an `EditorError`.
- **Verification:** Add a `harness = false` ignored test in `cocoa-plugin-gui/tests/`, modelled on `modal_reentrancy.rs`, whose `EditorApp::ui` panics on frame 3. Assert that the process keeps pumping and `Editor::set_size` returns `Err(ChannelClosed)`. Run it by hand from a logged-in macOS session with `-- --ignored`. For parity, add a Wayland counterpart to `editor_size.rs`.

### [x] PLG-04 — Wayland `hide()` doesn't hide: the window stays mapped with a frozen frame, keeps taking input, and replays the queued clicks on the next `show()` — fixed @f28918c6
- **Severity:** medium
- **Confidence:** high
- **Category:** correctness
- **Location:** `wayland-plugin-gui/src/window_thread/event_loop.rs:127-131` (Hide only sets `visible=false`), `wayland-plugin-gui/src/window_thread/delegates.rs:224-235,264-266` (input is always queued; the first configure forces `visible=true`), `wayland-plugin-gui/src/window_thread/paint.rs:170` (the queue is drained only by a paint)
- **Problem:** `Command::Hide` only stops painting; the comment even says "Unmap would need a null buffer commit". The toplevel stays mapped, showing its last frame, and still gets pointer and keyboard focus. `pointer_frame` and `press_key` keep pushing onto `pending_events`, which only `paint_frame` drains, and nothing paints while hidden. So the queue grows without bound, and on the next `Show` egui receives every stale motion and click at once. Separately, the first `configure` sets `visible = true`, so the window maps and paints before the host ever calls `show()`, which contradicts `Editor::new`'s "Create (but do not show)".
- **Failure scenario:** A host that uses CLAP hide/show to toggle a floating editor (the standard pattern in hosts that keep the gui created) hides the editor. The window stays on screen, frozen. The user clicks a "frozen" button (say a preset chip or the bypass) a few times, then the host shows it again, and all those clicks fire in one frame and change parameters. Resonance's own `close_gui` does hide then destroy right away, so it hits only the "maps before show()" half.
- **Suggested fix:** On Hide, attach a null buffer and commit (`wl_surface.attach(None,0,0); commit()`) to unmap. On Show, force a redraw, which re-maps with a fresh buffer. For the EGL path, destroy and recreate the `wl_egl_window`, or at least only swap while visible. Clear `pending_events` (and reset `InputState`'s pointer position) on Hide. Don't set `visible = true` in `configure`; paint the first frame only after `Command::Show`.
- **Verification:** By hand: extend the ignored `the_editor_opens_resizes_and_closes` in `plugins/resonance-gate/tests/editor_open.rs` to call `hide()` and check with `hyprctl clients -j` that the app-id window is gone, then `show()` and check that it is back. Also run `cargo test -p wayland-plugin-gui --test editor_size -- --ignored --nocapture`.

### [x] PLG-05 — macOS: quitting with an editor open deadlocks the engine thread's `Editor::destroy` against the main thread's `AudioEngine::shutdown` busy-wait — fixed (Cocoa, uncompiled)
- **Severity:** low
- **Confidence:** medium
- **Category:** concurrency
- **Location:** `cocoa-plugin-gui/src/editor.rs:124-130,251-258` (`destroy` → `DispatchQueue::main().exec_sync`), `cocoa-plugin-gui/src/lib.rs:37-41` (the stated invariant); `resonance-audio/src/engine/mod.rs:1068-1097` (main thread polls and sleeps up to the deadline), `resonance-audio/src/engine/thread/mod.rs:640-643` (engine thread drops every `ClapInstance` → `close_gui` → gui `destroy`), `resonance-app/src/update.rs:162`, `resonance-app/src/update/project_io/mod.rs:167`, `resonance-app/src/update/ui.rs:139`
- **Problem:** The Cocoa runtime is only safe if the main thread never blocks on the thread calling `Editor::destroy`. `AudioEngine::shutdown` breaks that: it runs on the iced main thread and waits up to 150 ms for the engine thread. On `ShutDown` the engine thread drains the plugin map, which closes every open editor through a synchronous main-queue dispatch. The main thread isn't servicing its queue during the wait, so the engine thread blocks until the deadline passes. Plugin teardown (stop_processing, deactivate, destroy) then runs late, only if the main thread gets back to its run loop, and concurrently with window close and process exit. That is the teardown-vs-exit race the comment at `thread/mod.rs:620-640` was written to avoid.
- **Failure scenario:** On macOS, open any plugin editor and quit the app. The quit always burns the full 150 ms deadline, and the engine thread ends up stuck in `exec_sync`. Either the process exits with the plugins never destroyed (skipped cleanup), or the queued destroy runs during AppKit/iced teardown and drops `ClapInstance`s concurrently with process exit.
- **Suggested fix:** Close all plugin GUIs on the main thread before calling `shutdown()`, e.g. with a new `AudioCommand::CloseAllEditors` that the app awaits by pumping, or by having the app drop the handles itself. Alternatively, make `Editor::destroy` on Cocoa asynchronous when the caller isn't the main thread: `exec_async` the teardown and return. The registry id makes that safe, since a late teardown finds nothing. Either way, restate the invariant in `cocoa-plugin-gui/src/lib.rs` as a checked rule.
- **Verification:** Add a harness=false ignored test in `cocoa-plugin-gui/tests/`: the main thread creates an editor, a worker thread calls `destroy()` while the main thread sleeps 200 ms without pumping; assert the destroy completes (async variant) or document the expected block. Run by hand on macOS with `-- --ignored`.

### [x] PLG-06 — A panic in the extra-state saver leaves `params_gen` odd forever, which permanently disables both the state-load re-sync and the editor push-back — fixed
- **Severity:** low
- **Confidence:** medium
- **Category:** error-handling
- **Location:** `resonance-plugin/src/clap_bridge/state.rs:402-435` (`begin_param_publish` … `saver.load(&state)` … `end_param_publish` with no guard), `resonance-plugin/src/clap_bridge/process.rs:201-202` (re-sync requires an even generation), `:339-346` (push-back stops while the generation is odd)
- **Problem:** The active-path `state::load` opens the seqlock window and then calls arbitrary plugin code (`ExtraStateSaver::load`: IR file loading, wavetable rebuild, preset JSON). If that panics, clack's `catch_unwind` turns it into a failed `load` and the plugin keeps running, but `end_param_publish` never ran. `params_gen` stays odd. From then on every `process()` skips the `params_dirty` re-sync and breaks out of the editor push-back on the first changed slot.
- **Failure scenario:** A project with a corrupt IR path hits a panic path inside a saver while the plugin is active. The host reports the load failure. After that, every knob the user turns in the editor moves the DSP but never reaches `shared`, so `params.get_value` and a later `state.save` persist the pre-panic values: silent loss of all later edits for that instance. A later successful `state::load` doesn't recover either, because the generation goes back to odd after that load closes its window.
- **Suggested fix:** Use an RAII guard: `let _publish = self.shared.publish_guard();`, whose `Drop` calls `end_param_publish`. Optionally also wrap `saver.load` in `catch_unwind` so the params half still gets announced.
- **Verification:** Add a headless test to `resonance-plugin/tests/state_race.rs` with a saver that panics on `load`. Call bridge `state.load` through clack's catch, then assert that `param_publish_gen()` is even and that a subsequent editor-side param write is mirrored into `shared` after one `process()` block.

### [x] PLG-07 — `HostHandle`'s liveness check is check-then-use, so the "safe to leak into an editor thread that outlives the instance" guarantee doesn't hold under concurrency — fixed
- **Severity:** low
- **Confidence:** medium
- **Category:** unsafe/ffi
- **Location:** `resonance-plugin/src/host.rs:118-137` (`if alive.load() { self.host.request_*() }`), `:30-43,160-163`; `resonance-plugin/src/clap_bridge/shared.rs:256-260` (`retire` in `ClapMainThread::drop`); `wayland-plugin-gui/src/editor.rs:141-155` (an editor thread detached after the 2 s join timeout keeps running)
- **Problem:** `retire()` stores `alive=false` from the main thread during instance destruction, after which the host frees the `clap_host` / `HostData`. A different thread that loaded `alive == true` just before can still be about to call through `self.host`. Nothing makes the check and the call atomic with respect to `retire` plus the host's free. The type docs present exactly this, a clone leaked into an editor thread that outlives the instance, as safe. The detached-editor-thread path (the join watchdog timing out) is the concrete way such a thread exists.
- **Failure scenario:** An editor whose `ui()` was blocked in a modal dialog is detached by `Editor::destroy` after 2 s. The host then destroys the instance. The dialog returns, and the editor code calls `host_handle.set_latency_samples(...)`, which calls `request_callback()`. That thread passes the `alive` check, gets preempted, the main thread retires and frees `HostData`, and the thread resumes and calls through a dangling function-table pointer. No first-party editor calls `HostHandle` today, so this can't happen yet, but the API advertises it as allowed.
- **Suggested fix:** Guard calls with a reader count: `alive` becomes an `AtomicUsize` "in-flight" counter plus a retired bit, and `retire()` spins until in-flight reaches 0 after setting the bit. Or take a `parking_lot::RwLock<bool>` read lock around each call, with `retire` taking the write lock. Neither is used from the audio thread in a way that would block it: `retire` only happens after deactivate. Otherwise, reword the docs to forbid use after the editor is destroyed.
- **Verification:** Add a headless test in `resonance-plugin/tests/` with a fake `clap_host` whose `request_callback` sleeps. Thread A calls `request_callback` in a loop, thread B calls `retire()` and then poisons the fake host. Assert that no call lands after `retire()` returns, and run it under Miri/TSan if available.

### [x] PLG-08 — `deactivate`/`activate` unconditionally copy `shared` into the plugin, reverting editor edits that the push-back hasn't mirrored yet — fixed
- **Severity:** low
- **Confidence:** medium
- **Category:** concurrency
- **Location:** `resonance-plugin/src/clap_bridge/process.rs:29-34` (activate: shared → plugin), `:573-577` (deactivate: shared → plugin, "unconditional"), `:324-372` (push-back only runs inside `process()`)
- **Problem:** Editors write straight into the plugin's param atomics through their `Arc<Params>`. Those writes reach `shared` only through the push-back at the top of the next `process()`. Two gaps: (1) an edit made after the last block's push-back and before `deactivate` is overwritten by deactivate's unconditional copy from `shared`; (2) edits made while the plugin is inactive (between deactivate and activate in `cycle_activation`, or after a failed reactivation) never reach `shared`, and `activate` then overwrites them with the stale `shared` values. The comment's premise that "the push-back keeps `shared` tracking the plugin every block" only holds at block boundaries.
- **Failure scenario:** The user drags the IR latency-mode picker or any knob during a latency-driven restart (`poll_plugin_host_requests` → `restart()`, running on the engine thread concurrently with the editor). The value written between the last `process()` and `deactivate`, or during the deactivated gap, snaps back when the plugin is reactivated, and the knob visibly jumps back to its old position.
- **Suggested fix:** In `deactivate`, run the same push-back (plugin → shared, guarded by `params_dirty` / `params_gen` exactly as in `process`) before the shared → plugin copy, and copy only when `params_dirty` is set. In `activate`, likewise only copy when `params_dirty` is set, or run a push-back first. That keeps the #1376 fix, which exists for a load that landed while active.
- **Verification:** Add a headless test in `resonance-plugin/tests/clap_bridge_params_state.rs`: activate, run one block, write a param directly on the plugin's `Arc<Params>` (the editor path), deactivate, and assert the value survives both deactivate and the next activate.

### [x] PLG-09 — More than 8 output ports is only caught by a `debug_assert`; in release every `process()` panics — fixed
- **Severity:** low
- **Confidence:** high
- **Category:** maintainability
- **Location:** `resonance-plugin/src/clap_bridge/process.rs:479-494`; validation gap in `resonance-plugin/src/clap_bridge/mod.rs:121-136` (`new_shared` checks channel counts but not port count)
- **Problem:** `MAX_OUTPUT_PORTS = 8` is enforced only by a `debug_assert!` and an array index panic in release. `new_shared` already rejects bad port shapes with a clean `PluginError`, but it doesn't check the count. The drums plugin already declares 7.
- **Failure scenario:** Add one per-pad output to drums (9 ports). Release builds panic on every `process()`, clack catches each one and returns a process error, and the track is silent with a message on stderr every block, instead of the plugin failing to load with a clear reason.
- **Suggested fix:** In `new_shared`, return `PluginError::Message("at most 8 output ports are supported")` when `output_ports.len() > MAX_OUTPUT_PORTS`, with the constant moved to `shared.rs`.
- **Verification:** Add a headless test in `resonance-plugin/tests/process_abi.rs`: a test plugin with 9 ports fails `new_shared` with that message.

### [x] PLG-10 — The Wayland ready handshake in `Editor::new` has no bound, unlike teardown — fixed
- **Severity:** low
- **Confidence:** medium
- **Category:** concurrency
- **Location:** `wayland-plugin-gui/src/editor.rs:61-73` (`ready_rx.recv()`, no timeout), `wayland-plugin-gui/src/window_thread/event_loop.rs:67,181-185` (the `registry_queue_init` roundtrip and the wait-for-configure loop, which only exits on configure or Quit, and Quit can't be sent before `new` returns)
- **Problem:** Teardown got a 2 s watchdog because the CLAP host calls gui functions on the audio-engine control thread with the instance lock held. Creation runs on the same thread and waits unboundedly for the editor thread's first `configure`. The pre-configure loop can't be interrupted: the handle, and so `Command::Quit`, doesn't exist until `new` returns.
- **Failure scenario:** A compositor that is slow, hung, or never configures a toplevel (some kiosk or nested compositors, or a compositor restart mid-open) leaves `gui.create` blocked forever. The engine thread wedges with the instance mutex held, and the whole `AudioCommand` queue stalls. This is the same failure mode the destroy watchdog was added for.
- **Suggested fix:** Use `ready_rx.recv_timeout(DESTROY_JOIN_TIMEOUT)` (or a dedicated `CREATE_TIMEOUT`). On timeout, send `Quit`: create the channel before spawning, which it already is, and the pre-configure loop already checks `state.running`. Then reap with `join_with_timeout` and return `EditorError::WaylandConnect("compositor did not configure the window")`.
- **Verification:** A headless test in `wayland-plugin-gui/tests/` can't fake a compositor easily. Instead, factor the handshake wait into a pure helper next to `join_with_timeout` and test it in `tests/join_watchdog.rs` with a stuck producer thread. Then run `editor_open` by hand to confirm the happy path.

---

## DSP core + amp / wavetable / mastering / drums / granular-delay

### [x] DSP-01 — Drums ignore note-event timing: every hit in a block starts at frame 0 — fixed @141ffbb2
- **Severity:** high
- **Confidence:** high
- **Category:** correctness
- **Location:** `plugins/resonance-drums/src/lib.rs:358-376` (drains all events before rendering, `timing` discarded via `..`), `plugins/resonance-drums/src/dsp/sampler.rs:403` (`voice.position = 0`), `sampler.rs:560-590` (render loop starts every voice at frame 0)
- **Problem:** `NoteEvent::NoteOn { timing, .. }` is ignored. All note-ons are applied before `render_block`, and a new voice is rendered from frame 0 of the block. Every hit therefore fires up to one block **early**, with random per-hit error. The comment says this is "audibly indistinguishable", but that only holds for very small blocks. The wavetable plugin already handles timing to the sample (`dsp/render/mod.rs:77-155`).
- **Failure scenario:** Offline bounce runs `BOUNCE_CHUNK = 1024` frames (`resonance-audio/src/engine/bounce/render.rs:22`). At 48 kHz, each drum hit in a mixdown moves early by 0-21.3 ms depending on where it falls in the 1024-frame grid. That ruins groove and makes flams against other tracks. Two hits on the same pad in one block (a 32nd-note roll at 180 BPM is 41 ms apart, and a flam is 10-20 ms) both start at frame 0 and sum to one hit about 6 dB louder instead of two. Live at quantum 128 the jitter is still 0-2.7 ms, and it does not match the bounce.
- **Suggested fix:** Pass `timing` into `note_on` and store a `start_offset: u32` on `Voice`. In `render_block`, start the frame loop for that voice at `start_offset` in its first block and then clear it. Alternatively, split `render_block` at event boundaries the way the wavetable `drain_events` loop does. Choke and choke_note should use the same offset.
- **Verification:** Add a module to `plugins/resonance-drums/tests/` (existing group file). Send a NoteOn with `timing = 300` into a 1024-frame block and assert the port's first non-zero sample is at index ≥ 300. Also send two same-pad hits at timings 0 and 512 and assert two onsets. Assert non-silence per scenario.

### [x] DSP-02 — Wavetable `find_free_voice` ignores `max_voices` and breaks legato glide — fixed @3faa770d
- **Severity:** high
- **Confidence:** high
- **Category:** correctness
- **Location:** `plugins/resonance-wavetable/src/dsp/engine.rs:269-314` (steps 3 and 4)
- **Problem:** Once `active_count >= max_voices` and no voice is releasing, step 3 (`filter(|v| v.note == note)`) and step 4 (`min_by_key(age)`) search **all** 32 slots, idle ones included. Idle slots keep age 0 (or a stale age) and their stale `note`, so step 4 almost always returns an idle slot. The new note then plays *in addition to* the held voices. The voice cap is never enforced while there are idle slots, and there always are. The drums plugin fixed exactly this bug and documents it in `plugins/resonance-drums/src/dsp/janitor.rs:52-56` ("an idle slot would otherwise win on age and quietly lift the polyphony limit").
- **Failure scenario:** Set `max_voices = 1` (mono) with Glide on and play legato: hold C3, then press G3. Step 1 is skipped because 1 ≥ 1, and step 2 finds nothing releasing. Step 4 returns slot 1, which is idle with age 0. C3 keeps sounding, G3 starts on a fresh idle voice, and because `was_idle` is true the glide never happens. Mono and legato portamento are both broken. Glide only works in the non-legato case, where step 2 picks up the releasing voice. `max_voices = 4` with 9 held notes plays all 9, so the CPU cap does nothing.
- **Suggested fix:** Add `.filter(|(_, v)| v.state != VoiceState::Idle)` to steps 3 and 4, mirroring the drums fix. Also prefer the same-note voice before the releasing one for mono and retrigger.
- **Verification:** New module in the wavetable test group. With max_voices=1, send held note-on 48 then 55 and assert that `engine.voices` has exactly 1 non-idle voice. With glide on, assert `current_pitch` passes through values between 48 and 55. Check the `render_block_regression` / `null_test` goldens for the stealing scenarios: they currently pin the buggy allocation and must be re-blessed deliberately.

### [x] DSP-03 — Wavetable mip selection rounds the octave down, so every note aliases by up to one octave — fixed @d18863c5 (−80 dB from D2 up; ~−69 dB floor below 70 Hz from table interpolation — see follow-ups)
- **Severity:** high
- **Confidence:** high
- **Category:** dsp
- **Location:** `plugins/resonance-wavetable/src/dsp/oscillator.rs:64-71`, `plugins/resonance-wavetable/src/dsp/wavetable_gen.rs:78-80,118-120`, `plugins/resonance-wavetable/build.rs:12` (tables built for 44.1 kHz)
- **Problem:** Mip level `k` holds harmonics up to `44100 / (2·f_k)`, with `f_k = 8.1758·2^k`, so it is band-limited for a fundamental of exactly `f_k`. `plan_tap` uses `oct_lo = floor(log2(f/8.1758))` and blends levels `oct_lo` and `oct_lo+1` with weight `1-frac` on the lower one. For any `f` in `(f_k, 2f_k)` the lower level contains partials up to `(f/f_k)·22050 Hz`, which is as high as 44.1 kHz. These fold back as inharmonic aliases. Cross-fading with the next level scales them down but does not remove them. There is no oversampling anywhere in the voice path, and at 48 kHz the tables are still limited for 44.1 kHz.
- **Failure scenario:** 44.1 kHz, basic saw, note G5 (784 Hz, octave_f = 6.58, oct_lo = 6, f_6 = 523 Hz). Level 6 carries harmonics 1-42, and harmonics 29-42 (22.7-33 kHz) fold to 11-21 kHz at −29 to −32 dB each, × 0.42 weight. The result is a cluster of inharmonic partials about −26 dB below the fundamental. It is plainly audible on pitch bends and glides as a moving "birdie" whistle. The worst case is just above each `f_k`, where the lower level has weight ≈ 1.
- **Suggested fix:** Select levels so that both blended levels are alias-free. Use `octave_f = log2(f/MIP_BASE_HZ)` and blend levels `ceil(octave_f)` and `ceil(octave_f)+1`. Alternatively, generate level `k` for the top of its octave (`max_harmonic = sr / (2·f_k·2)`) and keep the floor selection. Scale by the runtime sample rate: add `log2(44100/sample_rate)` to `octave_f` so 96 kHz does not over-darken and 48 kHz stays safe.
- **Verification:** Test module rendering one sustained saw at 784 Hz, 44.1 kHz. FFT the output and assert the energy in bins that are not multiples of 784 Hz is < −60 dB relative to the fundamental. Assert non-silence.

### [x] DSP-04 — `DcBlocker` pole is a fixed `R = 0.995`: bass loss grows with sample rate (amp, mastering saturator, granular feedback) — fixed @3e36274c (5 Hz amp/mastering, 20 Hz granular fb)
- **Severity:** medium
- **Confidence:** high
- **Category:** dsp
- **Location:** `resonance-dsp/src/dc_blocker.rs:13,22`. Users: `plugins/resonance-amp/src/dsp/processor.rs:158` (always on, whole NAM output), `plugins/resonance-mastering/src/stages/saturator.rs:316` (whole wet path at mix 1.0), `plugins/resonance-granular-delay/src/dsp/feedback.rs:42`
- **Problem:** The −3 dB corner is ≈ `(1−R)/(2π)·fs·0.998`. That gives 35 Hz at 44.1 k, 38 Hz at 48 k, 76 Hz at 96 k and 152 Hz at 192 k. It is not the "≈20/22 Hz" the doc comment claims. Measured `(1−z⁻¹)/(1−Rz⁻¹)`:
  - 48 k: 30 Hz −4.2 dB, 60 Hz −1.5 dB
  - 96 k: 60 Hz −4.2 dB, 100 Hz −2.0 dB
  - 192 k: 60 Hz −8.7 dB, 100 Hz −5.2 dB
- **Failure scenario:** At 48 k, the amp takes 1.5 dB off at 60 Hz and 4.2 dB off at 30 Hz: low B on a 7-string (31 Hz), bass or baritone through NAM. With the mastering saturator enabled at mix 1.0, the master loses 1.5 dB at 60 Hz at 48 k and 8.7 dB at 192 k. The tonal balance depends on the project sample rate. In the granular Wet→Buffer loop, repeats thin out much faster at high sample rates.
- **Suggested fix:** Give `DcBlocker` a `set_cutoff(hz, sr)` / `new(sr)` with `R = exp(−2π·fc/fs)`. Use fc ≈ 5 Hz for amp and mastering (the corner lands near 5 Hz, with < 0.1 dB loss at 30 Hz) and keep a higher value for the feedback loop if that is intended. Call it from each plugin's `initialize`.
- **Verification:** `resonance-dsp/tests/dc_blocker.rs`: at 44.1, 48, 96 and 192 kHz, a 40 Hz sine passes within 0.1 dB and DC decays. Add an amp/mastering test asserting the 40 Hz level through the saturator is independent of SR within 0.2 dB. Use a non-silent input and assert the output RMS is > 0.

### [x] DSP-05 — Mastering whole-plugin un-bypass replays ~0.3 s of stale pre-bypass audio — fixed @b0a8664f
- **Severity:** medium
- **Confidence:** high
- **Category:** correctness
- **Location:** `plugins/resonance-mastering/src/lib.rs:119-125`, `plugins/resonance-mastering/src/chain.rs:104-114` (bypass skips every stage)
- **Problem:** While bypassed, only `bypass_delay_*` is fed, and no stage's `process_stereo` runs. The two linear-phase EQs (each has 6144 samples of latency with FFT history, FIFOs and FDL), the multiband crossovers and delay, and the limiter's lookahead ring all freeze with whatever audio was in flight when bypass engaged. On un-bypass the chain first emits `latency()` samples of that frozen material: 2·6144 + 6144 + 240 ≈ 18.7k samples ≈ 0.39 s at 48 k. This comes from minutes-old audio followed by a discontinuity. The stage-level bypasses explicitly avoid this, but the plugin-level bypass does not.
- **Failure scenario:** Play a song and bypass the mastering plugin during the chorus. Un-bypass a minute later in a quiet verse: you hear about 0.4 s of chorus, then a hard splice to the verse.
- **Suggested fix:** Either keep running the chain while bypassed and discard its output (simplest and consistent with the "warm delay" intent), or call `chain.reset()` on the un-bypass edge and crossfade from the bypass delay line into the chain output over ~10 ms. A reset alone still steps from bypass-delayed audio to zeros-then-audio.
- **Verification:** Test module: feed a 1 kHz tone for 1 s, bypass, feed silence for 1 s, un-bypass, feed silence. Assert the output is silent (< −120 dBFS) after un-bypass. Currently the tone reappears. Also assert the tone is present before bypass (non-silence guard).

### [x] DSP-06 — Linear-phase EQ FIR is 4097 taps at any sample rate: low-frequency bands collapse at 96/192 kHz — fixed @38a84f7e
- **Severity:** medium
- **Confidence:** high
- **Category:** dsp
- **Location:** `plugins/resonance-mastering/src/stages/linear_phase_eq/convolver.rs:24` (fixed `FIR_LENGTH`), `.../design.rs:61-95` (frequency sampling + 4097-tap Hann truncation)
- **Problem:** The FIR length is fixed in samples, so its time span, and with it the frequency resolution after Hann windowing, shrinks with the sample rate: 85 ms at 48 k, 21 ms at 192 k. Reproducing the designer in numpy (same frequency sampling, circular shift, Hann, 4097 taps) for a corrective 2×HP at 30 Hz, Q 0.707:
  - 44.1 k: 10 Hz −25.6 dB (target −38.3)
  - 96 k: 10 Hz −15.4 dB, 60 Hz −1.1 dB (target −0.5)
  - 192 k: 10 Hz −7.6 dB, 20 Hz −6.8 dB (target −15.7), 60 Hz −2.5 dB, so the passband bass is cut

  An 80 Hz +6 dB low shelf reads 5.5/5.0 dB at 20/40 Hz at 192 k instead of 6.0/5.6.
- **Failure scenario:** A project at 96 or 192 kHz whose mastering corrective EQ has a 30 Hz rumble HPF passes most of the rumble and dulls 60 Hz by 1-2.5 dB. The assistant's genre-curve decisions, tuned at 48 k, land differently.
- **Suggested fix:** Scale `FIR_LENGTH`/`FFT_SIZE`/`HOP_SIZE` with the sample rate, rounded to the next power of two (e.g. 8192 taps at 96 k, 16384 at 192 k). The latency grows in samples but stays constant in ms, and `latency()` already reports it. Alternatively, use a minimum-phase or IIR low band.
- **Verification:** Test module designing the HP30 FIR at 48/96/192 kHz and evaluating its DTFT at 10/20/60 Hz. Assert it is within 2 dB of the biquad target at every rate.

### [x] DSP-07 — `SwapFader::begin_swap` restarts the fade-out at full gain: gain jumps, and the granular Fade mode never lands under automation — fixed @c430306a
- **Severity:** medium
- **Confidence:** high
- **Category:** dsp
- **Location:** `resonance-dsp/src/swap_fader.rs:152-164` (`fade_out_remaining = self.fade_samples` unconditionally), `plugins/resonance-granular-delay/src/dsp/time.rs:169-173`, `plugins/resonance-granular-delay/src/dsp/feedback.rs` (Per-Grain recirc tap), `plugins/resonance-amp/src/dsp/processor.rs:88`
- **Problem:** When a swap is requested while a fade is in progress, the fader starts a new full-length fade-out from gain ≈ 1.0:
  - During a fade-out at gain 0.4, the gain jumps back to ~1.0.
  - During a fade-in at gain 0.2, the gain jumps to ~1.0 and then fades out.

  Both are step discontinuities in the wet gain. In granular Fade time mode, any target change greater than `TIME_EPSILON_SECONDS` (0.1 ms) calls `begin_swap` every block.
- **Failure scenario:** Granular delay in Fade mode with Time automated (a sweep from 200 to 400 ms over 2 s at 48 k / 128-frame blocks). Every 2.7 ms block the fade-out restarts at full gain, so the wet gain saw-tooths between ~0.73 and 1.0 with a jump up each block. The tap never moves until the automation stops. The result is audible buzzing at ~375 Hz, the block rate. For the amp, clicking a model twice within the 21 ms fade window gives a click.
- **Suggested fix:** In `begin_swap`:
  - If already fading out, keep `fade_out_remaining` and just replace `pending`.
  - If fading in, start the fade-out from the current gain: `fade_out_remaining = (fade_samples − fade_in_remaining)`, then set `fade_in_remaining = 0`.
- **Verification:** `resonance-dsp/tests/swap_fader.rs`: call `begin_swap` every 64 `next()` calls for 4096 samples and assert `|gain[n] − gain[n−1]| ≤ fade_step + ε` for all n, and that the swap eventually lands. For granular, add a Fade-mode test with a per-block ramped target that asserts no wet-gain step > 1/fade_leg.

### [x] DSP-08 — Granular Wet→Buffer / Ping-pong feedback adds one host block per repeat (block-size dependent) — fixed @043c780b
- **Severity:** medium
- **Confidence:** high
- **Category:** dsp
- **Location:** `plugins/resonance-granular-delay/src/dsp/feedback.rs:200-206` (bus written from this block), `plugins/resonance-granular-delay/src/dsp/source.rs:130-132` (consumed at the same index `i` of the *next* block)
- **Problem:** Wet sample `i` of block k is written into the buffer at sample `i` of block k+1, so the loop delay is `delay + block_len`. The doc comment says the extra latency is "inaudible in the repeat spacing". It is not: it adds up on every repeat and depends on the host block size.
- **Failure scenario:** Tempo-synced 1/8 at 120 BPM (250 ms), feedback 80 %. Live at quantum 128, the n-th repeat is n·2.7 ms late, so the 10th repeat is 27 ms off the grid. The offline bounce uses 1024-frame chunks, so the same repeat is 213 ms late. Live and bounce sound different, and the sync is visibly wrong. With variable host blocks the spacing jitters.
- **Suggested fix:** Subtract the block latency on the read side: grains on this route read at `delay − block_len` (clamped ≥ the head margin). Alternatively, render the feedback at sample rate by writing `wet[i]` into the ring at `write_pos + i + 1` inside the grain loop. That needs per-sample interleaving, but it is the only block-size-independent option.
- **Verification:** Test module: an impulse into Wet→Buffer at 250 ms with feedback 0.9, rendered once at block 128 and once at block 1024. Assert the second repeat's onset is at 500 ms ± 1 ms in both runs and that the outputs match. Assert the repeats are non-silent.

### [x] DSP-09 — Granular "anti-alias" one-pole runs after the resampling read, so it cannot remove aliasing — fixed @648a81ff
- **Severity:** medium
- **Confidence:** high
- **Category:** dsp
- **Location:** `resonance-dsp/src/granular.rs:580-593` (filter applied to `s` after `read_*_wrapped`), `granular.rs:669-677`
- **Problem:** A grain with `|rate| > 1` decimates the source. Content above `fs/(2·rate)` folds below Nyquist **at the read**. A lowpass applied to the already-resampled output cannot tell folded components from real ones, and a one-pole is only 6 dB/oct in any case. The HQ tier ("forced anti-alias") is therefore only a dulling filter.
- **Failure scenario:** 48 kHz, +24 st (rate 4), 10 kHz sine in the source. The read produces 40 kHz, which aliases to 8 kHz. The post-filter at fc = 5.4 kHz attenuates 8 kHz by only ~4 dB, so the inharmonic 8 kHz alias stays about −4 dB below where the correct output (nothing) should be.
- **Suggested fix:** Band-limit before or while reading. Options:
  - Keep 1-2 pre-decimated half-band copies of the source ring (mip levels) and read from level `ceil(log2|rate|)`, so the per-grain read is always `|rate| ≤ 1` relative to its level.
  - Use a windowed-sinc read whose cutoff scales by `1/|rate|`.

  If neither is done, rename the option so it does not claim anti-aliasing.
- **Verification:** `resonance-dsp/tests/granular_pitch.rs`: 10 kHz sine source, rate 4, HQ. FFT the output and assert the energy at 8 kHz is < −60 dB re the input. Assert the output is non-silent at a legitimate 1 kHz→4 kHz case.

### [x] DSP-10 — Linear-phase EQ swaps its FIR with no crossfade, and the redesign runs on the audio thread — fixed (completed by FU-M2a @0319fb91 — design worker)
- **Severity:** medium
- **Confidence:** medium
- **Category:** dsp
- **Location:** `plugins/resonance-mastering/src/stages/linear_phase_eq/mod.rs:71-84`, `resonance-dsp/src/convolver.rs:176-205` (`set_impulse_response` keeps FDL/history and switches instantly)
- **Problem:** Any band change swaps the FIR hard at the next overlap-save hop. Output samples from that hop on use the new filter against the same input history, which is a step discontinuity of `(h_new − h_old) * x`. Every change also costs, inside `process()`:
  - 4097 bins × up to 4 bands × 3 `sin_cos` + one 8192-point IFFT, per EQ;
  - 2 forward FFTs of 8192 points (`set_impulse_response`, one per channel).
- **Failure scenario:** Automating a corrective bell from 0 to +6 dB on a sustained mix gives clicks at the hop boundaries, every 4096 samples (85 ms at 48 k). The redesign repeats every block while the automation moves, adding tens of µs per block per EQ.
- **Suggested fix:** Keep two convolvers (old and new IR) and crossfade their outputs over one hop when the IR changes. Alternatively, crossfade the IR spectra across a few hops. Rate-limit redesigns, for example at most one per hop.
- **Verification:** Test module: a sustained 100 Hz sine through the EQ with the bell gain stepped once. Assert the maximum sample-to-sample delta of the output after the switch is < 2× the steady-state delta. Assert the output is non-silent.

### [x] DSP-11 — Amp tuner cannot see low strings at 96/192 kHz (lag range clamped to FRAME_LEN/2) — fixed @93d1d97a
- **Severity:** low
- **Confidence:** high
- **Category:** dsp
- **Location:** `plugins/resonance-amp/src/tuner.rs:66-67` (and `FRAME_LEN = 2048`)
- **Problem:** `tau_max = min(sr/65, 1023)`, so the lowest detectable pitch is 46.9 Hz at 48 k, 93.8 Hz at 96 k and 187.7 Hz at 192 k. The 1024-sample window is also only 5.3 ms at 192 k.
- **Failure scenario:** At 96 kHz, low E (82.4 Hz, period 1165 samples) is outside the lag range. YIN falls back to a harmonic, or rejects the note because of the `PITCH_MIN..MAX` check, so the tuner shows the wrong octave or nothing. At 192 kHz the A and D strings behave the same way.
- **Suggested fix:** Decimate the tuner input to ~24-48 kHz before `feed` (a simple 2×/4× lowpass-decimate), or scale `FRAME_LEN` with the sample rate at construction. Also feed the mono sum the model hears (it currently takes `left` only, `lib.rs:233`).
- **Verification:** `plugins/resonance-amp/tests/`: a harmonic 82.4 Hz tone into `Tuner::new(96000.)`/`(192000.)`. Assert the result is 82.4 ± 0.5 Hz.

### [x] DSP-12 — Multiband crossover redesign allocates on the audio thread — fixed @06e2803f
- **Severity:** low
- **Confidence:** high
- **Category:** rt-safety
- **Location:** `plugins/resonance-mastering/src/stages/multiband/lowpass.rs:64-78` (`Vec::with_capacity(CASCADE_ORDER)` in `redesign`, reached from `set_cutoff` in `multiband/mod.rs:239-241` during `process`)
- **Problem:** Moving a crossover by more than 0.5 Hz calls `redesign()` on the audio thread, and `redesign()` heap-allocates a `Vec`. The rest of the chain is deliberately allocation-free (see the comment in `linear_phase_eq/mod.rs:72-77`).
- **Failure scenario:** Automating or dragging crossover 1 does a malloc per block per crossover. This trips any audio-path no-allocation guard, with the usual allocator-lock risk.
- **Suggested fix:** Build a `[BandConfig; CASCADE_ORDER]` on the stack.
- **Verification:** Extend the existing no-alloc guard test (if the mastering crate has one; otherwise add one using a counting global allocator in a test binary) to sweep `crossover_hz[0]` across blocks.

### [x] DSP-13 — Dither "noise shaping" filters the dither itself, not the requantization error — fixed @6c3e62aa
- **Severity:** low
- **Confidence:** high
- **Category:** dsp
- **Location:** `plugins/resonance-mastering/src/stages/dither.rs:73-85`
- **Problem:** `shaped = d − 0.5·shaped_prev` is an IIR on the TPDF noise: DC −3.5 dB, Nyquist +6 dB, total power +1.25 dB. Noise shaping only works as error feedback around a quantizer, and this stage never quantizes: the DAW quantizes later with white error. The option adds noise and does not lower the audible floor. The doc claims it pulls "low-frequency noise down ~6 dB".
- **Failure scenario:** 16-bit export with noise_shape on measures a higher total noise floor than with it off, and an unchanged in-band floor from the export's own truncation.
- **Suggested fix:** Either quantize here (`q = round((x + d − e_fb)·2^(b−1))/2^(b−1)`, `e = q − (x − e_fb)`, with the error feedback being the shaper), or drop or rename the option.
- **Verification:** Test module: dither + 16-bit quantize a −60 dBFS 1 kHz tone. Assert the in-band (< 4 kHz) noise is lower with shaping than without.

### [x] DSP-14 — WSOLA (Transient) stretch has +2.5 dB gain from wrong OLA normalization — fixed @9452427b
- **Severity:** low
- **Confidence:** high
- **Category:** dsp
- **Location:** `resonance-dsp/src/timestretch/ola.rs:57-58`, `resonance-dsp/src/timestretch/wsola.rs:66-69`
- **Problem:** `Ola` accumulates `frame·w` and normalizes by `Σw²`. That is correct for the phase vocoder, which windows at analysis *and* synthesis. WSOLA passes a raw frame, so the output is `x·Σw/Σw²` = 4/3 for Hann at 75 % overlap (computed: 1.3333). The existing `output_gain_is_sane` test allows 0.5-2×, so it hides this. There are currently no consumers outside `resonance-dsp`, which is why this is low.
- **Failure scenario:** Transient-mode stretch at ratio 1.0 is 2.5 dB louder than its input and than Tonal mode.
- **Suggested fix:** Pre-window the WSOLA frame (analysis window) before `add_frame`, or give `Ola` a normalization mode that uses `Σw`.
- **Verification:** `resonance-dsp/tests/timestretch.rs`: steady-state RMS ratio within ±0.2 dB for both algorithms at ratio 1.0/1.5 on noise.

### [x] DSP-15 — Granular `process_block` leaves frames beyond `max_block` unprocessed — fixed @fa48f00d
- **Severity:** low
- **Confidence:** high
- **Category:** error-handling
- **Location:** `plugins/resonance-granular-delay/src/dsp/mod.rs:319-322`
- **Problem:** `frames.min(self.grains.capacity())` truncates the block. The tail is left as dry input with no wet, the write head skips those samples, and the recirculation clock loses them. Mastering handles the same host-contract violation by chunking (`multiband/mod.rs:201-213`).
- **Failure scenario:** A host that delivers a block larger than it declared at activation, or re-activates with a smaller max without a reallocation, loses wet signal and time continuity in the tail of each oversized block.
- **Suggested fix:** Loop over `capacity()`-sized chunks, as the multiband does.
- **Verification:** Test module: initialize with max 256 and process 1024. Assert that the output equals four 256-frame calls.

### [x] DSP-16 — Mastering convolvers all run their FFT in the same callback every 4096 samples (CPU spike) — fixed @adc13c6f
- **Severity:** low
- **Confidence:** medium
- **Category:** performance
- **Location:** `resonance-dsp/src/convolver.rs:233-240`. There are 10 single-partition convolvers in `plugins/resonance-mastering/src/chain.rs`: 2 EQs × 2 channels and 3 crossovers × 2 channels.
- **Problem:** Every convolver has hop 4096 and starts in phase, so every 4096 samples one host callback runs 10 × (FFT 8192 + IFFT 8192 + 8192 complex MACs). The other 31 callbacks at quantum 128 do almost nothing. The worst-case callback cost, not the average, decides xruns at PipeWire quantum 128 (2.7 ms).
- **Failure scenario:** On the pinned 48 k / 128 quantum setup, heavy projects xrun periodically every 85 ms when the mastering plugin sits on the master. This shows as a periodic `cycle_load` spike.
- **Suggested fix:** Stagger the convolvers' initial phase (pre-fill different amounts of `output_pending`/`input_pending` with compensating delay), or use a non-uniform partitioning / time-distributed FFT.
- **Verification:** A bench in `plugins/resonance-mastering/benches`: record the per-128-frame `process` time and assert max/mean stays under a threshold.

---

## Dynamics/EQ/delay/reverb/IR plugins + metering / music-theory / svs / common

### [x] LIB-01 — Workspace-wide sample-rate conversion is plain linear interpolation, with no anti-alias or anti-image filter — fixed @bbc5d3f3 (Kaiser windowed-sinc polyphase, zero added delay, ≤−91 dB stopband)
- **Severity:** high
- **Confidence:** high
- **Category:** dsp
- **Location:** `resonance-common/src/wav.rs:263` (`linear_resample_mono`), `resonance-common/src/wav.rs:287` (`linear_resample_stereo`), `resonance-common/src/wav.rs:334` (`StreamingLinearResampler`). Callers: `resonance-audio/src/engine/clips.rs:192` and `engine/import_pool.rs:75` (clip import via `decode_file`), `engine/audition.rs:171`, `engine/reference.rs:565`, `resonance-audio/src/recording.rs:212` and `platform.rs:728` (recording or input at a device rate that differs from the engine rate), `plugins/resonance-ir/src/ir_loader.rs:19` (IR loading), `plugins/resonance-drums/src/kit.rs:122`.
- **Problem:** Every conversion between sample rates reads `s0 + (s1-s0)*frac` at `i*ratio`. Used as a resampler, linear interpolation has the response |H(f)| = sinc²(f/fs_src). Nothing band-limits the signal before decimation or suppresses images after interpolation.
  - Upsampling (44.1 k → 48 k) loses treble: −3.4 dB at 15 kHz and −6.3 dB at 20 kHz.
  - Downsampling (96 k → 48 k) folds everything above 24 kHz back into the audio band, attenuated only by sinc². For example, a 30 kHz component aliases to 18 kHz at about −3 dB.
- **Failure scenario:**
  - Import a 44.1 kHz mix into the 48 kHz project (PipeWire is pinned to 48 k). The clip plays back audibly duller, with a 3–6 dB loss above 15 kHz.
  - Import a 96 kHz recording that has ultrasonic content (cymbals, synths, or anything bounced from a plugin that aliases). Inharmonic aliasing tones appear between 14 and 22 kHz.
  - The same applies to a 96 kHz cabinet IR loaded into a 48 kHz session: the convolver then bakes the aliasing into every note. A LUFS or true-peak measurement of a resampled reference track (`reference.rs`) is also biased by the HF loss.
- **Suggested fix:**
  - Replace the one-shot resamplers with a band-limited resampler, e.g. the `rubato` crate (`SincFixedIn` / `FftFixedIn`): a windowed-sinc kernel with cutoff at 0.95·min(fs_in, fs_out)/2 and at least 64 taps per phase.
  - For `StreamingLinearResampler`, which runs on the recording drain thread, use `rubato`'s streaming API with a pre-allocated chunk size.
  - Keep the linear path only for pitch-shifting or varispeed uses where it is intended.
- **Verification:** Add tests in `resonance-common/tests/` (e.g. `tests/resample_quality.rs`):
  - Resample a 15 kHz sine from 44.1 k to 48 k and assert the output RMS is within 0.1 dB of the input.
  - Resample a 30 kHz sine from 96 k to 48 k and assert the output RMS is < −80 dBFS. Also assert a companion 1 kHz sine rendered in the same buffer is non-silent, so the check is not vacuous.

### [x] LIB-02 — `Mode::Chromatic` keys produce nonsense diatonic chords and cadence degrees (the 12-note table is indexed as a 7-note one) — fixed @8b595d06
- **Severity:** medium
- **Confidence:** high
- **Category:** correctness
- **Location:** `resonance-music-theory/src/progression.rs:69-95` (`diatonic_chord`), `resonance-music-theory/src/generator/degree.rs` (`Degree::to_chord`, `intervals[(root-1) % 7]`), `resonance-music-theory/src/derive/cadence.rs:95-99` (`scale_degree_pc`, `% intervals.len()` = 12). Reachable through `resonance-app/src/update/control/harmony.rs:347` (`harmony.apply_progression` with roman numerals) and `update/compose/chord_inspector.rs:444`. The transport key can be Chromatic (`update/control/transport.rs:253`).
- **Problem:** `Mode::Chromatic.intervals()` has 12 entries, but degree math assumes a heptatonic table:
  - `diatonic_chord` takes `ivs[d]` with `d = (degree-1) % 7` and stacks thirds via `(d+step) % 7`. For C chromatic it therefore builds degree I from offsets {0,2,4}, which gives (third, fifth) = (2,4). That hits the `_ => Min` fallback, so I becomes C minor. IV becomes D# (offset 3) minor and V becomes E (offset 4) minor.
  - `Degree::to_chord` puts V on E and IV on D#.
  - `scale_degree_pc(…, 5)` returns E as the "dominant" cadence target.
  - Only `resonance-app/src/update/chord_track.rs:40` special-cases Chromatic.
- **Failure scenario:** With the key set to C chromatic, `harmony_apply_progression(["I","IV","V","I"])` writes Cm – D#m – Em – Cm, instead of a C major progression or an error. The generators (`walk_progression` and the markov/degree generators) likewise fill sections with chords rooted on C, C#, D, D#, E, F, F#. Nothing is reported to the user or the MCP agent.
- **Suggested fix:** Define a single policy in `Scale` and route all three call sites through it. Either:
  - add `fn diatonic_intervals(self) -> &'static [u8; 7]` that maps `Chromatic` to the Major table (matching `chord_track.rs`); or
  - make `diatonic_chord` / `Degree::to_chord` return `Option`/`Result`, and have the control API reject roman numerals in a chromatic key with `invalid_params`.

  Also guard `scale_degree_pc` against `degree == 0` (`degree as usize - 1` underflows).
- **Verification:** Add `resonance-music-theory/tests/chromatic_degrees.rs`. For `Scale::new(C, Chromatic)`, assert that `diatonic_chord(s, 5, false)` is G major (or an error, per the chosen policy) and that `Degree::V.to_chord(s).root == G`. Add a control-API test in the `control` group binary for `harmony.apply_progression` in a chromatic key.

### [x] LIB-03 — The delay's "tempo-synced" wet gate free-runs and is never locked to the transport — fixed @86e683a2
- **Severity:** medium
- **Confidence:** high
- **Category:** dsp
- **Location:** `plugins/resonance-delay/src/gate.rs:393-409` (`GateDuck::next_gain` phase accumulator), `plugins/resonance-delay/src/gate.rs:373` (`clear` sets phase to 0), `plugins/resonance-delay/src/lib.rs:165-174`. `TempoInfo::song_pos_beats` is populated by the bridge (`resonance-plugin/src/clap_bridge/process.rs:384`) but ignored here.
- **Problem:**
  - The gate only uses the tempo to size its period (`gate_period_samples`). Phase is a private accumulator that starts at 0 when the plugin activates, when the gate toggles on, or on `reset()`, and then advances by `1/period` per sample.
  - Where the window opens therefore depends on when playback or activation started, not on the bar grid. It drifts after any tempo change (the period changes but the phase is not re-derived), after seeks, and after loops.
  - With no host tempo it falls back to a 1 s period. The same issue applies to the rhythm of every transport-stopped block.
- **Failure scenario:**
  - Gate at 1/8, width 50%. Start playback from bar 3 beat 2.5: the gate opens on the off-beats instead of the beats.
  - Loop a 4-bar region at 123 BPM. The phase carries across the loop jump, so the chop pattern lands on different subdivisions each pass.
  - The echo-tap viz shows a synced rhythm that the audio does not follow.
- **Suggested fix:** When `tempo` is `Some` and `playing`, derive the phase per block from `song_pos_beats / division_beats(division)` and advance per sample from there, so each block's first-sample phase is `fract(song_pos_beats / div_beats)`. Keep the free-running accumulator only when the transport is stopped or no tempo is reported. Snap to the host phase on discontinuities (seek or loop) rather than slewing.
- **Verification:** Add `plugins/resonance-delay/tests/gate_duck.rs` cases:
  - Render two blocks with `song_pos_beats` = 0.0 and then = 0.5 (a jump). Assert the gate gain at sample 0 of the second block equals `gate_gain(fract(0.5/div), …)`.
  - Feed a constant non-zero wet signal and assert the output is non-silent in the open window and attenuated in the closed one, so the test cannot pass on silence.

### [x] LIB-04 — IR swap crossfade also fades the dry path, so switching IRs punches a hole in a fully dry signal; `reset()` leaves stale audio in the bypass delay — fixed @6823239c
- **Severity:** low
- **Confidence:** high
- **Category:** dsp
- **Location:** `plugins/resonance-ir/src/dsp.rs:262-275` (`fade_gain` multiplies `delayed * dry_amount` as well as the wet), `resonance-dsp` `SwapFader::next`/`begin_swap` (fades out to 0 over 64 samples, then fades in over 64 samples, and fades in from 0 on the first install), `plugins/resonance-ir/src/dsp.rs:218-222` (`IrEngine::reset` resets only the active convolver).
- **Problem:**
  - The crossfade is meant to hide the convolver swap, but `fade_gain` scales the whole output, dry signal included. Every `file_select` change, whether from automation, the editor or the control API, dips the complete signal to 0 for about 128 samples.
  - On the first load while audio is running (no active convolver), the dry path suddenly drops to 0 and ramps back up.
  - `reset()` does not clear `bypass_delay_l/r`, so after a transport reset up to `block_size` samples (up to 2048 in Efficient mode) of pre-reset audio are replayed.
- **Failure scenario:** Mix at 20% wet, automate `file_select` on a downbeat. You hear a gap and click in the dry guitar. At dry_wet = 0 (A/B-ing IRs "off") there is still a 2.7 ms dropout on every switch.
- **Suggested fix:**
  - Apply `fade_gain` only to the wet term: `delayed*dry_amount + wet*dry_wet*fade_gain`.
  - Better still, crossfade old and new wet outputs by running both convolvers during the fade. `SwapFader` retains `pending` until the fade-out completes, so both are available.
  - Clear both bypass delay lines in `IrEngine::reset`.
- **Verification:** In `plugins/resonance-ir/tests/dsp_block.rs`, set dry_wet = 0, feed a non-silent sine, call `begin_swap` mid-block, and assert that no output sample in the fade window drops below 0.99× the latency-aligned input. Add a reset test asserting that the first `block_size` outputs after `reset()` are exactly 0 when the input is 0.

### [x] LIB-05 — EQ stages that leave and re-enter the active set resume with stale delay-line state — fixed @440ad818
- **Severity:** low
- **Confidence:** medium
- **Category:** dsp
- **Location:** `plugins/resonance-eq/src/dsp.rs:47-63` and `:76-85` (only `active_stages[b]` stages run), `plugins/resonance-eq/src/band.rs:224-310` (`configure_stages` rewrites only coefficients and deliberately keeps z1/z2).
- **Problem:**
  - Disabling a band sets `active_stages = 0`, which freezes the z1/z2 of all its biquads at the values from the moment it was disabled. Reducing a cut from 48 to 12 dB/oct likewise freezes stages 1–3.
  - When the band is re-enabled, or the slope increased, those stages resume from states computed for a different input, and possibly for different coefficients (another kind, frequency, or Q up to 2.56 for stage 3 of the 48 dB Butterworth).
  - TDF-II state is a scaled history, so a frozen z1 is injected directly into the output, e.g. y = b0·x + z1.
  - Changing the kind while enabled (e.g. Bell → HighCut) has the same effect on stage 0.
- **Failure scenario:** Band 1 is a 48 dB/oct low cut on a loud bass bus. Toggle the band off for a few bars, then back on during playback. A click or thump the size of the frozen state (tens of percent of full scale on loud material) is heard on re-enable, and is not bounded by the smoothing the rest of the plugin does.
- **Suggested fix:** In `update_from_params`, remember the previous `active_stages[i]`. For every stage index in `prev_n..new_n`, and for all stages when `kind` or `enabled` changed, call `stage.reset()` before processing. Optionally crossfade old and new band output over about 5 ms when the kind changes.
- **Verification:** In `plugins/resonance-eq/tests/plugin.rs`:
  - Process loud noise with a 48 dB low cut, disable the band for one block, then feed silence and re-enable.
  - Assert the first 64 output samples after re-enable are exactly 0.
  - Separately assert the enabled band's output is non-silent on non-silent input.

### [x] LIB-06 — Compressor release is applied twice (the peak detector and the GR envelope both use `release_coef`) — fixed @cea54427 (AUDIBLE: compressor release now matches knob — golden re-blessed; revert if unwanted)
- **Severity:** low
- **Confidence:** medium
- **Category:** dsp
- **Location:** `plugins/resonance-compressor/src/dsp.rs:258-266` (the peak envelope decays with `ballistics.release_coef`) and `:292` (the GR envelope then releases again with the same coefficient).
- **Problem:** Two one-pole release stages with the same τ are cascaded. The peak level falls at 8.69 dB/τ, so the target GR ramps down over (GR/slope)/8.69·τ, and the GR envelope then lags that ramp by another τ. As a result:
  - The actual release is program-dependent.
  - It is roughly 1.5–2.5× the knob value for typical 6–15 dB of gain reduction.
  - The GR history and the ms readout disagree with the user's setting.
- **Failure scenario:**
  - Release = 100 ms, threshold −20 dB, ratio 4, peak mode. A 0 dBFS burst drops to silence and GR starts at 15 dB. GR takes about 190–200 ms to fall to 37% (5.5 dB), where a single one-pole at τ = 100 ms would take 100 ms.
  - On a drum bus, this pumping is slower than dialed.
- **Suggested fix:** Give the peak detector its own short fixed release, as the gate does with `DETECTOR_RELEASE_MS` (5–10 ms, enough to bridge zero crossings), so the user's release acts once, on the GR envelope. Alternatively, use a branching or decoupled peak detector on the gain signal only (Giannoulis/Massberg/Reiss 2012, "smooth decoupled").
- **Verification:** In `plugins/resonance-compressor/tests/` (e.g. `release_time.rs`), step a 0 dBFS sine to −60 dBFS. Measure the time for `viz` GR (or the output-envelope ratio) to fall from its steady value to 1/e of it, and assert it is within ±15% of `release_ms`. Assert the pre-step output is non-silent.

### [x] LIB-07 — Gate: 0.05 ms attack floor silently becomes 0.1 ms; `GateSettings::ratio` doc contradicts the implementation — fixed @5d2cbe62
- **Severity:** low
- **Confidence:** high
- **Category:** correctness
- **Location:** `plugins/resonance-gate/src/dsp.rs:84-92` (`gate_ballistics`), `resonance-dsp/src/dynamics.rs:64` (`attack_ms.max(0.1)`), `plugins/resonance-gate/src/params.rs:86` (attack range min 0.05), `plugins/resonance-gate/src/dsp.rs:48-49` (doc).
- **Problem:**
  - The comment in `gate_ballistics` claims that crossing the coefficients (rather than the arguments) preserves the 0.05 ms bottom of the attack range. But `Ballistics::from_times` still floors the *attack* argument to 0.1 ms, so every setting between 0.05 and 0.1 ms yields the 0.1 ms coefficient. The lower half of the knob's bottom decade is dead.
  - `GateSettings::ratio` is documented as "`1.0` is a hard gate; higher ratios expand more gently". The code (`slope = ratio − 1`) makes 1.0 a pass-through and higher ratios steeper, which the params.rs comment and the tests describe correctly.
- **Failure scenario:** A user dials attack from 0.1 to 0.05 ms to catch a snare transient and hears no difference; the GR trace is identical. Anyone setting `ratio: 1.0` in code, trusting the struct doc, gets no gating at all.
- **Suggested fix:** Compute the gate's opening coefficient directly, `(-1/(attack_ms.max(0.01)*1e-3*sr).max(1.0)).exp()`, or add a floor parameter to `Ballistics::from_times`. Correct the `GateSettings::ratio` doc to "1.0 = no expansion (pass-through); higher = steeper; ~20 ≈ hard gate".
- **Verification:** In `plugins/resonance-gate/tests/ballistics.rs`, assert that the opening time at attack 0.05 ms is measurably shorter than at 0.1 ms (samples to reach −1 dB of unity after a step from closed), with a non-silent input tone.

### [x] LIB-08 — `atomic_write` uses a fixed `.tmp` name and leaks it on write/fsync failure — fixed @e544e8a2
- **Severity:** low
- **Confidence:** medium
- **Category:** error-handling
- **Location:** `resonance-common/src/atomic_file.rs` (`atomic_write`: `tmp_name = file_name + ".tmp"`; the `create`/`write_all`/`sync_all` error paths return without removing the tmp file; only the `rename` failure path cleans up).
- **Problem:**
  - Two writers of the same target, e.g. `settings.json` or `recent.json` persisted from two threads or two app instances, share one tmp path. `File::create` truncates the other writer's in-progress file, the writes interleave, and whichever `rename` lands last can publish a spliced or truncated JSON.
  - A failed `write_all` or `sync_all` leaves a partial `<name>.tmp` behind. This matters on this machine, where "Disk quota exceeded" has happened; see MEMORY.
- **Failure scenario:** The disk fills during project save, `write_all` fails with `EDQUOT`, and `project.json.tmp` (partial) stays in the project dir, alongside a stale tmp for every clip/midi file written in that pass. With concurrent writers, `settings.json` can end up corrupted and quarantined as `.corrupt` on the next launch.
- **Suggested fix:** Use a unique tmp name (`format!("{name}.{pid}.{nanos}.tmp")`, or `tempfile::NamedTempFile::new_in(parent)` + `persist`). Wrap create/write/fsync in a closure and `remove_file(tmp)` on any error.
- **Verification:** Extend `resonance-app/tests/io/project_atomic_write.rs`:
  - Simulate a write failure (target dir made read-only after create, or a tiny `RLIMIT_FSIZE` in a child process) and assert no `*.tmp` remains.
  - Race two threads writing distinct payloads 1,000× and assert the final file always parses as one of the two payloads.

### [x] LIB-09 — resonance-svs: `SampleCurve::resample` panics on a zero timestep; unknown phonemes silently become token 0 — fixed @767d5db0
- **Severity:** low
- **Confidence:** high
- **Category:** error-handling
- **Location:** `resonance-svs/src/ds.rs:102-128` (`src_pos = t / self.timestep`, `lo = src_pos.floor() as usize`, `self.samples[lo]`), `resonance-svs/src/ds.rs:160-171` (the `f0_timestep` value is not range-checked), `resonance-svs/src/pipeline.rs:328-352` (`phonemes_to_tokens`).
- **Problem:**
  - With `f0_timestep == 0.0` (or any other curve's `*_timestep`), `t/0` is `+inf` for i ≥ 1. `inf as usize` saturates to `usize::MAX`, and `self.samples[usize::MAX]` panics.
  - A negative timestep silently resolves every read to index 0.
  - Separately, a phoneme missing from the voicebank dict is replaced with token 0 (`<PAD>`/`AP`) after only an `eprintln!` (not `tracing`). The segment renders with silence or breath where a phoneme should be, and nothing surfaces to the app UI or the `vocal_render` job status.
- **Failure scenario:**
  - A hand-edited or externally produced `.ds` with `"f0_timestep": 0` makes the `resonance-svs` CLI, or any in-app caller that loads user `.ds` files, panic with an index-out-of-bounds error instead of returning an `anyhow` error.
  - A lyric whose g2p emits a phoneme the chosen voicebank lacks renders a gap. `vocal_render` reports success.
- **Suggested fix:**
  - In `compile_segment`, reject non-finite or ≤ 0 timesteps with `anyhow!("f0_timestep must be > 0")`, and do the same for every optional curve timestep.
  - In `resample`, early-return an empty curve when `!(self.timestep > 0.0)`.
  - Make unknown phonemes a hard error listing the missing symbols, or return them in `RenderedAudio` as warnings the app can show. Use `tracing::warn!` in place of `eprintln!`.
- **Verification:** Add `resonance-svs/tests/ds_validation.rs` (no ONNX needed):
  - A `.ds` JSON with `f0_timestep: 0` yields `Err` from `load_ds_file`, not a panic.
  - `SampleCurve { samples: vec![1.0, 2.0], timestep: 0.0 }.resample(0.01, 10)` does not panic.

---

## Architecture

The workspace is in unusually good structural shape for its size (~112k LOC app, ~40k audio, 11 plugins). The crate DAG documented in `ARCHITECTURE.md` matches the manifests exactly: `resonance-control` has zero internal deps and is the single wire contract for both `resonance-app` and `resonance-mcp`; `resonance-audio` never sees iced or app types; the control API is a genuine façade (all 150 mutating handlers in `resonance-app/src/update/control/` go through `run_via_update`, zero touch `engine` directly, so an AI edit is a normal undoable edit); offline bounce and live playback share one `mixer::render_block` behind a `RenderStrategy`; `editor_host.rs` collapses the Wayland/Cocoa runtimes to one import surface; the view layer never reads the engine (0 `.engine.` hits under `view/`); plugin registration is down to one place (`plugins/<name>/` + workspace member, cross-checked both ways by `bundle.sh`). The three structural risks that will hurt most as the project grows are: (1) the **per-feature state tax** — every persisted/undoable field must be threaded through `Resonance` (89 fields), `ProjectFile`, `serialize.rs`, two replay paths, `UndoExtras` (a shadow project file that already holds unpersisted state), `engine_events`, and the control `view_model`, which is why `message.rs` (1821 lines, 406 variants) was touched in 128 of the last 300 commits and is the workspace's merge-conflict magnet for parallel agents; (2) the **engine's shared-lock concurrency model** — the audio callback `try_read`s the same `Arc<RwLock<IndexMap>>` maps the control thread `write()`s at 61 sites, so any write held longer than one quantum is a silent dropout by design; (3) **test-binary sprawl outside the app** — `resonance-audio` alone links 121 top-level test binaries (30k LOC), and the "no inline tests" rule has pushed ~24 blocks of engine internals into the public API as `#[doc(hidden)]` re-exports.

### [ ] ARCH-01 — Per-feature state tax: one field, eight-plus files, two replay paths, and a shadow project file
- **Severity:** high
- **Category:** modularity
- **Location:** `resonance-app/src/lib.rs` (struct `Resonance`, 89 fields), `resonance-app/src/project/model.rs` (`ProjectFile`), `resonance-app/src/update/project_io/serialize.rs`, `update/project_io/replay/{mod,entity,restore}.rs`, `update/project_io/replay_diff.rs` (1119 lines, 45 commits/300), `resonance-app/src/undo/snapshot.rs:40-106` (`UndoExtras`), `resonance-app/src/engine_events/*`, `resonance-app/src/update/control/view_model/`, `resonance-app/src/message.rs` (128 commits/300), `resonance-app/src/engine_events/dispatch.rs` (72 commits/300).
- **Problem:** A single boolean (`master_fx_bypassed`) is referenced in 12 files across state, model, serialize, both replay paths, templates, engine_events, control, view and a fingerprint. Worse, `ProjectFile` is not the whole declarative state: `UndoExtras` (`undo/snapshot.rs:40`) carries `chord_track`, `clip_fade_gain`, `external_instruments`, `track_freeze`, `compose_arrangements`, `vocal_clip_lyrics` — and `automation_lanes` even though it is *also* in `ProjectFile` (`model.rs:162`). `chord_track` appears in `replay_diff.rs` but in neither `serialize.rs` nor `model.rs`, i.e. it is undoable but not saved. The two engine-sync paths (`replay_loaded_project` and `try_diff_replay`) each hand-enumerate every domain, so each new field is a new arm in both or it silently desyncs on one of undo/load.
- **Consequence:** Every feature pays a linear tax of edits in hub files, which is the direct cause of the churn numbers above and of the repeated "re-integrate on top of master" merges in the log (`f11f23b2`, `46c9ccbd`). Divergence between the fast and slow replay paths is a bug class that has already shipped (#1394/#1399 take-group resync saga). State that lives only in `UndoExtras` is silently lost on save.
- **Suggested change:**
  1. Fold `UndoExtras` into `ProjectFile`: add each extras field to the model with `#[serde(default)]`, persist `chord_track` and `clip_fade_gain`, delete the duplicate `automation_lanes`. Land per field; each is independently landable and a project-format additive change.
  2. Make `UndoSnapshot` = `ProjectFile` + midi notes + plugin blobs only; delete `finalize_undo_restore` extras plumbing once the struct is empty.
  3. Introduce a per-domain `Reconcile` trait (`fn diff(old: &Domain, new: &Domain, out: &mut Vec<AudioCommand>)`) implemented next to each domain's state module, and have both `replay_loaded_project` (old = empty) and `try_diff_replay` (old = current) drive it. Do it one domain at a time — transport, then tracks, busses, plugins, clips — with a golden test that `diff(empty, X)` emits the same commands the existing replay emits for `X`.
  4. Add an invariant test: `build_project_file(restore(snapshot_for_undo(app))) == build_project_file(app)` over the demo project and each template.
  Pitfall: `replay_diff.rs` currently has structural-compatibility gating (`structurally_compatible`); keep the gate but make it the *only* thing that differs between the two paths.
- **Verification / done-when:** `UndoExtras` no longer exists; `grep -l chord_track resonance-app/src/project/model.rs` non-empty; a new field added to a domain state type compiles only after its `Reconcile` impl is updated (exhaustive struct destructuring in `diff`); `message.rs` churn drops (track commits/300 after a quarter).

### [ ] ARCH-02 — Audio callback shares RwLock'd project maps with the control thread; contention is a designed dropout
- **Severity:** high
- **Category:** concurrency-model
- **Location:** `resonance-audio/src/engine/mod.rs:583-607` (`Arc<parking_lot::RwLock<IndexMap<TrackId, Track>>>` etc.), `resonance-audio/src/mixer/callback/play.rs:32-36` (five `try_read`s per block), `mixer/callback/mod.rs:5-7` ("a contended block drops out rather than waiting"), 61 `.write()` sites under `resonance-audio/src/engine/`, e.g. `engine/plugins.rs:324` (`ctx.plugins.write().insert(...)` after instantiate), `engine/bounce/render.rs:71` (`try_lock_with_backoff`, a spin/sleep workaround for the same contention from the bounce side).
- **Problem:** Tempo map, latency comp and automation already use the right pattern (`ArcSwap` of an immutable snapshot, `engine/mod.rs:600-623`), but the five hot maps (tracks, busses, clips, midi_clips, plugins) are mutable-in-place behind RwLocks. The callback's only defence is `try_read` → render silence and advance the playhead. Any control-thread write that holds a guard across non-trivial work (clip decode/insert, take restore, plugin chain edits, `ClearAll`+replay on undo) is a guaranteed glitch at quantum 128 (2.7 ms). Bounce workers hit the same locks from a third thread and had to grow their own backoff.
- **Consequence:** Undo/redo on a structural change (slow path: `ClearAll` → full replay) and project load cause audible dropouts that are architectural, not bugs; the memory-noted stutter investigation and `cycle_load` meter exist because of this. As projects grow (more clips, more plugins), write-hold times grow and the dropout frequency rises with them.
- **Suggested change:**
  1. Instrument first: count `try_read` failures per map in the callback into atomics on `SharedState`, surface via the existing `cycle_load` meter and an `AudioEvent::CallbackContended { map, blocks }` so the tick handler can log it. Cheap, standalone.
  2. Shrink write scopes: audit the 61 sites and make each build the new value off-lock and swap in under the guard (plugin instantiate already does this; `engine/clips.rs` and `takes.rs` are the ones to check). Each site is one small PR.
  3. Migrate the maps one at a time to `ArcSwap<Arc<Graph>>` copy-on-write: `clips` and `midi_clips` first (immutable per block, cheap to clone as `Arc<[..]>`), then `busses`, `tracks`; `plugins` stays a map of `Arc<Mutex<SyncClapInstance>>` with `try_lock` per instance as today. Add a `RenderGraph` struct that the callback loads once per block (it already snapshots tempo/transport in `callback/context.rs`).
  4. Delete `try_lock_with_backoff` once the bounce reads the same immutable graph.
  Pitfall: `HandlerCtx` (`engine/thread/mod.rs:44`) holds `&Arc<RwLock<...>>` for every handler; migrate by adding a `graph: &ArcSwap<RenderGraph>` alongside and removing the old fields last.
- **Verification / done-when:** `grep -rn '\.write()' resonance-audio/src/engine | grep -E 'tracks|busses|clips|midi_clips'` is empty; the callback does no `try_read` on project maps; a test using `EngineHandlerHarness` loads a 500-clip project while a synthetic callback runs and asserts zero contended blocks.

### [x] ARCH-03 — Test-binary sprawl outside the app, and engine internals leaking into the public API for tests — fixed @811b08a0 (130→12 binaries, rebuild 12.1s→3.7s, test-internals feature)
- **Severity:** high
- **Category:** build-time
- **Location:** `resonance-audio/tests/` (121 files = 121 binaries, 29,930 LOC, `0` `[[test]]` groups in `resonance-audio/Cargo.toml`), `resonance-music-theory/tests/` (41), `plugins/resonance-amp/tests/` (28), `plugins/resonance-granular-delay/tests/` (22), `plugins/resonance-mastering/tests/` (21); `resonance-audio/src/lib.rs:56-120` and `:200-400` (`__test_support` plus 24 `#[doc(hidden)] pub use engine::…` blocks); `scripts/run-tests.py:1-12` ("~620 of them").
- **Problem:** The app solved this (11 group binaries, ba doc #285) but the fix was not applied to the rest of the workspace. Each `resonance-audio` test binary re-links the whole crate plus cpal/pipewire/symphonia/clack. Separately, the "no inline tests" rule (ARCHITECTURE.md *Test Layout*) forces every private helper a test wants into `pub` re-exports: `SharedState`, `EngineHandlerHarness`, `push_take`, `chunk_span`, `to_freeze_cache_spawn`, `try_lock_with_backoff`, … are now public API of `resonance-audio`, and `resonance-app` already reaches `resonance_audio::__test_support::Receiver` from non-test code (`resonance-app/src/lib.rs:732`, `test_support/project.rs:45`).
- **Consequence:** Link time dominates the audio suite; every private refactor of the engine is a public-API change that can break app compilation; the hidden surface is an invitation for app code to depend on engine internals (it already does for one type).
- **Suggested change:**
  1. Group `resonance-audio/tests/` into ~8 binaries mirroring `src/`: `engine.rs`, `mixer.rs`, `bounce.rs`, `midi.rs`, `clap_host.rs`, `takes.rs`, `types.rs`, `io.rs`, each `mod`-including the existing files (exactly the app's `tests/<group>.rs` + `tests/<group>/` shape). Pure move; no test changes. Repeat for music-theory (3 groups) and the three big plugins (2 groups each).
  2. Replace the ad-hoc `__test_support` re-exports with one `pub mod test_support` gated on a `test-internals` cargo feature that `[dev-dependencies]` of the same crate and `resonance-app`'s dev-deps enable; production builds of the app then cannot see it (`resonance-app/src/lib.rs:732` moves under `#[cfg(feature = "test-internals")]` or into the app's `test_support`).
  3. Make `EngineHandlerHarness` the preferred surface for new engine tests so future helpers do not need re-exporting.
- **Verification / done-when:** `cargo test -p resonance-audio --no-run 2>&1 | grep -c Executable` ≤ 10; `run-tests.py` wall clock recorded before/after; `grep -rn '__test_support' resonance-app/src` returns only `#[cfg(feature)]`-gated lines.

### [ ] ARCH-04 — (partial: A4-1..3 @0c09001c — `state/ids.rs`, collision test found+fixed a real engine hint-bump track/group collision; A4-4 app-owned ids → epic) Entity ids are allocated in two places with hand-partitioned bases
- **Severity:** medium
- **Category:** api-design
- **Location:** engine allocators: `resonance-audio/src/engine/thread/mod.rs` + `engine/*.rs` (`next_clip_id`, `next_track_id`, `next_plugin_id`, `next_bus_id`, `next_send_id`, `next_group_id`, `next_take_group_id`, `next_asset_id`, …); app allocators: `resonance-app/src/state/plugin_index.rs:158` (`allocate_control_plugin_id`, base `CONTROL_PLUGIN_ID_BASE = 3_000_000_000` in `resonance-audio/src/types/mod.rs:34`), `resonance-app/src/state/aux_sends.rs:121` (`CONTROL_SEND_ID_BASE = 2_000_000_000`), `resonance-app/src/compose/state.rs:31` (`DERIVED_CLIP_ID_BASE = 1 << 40`), plus `next_return_bus_id`, `next_sub_track_id`, `next_lane_id`; the `AudioCommand::ReserveAssetIds` command (`types/commands.rs:95`) and the hint-vs-base rule in `engine/plugins.rs:244-265`.
- **Problem:** The control API needed synchronous ids, so the app grew its own allocators for plugins/sends/busses/derived clips in disjoint numeric ranges, while the engine still allocates the "GUI" range and must accept hints above the base. Each new entity type re-decides who owns its ids. History shows the cost: asset-id collisions in the field reports (memory), `43c5ffef Reserve engine clip ids past a loaded project's take clip_refs (#1393)`, `ReserveAssetIds` as a patch command.
- **Consequence:** Load/undo/replay must reconcile high-water marks on both sides for every id space; collisions are only caught when they happen; every new entity repeats the question.
- **Suggested change:**
  1. Decide one owner: the app. It already needs ids synchronously for control, and the engine never needs to invent one (it only ever echoes). Add `IdAllocator { next: HashMap<IdSpace, u64> }` to `Resonance`, persisted in `ProjectFile`.
  2. Migrate one id space per PR: plugins first (the control path already does this), then sends, busses, tracks, clips, assets, take groups. For each, the command carries the id, the engine's handler asserts `!map.contains_key(id)` and emits `AudioEvent::Error` if violated, and the engine's `next_*_id` counter is deleted.
  3. Delete `ReserveAssetIds` and the `*_ID_BASE` constants when the last space moves.
  Pitfall: demo seeding (`demo.rs`) and templates construct entities before the engine echoes; they need the allocator too.
- **Verification / done-when:** `grep -rn 'next_[a-z_]*_id' resonance-audio/src/engine` empty; `grep -rn '_ID_BASE' resonance-app resonance-audio` empty; a test loads a project, adds one of each entity via GUI and via control, saves, reloads, and asserts no id reuse.

### [ ] ARCH-05 — (partial: tracing facade, RT print fix, invariants @9dcd3bcd + full app sweep @359709b8 — no eprintln left in library crates; error taxonomy A5-3/4 open → epic) No error taxonomy or logging facade: `String` errors and `eprintln!` everywhere
- **Severity:** medium
- **Category:** consistency
- **Location:** `resonance-audio/src/types/events.rs` (`Error(String)`, `BounceError(String)`, `TrackBounceError(String)`, `StemExportError(String)`, …); `Result<_, String>` counts: audio 41, app 27, common 23, amp 65, plugin 8; `eprintln!` counts: app 49, audio 34; only `resonance-mcp` and `resonance-svs` use `tracing`, only `resonance-control` and `resonance-music-theory` use `thiserror`; `resonance-app/src/engine_events/dispatch.rs:56` logs an engine report with `eprintln!` because "no UI surface yet".
- **Problem:** Three idioms coexist by crate rather than by decision. Engine failures reach the app as free text, so the control layer's `ErrorKind` mapping (`resonance-control/src/rpc.rs`) has to string-match or default; there is no level/filter for diagnostics; RT-safety of logging is handled per site (`mixer/callback/mod.rs:127` hand-rolls a latch).
- **Consequence:** Control clients cannot distinguish "not found" from "engine busy" from "plugin crashed" without parsing prose; a future headless/CLI build has no way to silence or route logs; every new failure path re-invents formatting.
- **Suggested change:**
  1. Pick `tracing` (already in two binaries): add `tracing` to `resonance-audio`, `resonance-app`, `resonance-common`, `resonance-plugin`, install `tracing_subscriber::fmt().with_writer(stderr)` in `resonance-app/src/main.rs`, and mechanically replace `eprintln!` with `tracing::{warn,error,info}` per crate (one PR per crate; no behaviour change). Rule for the RT thread: no logging — increment an atomic and let the tick handler log.
  2. Introduce `EngineError { kind: EngineErrorKind, message: String }` in `resonance-audio/src/types/` with kinds mirroring `resonance_control::ErrorKind` (NotFound, Busy, Unsupported, Io, Plugin, Internal). Change `AudioEvent::Error(String)` first, then the per-operation error variants; app handlers that only display keep `.message`, the control reply layer maps `.kind`.
  3. Convert `Result<_, String>` in `resonance-audio` and `resonance-common` to the new type behind `thiserror`; leave plugin crates for last (their strings are mostly internal).
- **Verification / done-when:** `grep -rn 'eprintln!' resonance-app/src resonance-audio/src` empty; no `AudioEvent` variant carries a bare `String` error; `resonance-app/src/update/control/reply.rs` maps `EngineErrorKind` → `ErrorKind` exhaustively with no wildcard arm.

### [ ] ARCH-06 — (partial: A6-1 landed @3492f217, message.rs 1771→1051 lines; Resonance sub-states A6-2/3 + exhaustive undo_action A6-4 open) `Resonance` and `message.rs` are the hub files every change touches
- **Severity:** medium
- **Category:** modularity
- **Location:** `resonance-app/src/lib.rs` (989 lines, `Resonance` with 89 fields; 85 commits/300), `resonance-app/src/message.rs` (1821 lines, 31 sub-enums, 406 variants; 128 commits/300), `resonance-app/src/undo/classify.rs` (543 lines, 30 commits), `resonance-app/src/engine_events/dispatch.rs` (72 commits).
- **Problem:** The per-domain handler pattern is applied to *behaviour* (update/, engine_events/) but not to *declarations*: every sub-message enum lives in one file and every top-level field lives in one struct. With devs split across four app subcomponents in isolated worktrees (memory: app-view/app-state/app-io/app-vocal), those two files are where parallel branches collide. Loose fields like `master_volume/master_level_l/master_level_r/master_plugins/master_fx_bypassed`, `midi_*` (7 fields), `plugin_scan_*`, `error_message/engine_disconnected_banner_shown/stream_lost_banner_shown` have no owning sub-state struct.
- **Consequence:** Merge conflicts and "re-integrate on top of master" commits; classification in `classify.rs` is a manual whitelist with catch-alls (`Message::Group(_) => Skip` at `classify.rs:136`, `Message::Ui(_)`, `Message::Browser(_)`) so a new variant added to one of those enums is silently non-undoable and non-dirtying.
- **Suggested change:**
  1. Move each sub-message enum next to its handler (`update/track.rs` owns `TrackMessage`, etc.) and re-export from `message.rs`; `message.rs` keeps only `Message`. Pure move, one domain per PR.
  2. Group the loose fields into `state::MasterState`, `state::MidiDevices`, `state::PluginCatalog`, `state::Banners`; each is a mechanical rename PR.
  3. Move undo classification beside the enum: `impl TrackMessage { fn undo_action(&self) -> UndoAction }` with an exhaustive match (no `_`), so adding a variant fails to compile until classified. Delete the sub-enum catch-alls in `classify.rs` as each domain migrates.
- **Verification / done-when:** `message.rs` < 300 lines; `Resonance` ≤ 40 fields; `grep -nE '\(_\) => UndoAction::Skip' resonance-app/src/undo/classify.rs` empty.

### [ ] ARCH-07 — (partial: A7-1/A7-2 landed @cae04146 — 8 plugins dropped resonance-common, allow-list invariant; A7-3 feature-gating open) `resonance-common` is a domain-model crate wearing a primitives label, and every plugin links it
- **Severity:** medium
- **Category:** layering
- **Location:** `resonance-common/src/` (`take.rs` 733, `audio_probe.rs` 402, `midi_map.rs` 334, `device_definition.rs` 328, `automation.rs`, `freeze.rs`, `external_instrument.rs`, `track_group.rs`, `device_registry.rs`); `resonance-common/Cargo.toml` deps (symphonia, serde_json, dirs, time); every `plugins/*/Cargo.toml` depends on it but the plugins import only `flush_denormals`, `scan_directory`, `registry`, `drum_map`, `decode_wav_*`.
- **Problem:** ARCHITECTURE.md calls `resonance-common` "framework-agnostic building blocks", but most of it is app/engine domain state (take groups, comps, MIDI controller maps, external-instrument config, freeze cache refs). Eleven cdylibs pay to compile symphonia + serde_json + the whole model for two utility functions. The reverse risk is worse: domain types placed here are reachable from plugins, so a future plugin can grow a dependency on DAW model types.
- **Consequence:** Plugin build time and dependency surface; the "lowest layer it fits" rule from ARCHITECTURE.md has no lower layer to fit utilities into, so more model types will accrete here.
- **Suggested change:**
  1. Create `resonance-model` (serde types only: take, automation, midi_map, device_*, external_instrument, freeze, track_group, group_identity) and move those modules with `pub use` shims left in `resonance-common` for one release.
  2. Keep in `resonance-common`: `denormal`, `scan`, `registry`, `atomic_file`, `factory_presets`, `drum_map`, and `wav`/`audio_probe` behind a `decode` feature (only drums/ir/amp need symphonia).
  3. Flip `resonance-audio`/`resonance-app` to depend on `resonance-model`, delete the shims, and add the rule to ARCHITECTURE.md: plugins may depend on `resonance-common` and `resonance-dsp` only.
- **Verification / done-when:** `cargo tree -p resonance-eq --no-default-features -e normal | grep -c symphonia` is 0; `grep -l resonance-model plugins/*/Cargo.toml` empty.

### [x] ARCH-08 — Platform and host-GUI leakage in the plugin SDK: 11 manifests name Wayland, and an iced module lives in the plugin crate — fixed @7eae1a86/be8479d7 (manifests platform-neutral; iced UI moved to app)
- **Severity:** low
- **Category:** layering
- **Location:** `plugins/*/Cargo.toml` `[features] editor = ["dep:wayland-plugin-gui", "dep:plugin-gui-core", …]` (11 copies; only one manifest mentions cocoa), versus `resonance-plugin/Cargo.toml:35-39` which already selects the runtime per target; `resonance-plugin/src/ui.rs` (`pub use iced;`, `UiParam`, `view_generic_params`) used only by `resonance-app` (`resonance-app/Cargo.toml` enables `features = ["ui"]`); `resonance-audio/src/latency.rs:3-6` module doc still says the host "doesn't implement `clap_host_latency`" while `clap_host/mod.rs:46-64` does.
- **Problem:** `editor_host.rs` promises "the only places in the plugin stack that name a platform" but each plugin's manifest still does, so a new platform (win32) or a runtime rename touches 11 files; plugin source itself is clean (0 non-comment `wayland_plugin_gui` references). `ui.rs` is host-side code in the SDK; a third-party plugin author gets an optional iced dependency in their SDK for no reason. The stale PDC doc will mislead the next latency change.
- **Consequence:** Manifest drift across plugins (the cocoa dep is already inconsistent: 1 of 11), and the SDK's dependency graph is wider than its contract.
- **Suggested change:**
  1. Have `resonance-plugin`'s `editor-widgets` feature carry the platform runtime deps (it already declares them per target) and re-export `egui`; change each plugin's `editor` feature to `["resonance-plugin/editor-widgets", "dep:rfd", …]` and drop `dep:wayland-plugin-gui`/`dep:plugin-gui-core`/`dep:egui` from the 11 manifests (plugins import via `resonance_plugin::editor_host` already).
  2. Move `resonance-plugin/src/ui.rs` and the `ui` feature into `resonance-app/src/plugin_ui.rs` (only three symbols are used: `UiParam`, `PluginUiEvent`, `view_generic_params`).
  3. Fix the `latency.rs` header to describe the `clap_host_latency.changed` path.
- **Verification / done-when:** `grep -l 'wayland-plugin-gui' plugins/*/Cargo.toml` empty; `resonance-plugin/Cargo.toml` has no `iced` dependency; `grep -n "doesn't implement" resonance-audio/src/latency.rs` empty.

### [ ] ARCH-09 — (partial: A9-1/A9-2 @018839d9 — plugin blobs shared via Arc, cheap gesture check, snapshot 425→289 µs; A9-3 `Arc<Vec<MidiNote>>` + PartialEq on ProjectFile open) Undo snapshots deep-copy the whole project per edit
- **Severity:** low
- **Category:** modularity
- **Location:** `resonance-app/src/undo/snapshot.rs:178-230` (`snapshot_for_undo` calls `build_project_file` and clones every MIDI note vec, automation lane, chord track, and plugin state blob), `resonance-app/src/undo/history.rs:68` (capacity 200 via `resonance_audio::DEFAULT_HISTORY_CAPACITY`, oddly owned by the audio crate at `resonance-audio/src/limits.rs:60`).
- **Problem:** Snapshot cost is O(project) per undoable message; coalescing (`try_extend_coalesced`) avoids it for slider runs, but every discrete edit (note insert, clip move, control-API bulk write) pays a full serialize-shaped clone, and 200 of them are retained. With plugin state blobs in each snapshot, memory is 200 × Σ(blob sizes). The capacity constant living in `resonance-audio` is a small inverted dependency: undo is purely an app concern.
- **Consequence:** Fine today (single user, small projects); becomes visible with large MIDI arrangements and stateful plugins (NAM/IR blobs), and `notes.insert_many` from an agent snapshots per call.
- **Suggested change:**
  1. Move `DEFAULT_HISTORY_CAPACITY` to `resonance-app/src/undo/history.rs`.
  2. Make the heavy leaves persistent-shared: store `midi_notes` as `HashMap<ClipId, Arc<[MidiNote]>>` and plugin blobs as `Arc<[u8]>` in both live state and snapshots so unchanged clips/plugins are pointer copies. Independently landable per leaf.
  3. Only after ARCH-01 step 3: derive snapshots from the same `Reconcile` diff (store deltas, replay inverses) — not before, as two diff engines would be worse than one deep copy.
- **Verification / done-when:** a benchmark test snapshots a 2,000-note project 200 times and asserts memory growth is sublinear in note count; the capacity constant no longer appears in `resonance-audio`.

### [x] ARCH-10 — Rules that exist only in prose: encode the load-bearing invariants as tests — fixed @3519eb79 (tools/arch-invariants, 8 rules)
- **Severity:** low
- **Category:** testability
- **Location:** `ARCHITECTURE.md` (crate DAG, "no direct getters on `AudioEngine`", "view never mutates state", "no `resonance-app` dep from audio"); `resonance-mcp/tests/agent_plugin_lockstep.rs` (the one existing rule-as-test, and a good template); `resonance-app/src/update/control/view_model/` (no wildcard arms today — correct, but unguarded).
- **Problem:** The layering is currently right, but nothing fails when it stops being right: a `resonance-audio → resonance-app` path dep, a `pub fn` getter on `AudioEngine`, an `.engine.` read in `view/`, a `_ =>` arm in a wire-mapping `From` impl, or a plugin importing `wayland_plugin_gui` directly would all compile.
- **Consequence:** With a fleet of autonomous agents landing code, prose rules erode one PR at a time; the lockstep test is the only reason the MCP/skill contract has not.
- **Suggested change:** Add one `tests/architecture.rs` in `resonance-app` (or a tiny `xtask`) that reads `cargo metadata` and asserts the DAG from ARCHITECTURE.md (allowed edges list), then a handful of grep-style assertions over `src/` (no `.engine.` under `view/`, no `eprintln!` under `mixer/`, no `wayland_plugin_gui` in `plugins/*/src`). Keep it in the `control` or `io` group binary so it costs no new link.
- **Verification / done-when:** the test exists and each of the five rules above has a failing-case comment showing it was exercised once (flip an edge locally, watch it fail, revert).

---

