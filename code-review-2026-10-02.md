# Code review — findings (2026-10-02)

A whole-workspace, read-only review of `master` @ `b109a46c`, done by 8 parallel Opus agents. The previous review (`code-review-todo.md`) was at `09041eee`; about 1150 commits have landed since. Findings already listed in `code-review-todo.md` or `refactor-intent.md` are not repeated.

**121 findings: 11 high, 46 medium, 63 low, 1 info.** They were traced by reading code. A few were checked numerically: the RT-03 benchmark, and numpy checks for DSP2-01/02/07. Binary sizes for the DEP findings were measured with `nm`/`readelf`. None were confirmed by a failing test. **When fixing any finding, write the reproducing test described under *Verification* first, and watch it fail.**

| Area | Prefix | High | Medium | Low | Notes |
|---|---|---|---|---|---|
| Architecture & layering | ARCH2 | 0 | 7 | 5 | |
| Dependencies | DEP | 0 | 4 | 9 | +1 info |
| App UI/UX | UX | 3 | 10 | 10 | |
| Plugin editors & GUI runtime | PUX | 3 | 5 | 4 | |
| Engine realtime path | RT | 2 | 6 | 10 | |
| CLAP hosting & plugin framework | HOST | 1 | 5 | 10 | |
| Plugin & shared DSP | DSP2 | 0 | 6 | 10 | "medium-low" counted as low |
| App state, undo, persistence, control API | STATE2 | 2 | 3 | 5 | |

## High-severity findings at a glance

| ID | Summary |
|---|---|
| STATE2-01 | `presets.*` treats `plugin_id` as a raw path. A "read-only" `presets_search` can delete `NN-*.flac` files and quarantine JSON files in any directory. |
| STATE2-02 | A failed clip copy or transcode during save blocks every later save, open and new for the rest of the session (e.g. after the disk fills). |
| HOST-01 | Editor edits made while transport is stopped never reach the plugin's saved state. Save, then reopen, and they are lost (every first-party plugin except Drums). |
| PUX-01 | 12 of 13 editors never announce edits to the host: the mirror goes stale, there is no undo, MCP readback is wrong, and on reopen the stale host overrides can revert editor changes. |
| PUX-02 | Knobs on bool/int params can't be dragged. In the delay editor, Sync, Freeze and Gate can't be toggled at all. |
| PUX-03 | The delay editor's controls run off the window, so Gate and Duck are unreachable at the default size. |
| RT-01 | Recorded takes ignore PDC and master-chain latency, so every overdub lands late when a latent plugin is present. |
| RT-02 | Loop-record passes are cut on the engine tick, not at the loop seam, and later passes get no latency compensation. |
| UX-01 | Ctrl+O / Open Project discards unsaved edits without asking. |
| UX-02 | The chord track is never drawn, but palette commands edit it and pinned regions silently override generated chords. |
| UX-03 | Untitled projects (Ctrl+N, templates) record no undo, while the header says "saved". See also STATE2-04: the MCP promises undo too. |

## Cross-cutting themes

Several agents reached the same root cause from different sides. Fix these together:

1. **Plugin-side edits don't reach the host** (PUX-01 + HOST-01). The mirror misses edits (no `announce_param_change` outside Drums), and the plugin's own `shared` values only refresh in `process()`. The robust fix is HOST-01(a): read live values in `save_bytes`/`get_value`. Do it together with PUX-01's central announce in the shared widget bindings.
2. **Instance-mutex contention drops plugin blocks** (HOST-02 = RT-07, HOST-03). The engine thread try-locks every instance after every command and serialises project state under the lock. The audio thread then skips the plugin, so dry audio leaks through or the instrument goes silent. Gate the poll on lock-free flags, save outside the lock, and count misses.
3. **Latency is handled inconsistently** (RT-01, RT-02, RT-04, HOST-04, HOST-07). Recording ignores PDC, PDC lines are rebuilt from silence, and the bypass crossfade blends against undelayed dry. Mastering can't declare its own bypass.
4. **Untitled projects have no undo** (UX-03 + STATE2-04). There is one root cause, `can_record_undo` requiring `project_path`, and it affects both the GUI and the agent.
5. **Engine features with no caller** (ARCH2-01). Stem export (a dead button on Cmd+Shift+E), MIDI learn and clip warp exist in the engine only.
6. **Widget-kit drift in plugin editors** (PUX-02, -05, -06, -11). Two knob families, missing skew, missing typed entry and reset. Converging on one param-bound `ThemedKnob` fixes most of these at once.
7. **Discontinuities on switch, retrigger and toggle** (DSP2-03, -05, -08, -11, HOST-09). Mostly missing crossfades or smoothers. The existing crossfade machinery should be reused.
8. **Silent failure surfaces** (UX-04, UX-11, UX-13, PUX-09, ARCH2-06). Errors are overwritten, autosave failures are only logged, the CPU meter is a placeholder, and unknown control params are silently ignored.

## Fix campaign status (started 2026-10-02)

Each agent works in its own worktree and commits on its branch. The orchestrator merges every batch into master and updates this table. Agents do **not** edit this file.

Batches are grouped into waves by **file ownership**, so batches running at the same time never edit the same files. A batch can start once every batch it shares files with has merged. Some items were moved out of the batch the review suggested so that each file has a single owner:
- UX-05 moved to S2 (it owns `view/mod.rs`).
- UX-11, UX-15 and UX-16 moved to U1 (it owns `view/transport.rs`).
- The PUX-01 widget announce moved to P2. P1 keeps the host/bridge side, and the PUX-01 preset-bar recall moved to P3 (it owns `preset_ui.rs`).
- DEP-03 moved to P3 (rfd picker).
- HOST-08 and HOST-10 moved to R2 (they own `engine/plugins.rs` and `clap_host/state.rs`).
- HOST-07 and HOST-14 moved to R3 (they own `clap_host/mod.rs` bypass).
- STATE2-05, -06 and -08 moved to S1.
- The A1 refactors (ARCH2-02, ARCH2-05, the ARCH2-12 splits) run last and alone.

| Wave | Batch | Items | Model | Status | Merge |
|---|---|---|---|---|---|
| 1 | S1 control-surface safety | STATE2-01, STATE2-10 (mixdown ext), ARCH2-06, STATE2-05, STATE2-06, STATE2-08, STATE2-09 | opus | running | |
| 1 | S2 save robustness + error surface | STATE2-02, UX-13, UX-04, UX-05 | opus | running | |
| 1 | P1 plugin live values (host/bridge side) | HOST-01, PUX-01 (host side: refresh before serialise), STATE2-03, STATE2-07, STATE2-10 (read-only param undo) | opus | running | |
| 1 | R1 recording alignment | RT-01, RT-02, RT-08, RT-13, RT-17 | opus | running | |
| 1 | D1a DSP: mastering/compressor/shared | DSP2-04, -05, -06, -07, -08, -11 (mastering/eq/stereo), -15, -16 | opus | running | |
| 1 | D1b DSP: instruments/effects | DSP2-01, -02, -03, -09, -10, -12, -13, -14 | opus | running | |
| 2 | P2 editor widgets + announce | PUX-01 (widget announce), PUX-02, -03, -05, -06, -08, -11 | opus | after P1 | |
| 2 | R2 lock contention | HOST-02/RT-07, HOST-03, HOST-08, HOST-10 | opus | after P1 | |
| 2 | U1 project lifecycle UX | UX-01, UX-03/STATE2-04, UX-11, UX-12, UX-14, UX-15, UX-16 | opus | after S1, S2 | |
| 2 | U2 view correctness | UX-02, UX-06, UX-10, UX-22 | opus | after S2 | |
| 2 | H1 host spec | HOST-05, -06, -09, -11, -12, -13, -15, -16 | opus | after P1, R1 | |
| 3 | R3 latency changes | RT-04, HOST-04, HOST-07, HOST-14 | opus | after R2, H1 | |
| 3 | R4 RT correctness | RT-03, RT-05, RT-06, RT-09, RT-10, RT-11, RT-12, RT-14, RT-15, RT-16, RT-18 | opus | after R1, H1 | |
| 3 | P3 editor runtime | PUX-04, -07, -09, -10, -12, PUX-01 (preset-bar recall), DEP-03 | sonnet | after P2 | |
| 3 | X1 dependencies | DEP-01, -02, -04 (license decision → report only), -05..-14 except -03 | sonnet | after wave 2 | |
| 4 | U3 visual polish | UX-07, -08, -09, -17, -18, -19, -20, -21, -23 | sonnet | after U1, U2 | |
| 4 | A1a architecture (small) | ARCH2-01, -03, -04, -07, -08, -09, -10, -11, ARCH2-12 (rename only) | opus | after wave 3 | |
| 5 | A1b architecture refactors | ARCH2-02, ARCH2-05, ARCH2-12 (splits) | fable | last, alone | |

## Suggested fix batches (original grouping)

| Batch | Items | Model | Notes |
|---|---|---|---|
| S1 control-surface safety | STATE2-01, STATE2-10 (render path ext), ARCH2-06 | opus | Do first: a filesystem write path reachable by an agent |
| S2 save robustness | STATE2-02, UX-13, UX-04 (persistent engine status) | opus | |
| P1 plugin edit propagation | HOST-01, PUX-01, STATE2-03, STATE2-07 | opus | Data loss; one design for "where live values come from" |
| P2 editor widgets | PUX-02, PUX-05, PUX-06, PUX-11, PUX-03, PUX-08 | opus | Fix the knob families, then the delay/granular layouts plus layout tests |
| P3 editor runtime | PUX-04, PUX-07, PUX-10, PUX-12 | sonnet | |
| R1 recording alignment | RT-01, RT-02, RT-08 | opus | |
| R2 lock contention | HOST-02/RT-07, HOST-03 | opus | |
| R3 latency changes | RT-04, HOST-04, HOST-07 | opus | |
| R4 RT correctness | RT-03, RT-05, RT-06, RT-09..RT-18 | opus | |
| H1 host spec | HOST-05, HOST-06, HOST-08..HOST-16 | opus | HOST-05 can crash on third-party plugins |
| U1 project lifecycle UX | UX-01, UX-03/STATE2-04, UX-12, UX-14, UX-15 | opus | Shared "Save / Don't save / Cancel" modal |
| U2 view correctness | UX-02, UX-05, UX-06, UX-10, UX-11, UX-22 | opus | |
| U3 visual polish | UX-07, UX-08, UX-09, UX-16..UX-21, UX-23 | sonnet | Theme contrast change means a mass re-bless |
| D1 DSP | DSP2-01..DSP2-16 | opus | Several are sound changes (DSP2-10, -12): re-bless the goldens |
| A1 architecture | ARCH2-01..ARCH2-12, STATE2-05, STATE2-06 | fable/opus | ARCH2-02 and ARCH2-05 are refactors; plan them first |
| X1 dependencies | DEP-01..DEP-14 | sonnet | DEP-01/02 first (advisory, build-time download) |

---

## ARCH — Architecture & layering

Scope: crate DAG vs ARCHITECTURE.md and `tools/arch-invariants` (`cargo test -p arch-invariants` passes 17/17), module boundaries inside `resonance-app`, the control/MCP boundary, duplicated concepts, dead paths, doc drift and size hotspots. ARCH-04 / Epic D (`next_take_group_id`, `next_clip_id` in `engine/takes.rs`, `engine/clips.rs:60`) is still open, but there is no new evidence, so it is not repeated here.

### ARCH2-01 [medium] Export Stems, MIDI learn and clip warp are reachable from the UI but go nowhere
- **Where:**
  - **Export Stems:**
    - `resonance-app/src/commands/bindings.rs:149` binds Cmd+Shift+E to `ExportStemsMidi`, and `commands/resolve.rs:276` opens the Export modal.
    - `view/export_dialog.rs:144` wires the button to `ExportMessage::Confirm`, but `update/export.rs:61-67` handles it with `Task::none()` ("orchestration lands in #330/#331").
    - The engine side exists (`AudioCommand::ExportStems` `types/commands.rs:712`, `CancelStemExport` :813), but nothing in `resonance-app/src` sends them. `engine_events/dispatch.rs:93-101` discards all six `StemExport*` events.
  - **MIDI learn:** `EnterMidiLearn`/`CancelMidiLearn` (:1407/:1411) are never sent, although `dispatch.rs:375` handles `MidiLearnCaptured`. Only `test_support/mixer_plugins.rs:226` arms learn.
  - **Other commands never sent:** `ClearMidiBinding`, `ClearAllMidiBindings`, `SetControllerMap`, `SetControlSurfaceInput` (:1386-1401), `SetLoopRecordMode` (:370), `QueryIoLatency` (:1533).
  - **Clip warp:** `SetClipWarp`, `SetClipWarpMarkers`, `DetectClipTempo` (:135-156) have been in the engine since 2026-06-22. Their events are discarded (`dispatch.rs:206,211`).
  - None of this is in todo.md or code-review-todo.md.
