# A-12 survey — the remaining `Resonance` fields

Written at the end of A-12 batch 4 (`MasterState`), the last batch the
plan's original 8-group table named (arch-migration-plan.md → "ARCH-06"
A6-2/A6-3). `Resonance` is **64 → 60 fields** after this batch. The
"done when" bar in `refactor-intent.md` is ≤ 40, so this survey lists
every field left, proposes a home for each, and proposes the next
groupings needed to close the remaining ~20-field gap — which the plan
itself flagged as needing "more groups beyond the plan's 8" (A-12c's
progress note).

**Headline finding:** batches 1–4 already extracted every *loose
primitive* group the plan identified. Nearly everything left is
**already a single field holding its own dedicated sub-state type**
(`aux: AuxSendState`, `sidechain: SidechainState`, `freeze: FreezeState`,
…) — the shape A6-2/A6-3 was aiming for in the first place. Reaching
≤ 40 from here means a *second tier* of grouping: bundling several
already-independent sub-states under one umbrella struct
(`r.browser` → `r.media.browser`), which is a materially different,
more invasive move than A-12a–d's "pull loose fields into a new
struct" — every consumer of the folded field gains one more level of
indirection, not just a renamed access. The groupings below are chosen
to keep that cost low (cohesive domains, few or no concurrent-work
files), but the last one (`SessionMetaState`) is flagged as
high-blast-radius and worth a dedicated round rather than doing it
alongside anything else.

## Full field list (60, post batch-4)