- **Impact:** Cmd+Shift+E → pick stems → Export renders nothing and shows no error. About 13 engine commands and their handlers (and the warp render paths) are maintained and tested for no reachable caller.
- **Fix:** Wire `Confirm` to `ExportStems` and mirror the `StemExport*` events (#330/#331), or hide `ExportStemsMidi` until that is done. For each orphaned engine feature, file a todo or delete it.
- **Verification:**
  - App test (io group): `ExportMessage::Confirm` with `new_for_test_with_capture()` captures `ExportStems`.
  - Optional arch-invariant: every `AudioCommand` variant is named in `resonance-app/src` outside `test_support`, with an explicit allow-list.

### ARCH2-02 [medium] The track/bus/master plugin chain is written out three times at every layer, with four app-side owner enums
- **Where:**
  - **Engine commands (`types/commands.rs`):**
    - `AddPlugin`/`RemovePlugin`/`MovePlugin` (:283,294,318)
    - `AddPluginToBus`/`RemovePluginFromBus`/`MovePluginInBus` (:1180,1191,1212)
    - `AddPluginToMaster`/`RemovePluginFromMaster`/`MovePluginInMaster` (:1269,1279,1302)
    - `SetTrack/Bus/MasterFxBypass` (:1312-1320)
  - **Events:** `PluginAdded/Removed/Moved`, `BusPlugin*`, `MasterPlugin*` (`types/events.rs:428-1022`).
  - **App handlers:** `engine_events/plugins.rs` has three each of `*_added`, `*_moved`, `mirror_*_plugin_move` and `*_removed(_echo)`.
  - **Wire:** 13 methods × 3 surfaces = 39 param structs (`resonance-control/src/methods/{track,bus,master}.rs`) and 39 MCP tools.
  - **App enums for the same three-way choice:** `state::PluginLocator` (`state/tracks.rs:480`), `update::control::plugin_target::ChainOwner` (:21), `view::mixer::picks::PluginOwner` (:13), `message::PresetAddOwner` (`message.rs:172`). The add/move/remove command fan-out is in `update/plugin_replace.rs`.
- **What:** The app has already unified the concept internally, but the engine API forces nine command/event pairs, so every chain behaviour is written and fixed three times.
- **Impact:** This has already happened: FU-A13c/h fixed immediate-mirror-on-remove separately per surface. The next chain feature will get fixed on two surfaces and missed on the third.
- **Fix:** A single `ChainOwner { Track(TrackId), Bus(BusId), Master }` in `resonance_audio::types`; `AddPlugin { owner, … }`, `PluginAdded { owner, … }`; the four app enums become that one type. The wire triplication is a documented choice and can stay, but its handlers should funnel into `ChainOwner`.
- **Verification:** A grep invariant that no `AudioCommand`/`AudioEvent` name contains `ToBus|InBus|FromBus|ToMaster|InMaster|FromMaster`. `tests/io/undo_snapshot_fixed_point.rs` must stay green across the change.

### ARCH2-03 [medium] The app hard-codes first-party plugin ids and param keys with no lockstep test
- **Where:**
  - `resonance-app/src/drums_mirror.rs:16-26` (`"com.resonance.drums"`, `"kit_select"`, `"kit_load_progress"`, `"output_mode"`, `OUTPUT_MODE_MULTI = 1.0`)
  - `update/control/track/params.rs:164-165` (`"com.resonance.amp"`, `"file_select"`)
  - `update/control/sidechain.rs:80` (`SECONDARY_KEY_PLUGINS`)
  - `update/project_io/templates.rs:529-535` (seven CLAP ids)
  - `resonance-control/src/methods/master.rs:476` (`MASTERING_PLUGIN_ID`)
  - The plugin-side definitions are in `plugins/resonance-drums/src/lib.rs:573,679-682` and `plugins/resonance-amp/src/params.rs:170`.
- **What:** "The app depends on no plugin crate" is enforced, but the app depends on plugin internals by string literal. Only mastering has a lockstep test (`assistant_lockstep.rs`).
- **Failure scenario:**
  - Rename `output_mode` (renames are explicitly supported via `ParamRename`): `routes_to_ports` falls through to `None => true`, every drums track spawns multi-out sub-tracks in Stereo mode, and the kit picker shows no kit. All tests still pass.
  - Rename a CLAP id: every template track becomes a missing-plugin slot.
- **Fix:** A `first_party` constants module (ids plus the param keys the host relies on) in `resonance-common` or `resonance-plugin`. Plugins define their params from it, and the app imports it.
- **Verification:**
  - A per-plugin lockstep test: every host-named key resolves to a real param.
  - Each template CLAP id equals some plugin's `CLAP_ID`.

### ARCH2-04 [medium] The NAM and drum-kit libraries are near-verbatim copies of one content-index implementation
- **Where:** `resonance-common/src/nam_library.rs` (1091 lines) and `drumkit_library.rs` (1853 lines).
  - Mirrored types: `Source`, `EntryStatus`, `Entry`, `LibraryError`, `ScanReport`, `ImportOutcome`.
  - Mirrored `Library` methods: `open`, `open_and_scan`, `watch_paths`, `generation`, `entries`, `by_slot`, `slot_of`, `find`, `reload_if_changed`, `rescan`, `import`, `delete`.
  - Character-identical code:

    | Function | nam_library.rs | drumkit_library.rs |
    |---|---|---|
    | `find()` | :582-617 | :728-758 |
    | `reload_if_changed` | :626 | :767 |
    | slot allocation | :956-959 | :1665-1668 |
    | `stat_stamp` | :383 | :536 |
    | `read_index` | :982 | :1755 |

  - Already diverged: drums has a `locked()` helper (:784), while nam opens the lock inline (:660-676).
  - Separately, `BlockPeaks {in_l,in_r,out_l,out_r}` is defined three times (amp `dsp/processor.rs:20`, ir `dsp.rs:156`, delay `dsp.rs:23`).
- **Impact:** A locking or quarantine fix (the STATE-15/LIB-08 class) lands in one library only. A third library (IR) will copy again. Lookup semantics that agents rely on can silently differ between `amp_models.*` and `drum_kits.*`.
- **Fix:** Extract `content_index::Index<R: Record>` (index document, generation/stamp, slot table, file lock, `find`, reload), with per-library record types and scan/import hooks. Move `BlockPeaks` to `resonance-dsp`.
- **Verification:** One shared conformance test module (find ambiguity, slot reuse after delete, reload on stamp change, lock contention) run against both libraries.

### ARCH2-05 [medium] Inside resonance-app, `state` depends on `view`, `update` and the socket
- **Where:**
  - `state/ui_transient.rs:35,44,59,72,82` holds `view::ui_caches::UiViewCaches`, `view::transport_labels::TransportLabels`, `update::arrangement::ShiftOutcome`, `update::shortcuts::TypingProbe` and `update::keymap::KeymapEditorState`.
  - `state/arrange.rs:20` imports `view::arrange_layout`.
  - `state/project_io.rs:63` imports `update::project_io::reconcile::Origin`.
  - `state/control.rs:9,90,94,103-115` holds `update::control::{AmpLibraryCache, DrumKitLibraryCache}` and `control_socket::{ConnId, ControlServer, ReplySender}`.
  - The view imports handler-side query code: `engine_events::performance::{chord_readout, section_readout}` (`view/performance/mod.rs:230,376,468,513`, `beat_cue.rs:38`). There are 21 `crate::update::` references under `view/`.
- **What:** The intended direction (update → state ← view) is cyclic in practice. No invariant covers it; `view_layer_never_reads_the_engine` only greps for `.engine.`.
- **Impact:** State can't be built or tested without view and update, which blocks any headless model extraction (A7-4 / `resonance-model`). The next cross-layer type will land in `state/` without anyone noticing.
- **Fix:**
  - Move these types into `state/` (or `state/cache`).
  - Move `chord_readout`/`section_readout` into `state` or a `query` module.
  - Add an invariant: no `crate::view::`, `crate::update::` or `crate::control_socket::` under `state/`.
- **Verification:** The new invariant fails today on the 11 lines above and passes after the move.

### ARCH2-06 [medium] The control protocol silently ignores unknown parameters, and `PROTOCOL_VERSION` has never changed
- **Where:**
  - `resonance-control/src/rpc.rs:99-115`: plain `serde_json::from_value`, with zero `deny_unknown_fields` in the crate.
  - `resonance-mcp/src/tools/*.rs`: tools deserialize into typed `Parameters<…>` and re-serialize, so the MCP side drops unknown arguments.
  - `lib.rs:61`: `PROTOCOL_VERSION = 1` since a97c68c0; the handshake compares with `!=` (`update/control/mod.rs:423`).
  - CTL-01 (102ec1da) changed the meaning of `beat` in `PositionSpec` without a version bump.
- **What:** For requests, "tolerant of unknown fields" means a misspelled or wrong-surface field is acknowledged as a success that did nothing. CTL-13 was exactly this class of bug and was fixed one method at a time.
- **Failure scenario:**
  - `transport_seek` called with `beats` instead of `beat`: the call is acknowledged, and the agent reports success on a no-op.
  - An installed `resonance-mcp` built before CTL-01 still handshakes, and seeks to the wrong beat in 6/8.
- **Fix:**
  - Add `#[serde(deny_unknown_fields)]` to every `*Params` (requests only; results stay tolerant), or centrally compare the request keys against the schemars schema and return `invalid_params` naming the field.
  - Document that a change in meaning bumps the version or adds a capability flag.
- **Verification:**
  - Every `*Params` type rejects `{"__unknown": 1}`.
  - An MCP tool call with an unknown argument returns an error (`resonance-mcp/tests/translation.rs`).

### ARCH2-07 [medium] The arch-invariants tests are weaker than ARCHITECTURE.md claims
- **Where:** `tools/arch-invariants/tests/architecture.rs`
  - :343 says "dependency name prefix", but :377 matches names exactly. `iced_widget`, `iced_core`, `iced_runtime`, `egui_extras`, `egui-wgpu`, `winit` and `wgpu` in a non-app crate all pass.
  - :190-199 gives all plugins one shared allow-list. Any plugin may depend on `resonance-mastering-assist` or `resonance-music-theory`, though the diagram grants them only to mastering and granular-delay. `resonance-dsp-test-support` is allowed as a normal dependency.
  - :840 catches only lines starting with the literal `#[cfg(test)]`; `#[cfg(all(test, …))]` and `#![cfg(test)]` pass.
  - :153 cuts every line at the first `//`, so code after a `"https://…"` string is invisible to every rule.
  - Rules that are not encoded at all:
    - "control handlers never call `engine.send`" (0 sites today);
    - "state imports no view/update" (ARCH2-05);
    - the test-binary rule outside app and audio (~410 top-level test files: music-theory 42, drums 42, mastering 35, dsp 33, resonance-plugin 30, amp 28);
    - RT no-logging for `render_pool/` (only `mixer/` is checked, :1088).
- **Impact:** Adding `iced_widget` to `resonance-audio`, or `resonance-mastering-assist` to the compressor, keeps the suite green, so the documented guarantee erodes the way ARCH-10 was meant to prevent.
- **Fix:**
  - Prefix matching (`p`, `p_*`, `p-*`) and more forbidden crates (`winit`, `wgpu`, `eframe`).
  - A per-plugin allow row; `resonance-dsp-test-support` only as `kind == "dev"`.
  - Match any `cfg(…test…)`.
  - A string-aware comment stripper.
  - The missing rules above.
- **Verification:** The file's own "exercised once" ritual for each rule: add `iced_widget` to resonance-audio, `#[cfg(all(test, unix))]` to resonance-dsp, and `r.engine.send(..)` under `update/control/`. Each must fail, then be reverted.

### ARCH2-08 [low] Test scaffolding ships in production builds of the app and engine, and resonance-audio turns off dead-code warnings crate-wide
- **Where:**
  - `resonance-app/src/lib.rs:24,38`: `pub mod demo;` (743 lines) and `mod test_support;` (3120 lines, ~260 `impl Resonance` methods), with no gate.
  - Engine: `engine/thread/mod.rs:17` (`test_support`, 1278 lines) and `mixer/mod.rs:48`, also ungated.
  - `resonance-audio/src/lib.rs:1`: `#![cfg_attr(not(feature = "test-internals"), allow(dead_code, unused_imports))]`.
- **Impact:** Test mutators that bypass `update()`, undo and the mutation gate are `pub` on the production `Resonance`. About 5k lines of fixtures ship. Real dead code in resonance-audio is never flagged.
- **Fix:** A `test-support` feature on resonance-app, enabled through a self dev-dependency (as `resonance-plugin` already does). Gate the engine modules on `test-internals`, and narrow the `allow` to those modules.

### ARCH2-09 [low] ARCHITECTURE.md and refactor-intent.md have drifted from the code
- **Where:**
  - **ARCHITECTURE.md:9-23:**
    - The diagram omits `resonance-control`, `resonance-mcp` and `resonance-dsp-test-support` (all three are in `allowed_internal_deps`, architecture.rs:214-221).
    - It omits the edges `resonance-metering → resonance-app` and → 10 plugins, `resonance-common → resonance-app`, and `resonance-control → resonance-app`.
  - **ARCHITECTURE.md:20** labels `resonance-plugin → resonance-app` as "(UI helpers)". In fact the app uses its preset library, `BrowserModel`, `kit_rows`, `nam_rows` and `stable_hash`, and that edge links `clack-plugin` (the plugin-side CLAP SDK) into the host.
  - **ARCHITECTURE.md:65** says "canonical tokens — see above", but nothing above describes them.
  - **refactor-intent.md:3** and its "Current state" blocks (:84-95: "88 fields", "32 catch-all arms", "UndoExtras still has 8 fields") are unchanged, with no done markers. In reality `Resonance` has 40 fields, `UndoExtras` is gone, and epics A/B/C/E have landed.
  - **`resonance-audio/src/types/tempo/map.rs:225-229`** still describes the read-lock escalation and `try_read()` that Epic B removed.
- **Impact:** refactor-intent.md:8 tells agents to "pick up one todo and land it", so the next agent re-runs finished epics.
- **Fix:**
  - Update the diagram and relabel the `resonance-plugin → resonance-app` edge.
  - Consider moving the headless preset/browser library out of the CLAP SDK.
  - Add a status column to refactor-intent §2.
- **Verification:** A test that every `cargo metadata` package is named in ARCHITECTURE.md's layering section.

### ARCH2-10 [low] The project's saved `bpm` and time signature follow the playhead, not the project
- **Where:**
  - `update/project_io/serialize.rs:411-413` saves `r.transport.bpm` and `time_sig_*`.
  - That field is overwritten with the tempo under the playhead by:
    - `update/tick.rs:341` (`sync_tempo_at_playhead`)
    - `update/global_track.rs:88` (`sync_tempo_display`)
    - MIDI-clock detection (`engine_events/transport.rs:218`)
  - The same field feeds `rebuild_tempo_map` (`global_track.rs:77`).
  - `reconcile/globals.rs:53-55` sends `SetBpm` from the file value.
  - `restore_tempo_events` (`replay/restore.rs:649-656`) uses `file.bpm` as the bar-0 tempo when `tempo_events` is empty.
- **What:** There are three copies of the project tempo, and the one that tracks the playhead for display is the one persisted.
- **Failure scenario:** 90 BPM with a change to 140 at bar 33; stop at bar 40 and save. `project.json` gets `bpm: 140` and the bar-33 time signature, and `song.summary` reports 140. A clock-slaved session saves the external clock's tempo with no undo entry. (Whether this also churns undo or the dirty flag is unconfirmed.)
- **Fix:** A separate `transport.display_bpm`. Serialize bpm and time signature from `tempo_events[0]` and `signature_events[0]`, and have `rebuild_tempo_map` read from there.
- **Verification:** Seed tempo events 90→140 at bar 33, seek to bar 40, run `sync_tempo_display`, build the project file, and assert `bpm == 90.0` and the bar-0 time signature.

### ARCH2-11 [low] Wire-protocol types are used as app domain types, and the preset-source enum exists three times
- **Where:**
  - `resonance_control::methods::plugin_preset::PluginPresetSource` is used in app state (`state/presets.rs:17,34,117,212`), `message.rs:294`, `engine_events/plugins.rs:774,1060` and `view/preset_browser.rs:125`.
  - The preset library has its own `resonance_plugin::presets::PresetSource`, mapped at `update/plugin_preset_ui.rs:329-332`.
  - `compose/invariants.rs:8` sets `MAX_SECTION_BARS = resonance_control::MAX_BARS`.
- **Impact:** A protocol-only change forces edits to app state, messages and the undo classification. A wire limit change silently changes how far a section can grow in the GUI.
- **Fix:**
  - Use an app-owned enum or `resonance_plugin::presets::PresetSource` in state and messages, and map to the wire type only in `update/control/view_model`.
  - Define `MAX_BARS` on the domain side.
- **Verification:** A grep invariant: no `resonance_control` outside `update/control/`, `control_socket.rs`, `control_jobs.rs`, `state/control.rs` and `test_support`.

### ARCH2-12 [low] Size hotspots and grab-bag modules break the ARCHITECTURE.md size rules, and nothing checks them
- **Where:**
  - Largest functions:

    | Function | Location | Lines |
    |---|---|---|
    | `route_engine_event` | `engine_events/dispatch.rs:26` | 698 |
    | `handle` | `update/track.rs:306` | 481 |
    | `AudioEngine::with_options` | `engine/mod.rs:727` | 446 |
    | `process` | `clap_bridge/process.rs:101` | 443 |
    | `build_project_file` | `serialize.rs:123` | 373 |
    | `engine_thread` | `engine/thread/mod.rs:440` | 370 |

  - 64 `mod.rs` files exceed the ~200-line rule (ARCHITECTURE.md:165). Worst:

    | File | Lines |
    |---|---|
    | `view/timeline/mod.rs` | 1414 |
    | `resonance-audio/src/engine/mod.rs` | 1357 |
    | `drums/src/kit_loader/mod.rs` | 1087 |
    | `drums/src/stream/mod.rs` | 932 |
    | `render_pool/mod.rs` | 920 |
    | `view/performance/mod.rs` | 903 |
    | `update/compose/mod.rs` | 882 |

  - `UiMessage` (52 variants) mixes in project lifecycle, quit confirm, autosave settings and MIDI-clock routing that sends `AudioCommand::SetMidiClock*` (`update/ui.rs:226-247`).
  - `is_read_only_method` (`update/control/mod.rs:463-483`) classes the mutating `project.*`, `presets.*` and `drum_kits.*` methods as read-only. The behaviour is correct; the name is wrong.
- **Impact:** These files are merge-conflict magnets for parallel agents. A future caller that trusts `is_read_only_method` will mishandle `presets.delete`.
- **Fix:**
  - Split `route_engine_event` into per-domain functions.
  - Make `engine/mod.rs` a re-export-only file.
  - Move the project, quit and MIDI-clock variants out of `UiMessage`.
  - Rename `is_read_only_method` to `bypasses_mutation_gate`.
  - Add an optional size ratchet to the invariants.

### Strengths / no-action
- **Crate DAG:** the manifests match the invariant table exactly.
  - `resonance-control` has zero internal dependencies.
  - The app links no plugin crate.
  - No plugin names a platform GUI runtime.
- **No orphan `.rs` files.**
- **`AudioEngine`:** the surface is send / try_recv / lifecycle only.
- **Control path:** `update/control/` has zero `engine.send` calls; every control mutation goes through `run_via_update` and the compound undo.
- **`route_engine_event` and the undo classifiers** are exhaustive, with no wildcard arms.
- **MCP tools** are cross-checked 1:1 against `capabilities()`.
- **Epic A targets met:** `Resonance` is at ≤40 fields and `UndoExtras` is gone.
- **Sidechain routing** exists once now.
- **The plugin pattern** holds in all 13 plugins.

---

## DEP — Dependencies

No high-severity findings. Most serious: an unsound/unmaintained proc-macro chain under `printpdf`, a ~1.5 MB D-Bus/async stack in every plugin binary, a build-time network download in `ort`, and statically linked LAME (LGPL). The rest is hygiene.

*Method:* `cargo-audit`/`deny`/`udeps`/`machete`/`outdated` are not installed — advisory IDs are from memory and marked (unconfirmed). Everything else is from `cargo tree`, `cargo metadata`, `Cargo.lock`, the local registry index, and `nm`/`readelf` on the release bundles `target/bundled/*.clap` (built 2026-10-02 09:14).

### DEP-01 [medium] `printpdf 0.9` pulls an unsound `ouroboros` 0.17, an unmaintained `proc-macro-error`, and a third copy of `syn`
- **Where:** `resonance-app/Cargo.toml:26` — `printpdf = { version = "0.9", default-features = false }`.
- **What:** printpdf 0.9.1 → allsorts 0.16.1 → ouroboros 0.17.2 → ouroboros_macro → proc-macro-error 1.0.4 → syn 1.0.109. This chain is the only source of syn 1, ouroboros 0.17, heck 0.4 and one itertools 0.10 copy (iced already uses ouroboros 0.18.5). ouroboros < 0.18 is RUSTSEC-2023-0042 (unsound self-referencing); proc-macro-error is RUSTSEC-2024-0370 (unmaintained). IDs (unconfirmed).
- **Impact:** A soundness advisory in the shipped app, on the chord-sheet PDF export path (`resonance-app/src/chord_sheet_pdf.rs`), plus one more proc-macro stack to compile.
- **Fix:** Upgrade to printpdf 0.12.x (0.12.8 → allsorts ^0.17.1 → ouroboros ^0.18). The API changed, so `chord_sheet_pdf.rs` needs porting. Its op-stream golden should stay byte-for-byte unchanged (see `chord_box.rs:11`).
- **Verification:** `cargo tree -i proc-macro-error` and `cargo tree -i ouroboros@0.17.2` both print "did not match"; Cargo.lock has no `syn` 1.x entry.

### DEP-02 [medium] `ort` default features download a prebuilt ONNX Runtime at build time, pinned to an exact pre-release
- **Where:** `resonance-svs/Cargo.toml:41` — `ort = { version = "=2.0.0-rc.12", features = ["ndarray"] }`, with default features on.
- **What:** The resolved features include `download-binaries`, `copy-dylibs` and `tls-native`.
  - ort-sys's build script fetches ONNX Runtime from pyke's CDN (cache: `~/.cache/ort.pyke.io`, 87 MB).
  - That binary is statically linked into `resonance-app`.
  - `tls-native` pulls in openssl as a build dependency.
  - The app depends on resonance-svs unconditionally, so every app build and app test build goes through this.
- **Impact:**
  - Supply chain: an unhashed binary is fetched at build time and linked into the app.
  - A fresh clone or empty cache cannot build offline, and the build needs openssl headers.
  - The `=` rc pin blocks semver updates.
- **Fix:** Set `default-features = false` and list the features explicitly. Then either use `load-dynamic` with a documented `ORT_DYLIB_PATH`, or keep `download-binaries` with `ORT_LIB_LOCATION` pointing at a vendored, checksummed copy (and `tls-rustls`). Move to ort 2.0 final when it is released.
- **Verification:** `cargo tree -p resonance-svs -e features -i ort | grep download-binaries` is empty; `cargo tree -p resonance-svs -i openssl` does not match.

### DEP-03 [medium] Every plugin binary carries ~1.5 MB of zbus/async-io just for rfd's file dialog
- **Where:**
  - `resonance-plugin/Cargo.toml:17` (feature `editor-widgets` → `dep:rfd`).
  - `Cargo.toml:28` (`rfd = "0.15"`, default features on).
  - Callers: `resonance-plugin/src/preset_ui.rs:307,313`, plus amp, drums, ir and wavetable.
- **What:** rfd 0.15 defaults to `xdg-portal` + `async-std`, which pulls in ashpd 0.11, zbus 5, zvariant, async-io, blocking and other async crates.
  - Measured in `resonance-gate.clap` with `nm -S`:

    | Component | Size |
    |---|---|
    | zbus/async stack | ≈1489 KB |
    | egui/epaint | ≈997 KB |
    | wayland | ≈431 KB |
    | rfd's own code | ≈25 KB |

  - So the D-Bus stack is bigger than the GUI toolkit. It is identical in all 13 bundles (~19 MB total).
  - The app also uses the async-io stack even though it already runs tokio.
- **Impact:** Each 11.7 MB plugin is ~13% zbus. That is binary size, plus a large, unsafe-heavy dependency surface inside every plugin a host loads.
- **Fix:** Pick one:
  - Accept it and document it in ARCHITECTURE.md.
  - Move to rfd 0.17 (defaults `["xdg-portal","wayland"]`, no async-std), measure, and turn on rfd's `tokio` feature in the app.
  - Replace the dialog in plugins with a small portal/zenity/kdialog subprocess helper in `resonance-plugin`.
- **Verification:** `nm -C -S target/bundled/resonance-gate.clap | grep -c zbus::` drops to 0 (or the size drops); `cargo tree -p resonance-gate -i zbus`.

### DEP-04 [medium] LAME (LGPL-3.0) is statically linked into the app, and the workspace declares no license
- **Where:** `resonance-audio/Cargo.toml:38,51` (`mp3lame-encoder`, default feature `mp3`). No member manifest has a `license` field, and there is no LICENSE file.
- **What:** `mp3lame-sys` builds LAME from source and links it statically (`rustc-link-lib=static=mp3lame`).
  - Licenses found in the dependency tree:

    | Crate(s) | License | Where it ends up |
    |---|---|---|
    | mp3lame-encoder, mp3lame-sys | LGPL-3.0 | app (static) |
    | 13 symphonia-* crates | MPL-2.0 | app, IR, drums, mastering plugins |
    | option-ext | MPL-2.0 | — |
    | self_cell | Apache-2.0 OR GPL-2.0-only (choose Apache) | — |

  - Everything else is permissive.
- **Impact:** Blocks distribution. A statically LGPL-linked binary requires shipping relinkable objects or the source, and MPL requires notices. This does not matter while Jorrit is the only user.
- **Fix:**
  - Choose a project license and set it in `workspace.package`.
  - Link LAME dynamically, or document LGPL relink compliance.
  - Generate a third-party notices file with `cargo about` or `cargo deny`.
- **Verification:** `cargo metadata --no-deps | jq '.packages[].license'` has no nulls; `ldd` shows libmp3lame linked dynamically.

### DEP-05 [low] The single bundle build merges features across plugins, so the ARCH-07 A7-3 trim only holds at manifest level
- **Where:** `scripts/bundle.sh:111-115` (one `cargo build --release -p a -p b …` call); `plugins/resonance-drums/Cargo.toml` (`decode` + `drumkit-zip`).
- **What:**
  - In the bundle build, `cargo tree -e features` shows `decode` and `drumkit-zip` enabled for every plugin.
  - Workspace and `run-tests.py` builds give `resonance-plugin` the `editor-widgets` feature, so app test builds also compile wayland-plugin-gui, egui and rfd.
  - LTO strips most of this out of the binaries (gate.clap has 0 symphonia symbols).
  - The arch-invariant checks manifests, not the resolved feature set.
- **Impact:** Compile time, and the invariant implies more isolation than the build gives. Code that is reachable but unused is not stripped.
- **Fix:** Document this next to the invariant, or build each plugin in its own cargo invocation (slower builds).
- **Verification:** Run the same `cargo tree … -e features` over the bundle's `-p` set.

### DEP-06 [low] IR and drums link AAC, MP4, Vorbis, Ogg and MP3 decoders they never need
- **Where:** `Cargo.toml:30` (one symphonia feature set for everyone); `resonance-common/src/audio_probe.rs:299,329,372` (`symphonia::default::get_probe/get_codecs`).
- **What:** In resonance-ir.clap these unneeded decoders take ≈480 KB: aac 88, mp4 157, mp3 82, vorbis 82, ogg 68 (KB). drums.clap is the same. Separately, `plugins/resonance-mastering/src/assistant/reference.rs` has its own symphonia decode loop instead of using `resonance_common::decode_file`.
- **Impact:** ~0.5 MB per binary, plus a second decode path to maintain.
- **Fix:** Build an explicit `CodecRegistry`/`Probe` (riff + pcm + flac) for the plugin-side loaders. A cargo feature won't help because features are merged across the bundle build. Route mastering through the common decoder.
- **Verification:** `nm -C target/bundled/resonance-ir.clap | grep -c symphonia_codec_aac` returns 0.

### DEP-07 [low] `resonance-svs` makes the app compile `clap` (derive) and other CLI-only dependencies
- **Where:** `resonance-svs/Cargo.toml:47` (`clap` with derive), `:50` (`tracing-subscriber`).
- **What:** Both are used only by `src/main.rs` and `examples/`, but every app build compiles clap and clap_derive.
- **Impact:** App and test build time; the binary is unaffected.
- **Fix:** Add a `cli` feature: make both dependencies optional, and set `required-features = ["cli"]` on the bin and the examples.
- **Verification:** `cargo tree -p resonance-app -i clap` does not match.

### DEP-08 [low] Unused or misplaced dependencies
- **Where / what:**
  - `wayland-plugin-gui/Cargo.toml:31` `libloading`, `:39` `raw-window-handle`, `:26` `wayland-protocols`: none is used in `src/`. Their needs are already covered transitively (khronos-egl `dynamic`, sctk 0.20 features).
  - `serde_json` is a normal dependency of `plugins/resonance-delay:21`, `compressor:20`, `eq:27` and `reverb:18`, but only `tests/` uses it.
  - `plugins/resonance-amp/Cargo.toml:61` repeats `dirs` (already at `:39`).
  - The `resonance-common` dev-dependency `zip` duplicates what the self dev-dependency with `drumkit-zip` already provides.
- **Fix:** Delete the unused entries; move `serde_json` to `[dev-dependencies]`.
- **Verification:** `cargo check -p wayland-plugin-gui` passes; `cargo tree -p resonance-reverb -e normal --depth 1 | grep serde_json` is empty.

### DEP-09 [low] Versions are pinned per crate instead of as workspace dependencies, causing avoidable duplicates
- **What:**
  - `base64 = "0.22"` is pinned in 5 manifests while ureq 3 and rmcp use 0.23, so amp.clap and drums.clap link both versions.
  - `glow = "0.17"` (both GUI runtimes) must match egui_glow.
  - `clap-sys = "0.5"` (`resonance-audio:11`) must match clack's copy.
  - Also repeated per crate: tokio, anyhow, libc (5 places; resonance-audio has separate linux and macos tables that could be one `cfg(unix)`), libloading (3), sha2 (3), ureq (2), zip (3).
  - `resonance-mcp` and `resonance-svs` pin their own serde, serde_json and tracing versions.
  - `rand = "0.10"` is used only for the PKCE verifier (`plugins/resonance-amp/src/tone3000/auth.rs:57,66`); `getrandom` alone would do.
- **Impact:** Small size and build cost, plus drift risk. The clap-sys and glow cases break if their versions diverge.
- **Fix:** Move these to `[workspace.dependencies]` and bump base64 to 0.23.
- **Verification:** `cargo tree -p resonance-amp -d -e normal | grep base64` shows one version.

### DEP-10 [low] The release profile doesn't strip symbols, and `codegen-units = 1` applies to the whole workspace
- **Where:** `Cargo.toml:42-48`.
- **What:**
  - The bundles are not stripped: `.symtab` + `.strtab` ≈ 2.1 MB of gate.clap's 11.7 MB (~18%), ~25 MB across 13 bundles. Only 2 dynamic symbols are exported, so stripping is safe.
  - `codegen-units = 1` also applies to the app (iced, wgpu, ort, naga), though the comment justifies it only for the DSP plugins.
- **Fix:** Add `strip = "symbols"` (or `"debuginfo"`), and a per-package override `[profile.release.package.resonance-app] codegen-units = 16`.
- **Verification:** `readelf -S target/bundled/resonance-gate.clap | grep symtab` is empty.

### DEP-11 [low] The dev profile keeps full debuginfo for every dependency (disk pressure)
- **What:** wgpu, naga, iced, ort, zbus and the rest all build with debug=2, in ~16 worktrees (629 GB of `target/` dirs per the memory note). (unconfirmed) rustfft and rubato compile at -O0 in debug builds even though resonance-dsp and resonance-common are at -O2. Measure before adding overrides.
- **Fix:** Add `[profile.dev.package."*"] debug = "line-tables-only"`.
- **Verification:** `du -sh target/debug/deps` before and after a clean build.

### DEP-12 [low] `tracing-subscriber` keeps its default features in every plugin binary
- **Where:** `Cargo.toml:19`, used by `resonance-plugin/src/logging.rs`. That file deliberately installs no log bridge (:37-38), yet the default `tracing-log` feature still compiles one in.
- **Fix:** `default-features = false, features = ["std","fmt","ansi","env-filter","smallvec"]`.

### DEP-13 [low] The clack git pin is six months old and has no registry release
- **Where:** `Cargo.toml:6-13`, rev `4541f037` (2026-03-31).
- **Impact:** Builds break if GitHub is unavailable or upstream history is rewritten. There is no semver signal, and upstream fixes since March are not picked up.
- **Fix:** Document a bump procedure (run the `clap_host` group + `editor_open` after each bump). Consider a mirror or a `cargo vendor` snapshot. Keep `clap-sys` in lockstep with clack through the workspace dependencies.

### DEP-14 [info] Native code built at build time
- LAME is built with autotools.
- `audiopus_sys` falls back to building libopus with cmake.
- `pipewire-sys`/`libspa-sys` run bindgen through libclang.
- `paste` (RUSTSEC-2024-0436, unmaintained, unconfirmed) comes in only through metal/wgpu on macOS.

### Duplicate crate versions (Cargo.lock: 782 packages)
| Crate | Versions | Pulled in by | Act? |
|---|---|---|---|
| syn | 1.0.109 / 2.0.119 / 3.0.6 | printpdf chain / strum, phf, bindgen / serde_derive, thiserror 2, zbus, rmcp | Yes, drop syn 1 (DEP-01) |
| ouroboros | 0.17.2 / 0.18.5 | allsorts / iced_widget | Yes (DEP-01) |
| itertools | 0.10.5 / 0.13.0 | allsorts, criterion 0.5 / others | printpdf upgrade; criterion 0.8 exists |
| base64 | 0.22.1 / 0.23.1 | our manifests / ureq 3, rmcp | Yes (DEP-09) |
| rand / rand_core | 0.8 / 0.9 / 0.10 | phf_macros (build only) / ashpd / resonance-amp | Partly |
| getrandom | 0.2 / 0.3 / 0.4 | ring / printpdf etc. / rand 0.10 | Partly |
| alsa | 0.9.1 / 0.10.0 | midir 0.10 / cpal 0.17 | midir 0.11 exists (unconfirmed) |
| glow | 0.16 / 0.17 | wgpu-hal / egui_glow | No, separate binaries |
| sctk, calloop, rustix, thiserror 1 | 2 versions each | winit 0.30 / wayland-plugin-gui | No, upstream |
| skrifa, read-fonts, font-types | 3 versions each | cosmic-text / epaint / swash | No, upstream |

### Strengths / no-action
- resonance-mcp is lean: 86 crates, and its only workspace dependency is resonance-control.
- Test support doesn't leak into normal builds:
  - `resonance-dsp-test-support`, criterion, clack-host, iced_test and tempfile appear only as dev-dependencies.
  - `test-internals` is enabled only through dev-dependencies.
- Headless plugin builds are 107 crates. The GUI runtimes are correctly gated by `cfg(target_os)`.
- Plugin TLS is rustls + ring with no openssl. ort's native-tls build dependency doesn't leak, because resolver 2 keeps build-dependency features separate.
- crossbeam-channel, ring, idna, rustls, tokio and zip are all at or above the patched versions for the advisories I know of.
- LTO dead-stripping works.
- ARCH-07 still holds.
- Outside dependencies, but notable: `resonance-wavetable.clap` is 54 MB because `build.rs` embeds a 40.5 MB `wavetables.bin`.

---

## UX — App UI/UX

Scope: `resonance-app/src/{view,compose,palette,theme,focus,commands}` and the update handlers they drive, plus 8 golden snapshots I opened and looked at. Items fixed in the 2026-09-26 review (VIEW-01..36) are not repeated.

### UX-01 [high] Ctrl+O / "Open Project…" discards unsaved edits with no confirmation
- **Where:** `update/project_io/mod.rs:223-244` (`OpenProject` → `OpenPathSelected` → `start_open`).
  - It checks only `refuse_project_switch_during_render` and `recovery::prompt_before_open`, which concerns the *target's* autosave.
  - `commands/resolve.rs:121` gates the command only on load or save being in progress.
  - Settings › Open Project (`view/settings.rs:42`) takes the same path.
- **What:** `NewProject` refuses while the project is dirty (`resolve.rs:118`, `update/ui.rs:74`), and quit has `confirm_quit`, but Open has no `session.dirty` check. Undo history does not survive a project switch.
- **User impact:** Everything since the last manual save is lost. Recovery offers `project.autosave.json` only after a crash, not after a deliberate open.
- **Fix:** Route Open (and New) through a "Save / Don't save / Cancel" modal that reuses `confirm_quit`'s overlay with a pending-action enum.
- **Verification:** iced_test: make the project dirty, dispatch `OpenPathSelected(Some(p))`, and assert that the confirm overlay is up and the project was not replaced. Add a modal snapshot.

### UX-02 [high] The global chord track is never drawn, yet palette commands edit it and it changes Compose output
- **Where:**
  - `grep chord_track view/` finds nothing. The Arrange "Chords" lane is built "from sections" (`view/track_header/shelf.rs:248-261`).
  - `commands/resolve.rs:248-253` maps `AddChordAtPlayhead`, `DeleteChordAtPlayhead` and `ToggleChordPinAtPlayhead` to `ChordTrackMessage::*` on `r.chord_track.regions`.
  - `compose/generate.rs:65-86` (`overlay_pinned_chords`) replaces section chords with pinned regions.
  - `ChordTrackMessage::SetSymbol` and `AddRegion` are sent only from tests. `chord_track.last_error` (`update/chord_track.rs:205,223,250`) is described as a banner in `project/model.rs:1030`, but it is never shown.
- **What:** Three enabled palette commands change state the user cannot see. Pinned regions then silently override the generated chords.
- **User impact:** Generated parts change harmony for no visible reason, and undo entries appear for edits nobody can see.
- **Fix:** Render chord-track regions (as an Arrange shelf lane that shows pin state), or mark the three commands `Available::No(...)` until a surface exists. Either show `last_error` or drop it.
- **Verification:** A snapshot with a pinned region, or a registry test that an available command never mutates unrendered state.

### UX-03 [high] Untitled projects have no undo, and the header says "saved"
- **Where:**
  - `undo/snapshot.rs:315-323`: `can_record_undo` requires `io.project_path.is_some()`.
  - `update/project_io/instantiate.rs:72,85` sets `project_path = None; session.dirty = false` for Ctrl+N and templates.
  - `view/transport.rs:66` shows "· saved" whenever `!dirty`.
- **What:** After Ctrl+N or starting from a template, no edit is recorded. Undo shows "Nothing to undo", and the chrome reads "Untitled · saved" for a project that has never been written.
- **User impact:** Ctrl+Z does nothing until the first Save As, and the label implies the work is safe.
- **Fix:** Record undo for untitled projects (the autosave scratch dir gives clips a home), or prompt for a location at creation as `StartNewProject` does. Show "· not saved" when `project_path` is `None`.
- **Verification:** iced_test: `NewEmptyProject`, add a track, `Undo`, and assert the track is gone.

### UX-04 [medium] A single error-banner slot: critical engine states get overwritten, and Ctrl+N's refusal is a dead end
- **Where:**
  - `banners.error_message: Option<String>` is written from about 40 sites.
  - `update/tick.rs:159-170` latches "Audio engine stopped responding" once (`engine_disconnected_banner_shown`).
  - `update/ui.rs:74-77` answers Ctrl+N on a dirty project with "Save the project first".
- **What:**
  - A later trivial error replaces the engine-death or stream-lost text, and the latch keeps it from ever coming back.
  - Banners never expire or stack.
  - Ctrl+N's refusal is an error, not a choice.
- **User impact:** After the engine dies and one more error lands, nothing tells the user that edits no longer reach audio.
- **Fix:** Show engine and stream health as persistent status (e.g. a BAD chip in the transport), separate from transient errors. Put transient errors in a queue or toast. Use the UX-01 modal for New.
- **Verification:** Test: the engine disconnects, then a bounce error arrives; the engine status is still rendered.

### UX-05 [medium] Showing or dismissing the error banner resets all widget state in the main area
- **Where:** `view/mod.rs:132-159` builds `column![transport, error_bar, main_area]` when there is an error, and `column![transport, main_area]` when there isn't.
- **What:** `main_area` moves from child 1 to child 2, so iced builds its tree fresh. That throws away:
  - scrollable offsets (mixer, browser, inspector)
  - canvas `Program::State`, including `KeyFocus` and in-progress drags
  - text-input focus

  `view/compose/page.rs:43` avoids this correctly with a zero-height placeholder.
- **User impact:** Any error (e.g. a failed preset star) scrolls the mixer back to the start and drops canvas key focus, and clicking × does it again.
- **Fix:** Always render the slot (the bar or a zero-height `Space`), or put the banner in a `stack` overlay.
- **Verification:** iced_test: scroll the mixer, set `error_message`, re-render, and assert the scroll offset was kept.

### UX-06 [medium] Drum-grid pad names collide with their share-% labels
- **Where:** `view/compose/drumroll/canvas.rs:384-408`. The name starts at `x + 26` with no width limit; the share starts at `x + 6 + PAD_LABEL_WIDTH(76) - 28 = x + 54`, which leaves the name 28 px.
- **What:** In `compose_chord_rail_generator_schema-wgpu.png` the labels overprint: "Snare70%", "Closed0%", "Tom H8%", "Tamb15%". The share is 9 px in `TEXT_4`.
- **Fix:** Right-align the share and ellipsize the name, or widen `PAD_LABEL_WIDTH` (it is a constant; move it to theme.rs). Use `TEXT_3` at 10–11 px or larger.
- **Verification:** Re-bless that golden and `drum_grid_chained_arrangement_tints_separator`, then check them visually.

### UX-07 [medium] Text contrast and size fall below what ux-guidelines.md promises
- **Where:**
  - Contrast on the `BG_2` background (#1b1d23):

    | Token / element | Colour | Contrast |
    |---|---|---|
    | `TEXT_3` | #5d626d | ≈2.8:1 |
    | `TEXT_4` | #3f434c | ≈1.7:1 |
    | Error bar text (`view/mod.rs:136-142`): `Color::WHITE` on `BAD` | #e87b8b | ≈2.75:1 |

  - The error bar's pure white also breaks the colour rule.
  - Sizes under `view/`: 14 sites use `.size(8)`, 71 use 9 and 143 use 10 (e.g. the fader dB label at `controls.rs:374` is 9 px). The guideline says "never below 11px".
- **What:** `TEXT_3` carries real information (track kind, the dirty label, rail hints), and `TEXT_4` carries drum shares and the shelf letters.
- **Fix:**
  - Raise `TEXT_3` to ≥4.5:1 (≈#8a909b) and keep `TEXT_4` for disabled text only.
  - Draw the error bar with `BG_0`/`TEXT_1`.
  - Either lower the 11 px rule to the floor actually in use or enforce it with an arch-invariants grep.
- **Verification:** A contrast unit test over the theme tokens, then a snapshot re-bless.

### UX-08 [medium] Mute/solo/monitor/freeze show state by glyph colour alone, and icon buttons have no tooltips
- **Where:** `view/controls.rs:114-138` (mute and solo tint only the glyph), `freeze_button` at `:89`. Only `transport.rs` and `preset_browser.rs` use `shortcut_hint::with_hint`, and there are 3 `tooltip(` calls in all of `view/`.
- **What:**
  - The guideline says state "never relies on color alone". Record arm does it right, with a filled style and border.
  - The eye glyph means *input monitor* but reads as show/hide.
  - No mini-button, FX bypass or the trash icon explains itself or shows its shortcut.
- **Fix:** Use a filled background plus border for the active state, wrap each button in `with_hint`, and pick a different monitor glyph.
- **Verification:** Snapshots of a track header and a mixer strip with mute and solo on.

### UX-09 [medium] Faders lack reset and fine-adjust, and use iced's default colours
- **Where:** `view/controls.rs:357-367`, `vertical_slider(-60.0..=6.0, …).step(0.1)`, with no `.default(0.0)` or `.shift_step(..)` and the default style (iced palette blue, not `ACCENT`).
- **What:** ux-guidelines promises faders "the same shift-for-fine and double-click-reset as knobs".
- **Fix:** Add `.default(0.0)` and `.shift_step(0.01)`, or build a custom fader. Add a theme `fader_style`.
- **Verification:** iced_test fader interaction, then a mixer re-bless.

### UX-10 [medium] The Compose view has no `lazy` regions and is rebuilt every frame during playback
- **Where:**
  - `grep "lazy(" view/compose` finds nothing.
  - `pick_list` option `Vec`s are built on every call: `lane_inspector/drums/arrangement.rs:209-217,328,545` clones every pattern name each frame; also `melody.rs`, `bass.rs`, `chord/body.rs:243`.
  - There are 97 `format!` calls in `view/compose`.
  - The fast tick runs at 16 ms while playing (`update/tick.rs:38`).
- **What:** This violates the project's own lazy-wrap and pick_list-caching rules.
- **Fix:** `lazy`-wrap the right rail and the non-playhead lane headers, keyed on (definition revision, selection, collapse set). Move the option lists into `UiViewCaches`.
- **Verification:** A fingerprint test like `compose_canvas_cache_fingerprints.rs`, plus a `view_compose` timing benchmark.

### UX-11 [medium] The transport's CPU readout is a hard-coded placeholder
- **Where:** `view/transport.rs:397`, `text("CPU —")`. `resonance-audio/src/cycle_load.rs` already publishes smoothed load and peak "for lock-free UI reads", and nothing in the app reads it.
- **Fix:** Poll it on the tick, show `CPU nn%`, and tint it WARM/BAD near or over budget.
- **Verification:** Transport snapshot with a stubbed load.

### UX-12 [medium] Undo/redo give no feedback; the labels exist but go only to MCP
- **Where:** `undo/history.rs:109-114` (`undo_label`/`redo_label`) is read only in `update/control/edit.rs:36`.
- **User impact:** Undo stops transport and can change another tab (`snapshot.rs:406`), and users can't see what changed.
- **Fix:** Show "Undo delete bus" in the palette rows, plus a short toast after undo/redo.

### UX-13 [medium] Autosave failures are only logged
- **Where:** `update/project_io/mod.rs:327-331, :491, :505`.
- **What:** A persistent failure (disk full, permissions) is never shown. "Disk quota exceeded" has happened on this machine before.
- **Fix:** Count consecutive failures; after N, show an "Autosave failing: <reason>" chip.
- **Verification:** Three `ProjectSaved(Err, true)`, then assert the indicator is visible.

### UX-14 [low] Deleting a user track preset is a tiny "×" with no confirm or undo
- **Where:** `view/menus.rs:44-51` (≈12×14 px, inside the add-track row button); `update/track.rs:738-743` → `presets::delete_user_preset` removes the file (`presets.rs:320`).
- **Fix:** Confirm inline, or use a hover-revealed button at least 22 px square.

### UX-15 [low] The BPM field silently keeps uncommitted text on blur
- **Where:** `view/transport.rs:336-344` (`on_submit` only); `update/transport.rs:211-213`.
- **What:** Type "95" and click away: the field shows 95 while the song plays at 90.
- **Fix:** Revert the field on the next unrelated message, or on a mouse press outside, as `inline_rename` does.

### UX-16 [low] Clickable transport readouts give no visual cue, and one click on SIG changes the song meter
- **Where:** `view/transport.rs:346-354`. SIG is a bare `mouse_area(text)` that emits `CycleTimeSignature`; KEY looks the same but does nothing.
- **Fix:** Give SIG a hover style, a pointer cursor and a hint, or open a meter picker.

### UX-17 [low] dB readouts are formatted three different ways
- **Where:** `util::format_db` gives "0.0" / "-inf" (faders). Sends, bus members, automation, clip gain and reference offset use `"{:+.1} dB"`.
- **Fix:** One `format_db_signed` helper ("+0.0 dB", "−∞ dB").

### UX-18 [low] Track names are hard-clipped with no ellipsis or tooltip
- **Where:** `view/track_header/track.rs:67-90` (`Wrapping::None` + `clip(true)`). The snapshots show "Drums Bour" and "Resonance Wa".
- **Fix:** Use `util::short()` and add a tooltip.

### UX-19 [low] Colours defined outside theme.rs
- **Where:**

  | Place | Colour used | Should be |
  |---|---|---|
  | `view/menus.rs:32,98,102` | cyan | a token |
  | `view/compose/global_tracks.rs:236-293` | orange | `WARM` |
  | `global_tracks.rs:366` | blue | a token |
  | `view/compose/page.rs:33` | salmon | `BAD` |
  | `view/knob.rs:138-180` | greys | `BG_3`/`LINE` |
  | `export_dialog.rs:52`, `bounce_dialog.rs:104` | `Color::WHITE` | a token |
  | `lane_inspector/chord/mod.rs:193` | white background | a token |
  | `kit_picker.rs:214` | `Color::BLACK` | a token |

  About 71 inline colours in total.
- **Fix:** Add tokens (`INSTRUMENT_TINT`, `ON_ACCENT_TEXT`), and an arch-invariants grep for `Color::WHITE`/`BLACK` in `view/`.

### UX-20 [low] Some palette glyphs are misleading
- **Where:** `commands/mod.rs:427-437`: loop commands and Undo/Redo all use `ARROW_ROTATE_LEFT`, and `LoopSelection` falls back to the clock glyph.
- **Fix:** `fa::REPEAT` for loop commands, rotate-left/right for Undo/Redo.

### UX-21 [low] Some UI actions are missing from the command palette
- **Where:** `commands/mod.rs:166-262`. Missing:
  - add external instrument track
  - bounce-in-place for the selected track
  - rename the selected track or bus
  - delete the selected bus
  - toggle FX bypass
  - toggle mono
  - save track as preset
- **Fix:** Register them, with availability based on the current selection.

### UX-22 [low] `Message::Tick` used as a do-nothing click message
- **Where:** `view/compose/drum_groups_manager/kit_picker.rs:284`.
- **What:** `handle_tick` drains engine events, decays the meters an extra step, ticks autosave and flushes palette recents on every click of an inactive pad row.
- **Fix:** Don't set `on_press` when there is no active group.

### UX-23 [low] Code points to a doc that no longer exists
- **Where:** `view/ui_caches.rs:13` (and MEMORY.md) cite `.claude/skills/ui-work.md §11`, which no longer exists.
- **Fix:** Move the view-performance rules into ux-guidelines.md and update the pointers.

### Strengths / no-action
- Keyboard dispatch (`update/shortcuts.rs`) is well layered, with a typing gate for every bare key and focus probing. I found no new shortcuts firing while typing.
- The command registry is solid: a macro-derived `ALL`, unavailable rows dimmed with a reason, preset keymaps. The palette's empty state is clean.
- The undo classifier is exhaustive, and arch-invariants guard it. Track delete confirms even though it can be undone.
- Mixer, browser, performance and track headers follow the lazy/UiViewCaches rules. Compose's status slot keeps the tree shape stable.
- The freeze-failed banner (Retry / Dismiss, icon plus colour) is the pattern the global error bar should copy.

---

## PUX — Plugin editors & GUI runtime

Scope: `plugin-gui-core`, `wayland-plugin-gui`, `cocoa-plugin-gui` (read only, not compiled), `resonance-plugin/src/{editor_host,editor_widgets,param,preset_ui}.rs`, the host-side param plumbing, and all 13 editors. No plugin snapshot PNGs exist, so layout findings come from arithmetic on the code, not from renders.

### PUX-01 [high] 12 of 13 editors never announce edits to the host: the host mirror goes stale, and save→reopen can revert editor tweaks
- **Where:**
  - `resonance-plugin/src/host.rs:282` (`announce_param_change`) is the only path to CLAP output param events, and only the drums call it (`plugins/resonance-drums/src/lib.rs:375,391`).
  - The shared helpers (`resonance-plugin/src/editor_widgets.rs:87-158`: `float_knob`, `float_slider`, checkbox, combo) only call `set_normalized`/`set_value`.
  - The classic `widgets::knob` (`plugin-gui-core/src/widgets.rs:177`) returns only `bool`, so it has no gesture begin or end.
  - The preset bar (`preset_ui.rs` via `PresetBank::apply`) writes params silently, and its identity report (`resonance-audio/src/engine/plugins.rs:292`) triggers no refresh.
- **What:** The host learns plugin-side values only from output events or from a refresh, and a refresh runs only after a host preset or state load (`engine/plugins.rs:998-1007`). So for amp, color, compressor, delay, eq, gate, granular, ir, mastering, reverb, stereo and wavetable, `slot.params[].current_value` is stale.
- **Impact:**
  - Editor edits create no undo entries, and automation can't be recorded from the GUI.
  - The mixer inspector, the automation lane seed (`update/automation.rs:488`) and MCP `param_view` all show stale values, which breaks the dual-surface rule.
  - **Data loss:** `project_plugin` (`update/project_io/serialize.rs:41-58`) writes every mirror value that differs from its default as a host override. On reopen, `apply_pending_param_overrides` (`engine_events/plugins.rs:259-283`) re-sends those values *after* the state blob, by design, and undo does the same (`reconcile/plugin_state.rs:230-273`).
  - Example: host-load preset A, then pick preset B or turn knobs in the editor, then save and reopen. Every param where A differs from its default comes back as A.
- **Fix:**
  - `widgets::knob` returns `GestureEdit`.
  - `editor_widgets` gets a `HostHandle`/`EditAnnouncer` and announces on `ended`.
  - Preset-bar recalls call `request_params_rescan(VALUES)`.
  - Belt and braces: refresh param values before `project_plugin` serialises.
- **Verification:** A hermetic app test (`new_for_test_with_capture`): change a param plugin-side, save, reopen, and assert it survives. Repeat after a host preset load followed by an editor-side preset pick.

### PUX-02 [high] Knobs on stepped (bool/int) params can't be dragged: in the delay editor, Sync, Freeze and Gate can't be switched at all
- **Where:**
  - `plugin-gui-core/src/widgets.rs:141-148,213-255,663-677`: the knob recomputes from the caller's *current* value every frame.
  - `plugins/resonance-delay/src/editor/widgets.rs:7-27` draws all 22 params as knobs, including the bools sync, freeze and gate_on, and the ints division, character, routing and gate_shape.
  - `IntParam::set_plain` rounds and `BoolParam` thresholds at 0.5 (`param.rs:600,707`).
  - Same problem in `plugins/resonance-wavetable/src/editor/tabs/mod.rs:108-124` (`int_knob_inner`: dist Mode, OS, Coarse, Voices, LFO/S&H Div, LFO Shape).
- **What:** Each frame the knob moves about 0.005/px from the re-read, already *rounded* value, so any per-frame delta smaller than half a step snaps back.

  | Control | Drag needed in one frame |
  |---|---|
  | delay bool | ~100 px |
  | `routing` | 50 px |
  | `division` | 9 px |
  | wavetable dist Mode | 20 px |

  Shift multiplies these by 5. A normal drag moves 1–3 px per frame. `HSlider` fixed this by accumulating travel (`widgets/slider.rs:276-293`); the knobs never did.
- **Impact:** In the delay you can't turn Sync off, engage Freeze or enable the Gate. Double-click only resets to the default, and there is no other control for them. The wavetable's discrete knobs respond only to fast flicks.
- **Fix:** Keep the drag start value and the accumulated travel in `ui.data`, keyed by response id, in both knob families. Render bool and choice params as chips or segmented controls.
- **Verification:** Headless harness (pattern from `plugins/resonance-amp/src/editor/mod.rs:83`): drag 120 px in 2 px steps and assert the value changes.

### PUX-03 [high] The delay editor's controls run off the window: Gate and Duck are unreachable at the default size
- **Where:** `plugins/resonance-delay/src/editor/controls.rs:6-78`: one `ui.horizontal` holding 22 × 64 px knobs in 7 groups, with no wrap and no scroll. `editor/factory.rs:19-20,66-67`: default 1200×600, min 900×480.
- **What:** The row is about 1710 px wide. The gate group spans ≈1110–1476 px and duck ≈1488–1710 px.
- **Impact:**
  - At 1200 px, almost all of Gate and all of Duck are clipped.
  - At the tiled 1571 px, Duck Threshold and Release are still cut off.
  - At 900 px, half the editor is gone.
  - Delay is the one editor the audit migration missed, and it has no `tests/editor_*` file.
- **Fix:** Use a data-driven `GROUPS` table with wrapping cards, as gate and stereo do, plus a layout test.
- **Verification:** At 900×480, 1200×600 and 1571×856, assert every knob rect lies inside `screen_rect`.

### PUX-04 [medium] Editors repaint at the monitor refresh rate forever, even when silent; 30 Hz requests are ignored
- **Where:**
  - `plugin-gui-core/src/repaint.rs:17,44-49`: any delay under 50 ms becomes `RepaintPlan::Now`. On Wayland that is paced only by frame callbacks.
  - Every editor except drums calls `request_repaint_after(16ms|33ms)` on every frame.
  - `theme::apply` → `ctx.set_visuals` also runs every frame (`theme.rs:73-95`).
- **Impact:** A visible editor repaints at 60/144/240 Hz, and an occluded one at about 4 fps. That is constant GPU and CPU load next to the audio work. Drums shows the right model (30 Hz while lit, 10 Hz live, slow idle poll; `drums/src/editor/app.rs:838-848`).
- **Fix:**
  - The runtimes honour the requested delay as a minimum frame interval.
  - Editors request a fast repaint only while meters are moving.
  - Call `theme::apply` once, at creation.
- **Verification:** `plan_repaint(now, 33ms, None)` returns `At`; a frame-counter test with stubbed frame callbacks.

### PUX-05 [medium] Delay and granular-delay knobs ignore the declared skew (missed by audit F4/C5)
- **Where:**
  - `plugins/resonance-delay/src/editor/widgets.rs:7-27` (linear) and `plugins/resonance-granular-delay/src/editor/widgets.rs:128-157` (linear).
  - The skewed params:

    | Plugin | Skewed params |
    |---|---|
    | delay | `time_ms`, `hi_cut`, `lo_cut`, `mod_rate` |
    | granular-delay | `time_ms`, `grain_size_ms`, `density_hz`, `spray_ms`, `filter_hz` |

- **Impact:** On granular Filter, 20–500 Hz occupies about 5 px. The editor arc also disagrees with the host automation lane. plugin-audit.md F4 even cited the granular file as "the pattern to copy".
- **Fix:** Bind through `FloatParam::normalized_value`/`set_normalized`, as the wavetable does. Extend `editor_param_binding.rs` to cover these two crates.

### PUX-06 [medium] Exact-value entry and double-click reset exist on only some widget families
- **Where:**
  - Typed entry exists only in `editor_widgets::float_knob` (classic knob).
  - `ThemedKnob` (drums, granular, wavetable: about 60 knobs) and the delay knob have none.
  - The EQ moved to `HSlider` (todo #1335) and lost click-to-type. It passes no `default_unit`, so double-click does nothing (`control_strip.rs:228-235`).
  - Wavetable `float_slider` (`tabs/mod.rs:126-139`) has the same problem.
  - The EQ also restates its ranges (`control_strip.rs:237-242`) with a true log curve, while the params are declared `Skewed`.
- **Fix:** Move typed entry into the widget kit (`Param::apply_typed_entry`), make `default_unit` mandatory in the param bindings, and bind the EQ to `normalized_value()`.

### PUX-07 [medium] Synchronous file dialogs block the Wayland editor thread
- **Where:** `plugins/resonance-ir/src/editor/header.rs:123`, `amp/src/editor/actions.rs:207-214`, `wavetable/src/editor/tabs/osc.rs:224`, `resonance-plugin/src/preset_ui.rs:305-318` (every plugin).
- **What:** `rfd::FileDialog::pick_file()` runs inside `ui()`, so there is no repaint and no Wayland dispatch while the dialog is open. A host destroy during that time costs the engine control thread the full 2 s `DESTROY_JOIN_TIMEOUT`, then leaks the thread (`wayland-plugin-gui/src/editor.rs:29,193-206`). Drums already gets this right with a polled `Picker` (`drums/src/editor/jobs.rs:205-250`).
- **Fix:** Move `Picker` into `resonance-plugin`/`plugin-gui-core` and use it on Linux. Keep the modal call only under Cocoa.

### PUX-08 [medium] Granular-delay strip clips below 1304 px, but the window minimum is 1000 px
- **Where:** `plugins/resonance-granular-delay/src/editor/controls/mod.rs:160-180` (fixed `GROUP_W` sums to 1280 px, plus 24 px of gaps); `factory.rs:70` (`min_size: (1000, 560)`).
- **Impact:** Between 1000 and 1303 px wide, OUTPUT (Mix, Quality) and part of SPACE are cut off. This affects floating window managers and macOS.
- **Fix:** Raise `min_size` to 1310 px, or let the groups wrap. Add a min-size layout test.

### PUX-09 [low] IR load failure or missing file shows only as text in the filename slot
- **Where:** `plugins/resonance-ir/src/loader.rs:165-168` writes `"Error: {e}"` into `ir_name`, which `editor/header.rs:79-101` draws as if it were a filename.
- **Fix:** A WARM/BAD banner with "Locate…", reusing the amp's `missing_banner.rs`.

### PUX-10 [low] Wayland input gaps: no key repeat, keyboard focus loss ignored, `focused` hard-wired to true
- **Where:** `wayland-plugin-gui/src/window_thread/delegates.rs:119-121` (`get_keyboard(.., None)`, so no client-side repeat), `:165-172` (empty `leave`); `paint.rs:206` (`focused: true`); `input.rs:144` (`repeat: false`).
- **Impact:** Holding Backspace or an arrow key does only one step. A key held when focus leaves stays "down", which keeps the slider's key gesture open.
- **Fix:** Use `get_keyboard_with_repeat`. On `leave`, emit key-ups for held keys and `WindowFocused(false)`.

### PUX-11 [low] Two knob families with different look and behaviour
- **Where:**

  | Knob | Look | Used by |
  |---|---|---|
  | Classic `widgets::knob` (`widgets.rs:177-330`) | label above, 10 px caption, 8 px sub-label | amp, color, compressor, delay, gate, IR, mastering, reverb, stereo |
  | `ThemedKnob` | uppercase label below | drums, granular, wavetable |

- **What:** Only the classic family has typed entry, and only the themed family reports gestures.
- **Fix:** Converge on one `ThemedKnob`-based binding in `editor_widgets` that has gestures, typed entry and skew. Add an arch-invariant that no plugin calls `widgets::knob(` directly.

### PUX-12 [low] Small per-frame waste
- **Where:** `wayland-plugin-gui/src/window_thread/paint.rs:157-160,251` reads env vars and clones the title every frame. Editor headers rebuild `Vec<&dyn Param>` every frame. `ThemedKnob` calls `to_uppercase()` per knob per frame (`widgets.rs:648`).

### Carry-overs still open
- FU-M8c: the Cocoa `Editor::new` is still an unbounded `run_on_main_blocking` (`cocoa-plugin-gui/src/editor.rs:79-112`). Cocoa is still not compiled or tested on a Mac (FU-M1a).
- `float_slider` (`editor_widgets.rs:87`) is a raw `egui::Slider` that no first-party editor uses. The mastering Input Trim (`mastering/src/editor/controls/assistant.rs:50-58`) is the last raw `egui::Slider` (hardcoded range, no reset).

### Strengths / no-action
- **Editor lifecycle is solid:** bounded joins plus a watchdog, quit checked before re-entering `ui()`, `catch_unwind` around the editor thread, closed-callback disarmed on destroy. Plugin binaries are never `dlclose`d.
- **Hide unmaps the window:** input is dropped while hidden, held buttons are released on show, and a hidden editor wakes only twice a second. Scale is consistent across buffer, EGL and egui.
- **No RT locks:** `process()` paths take no locks, and meters use atomics or lock-free rings.
- **Drums is the reference editor:** per-gesture announce, gesture close on drop or scroll-away, cached labels, adaptive repaint, a non-blocking picker, and headless layout tests at 1571×856.
- **`HSlider` is correct:** relative drag, quantised params, Shift ratio, arrow-key runs as one gesture, AccessKit. The EQ curve and the color harmonics probe are cached off the paint path.

---

## RT — Engine realtime path

Scope: read-only review of `resonance-audio`. Covered: the callback, seam handling, render pool, track and bus passes, PDC, bypass, monitor, recording, PipeWire in/out, audition, click, meters and retire. The only thing run was a scratch micro-benchmark for RT-03.

### RT-01 [high] Recorded takes ignore PDC and master-chain latency, so every take lands late
- **Where:** `src/engine/thread/mod.rs:661-674`: `start_sample = latched − (capture_latency + playback_latency)`. Recording never reads `latency_comp.max_latency()` or `master_latency_samples`; grep finds `max_latency()` only in bounce, reference and automation code.
- **What:** The performer plays against a mix that leaves the engine `max_latency` (track + bus stages) plus master-chain latency behind the raw playhead (`latency.rs:30`, `master_pass.rs:81-84`). The take is shifted only by device I/O latency.
- **Failure scenario:** With a 2048-sample lookahead limiter on master, or a linear-phase EQ on any track, every overdub lands ~43 ms late. Takes are correct only in projects with no latent plugins.
- **Fix:** Also subtract `latency_comp.load().max_latency() + master_latency_samples` when the latch is applied (performer path only, not the realtime bounce). Latch both values at record start, so a mid-take PDC change can't move the take. Apply the same correction to the per-pass placement in RT-02.
- **Verification:** Hermetic engine test: fake plugin with latency N, capture/playback latency 0, loop back a click. Assert `clip.start_sample == latched − N`; today it is `latched`.

### RT-02 [high] Loop-record passes are cut on the engine tick, not at the loop seam, and later passes get no compensation
- **Where:**
  - `src/engine/transport.rs:586-600` detects the wrap by polling `playhead < last_playhead`.
  - `thread/mod.rs:678-681` drains before the seam poll at :753.
  - `src/recording.rs:549-560` (`roll_audio_pass`) drains everything present and closes the writer.
  - `transport.rs:621-625` places passes ≥1 at `slot.start`.
- **What:** The pass boundary falls wherever the ring happened to be when the 16 ms tick noticed the wrap. Samples captured after the wrap go into the previous take, and the next take starts late but is placed exactly at `loop_in`. Passes ≥1 never subtract I/O latency or PDC.
- **Failure scenario:** Cycle-record 4 passes, then comp them. Each pass is offset by a different 0–20 ms+ plus I/O latency, so comp seams flam or double. `tests/engine/loop_record_takes.rs` pushes exactly loop-length frames per pass synthetically, so drain timing is never exercised.
- **Fix:** Split the capture stream by sample count, not wall clock. The stream start is known from the latch, and every pass after pass 0 is exactly `loop_len` input frames on the same clock. Cut at `frames_written == expected`, carry the remainder into the next writer, and place each pass at `slot.start` minus the RT-01 compensation.
- **Verification:** Push 1.5 loops of a ramp before `poll_loop_record_seam`. Pass 0 must end exactly on the loop boundary, and pass 1 must start at ramp index `loop_len`.

### RT-03 [medium] The live mix meter recomputes integrated LUFS over the whole session history on the audio thread, and is never reset
- **Where:**
  - `src/mixer/callback/master_pass.rs:94` calls `mix.snapshot()` on every playing callback.
  - `src/engine/reference.rs:381-390`: `cached_integrated_lufs()` recomputes on every new block (each 100 ms hop).
  - `resonance-metering/src/lufs/gating.rs:41-80`: two O(n) passes with a `log10` per block. Its own header says it "never runs on the audio thread".
  - `ABMeterTap::reset` (`reference.rs:406`) has no live caller; the same holds for the reference tap (`callback/reference.rs:54`).
- **Failure scenario:** Cost grows with total playback time since app start, up to the 60-minute cap (36,000 blocks). Benchmark of the identical code on this machine:

  | Time played | Cost per recompute |
  |---|---|
  | 10 min | 67 µs |
  | 20 min | 134 µs |
  | 60 min | 404 µs |

  That spike lands every 100 ms, inside a 2.67 ms budget at quantum 128 (1.33 ms at quantum 64). The "integrated" value is also meaningless across seeks; `bounce/measure.rs:183` reports it.
- **Fix:** Use incremental gating with running sums (as the LRA histogram does), or compute on the engine thread from a published block counter. Reset the tap on transport start and on seek.
- **Verification:** A test that `snapshot()` is O(1) in block count, e.g. a counter of `gated_integrated_lufs` calls on the RT path.

### RT-04 [medium] Any latency-affecting edit during playback rebuilds every PDC delay line, so the whole mix drops out or skips
- **Where:** `src/engine/plugins.rs:197-213` publishes a fresh `LatencyComp::new` (`latency.rs:375-401`): empty lines, `next_playhead: None`. `apply_comp` (`latency.rs:313-316`) then warms up with `delay` samples of silence. Triggers: `affects_latency` (`plugins.rs:40-105`), which covers `SetTrackFxBypass`, `SetBusFxBypass`, `SetPluginBypass`, `LoadPluginState` and `AddPlugin`.
- **What:** Lines whose delay didn't change are discarded too. During the 5 ms bypass crossfade (`bypass.rs:395-430`), the latent wet output is also blended against undelayed dry, which combs.
- **Failure scenario:** One track has a 2048-sample plugin, so every other track and bus is delayed 2048 samples. Bypass it while playing and every other track jumps 43 ms ahead. Re-enable it and they all go silent for 43 ms. A preset load that changes latency does the same.
- **Fix:** In `refresh_latency_comp`, carry `DelayState` over for unchanged ids. For changed ids, copy the old line's history so the change becomes a time shift (optionally with a crossfade). Apply the new comp table when the bypass fade completes, not when the target flips.
- **Verification:** Latent stub on track A, sine on track B. Toggle A's bypass mid-play and assert B has no zero run longer than the fade.

### RT-05 [medium] Note-offs are stateless: editing, deleting or retiming a sounding MIDI note hangs the voice
- **Where:** `src/mixer/midi_events.rs:247-260` emits a NoteOff only if `note_abs_end ∈ [playhead, buf_end)`. In-process instruments have no sounding-note state (only `engine/midi/state.rs` tracks hardware outputs). Panics are sent only from transport handlers (`transport.rs:410,454,511`).
- **Failure scenario:** While a pad note sounds, the user shortens, deletes or moves it, deletes or trims its clip, or raises the tempo. No NoteOff is sent, and the note hangs until the next loop seam or Stop. This is very common when editing during playback.
- **Fix:** Track a per-instrument 128-key × 16-channel sounding set on the audio thread, and release any held key that no clip note covers at the playhead. Alternatively, panic the affected instrument whenever an edit touches a note spanning the playhead.
- **Verification:** Render-core test with a recording stub instrument: shorten a sounding note to before the playhead, render, and assert a NoteOff arrives.

### RT-06 [medium] The sidechain key is corrupted on every loop-seam callback
- **Where:**
  - `src/types/sidechain.rs:183-259`: `begin_block` flips the bank once per callback (`play.rs:237`), but `capture_shared` always writes from offset 0 and zero-fills the tail.
  - `mixer/callback/seam.rs:401-447`: the head and tail sub-blocks both capture into the same bank.
  - `mixer/render/context.rs:333` and `clap_host/process.rs:250` read the key from offset 0.
- **What:** In a seam callback, the tail consumer re-reads key samples `0..tail` of the previous block instead of `head..frames`. The next callback's key is the tail capture followed by `head_frames` of zeros.
- **Failure scenario:** A kick-keyed ducker or gate on a looped section double-ducks or misses the duck at every loop pass.
- **Fix:** Pass a sub-block offset through `BlockInputs`, capture at `[offset..offset+frames]`, hand consumers `key[offset..]`, and zero-fill only once per callback.
- **Verification:** A loop whose length is not a multiple of the block size, with a ramp as the key: the consumer must see a contiguous ramp across the seam.

### RT-07 [medium] Live render silently skips any plugin whose mutex is held, and the engine thread takes every plugin mutex every 16 ms
- **Duplicate of HOST-02**, with more detail on the main-thread work done under the lock.
- **Where:**
  - `mixer/render/strategy.rs:232-235` and `context.rs:319` skip a contended FX; `track_pass.rs:653-664` drops a contended instrument's block.
  - `engine/plugins.rs:234-320` (`poll_plugin_host_requests`) runs `on_main_thread`, `service_flush_request`, `take_param_edits` (value_to_text), `refresh_param_values` and `poll_kit_info` under the lock.
  - `handle_set_plugin_param` (`plugins.rs:638-657`) calls `params.flush` under the lock.
  - `cycle_load.rs` has no lock-miss metric.
- **Failure scenario:** An amp sim, or a −20 dB utility that asks for main-thread callbacks, blips to raw DI (+20 dB). A latent compressor flams for a block. None of it shows up in diagnostics.
- **Fix:**
  - Split the main-thread surface from the audio surface (CLAP allows `on_main_thread` to run concurrently with `process`).
  - Queue param changes to the audio thread instead of calling `flush` under the lock.
  - At minimum, count misses and hold the previous wet output rather than passing dry.
- **Verification:** Hold an FX mutex from another thread for one block and assert the output is not the dry input; assert the miss counter increments.

### RT-08 [medium] Count-in to record: the downbeat is delayed by the input-stream rebuild, and monitoring drops at the punch
- **Where:**
  - `src/engine/transport.rs:134-149`: once `count_in_remaining` reaches 0, `poll_precount` (on the 16 ms tick) calls `begin_recording_stream`, then clears `count_in_active`.
  - `transport.rs:244-249`: `state.rec.input_stream = None` drops the monitor stream and builds a new one, which the code's own comment says can take up to 500 ms.
  - `mixer/callback/count_in.rs`: the playhead is pinned and output is silent until then.
- **Failure scenario:** The count-in clicks 1-2-3-4, then there are 20–500 ms of silence before the downbeat. A performer coming in on "1" is early, or their first notes are lost.
- **Fix:** Open or reuse the capture stream, with the recording producer attached, when the count-in starts. The audio thread flips into recording at the exact sample, via a pre-armed atomic.
- **Verification:** The first playing block starts exactly `count_in_total` frames after the count-in starts, with no idle blocks.

### RT-09 [low-medium] The offline-render gate doesn't wait for an in-flight callback
- **Where:** `src/engine/bounce/mod.rs:177-186` (`mark`/`try_acquire_exclusive` only increment), checked at `mix_audio` entry (`callback/mod.rs:65`).
- **What:** A callback that passed the gate just before `mark()` keeps processing live plugins while the worker resets and processes the same instances. The mutex prevents simultaneous calls but not interleaving.
- **Failure scenario:** The first export chunk contains live state (a reverb tail, held voices) or a reset racing a live `process`.
- **Fix:** A callback sequence counter; after raising the gate, wait until it advances.

### RT-10 [low] Master-gain automation is evaluated at the wrong position on seam callbacks
- **Where:** `src/mixer/callback/master_pass.rs:84` uses `tail.playhead + tail.frames` (the pre-seam playhead) even when `tail.seam` is set.
- **Failure scenario:** If master volume is automated differently past `loop_out`, every loop pass gets a one-block gain excursion toward that value.
- **Fix:** When a seam is present, use `loop_in + tail_frames`.

### RT-11 [low] Audition restart race: the audio thread overwrites the new start position
- **Where:** `src/engine/audition.rs:131-150` stores pos, source and playing; `src/mixer/audition.rs:47-93` loads pos at block start and stores it unconditionally at block end.
- **Failure scenario:** Clicking a second sample while one is previewing can start the new one at the old position, or finish it immediately.
- **Fix:** A generation counter, a CAS on `audition_pos_bits`, or a restart-request atomic.

### RT-12 [low] A loop shorter than one block escapes the loop
- **Where:** `src/mixer/callback/seam.rs:353-369,447`: the tail isn't reduced modulo the loop length (`advance_playhead_silent`, `common.rs:135-137`, does reduce it).
- **Fix:** Wrap with `tail % loop_len`, or enforce a minimum loop length of at least the max block size.

### RT-13 [low] The loop range is three independent relaxed atomics and can tear
- **Where:** `src/engine/transport.rs:566-568` (store), `seam.rs:354-358` (load).
- **Failure scenario:** Moving the loop while playing: one block can see the new `loop_in` with the old `loop_out` and wrap to the wrong place, or skip the wrap.
- **Fix:** Pack the range into one seqlocked value, or publish an `ArcSwap<LoopRange>`.

### RT-14 [low] The stopped and count-in branches bypass busses, aux sends and the master FX
- **Where:** `src/mixer/monitor.rs:268-327,401-460` sum straight into the output; `stopped.rs` has no bus or master pass.
- **Failure scenario:** A keyboard played while stopped sounds different from while rolling (no reverb send, bus compressor or master chain). Stopping cuts bus and master reverb tails with a step.
- **Fix:** A reduced render path (sends → busses → master) when stopped, or at least ramp the tails out.

### RT-15 [low] `MonitorResampler` allocates on the cpal input realtime thread
- **Where:** `src/platform.rs:780-791` starts with `Vec::new()`; `process()` pushes on the RT callback (`:957`, `:1009`). This only happens on the cpal fallback with a rate mismatch.
- **Fix:** Pre-size the buffers with `with_capacity`.

### RT-16 [low] The cycle-report seqlock lacks fences and can tear on aarch64
- **Where:** `src/cycle_load.rs:132-155` and `160-185`. It holds on x86 but can tear on aarch64, which is the macOS target.
- **Fix:** `fence(Release)` after the first increment, `fence(Acquire)` before the re-check.

### RT-17 [low] The recording ring is sized in samples, not frames
- **Where:** `src/engine/mod.rs:12` (`96000*2*10`). Only the engine thread drains it, and that thread also runs every blocking command handler.
- **What:** 20 s of stereo, but only ~2.2 s at 18 input channels.
- **Fix:** Size it as `seconds × rate × input_channels`, or drain it on a dedicated writer thread.

### RT-18 [low] The scan path drops the replaced plugin chain without retiring it, despite its comment
- **Where:** `src/engine/scan.rs:55-58`: `drop(track.clear_plugins())` can leave a callback as the last owner, so a free can land on the RT thread. Startup scan only; every other publish site uses `retired.retire(...)`.

### Strengths / no-action
- **Render pool:** the epoch/active protocol is sound (SeqCst on both sides of the close). Panics count as done. Jobs write only their own slots, and the ordered reductions (`track_pass.rs:289-358`, `bus_pass.rs:132-145`) are independent of schedule; no order-dependent float sum was found. LPT scheduling, serial-only handling and the fallback to serial when realtime priority is denied are all good.
- **Non-finite scrubbing:** happens at every plugin output and again at master.
- **Retire queue:** used for the graph, PDC, tempo, frozen-cache and audition publishes; the arc-swap debt makes `strong_count==1` a valid test.
- **Transport:** `commit_playhead` CAS and `TransportContinuity` handle seek races. The seam split handles the aligned `>=` case. PDC and metronome map across the wrap correctly.
- **Monitor and recording rings:** whole-frame discipline, plus the `MonitorDrain` lockout.
- **Bypass:** target and position packed into one atomic, with smoothstep fades.
- **MIDI per-block caps:** allocation-free, with correct note-off priority.
- **RT hygiene:** scratch buffers pre-faulted, arc-swap nodes pre-claimed, no logging on the RT path, and the slot pool grows via an engine-side offer channel.
- **Automation:** gain ramps chain across blocks, and post-PDC automation is evaluated at the comp-delayed position.

---

## HOST — CLAP hosting & plugin framework

Scope: `resonance-audio/src/clap_host/*` and the engine paths that drive it (`engine/plugins.rs`, `mixer/render/*`, `mixer/callback/stopped.rs`, `bypass.rs`, `automation_apply.rs`), plus `resonance-plugin` (clap_bridge, host.rs, state.rs, smoother.rs, param.rs, presets). I only read the code; nothing was built or run.

### HOST-01 [high] Editor edits made while transport is stopped never reach saved state (every first-party plugin except Drums)
- **Where:**
  - `resonance-plugin/src/clap_bridge/state.rs:107-130` (`save_bytes`): while the plugin is active it serialises `TempParamOwned::all_from(&self.shared)`.
  - `clap_bridge/params.rs:59-82` (`get_value`) reads `shared`.
  - The only path that copies plugin values back into `shared` is `push_back_params`, which is called only from `process()` (`clap_bridge/process.rs:357,486`). The audio-processor `flush` (`params.rs:181-234`) does not push back.
  - Editors write the param atomics directly, e.g. `plugins/resonance-eq/src/editor/control_strip.rs:220-223`.
  - Host side: while stopped, `mixer/callback/stopped.rs:12-15` renders nothing. `monitor.rs:346-371` (`chain_wants_idle_process`) covers only `request_process` and live notes.
- **What:** Instances stay active, but while stopped, effects on unmonitored tracks, buses and master get no `process()` call. An edit in the plugin's own editor therefore changes the DSP but not `ClapShared::param_values`. `state.save`, `get_value`, `query_params`, refresh and MCP readback all serve the old value.
- **Failure scenario:** With transport stopped, the user moves an EQ band, saves and closes. On reopen the old value is back: silent data loss. Undo captures and the "modified preset" check store the stale value too. (Same family as PUX-01, which covers the missing announce: PUX-01 is the mirror side, HOST-01 is the plugin's own saved state.)
- **Fix:**
  - (a) Preferred: while active, `save_bytes`, `get_value` and `compare_preset_sound` read live values. Make `param_text_source.live_value` mandatory, or extend it to every param.
  - (b) Alternatively: announce every editor write centrally in the shared widget binding, and have `flush` run `push_back_params()`.
- **Verification:** Bridge test in `resonance-plugin/tests/clap_bridge_params_state.rs`: activate, `set_plain(x)` with no `process()`, `save_state` → the blob and `get_value` hold x. Then an engine-level stopped-transport test.

### HOST-02 [medium] The host-request poll try-locks every instance after every command, so the audio thread drops whole plugin blocks
- **Where:**
  - `resonance-audio/src/engine/plugins.rs:228-325` (`poll_plugin_host_requests`): `try_lock` on each instance, then about 10 operations under the lock (`take_param_edits`, `take_preset_reports` with a second mutex, `kit_info`).
  - It runs unconditionally at `engine/thread/mod.rs:563`, after every command and on every tick.
  - On contention the audio side skips: `mixer/render/strategy.rs:233` → `render/context.rs:318-320` (`continue`) skips an effect; `render/track_pass.rs:647-657` skips an instrument block.
- **Failure scenario:** During a fader drag or an MCP burst, a NAM amp or compressor briefly drops to raw/uncompressed audio for 2.7 ms. On a plugin with latency, the dry signal is also misaligned with PDC. An instrument goes silent for the block. The result is random clicks that are hard to reproduce.
- **Fix:** Check pending flags lock-free first (move the `HostData` atomics, or a pending bitset, into `PluginSlot`; set `has_out_events` and `preset_reports_pending` atomics), and only lock when something is pending. Poll at most once per tick.
- **Verification:** Add a contended-`lock_fx` counter. 1000 `SetTrackVolume` commands with 20 instances should give zero contended blocks.

### HOST-03 [medium] Project, single-instance and preset saves still serialise state while holding the instance lock
- **Where:** `engine/plugins.rs:868-884` (`handle_save_plugin_state`), `:905-921` (preset), `:1150-1160` (`handle_save_all_plugin_states`) all call `save_state()` inside `try_lock`. Compare `capture_unlocked` (`:929-951`, `clap_host/state.rs:35-47`), whose doc names exactly this problem ("a large state — user wavetables — made the audio thread miss blocks").
- **Failure scenario:** Every autosave silences a wavetable that has user tables, or a drum kit, for several blocks. With HOST-02's skip behaviour, effects leak dry audio during the save.
- **Fix:** Route all three handlers through `state_save_handle()`, and add a `preset_save_handle`.
- **Verification:** A fake plugin whose `save` sleeps 50 ms; a concurrent `try_lock` must succeed during save-all.

### HOST-04 [medium] Host crossfade on a plugin's own (latency-preserving) bypass clicks for any plugin with latency
- **Where:** `clap_host/mod.rs:561-575` (`PluginSlot::stage` keeps `Fade` during the transition for own-bypass plugins), `:580-589` (`sync_own_bypass`), `bypass.rs:395-430` (`run_faded` takes the dry copy from the slot input, undelayed).
- **What:** The plugin's bypassed output is the dry signal delayed by its latency, but the host fades toward undelayed dry for 5 ms, then snaps. Output jumps by `latency` samples at both ends of the fade. The comment at :564-566 ("click-free anyway") ignores latency.
- **Failure scenario:** Bypassing a third-party linear-phase EQ or lookahead limiter that flags `CLAP_PARAM_IS_BYPASS` (most commercial CLAPs do) clicks and comb-filters for 5 ms.
- **Fix:** When `bypass_param.is_some()`, return `Wet` for every stage and let the plugin handle its own transition. Or delay the dry copy by the plugin's latency.
- **Verification:** Fake plugin with 256 samples latency and a bypass param: toggle bypass on a sine and assert no discontinuity.

### HOST-05 [medium] `process()` passes plugins audio-port and channel counts that differ from what they declared
- **Where:**
  - `clap_host/process.rs:245-288`: inputs count is 1 unless a key is routed, even for a 2-input sidechain or a 0-input instrument; `channel_count: 2` is hard-coded; outputs are `min(outputs.len(), port_count)`.
  - `engine/transport.rs:497-498`: the panic path calls single-output `process()` on multi-out Drums.
  - `track_pass.rs:633`: capped by `MAX_PLUGIN_OUTPUT_PORTS = 8` (`limits.rs:10`).
  - `bundle.rs:474-483`: unconnected ports keep `data32 = null`, or stale pointers from the previous call.
- **What:** `clap/process.h` requires the buffer count to match `audio_ports->count()` and each buffer's `channel_count` to match its port. Our own bridge copes, but third-party plugins may index every declared port. Stale `data32` pointers on the panic path point into a render-pool worker's `port_scratch`, which may be in concurrent use. A plugin with a port of more than 2 channels reads past the 2-element pointer array.
- **Failure scenario:** A third-party sidechain compressor reads `audio_inputs[1].data32[0]` with no key routed and dereferences null. A 16-out sampler writes through null on ports 8–15. Both crash the app.
- **Fix:**
  - Always pass every declared port with its declared channel count.
  - Back unrouted ports with pre-allocated silent or scratch buffers, sized from `audio_ports.get()` at activation.
  - Null out `audio_out_ptrs[i]` for ports not passed.
- **Verification:** A fake plugin asserting its port and channel counts through `process`, `process_multi_with_key(None)` and `panic_all_instrument_plugins`.

### HOST-06 [medium] Plugin-param automation is block-quantised, with a different quantum live (128) and in bounce (1024)
- **Where:** `mixer/automation_apply.rs:137-148` (one value per block at `eval_start`); `clap_host/params.rs:131-152` (every event has `time: 0`); `engine/bounce/render.rs:22` (`BOUNCE_CHUNK = 1024`); the bridge applies events at block start without smoothing (`clap_bridge/process.rs:134-139`).
- **Failure scenario:** A filter-cutoff sweep zippers audibly in the export but not on playback (2.7 ms steps live vs 21 ms in bounce). This undercuts the ENG-08 "freeze matches live" work, and third-party plugins get no timing inside the block.
- **Fix:** Emit time-stamped `PARAM_VALUE` events every N frames (e.g. 64), or at breakpoints, sorted together with the note events. In the bridge, either split the block at events or feed the smoothers from them. At minimum, bounce with the live cadence for param lanes.
- **Verification:** Null-test the same automated project rendered with 128-frame blocks and through `render_chunk`.

### HOST-07 [low] The framework can't declare `CLAP_PARAM_IS_BYPASS`, so Mastering's latency-preserving bypass is unreachable
- **Where:** `resonance-plugin/src/clap_bridge/params.rs:33-42` (flags are only automatable, read-only and stepped); the `Param` trait has no `is_bypass`; `plugins/resonance-mastering/src/params/mod.rs:145`; host preference at `clap_host/mod.rs:473-478` and `latency.rs:77-82`.
- **What:** Host-bypassing Mastering skips the slot, which drops its lookahead latency and republishes the whole PDC table, resetting every delay line.
- **Fix:** `Param::is_bypass()` → `IS_BYPASS`. Do this only after HOST-04 is fixed.

### HOST-08 [low] `is_main_thread()` is true on every non-audio thread, and unlocked saves can overlap other main-thread calls
- **Where:** `clap_host/thread_check.rs:71-73` (`!is_audio_thread()`); `clap_host/state.rs:40-47` + `engine/plugins.rs:929-951` (unlocked save); bounce thread calls `set_render_mode`/stop/start under the lock (`bounce/render.rs:281-287`); `clap_host/mod.rs:451-454`.
- **What:**
  - A plugin's own loader or GUI thread is told it is the main thread.
  - An unlocked capture save on the engine thread can overlap a bounce thread's `render.set`/`start_processing` on the same instance (unconfirmed in practice).
  - `unsafe impl Sync for SyncClapInstance` is unnecessary.
- **Fix:** Record the engine thread id and compare against it. Make unlocked saves exclusive with bounce main-thread calls. Drop the `Sync` impl.

### HOST-09 [low] The "Logarithmic" smoother snaps the last ~5% of every ramp, and smoothing is off until `set_sample_rate`
- **Where:** `resonance-plugin/src/smoother.rs:79-88,131-142,167-172`; `new()` at `:29-39` leaves `ramp_samples = 0`.
- **What:** The coefficient `1-e^(-3/N)` reaches ~95% when `remaining == 0`, then jumps to target. On a 0→1 gain step that is a −26 dB step. A plugin that never calls `set_sample_rate` gets no smoothing at all, with no warning.
- **Fix:** Use a coefficient that lands within 1e-4 (`-ln(1e-4)/N`), or a true multiplicative ramp. Debug-assert or default `ramp_samples`.

### HOST-10 [low] Bundle resolution ignores the requested path when plugin ids collide
- **Where:** `engine/plugins.rs:1178-1192` (`ensure_bundle` matches by descriptor id first); `engine/scan.rs` (no dedupe).
- **What:** The same `CLAP_ID` in two bundles (dev `target/bundled` plus an installed copy) always instantiates the first one loaded, regardless of `clap_file_path`, and the browser lists both. An empty id re-dlopens on every add (latent).
- **Fix:** Key the lookup by canonical path, then id. Warn about duplicate ids at scan time.

### HOST-11 [low] A param-id rename keeps state but breaks automation lanes and MIDI maps
- **Where:** `resonance-plugin/src/param.rs:144-146` (`clap_id = stable_hash(id)`); `resonance-common/src/automation.rs:56-59` (lanes keyed by u32 `param_id`); `state.rs:38-54` (`ParamRename` migrates only the blob).
- **What:** The advertised rename mechanism changes the CLAP id, so lanes and MIDI-learn bindings on the old id go dead silently. Latent: no plugin uses renames yet.
- **Fix:** `ParamRename` pins the old CLAP id, or the host migrates lane targets through the rename table.

### HOST-12 [low] FFI hardening gaps on plugin-filled structs and caller slices
- **Where:**
  - `clap_host/instance.rs:378-383,408-437,555-559,690-696`: `MaybeUninit::uninit()` + `assume_init`, then an unbounded `CStr::from_ptr` on `name`/`module`.
  - `:321-328`: port names (zeroed, but still unbounded).
  - `process.rs:232,409-411`: no check that buffer and key slices are at least `frames` long.
- **Fix:** Use `zeroed()` everywhere, a bounded name scan (as `param_text` already does), and clamp `frames` with a `debug_assert`.
- **Verification:** Extend `tests/clap_host/clap_ffi_hardening.rs` with a 256-byte non-NUL name.

### HOST-13 [low] No note-port or dialect negotiation; MIDI CC, pitch bend and aftertouch never reach plugins
- **Where:** `clap_host/process.rs:75-92` sends only `NOTE_ON`/`NOTE_OFF` on port 0, channel 0. Nothing in `resonance-audio/src` handles `note_ports`, `CLAP_EVENT_MIDI` or `NOTE_EXPRESSION`. The bridge already declares the MIDI dialect so controllers can arrive (`ports.rs:176`, ba todo #1295).
- **Impact:** A MIDI-dialect-only third-party instrument hears nothing. Mod wheel, pitch bend and aftertouch never reach any instrument.
- **Fix:** Query `clap.note-ports` and use the preferred dialect. Forward live CC, pitch bend and aftertouch as `CLAP_EVENT_MIDI`.

### HOST-14 [low] `set_param` silently drops changes at the 128-entry cap, and `sync_own_bypass` can lose the bypass
- **Where:** `clap_host/instance.rs:766-776`; `clap_host/mod.rs:584-588` swaps `own_bypass_sent` before calling `set_param`.
- **Fix:** Make `set_param` return whether it queued, and update `own_bypass_sent` only on success.

### HOST-15 [low] Spec nits and stale comments
- `clap_host/gui.rs:71-80` calls `set_size` on a floating window; gui.h marks it `[main-thread & !floating]`.
- `engine/bounce/render.rs:24-28` says plugins are activated with `min_frames = 32`; they are activated with 1 (`clap_host/mod.rs:85`).
- `clap_host/bundle.rs:393-403` says instruments default to 0 inputs; the code returns 1.
- `resonance-plugin/src/state.rs:98-102` says it "still stamps the version"; it doesn't.

### HOST-16 [low] (impact unconfirmed) FTZ/DAZ differs between live and offline render threads
- **Where:** `mixer/callback/mod.rs:50` and `render_pool/mod.rs:642,722` set FTZ/DAZ; `engine/bounce/render.rs:378-380` (`render_chunk`) does not. First-party plugins call `flush_denormals()` without restoring MXCSR, which also leaves a foreign host's thread state changed.
- **Fix:** Call `flush_denormals()` at the top of `render_chunk`. In the bridge, save and restore MXCSR around `process()`.

### Strengths / no-action
- **Host callbacks:** they only latch atomics and are serviced at a safe point; restart failures are reported once (ENG-12).
- **Activation:** activate → latency → start with correct rollback; latency is re-read only across a reactivate.
- **`params.flush`:** structurally excluded from overlapping `process()`; `AudioThreadScope` is Hive-safe.
- **Note events:** sorted allocation-free, clamped and carried across seams; a note-off is never the event dropped.
- **Output scrub:** non-finite plugin output is scrubbed at the boundary.
- **State streams:** bounded at 256 MiB and `catch_unwind`-wrapped; kit-info and `value_to_text` are bounded.
- **Bundles:** never `dlclose`d; rescans skip loaded paths.
- **Bridge params and state:** param-id collisions are rejected at load; active and inactive state loads agree; the `params_gen` seqlock closes the load-vs-push-back race; `reconcile_params` is correct (PLG-08).
- **Panics:** clack wraps every entry point in `catch_unwind`, and the saver panic guard is in place (PLG-06).
- **Accepted limit:** there is no in-process plugin sandboxing; this is an architecture decision, not a regression.

---

## DSP — Plugin & shared DSP

Scope: resonance-dsp, resonance-metering, resonance-mastering-assist and every plugin. The drums, wavetable, amp, granular, IR and mastering chain were read directly; two sub-forks covered the mastering stages, the small effects and the metering. Key claims were re-checked numerically with numpy scripts (`bs.py`, `bq.py`, `mag.py`, `fdn.py` in the session scratchpad). **No high-severity finding.**

### DSP2-01 [medium] An IR resampled to the session rate changes the wet level by the rate ratio
- **Where:** `plugins/resonance-ir/src/ir_loader.rs:18-25` → `resonance_common::decode_wav_channels(data, target_sample_rate)` → `loader.rs:135-160`. The resampler normalises every kernel row to unit DC gain (`resonance-common/src/resample.rs`).
- **What:** Unit DC gain is right for audio but wrong for an impulse response. A convolution's gain is the sum of its taps, and that sum scales by `out_rate/in_rate` when the IR is resampled.
- **Audible effect:**

  | IR file | Session | Wet level |
  |---|---|---|
  | 96 kHz | 48 kHz | −6 dB |
  | 192 kHz | 48 kHz | −12 dB |
  | 48 kHz | 96 kHz | +6 dB |
  | 44.1 kHz | 48 kHz | +0.74 dB |

- **Fix:** On the IR path only (not in the shared resampler), multiply the resampled IR by `source_rate / target_rate`.
- **Verification:** Load a 96 kHz impulse at 48 kHz and assert 1 kHz sine RMS matches the native load within 0.1 dB. Repeat 44.1→48 kHz.

### DSP2-02 [medium] Granular delay HQ tier is audibly darker than Normal (B-spline read with no prefilter)
- **Where:** `resonance-dsp/src/interp.rs:60-91` (`bspline6`, `read_bspline6_wrapped`), used by the `InterpQuality::Bspline6` arm in `granular.rs:~588-594`.
- **What:** A quintic B-spline approximates rather than interpolates: at frac=0 it is the FIR [1,26,66,26,1]/120. At 48 kHz:

  | Frequency | Loss |
  |---|---|
  | 5 kHz | −0.9 dB |
  | 10 kHz | −3.8 dB |
  | 16 kHz | −10 dB |
  | 20 kHz | −15 to −17 dB |

  HQ uses sinc only for `|rate| > 1`.
- **Audible effect:** HQ dulls the wet signal compared with Normal (Hermite). On Wet→Buffer and Ping-Pong routes the loss compounds on every recirculation. `tests/quality.rs` checks only alias energy, not passband.
- **Fix:** Pick one: an interpolating 6-point kernel (Lagrange or optimal-2x Hermite), the B-spline prefilter, or `BandlimitedReader` at all rates.
- **Verification:** A 10 kHz sine through the reader loses <0.5 dB. HQ vs Normal on white noise at pitch 0 differ by <1 dB in the 8–16 kHz band.

### DSP2-03 [medium] Wavetable: retriggering or stealing a sounding voice clicks
- **Where:** `plugins/resonance-wavetable/src/dsp/voice.rs:268-321` (`Voice::trigger`: `clear_filters()`, unison phase reset to 0, sub/noise reset), while `amp_env.trigger()` keeps the current level (`envelope.rs:68-71`). Callers: `engine.rs:301-345`, `537-579`.
- **What:** An audible voice restarts with its oscillators jumped to phase 0 and its filter state zeroed, but at non-zero amplitude, with no fade of the outgoing voice. Drums has `steal_to_tail` for exactly this case.
- **Audible effect:**
  - Mono: a non-legato note during the previous note's release tail clicks (staccato bass and leads).
  - Poly at the voice ceiling: every steal clicks.
  - Worst with a resonant low-pass.

  `tests/voice_allocation.rs` checks which voice is chosen, not whether the output stays continuous.
- **Fix:** Pick one: on retrigger of a non-idle voice keep phases and filter state and restart only the envelopes; or copy the voice into a 3–5 ms fade tail; or at least a 2 ms ramp to zero before reset.
- **Verification:** Mono saw, release 300 ms, retrigger 50 ms after note-off: bound the maximum sample-to-sample delta. Same test in poly with `max_voices=2` and 3 held notes.

### DSP2-04 [medium] Track compressor detects on the mono sum, so panned, wide and out-of-phase material is under-compressed
- **Where:** `plugins/resonance-compressor/src/dsp.rs:297-305`: self and key detectors both use `0.5*(l+r)`.
- **What:** The mastering glue compressor had the same bug, already fixed there by using the louder channel (`glue_compressor.rs:149-157`). The EQ's dynamic bands also use the louder channel.
- **Audible effect:** A hard-panned source is detected 6 dB low and gets ~6 dB less gain reduction. Side-heavy or out-of-phase content barely compresses, and bus compression changes with the stereo image.
- **Fix:** Detect on `max(|l|,|r|)` after a per-channel sidechain high-pass. Same for the key input.
- **Verification:** −6 dBFS sine, threshold −20, ratio 4: L-only and R=−L give the same gain reduction as centred, within 0.5 dB.

### DSP2-05 [medium] Mastering multiband: enable/disable steps the level and misroutes bands while the filters refill
- **Where:** `plugins/resonance-mastering/src/stages/multiband/mod.rs:311-366` `process_chunk`. When not splitting it returns the delay at once; `just_split` resets xo1–3; band gains and compressors apply from chunk one.
- **What / audible effect:**
  - Disabling: band gains and gain reduction vanish at a block edge.
  - Enabling: for about 85 ms (one FIR length) y1..y3 are near zero, so band 3 (`xd − y3`) carries the whole mix through band 3's gain and compressor.
  - Result: steps of several dB, then 85 ms of the whole mix treated as the top band. `tests/stages_multiband.rs:43` only covers unity gains with the compressors off.
- **Fix:** A 10 ms wet/dry enable crossfade against the delayed dry signal, as the imager and clipper do. Hold gains and compressors at unity for one FIR length after a reset.
- **Verification:** 1 kHz sine, band 3 at +6 dB, toggle at a block boundary: bound the maximum sample-to-sample delta; RMS over the first 85 ms is within 0.5 dB of steady state.

### DSP2-06 [medium] Bounce-report LRA covers only the first 6 minutes of an export
- **Where:** `resonance-metering/src/lra.rs:36`: `BLOCK_CAP = 60*60` ("60 minutes of 1 s blocks"). `lra.rs:96-100` silently drops blocks past the cap, gated silence included. `resonance-audio/src/engine/bounce/measure.rs:570-590` pushes one value every 0.1 s, so the cap is reached at 360 s.
- **Impact:** The loudness range is wrong for exports longer than 6 minutes, both in delivery reports and in agent mastering decisions. The `dropped` counter is never surfaced.
- **Fix:** Remove the cap (the histogram is constant memory), or size it for about 24 h at 10 Hz.
- **Verification:** Push 7,200 values: the first 360 at −30 LUFS, the rest at −10. Expect ~20 LU; it reports ~0 today.

### DSP2-07 [medium-low] `Biquad::magnitude` in f32 is wrong at low frequencies and high sample rates, which skews the mastering FIR designs
- **Where:** `resonance-dsp/src/biquad.rs` `magnitude()`: `den_re = 1 + a1*c1 + a2*c2` cancels in f32. The linear-phase EQ and crossover FIRs are designed from it, bin by bin (`linear_phase_eq/design.rs`).
- **What:** RBJ high-pass at 20 Hz, Q 0.707:

  | Sample rate | At | f32 result | Exact |
  |---|---|---|---|
  | 192 kHz | 20 Hz | −4.6 dB | −3.0 dB |
  | 192 kHz | 5 Hz | −16.3 dB | −23.9 dB |
  | 48 kHz | 5 Hz | −25.5 dB | −23.9 dB |

  At 96 and 192 kHz the error is up to ~2 dB near the corner and ~7 dB in the stopband. Editor curves are affected too. It is small at the pinned 48 kHz.
- **Fix:** Evaluate in f64, or use the sin²(w/2) form.
- **Verification:** Compare against an f64 reference from 5 Hz to Nyquist at 48–192 kHz; error must stay below 0.05 dB.

### DSP2-08 [medium-low] Mastering band gains and glue makeup/mix change in block-sized steps; enabling glue jumps by its makeup
- **Where:** `stages/multiband/mod.rs` `sum_bands` (per-block `band_gain`). `stages/glue_compressor.rs` `process_stereo`: unsmoothed `makeup_lin`/`mix`, and the enable edge applies full makeup from sample 0 while gain reduction builds over the attack time.
- **Audible effect:** Automating these zippers at block rate (375 Hz at 128 frames). Enabling glue with +4 dB makeup gives an instant +4 dB step, then a dip during the attack. The other stages use 10 ms smoothers.
- **Fix:** `Smoother`+`retarget` (linear, 10 ms) for these values. Start an enable smoother at 1/makeup, or crossfade dry→wet.
- **Verification:** Sweep a band gain 0→6 dB over 20 blocks on a 50 Hz sine and bound the per-sample step. Enable glue mid-stream and bound the maximum sample-to-sample delta.

### DSP2-09 [low-medium] Drums: tuning a pad up aliases (4-point Hermite, no anti-alias filter, up to +24 st)
- **Where:** `plugins/resonance-drums/src/dsp/sampler.rs:~1720-1770` (the `!unity` branch; `hermite()` at :2061). `MAX_TUNE_ST = 24` (`params.rs:491`).
- **Audible effect:** Content above `fs/(2·rate)` folds back: at 48 kHz, above 6 kHz at +24 st and above 12 kHz at +12 st. Hats and cymbals tuned up get inharmonic grit. Spec E8 only tests the FFT peak.
- **Fix:** For `rate > 1`, read through `resonance_dsp::interp::BandlimitedReader` (rate-tracked windowed sinc), or use decimated copies of each take.
- **Verification:** White-noise take at +12 st: the 12–24 kHz band is ≥40 dB below the 0–12 kHz band.

### DSP2-10 [low-medium] Delay plugin: Hi Cut, Lo Cut and Drive only act on the feedback path
- **Where:** `plugins/resonance-delay/src/dsp.rs:153-198`. `process` returns the raw taps; the filtered and saturated signal only feeds recirculation.
- **Audible effect:** The first echo is unfiltered and carries full bass despite the default 120 Hz Lo Cut. At feedback 0 these three controls do nothing.
- **Fix:** Take the wet signal after the filters and drive (re-bless the delay goldens).

### DSP2-11 [low-medium] Switches that cut over with no crossfade
- **Where:**
  - EQ band enable: `plugins/resonance-eq/src/dsp.rs` `update_from_params`. The crossfade fires only on a kind or routing change; `band.rs:268-273` passes through instantly.
  - Stereo widen mode: `plugins/resonance-stereo/src/dsp.rs:528-545`, which resets.
  - Mastering imager `side_hpf_on`, saturator mode (`restart_state`), de-harsh mode (resets its detector).
- **Audible effect:** Clicks when these are toggled or automated during playback. Worst case: bypassing a +12 dB bell or a 48 dB/oct cut.
- **Fix:** Route these through the existing 5–20 ms crossfade machinery.

### DSP2-12 [low] Wavetable envelope times don't match their labels
- **Where:** `plugins/resonance-wavetable/src/dsp/envelope.rs:96-130`, `exp_coeff` :139.
- **What:**
  - The parameter is a one-pole τ with an overshoot target. Attack reaches its peak at 1.47τ.
  - Decay and release run to within 1e-4, about 6.8τ.
  - Curve changes duration as well as shape (`1+0.8·curve`); the comment says the top is 5.0.
- **Audible effect:** "Attack 100 ms" takes 147 ms. "Release 10 s" holds the voice for ~68 s, which eats polyphony and triggers DSP2-03 steals. Sustain is block-snapshotted, so automating it steps the level.
- **Fix:** Derive the coefficients so the label is time-to-target (release = time to −60 dB: `τ = t/ln(1000)`). Re-bless presets and goldens.

### DSP2-13 [low] Wavetable unison sub-voices start phase-locked by default
- **Where:** `voice.rs:315-320` resets all phases; `osc_phase_random` defaults to 0 (`params/analog.rs:27-31`); the sum is scaled by 1/√N (`render/kernel.rs:522`).
- **Audible effect:** Each onset is a coherent peak about √N above steady state (+9 dB at 8 voices), followed by a slow phasey sweep.
- **Fix:** A fixed per-index phase spread, keeping sub-voice 0 at phase 0 so mono patches stay bit-identical.

### DSP2-14 [low] Amp model input is −6 dB for a one-sided signal (unconfirmed design intent)
- **Where:** `plugins/resonance-amp/src/dsp/processor.rs:~196,:228` feeds `0.5*(l+r)`.
- **What:** The tuner accepts either input (DSP-11), but a DI on only one side is halved. NAM captures are level-sensitive, so this changes drive and tone, not just volume.
- **Fix:** Use the louder channel or an L/R/sum input choice, or document that a centred source is expected.

### DSP2-15 [low] Rare audio-thread work: inline FIR redesign, and SwapFader drops
- **Where / what:**
  - `stages/linear_phase_eq/worker.rs`: `StereoFir::process`/`land_pending` design inline when the worker result is late: ~4097 bins × bands × 2 `sin_cos` plus 8192-point FFTs, scaling with sample rate. Known as FU-M2a; cost not measured.
  - `resonance-dsp/src/swap_fader.rs`: `park()` drops the outgoing object on the audio thread when the janitor queue and all 4 slots are full; `retire()` drops it when no janitor is set.
- **Fix:** Defer landing by one more hop instead of designing inline. Never drop in SwapFader: keep the object or refuse the swap.

### DSP2-16 [low] Smaller items
- **Correlation meter sticks on NaN:** one Inf/NaN sample poisons the running sums until reset (`resonance-metering/src/correlation.rs`). Treat non-finite input as 0.
- **Saturator mix not phase-aligned in the sub band:** the wet path alone runs the 5 Hz DC blocker, Tape's head bump and Transformer's 18 Hz high-pass (`stages/sat_modes.rs`), so mix 0.5 dips slightly in the sub band.
- **Wavetable control rate is 16 samples, restarting each block:** fast LFOs on amp or cutoff zipper slightly, and output depends a little on block size (bounce vs live are not bit-identical).
- **Wavetable `Box::leak(format!(...))`** for param ids and names leaks per instance (`params/env.rs:25-29`).

### Strengths / no-action (verified)
- **Drums streaming:**
  - Lock-free on the audio thread; ring claims are swept every `end_block`.
  - Reads are bounded by `published()`, and the read position is published every 512 frames.
  - Offline waits only happen offline. Kit swaps park old kits and free them on the janitor.
  - Steal and choke fades are equal-power and sample-rate-scaled. Per-pad gain and pan ramp per sample.
  - Events are split sample-accurately. Silent velocity layers are handled. `reset()` re-seeds the RNGs.
- **Wavetable:** mip selection is alias-free; the 6-point Lagrange is correct term by term; polyBLEP/BLAMP signs are correct; the ZDF filters are bounded; the distortion oversampler mixes dry and wet inside the oversampled loop; voice ceiling and mono legato are correct.
- **Amp/NAM:** fixed block chunking; the janitor frees swapped models; smoothers and the DC blocker are set in Hz.
- **Granular:** feedback is chunked to 128 samples; tanh, a DC blocker and a feedback clamp; freeze is an equal-power crossfade; the 10 ms minimum delay exceeds the chunk size.
- **Mastering:**
  - Limiter lookahead equals reported latency, and the bypass delay is exact.
  - FIR latency `(len−1)/2 + hop` matches the real delay, and the subtraction crossover reconstructs exactly.
  - TPDF dither and noise shaping are correct; the ADAA is continuous; de-harsh latency is exactly one frame.
  - No allocation in process; every `reset()` clears state.
- **IR:** latency equals the block size, and the dry path is aligned.
- **Shared DSP:** RBJ biquads, Ballistics, K-weighting (BS.1770), the polyphase oversampler and the ~90 dB Kaiser resampler are correct. Every plugin calls `flush_denormals`. Hardcoded 44.1 and 48 kHz values are only constructor defaults.
- **Small effects:** the gate is sound; reverb FDN RT60 is within ~13% of the setting; delay feedback is capped at 0.95; the stereo decorrelator is mono-compatible.

---

## STATE — App state, undo, persistence & control API

Scope: the app↔engine boundary, undo, persistence and the control surface. That covers resonance-app's `state/update/undo/project/engine_events/control_*`, plus resonance-control, resonance-mcp and the agent plugin. I also read the diffs of b109a46c, 0984dfc6 and 22c8a37b. Every finding comes from reading the code: nothing was built or run.

### STATE2-01 [high] `presets.*` treats `plugin_id` as a raw path, so a "read-only" search can delete and rewrite files outside the preset root
- **Where:**
  - `resonance-plugin/src/presets/library.rs:296-302`: `plugin_dir = root.join(plugin_id)` and `trash_dir = root.join(".trash").join(plugin_id)`.
  - `library.rs:1019`: `ensure_fresh` → `migrate::convert_legacy_dir`.
  - `library.rs:1029`: spawns `purge_trash_dir`.
  - `library.rs:1057-1068`: `purge_trash_dir` deletes any file whose name prefix before the first `-` parses as a `u64` "older than 30 days".
  - Callers: `resonance-app/src/update/control/presets.rs:155` (`search`) and `:45` (`resolve`, used by set_marks / update_meta / rename / delete).
  - The MCP tool is annotated `read_only_hint = true` (`resonance-mcp/src/tools/presets.rs:81`).
- **What:** `plugin_id` is a free-form string from the wire that is never validated against the catalog. With `PathBuf::join`, an absolute path *replaces* the root, and `../..` escapes it.
  - The first index of the target folder then:
    - deletes expired `*.legacy` files;
    - renames every unparsable `*.json` to `.corrupt`;
    - converts any JSON object with a `params` key into a preset envelope;
    - starts the trash purge on the same folder. That purge deletes `01-Intro.flac`, `2024-08-invoice.pdf` and the like, because `"01"` → 1 and `"2024"` → 2024 both count as "old".
  - The id is also kept in the `plugins` map, so later searches with no plugin id re-scan the folder.
- **Failure scenario:** An agent (hallucinating, prompt-injected, or passing a path where an id belongs) calls `presets_search {"plugin_id": "/home/jorrit/Music/SomeAlbum"}`. Clients may auto-approve it because it is marked read-only. Every `NN-Title.flac` in that folder is unlinked on a background thread. The same holds for set_marks, rename and delete with a crafted `preset_id`.
  - (unconfirmed) There may be a second route: `chain_ui.rs:409` opens a bank for a slot's `clap_plugin_id`, which comes from the project file, so a crafted shared project could trigger it.
- **Fix:**
  - In `plugin_dir`/`trash_dir`, reject any id that is not exactly one `Component::Normal` (or map it through `sanitize_filename`).
  - In the control handlers, refuse an id that is not in `available_plugins` and has no factory bank.
  - Make `purge_trash_dir` match its own naming strictly (`<10+ digit stamp>-<name>.json`).
- **Verification:** Library test: `records("/tmp/x")` and `records("../x")` leave `01-a.flac` / `bad.json` fixtures outside the root untouched. Control test: `presets.search {plugin_id:"/abs"}` returns `not_found` and changes nothing on disk.

### STATE2-02 [high] A failed clip copy or transcode during save blocks every later save/open/new for the rest of the session
- **Where:**
  - `resonance-audio/src/engine/clips.rs:944-952` (no project dir), `:997-1010` (`create_dir_all`/`fs::copy` error) and `:1013-1019` (transcode error) all `return` without sending `ClipsSavedToProjectDir`; they only send `AudioEvent::Error`.
  - App side: `engine_events/transport.rs:25-28` only sets a banner. `io.save_state` is cleared only in `try_finish_save` (`engine_events/project_io.rs:61-65`).
- **What:** The `SaveCollector` waits forever for `clips_done`. After that:
  - every manual save just sets `manual_save_queued`;
  - autosave backs off forever;
  - control `project.save/save_as/open/new` answer `busy(...)` forever.
- **Failure scenario:** The disk fills during Save, which is a real risk here because worktree `target/` dirs fill it. The user frees space and presses Save: nothing happens, ever. Quitting loses the session, and the agent can't save or switch projects either.
- **Fix:** The engine always answers `SaveClipsToProjectDir`, e.g. `ClipsSavedToProjectDir { clip_files, error }` or a separate `ClipsSaveFailed`. On error the app:
  - takes `save_state` and clears `saving`;
  - fails the `ProjectSave` token;
  - shows a banner;
  - calls `start_queued_save`.

  A tick-driven watchdog on `save_state` age adds a safety net.
- **Verification:** Hermetic test: `begin_save`, inject `AudioEvent::Error` with no completion event, then assert the next `project.save` is not `busy`. Engine test: a copy failure still emits the completion event.

### STATE2-03 [medium] A host-side drum-kit change (MCP `track_set_plugin_param kit_select`) records an undo entry that restores nothing
- **Where:**
  - `update/plugin.rs:129-170`: the `SetPluginParam` arm never refreshes the state blob. The `ParamEditedByPlugin` arm (`:172-214`) does send `SavePluginState` when `state_excluded` (`:211`).
  - `update/project_io/serialize.rs`: the `host_persisted()` filter keeps `kit_select` out of the params.
  - `reconcile/plugin_state.rs:76` skips an `Arc::ptr_eq` blob; `:245` skips non-host-persisted params.
  - `kit_select` is `.excluded_from_state()` (`plugins/resonance-drums/src/params.rs:256`).
  - This path is the one the agent is told to use: `resonance-mcp/src/tools/trackmix.rs:76-78,363` and `skills/drumming/SKILL.md:34`.
- **What:** On undo:
  - the snapshot blob is pointer-equal to the live cache, so it is not pushed;
  - the params phase skips `kit_select`.

  So the kit stays, but `edit_undo` reports `undone: "plugin parameter"`. drums-plugin-rework.md §5.1 promises host-undoable kit changes, and only the editor path delivers that. No test covers host-side undo of a kit change.
- **Failure scenario:** The agent loads a kit, the user presses Ctrl+Z, the label says it was undone, and the new kit keeps playing.
- **Fix:** (a) After a host write to a `state_excluded` param, refresh the blob once the plugin has applied the write (e.g. defer `SavePluginState` until the next values rescan reports the new value). A plain `SavePluginState` straight after the send can race the audio-thread param queue. Or (b) carry state-excluded params in the *undo* snapshot only (not in the saved file), and have the undo reconcile re-drive them.
- **Verification:** Control test: `set_plugin_param kit_select=4` → `edit.undo`; the engine sees the pre-edit blob or `kit_select=<old>`.

### STATE2-04 [medium] Agent edits on an untitled project can't be undone, though the MCP instructions promise they can
- **Where:**
  - `undo/snapshot.rs:321-323`: recording requires a `project_path`.
  - `project.new` creates an untitled project (`instantiate.rs:72`).
  - `resonance-mcp/src/server.rs:35,42` promise that "every edit … lands in the app's undo history".
  - `update/control/edit.rs:70-74`: undo on an empty history succeeds with `undone: null`.
- **Failure scenario:** The agent runs `project_new`, makes a destructive multi-track edit, then calls `edit_undo` and gets `undone: null` with no explanation.
- **Fix:** Anchor untitled snapshots to `autosave_scratch_dir`. At minimum: add `recording_disabled: "project never saved"` to `EditStatus`, mention it in the tool descriptions and INSTRUCTIONS, and have the skills save right after `project_new`. Same root cause as UX-03.
- **Verification:** `project.new` → `track.add` → `edit.undo` removes the track, or `edit.status` reports the reason.

### STATE2-05 [medium] A control mutation that lands during a GUI drag gesture is undone along with the drag
- **Where:** `update/control/mod.rs:241-243`: the mutation gate never checks `session.undo.has_pending()`. Only `edit.undo/redo` refuse mid-gesture (`edit.rs:49-60`).
- **What:** While a drag is pending with its pre-drag snapshot, an agent call records its own entry. When the drag commits, it pushes the pre-drag snapshot, which predates the agent edit. `gesture_changed_since` also sees a difference, so even a click that moved nothing commits. Recording takes already split gestures correctly (`record_recording_edit`, FU-A2a); control edits don't.
- **Failure scenario:** The agent adds notes while the user clicks a clip; the user's next Ctrl+Z ("move clip") silently removes the agent's notes.
- **Fix:** Answer `busy("an edit gesture is in progress")` while `has_pending()`, or apply `split_pending`/`resume_split_gesture` in `record_undo`.
- **Verification:** Begin a clip drag → `notes.insert` → end the gesture without moving → one undo must not remove the notes.

### STATE2-06 [low] Every gated control call, read-only polls included, breaks the user's coalesced undo run
- **Where:** `update/control/mod.rs:241-243` wraps every non-read-only method (including the gated read-only ones: `master.summary`, `edit.status`, `meter.*`, `pool.list`, `automation.lanes`, …) in `with_compound_undo`. `undo/history.rs:288-291`: `begin_compound` clears `coalesce_key` unconditionally.
- **Impact:** An agent polling meters while the user rides a fader splits one gesture into many undo entries.
- **Fix:** Break the coalesce run lazily, only when the compound actually records. Or keep read-only gated methods out of the wrapper.
- **Verification:** Coalesced `SetPluginParam` ×2, then `execute(master.summary)`, then another `SetPluginParam` should give exactly one undo entry.

### STATE2-07 [low] (unconfirmed) A gap between an editor kit pick and its blob refresh
- **Where:** `update/plugin.rs:172-214` → `SavePluginState`; `engine_events/project_io.rs:22-33`.
- **What:** If another undoable edit is recorded before `PluginStateSaved` returns, its snapshot holds the old kit blob. Undoing that unrelated edit then reverts the kit too.
- **Fix:** Mark the instance "blob owed" and attach a `LateBlob` (as `pending_after` already does for preset loads).

### STATE2-08 [low] Library control calls do synchronous disk I/O on the update loop
- **Where:** `update/control/drum_kits.rs:61-73` and `amp_models.rs:62-71` (rescan, plus hashing new `.nam` files); `presets.rs:265-271` (`vocabulary` opens every plugin's preset dir).
- **Fix:** Rescan off-thread and answer from the last index, as `plugins.rescan` already does.

### STATE2-09 [low] b109a46c ("mcp fix") has no regression test
- **Where:** `resonance-mcp/src/server.rs:292-375` (`inline_defs`); `tests/schema_hygiene.rs` doesn't pin the absence of `$ref`/`$defs`.
- **What:** The inlining logic looks correct, but a future schemars change, or a recursive param type, would silently bring back the "42"-as-string id failure in the Claude desktop bridge.
- **Fix:** Add a schema-hygiene test: no published input schema contains `$ref` (with an allowlist), and id newtypes inline to `"type":"integer"`.

### STATE2-10 [low] Smaller items
- **`render.mixdown`** (`update/control/render.rs:93-126`) writes to any absolute path once `overwrite: true` is set, with no `.wav`/`.flac` extension check (`~/.bashrc` is accepted). Require an audio extension, as `project.save` requires `.rproj`.
- **Save-as over another existing `.rproj`** (with confirm) leaves that project's `backups/` behind. They then show up as restore points, and they seed the clip-WAV GC keep-set (`project/clip_gc.rs` `collect_dir_json_ids`).
- **A GUI `SetPluginParam` on a read-only param** returns early (`update/plugin.rs:132-141`) *after* `record_undo` has recorded an entry, marked the project dirty and bumped the revision.
- **`recent.rs:127-131`** keeps an inline `#[cfg(test)]` module, justified by "resonance-app is a binary crate". It now has a `lib.rs`, and the module breaks the no-inline-tests rule.

### Strengths / no-action
- **Control socket:**
  - 16 MiB frame cap, and parse errors don't kill the connection.
  - Per-connection reader and writer threads, so the update loop never blocks on I/O; `job.wait` is served on the reader thread.
  - 0700 socket dir with lstat checks on both ends.
  - flock instance lock with race-free stale-socket replacement.
- **MCP client:** read and write deadlines; the connection is dropped on transport errors; `job_wait` is sliced; error text gives the agent clear next steps.
- **Control dispatch:** one mutation gate; one undo entry and one revision bump per call; `needs_confirmation` on destructive ops; absolute-path and parent checks.
- **Persistence:**
  - `atomic_write` uses a unique temp file, `create_new`, fsync and rename.
  - Autosave writes separate sidecars.
  - Newer-format guard on load.
  - New fields carry `#[serde(default)]`.
- **Drums merge:** the host-persisted filter is applied the same way on serialize and reconcile.
- **Lockstep:** `agent_plugin_lockstep.rs` covers tool names, the protocol version, wire fields and skill param keys.