Legend: **Home** = `stays` (leave alone, with reason) / `existing
group` name / `NEW: <ProposedGroup>` (this survey's proposal).

| Field | Type | Home |
|---|---|---|
| `engine` | `AudioEngine` | stays — the live engine handle, not app state |
| `sample_rate` | `u32` | stays — engine config read everywhere (248 raw hits); grouping saves 1 field for a lot of indirection, low priority |
| `input_devices` | `state::InputDevices` | NEW: `DeviceState` |
| `midi_devices` | `state::MidiDevices` | NEW: `DeviceState` |
| `plugin_catalog` | `state::PluginCatalog` | stays (A-12a) — see optional `PluginRuntimeState` note below |
| `missing_plugins` | `state::MissingPluginState` | stays — see optional `PluginRuntimeState` note below |
| `view_caches` | `view::ui_caches::UiViewCaches` | NEW: `UiTransientState` |
| `transport_labels` | `view::transport_labels::TransportLabels` | NEW: `UiTransientState` |
| `banners` | `state::Banners` | stays (A-12a) — folding further re-touches every banner consumer for little gain |
| `master` | `state::MasterState` | done (this batch) |
| `view_mode` | `ViewMode` | NEW: `UiTransientState` |
| `pre_performance_view` | `Option<ViewMode>` | NEW: `UiTransientState` (alt: merge into `performance`, see below) |
| `performance` | `state::PerformanceState` | stays |
| `clips` | `Vec<ClipState>` | stays — core project data; a `SongStructureState` fold is HIGH CONFLICT with A-13/A-13d's in-flight Reconcile work, defer until that lands |
| `midi_clips` | `Vec<MidiClipState>` | stays — same reason |
| `control_pending_note_echoes` | `state::PendingNoteEchoes` | stays — small, tightly scoped to `notes.*` read-your-writes |
| `groove_library` | `Vec<resonance_audio::quantize::GrooveTemplate>` | **flag: dead/legacy**, see below |
| `compose` | `compose::ComposeState` | stays |
| `automation` | `state::AutomationState` | stays |
| `quantize` | `state::QuantizeState` | stays |
| `pool` | `state::MediaPool` | NEW: `MediaState` |
| `pool_import` | `state::PendingImports` | NEW: `MediaState` |
| `import_progress` | `state::ImportProgressTracker` | NEW: `MediaState` |
| `import_progress_modal_open` | `bool` | NEW: `MediaState` |
| `browser` | `state::BrowserState` | NEW: `MediaState` |
| `drag_placement` | `Option<state::DragPlacement>` | NEW: `MediaState` |
| `relink` | `state::RelinkState` | NEW: `MediaState` |
| `reference` | `reference::ReferenceState` | stays |
| `table_registry` | `TableRegistry` | stays — read-only startup registry, 8 refs |
| `tempo_events` | `Vec<state::TempoEvent>` | stays — deferred `SongStructureState` candidate, HIGH CONFLICT with A-13/A-13d |
| `signature_events` | `Vec<state::SignatureEvent>` | stays — same |
| `tempo_map` | `TempoMap` | stays — same |
| `chord_track` | `chord_track::ChordTrack` | stays — same |
| `midi_map` | `MidiMapState` | NEW: `DeviceState` |
| `transport` | `TransportState` | stays |
| `viewport` | `ArrangeViewport` | stays — already self-contained, low priority to fold |
| `markers` | `state::ArrangementMarkers` | stays — part of the deferred `SongStructureState` candidate |
| `last_arrangement_shift` | `Option<ShiftOutcome>` | NEW: `UiTransientState` |
| `update_depth` | `u32` | NEW: `UiTransientState` |
| `interaction` | `ClipInteractionState` | NEW: `UiTransientState` |
| `midi_quantize` | `state::MidiQuantizePanelState` | stays |
| `io` | `ProjectIoState` | stays — FU-A7a is actively editing `io.restoring_undo`; don't touch until that lands |
| `mixer` | `MixerUiState` | NEW: `UiTransientState` |
| `registry` | `TrackRegistry` | stays — already the tracks+busses grouping |
| `track_groups` | `state::TrackGroupRegistry` | stays |
| `aux` | `state::AuxSendState` | stays — D-2/D-3 is actively touching send/bus ids; don't touch concurrently |
| `sidechain` | `state::SidechainState` | stays |
| `take_groups` | `state::TakeGroupState` | stays |
| `undo` | `UndoHistory` | NEW: `SessionMetaState` |
| `external_instruments` | `crate::state::ExternalInstrumentMap` | NEW: `DeviceState` |
| `device_registry` | `resonance_common::DeviceDefinitionRegistry` | NEW: `DeviceState` |
| `freeze` | `crate::state::FreezeState` | stays — distinct bounce/freeze domain |
| `dirty` | `bool` | NEW: `SessionMetaState` |
| `revision` | `u64` | NEW: `SessionMetaState` (see risk note — highest blast radius of any field here) |
| `control` | `crate::state::ControlEndpointState` | NEW: `SessionMetaState` |
| `modals` | `state::ModalState` | stays (A-12b) |
| `plugin_mirror` | `state::PluginMirror` | stays (A-12c) |
| `settings` | `settings::AppSettings` | stays — persisted app config, distinct from project/session state |
| `session_id` | `String` | NEW: `SessionMetaState` |
| `presets` | `state::PresetState` | stays (A-12b) |

## Flagged: dead/duplicated field

**`Resonance::groove_library: Vec<GrooveTemplate>` looks dead.** There
are *two* groove libraries:

- `Resonance::quantize.groove_library: Vec<UserGroove>` (named,
  persisted in `ProjectFile`, exposed via `QuantizeState::next_groove_id`
  / `user_groove`) — this is the one the apply-groove UI actually reads
  (`view/editor_panel.rs:128`).
- `Resonance::groove_library: Vec<GrooveTemplate>` (unnamed, top-level)
  — only ever **pushed to**
  (`engine_events/midi.rs:465`, in the same handler that pushes to the
  real list) and read back by one test-only accessor
  (`test_support/vocal_compose.rs::test_groove_library`). The push-site
  comment says outright: *"The legacy unnamed list is kept in sync for
  any older readers."* No non-test reader exists.

Recommend a follow-up todo to delete `Resonance::groove_library` and
`test_groove_library` together (check nothing external depends on the
test hook first) rather than folding it into a group — it isn't state,
it's a leftover mirror.

## Proposed next groupings (4, ≥20-field reduction, reaches ≤ 40)

Reference counts are `grep -c '\.<field>\b'` over `resonance-app/src`
(a proxy — includes non-`Resonance` receivers of the same field name,
so treat as an order-of-magnitude estimate, not exact).

### 1. `MediaState` — 7 fields → 1, **−6**, ~177 refs

`pool` (43), `pool_import` (7), `import_progress` (7),
`import_progress_modal_open` (6), `browser` (76), `drag_placement` (8),
`relink` (30). All doc #175's media-import/browse pipeline; several
doc comments already say so explicitly ("same rule as collapse state",
"the twin of `RelinkState`"). Most affected: `update/browser.rs`,
`update/relink.rs`, `view/browser/{files_tab,pool_tab}.rs`,
`view/relink_dialog.rs`, `demo.rs`, `test_support/pool_media.rs`.
**Conflict: low** — none of these files are named in the concurrent
A-13d / D-2-D-3 / FU-A7a work.

### 2. `DeviceState` — 5 fields → 1, **−4**, ~137 refs

`input_devices` (23), `midi_devices` (39), `device_registry` (15),
`external_instruments` (51), `midi_map` (9). The hardware/external-device
domain — device lists, the device-definition registry, per-track
external-instrument config, and the MIDI-learn binding mirror. Most
affected: `update/external_instrument.rs`, `update/control/external.rs`,
`view/mixer/inspector/mod.rs`, `update/ui.rs`, `engine_events/midi_map.rs`.
**Conflict: low-moderate** — `external_instruments` is read during
project replay (`update/project_io/replay/entity.rs`), which is in
A-13d's neighbourhood but not the clip-reconcile path A-13d actually
touches; worth a `git diff` check against A-13d's branch before
landing, same as A6-2's own pre-landing check.

### 3. `UiTransientState` — 8 fields → 1, **−7**, ~332 refs

`view_mode` (25), `pre_performance_view` (5), `view_caches` (57),
`transport_labels` (4), `mixer` (70), `interaction` (165),
`last_arrangement_shift` (3), `update_depth` (3). Everything here is
pure view-layer/session UI bookkeeping — never persisted, never in the
undo snapshot. `interaction` dominates the ref count (165) and is the
main cost driver; consider landing it as two todos (the four small
fields first, `mixer` + `interaction` second) rather than one 332-ref
commit. Most affected: `update/clips.rs`, `update/ui.rs`,
`view/mixer/inspector/mod.rs`, `update/midi_editor.rs`,
`update/marker_ui.rs`. **Conflict: low.** Alternative for
`pre_performance_view` alone: merge it into the existing `performance`
sub-state instead (it is literally "the view before Performance mode")
— zero new struct, same field-count win, and it reads better next to
`PerformanceState`'s other fields.

### 4. `SessionMetaState` — 5 fields → 1, **−4**, ~203 refs — do LAST, alone

`dirty` (19), `revision` (57), `session_id` (9), `undo` (66), `control`
(52). Conceptually the cleanest group (session-level bookkeeping: is
the project modified, what's the edit counter, the undo stack, the
control-socket handle) but **`revision` is bumped from nearly every
mutating handler in the app** — its 57 dot-refs are spread across
`undo/mod.rs`, every `update/control/*.rs` handler, and
`update/project_io/*`. A mechanical `r.revision` → `r.session.revision`
sed touches far more files than the other three groups combined and
will collide with *any* concurrently in-flight todo that adds a new
handler. Recommend: land this only in a dedicated batch with nothing
else running against `update/`, or consider giving `Resonance` a
`bump_revision()` method first so future handlers stop touching the
field directly — that would cap the blast radius of this group
permanently, not just for this rename.

**Net effect:** 60 − 6 − 4 − 7 − 4 = **39 fields**, at or under the
≤ 40 done-when bar. `SongStructureState` (`clips`, `midi_clips`,
`tempo_events`, `signature_events`, `tempo_map`, `chord_track`,
`markers`) is deliberately left out of this set — it's the single
biggest remaining cluster by field count, but every one of its fields
is exactly what A-13/A-13d's Reconcile-trait migration is mid-flight
on; folding it now would conflict with that work on almost every
touched line. Take it up only after A-13 finishes.
