# The `Resonance` app state

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/refactor/a-12-resonance-fields.md` (field survey),
> `docs/history/refactor/arch-migration-plan.md` → "A-12a" … "A-12g landed",
> `docs/history/refactor/refactor-intent.md` (Epic A).

`Resonance` (in `resonance-app/src/lib.rs`) is the one struct that holds the
whole app: the engine handle, the mirror of everything the engine plays, the
project's song structure, UI state and session bookkeeping. iced owns it and
calls `update` and `view` on it. This doc is the map of what lives where.

## Behaviour

- **40 fields**, most of them domain sub-states defined in
  `resonance-app/src/state/<domain>.rs` (re-exported from `state.rs`). The
  ≤ 40 target was Epic A's done-when; it was reached by A-12a…g.
- **Fields are `pub(crate)`**; integration tests reach them through the
  `test_*` accessors in `test_support/` (see
  [hermetic-test-app.md](hermetic-test-app.md)). Only `engine` and
  `sample_rate` are `pub`.
- **The app mirrors the engine.** The engine never writes app state; the app
  rebuilds its mirror from `AudioEvent`s (see [update-loop.md](update-loop.md)).
  Many edits update the mirror optimistically and let the echo confirm.

### Field map

| Group | Fields |
|---|---|
| Engine link | `engine` (`AudioEngine`), `sample_rate` |
| Song structure (persisted) | `clips`, `midi_clips`, `tempo_events`, `signature_events`, `tempo_map`, `chord_track`, `markers`, `compose`, `automation`, `quantize`, `take_groups`, `freeze`, `reference`, `performance` |
| Entities and routing (persisted) | `registry` (`TrackRegistry`: tracks, busses), `track_groups`, `aux`, `sidechain`, `master` (`MasterState`: volume, levels, plugins, bypass) |
| Plugins | `plugin_catalog` (scan results), `plugin_mirror` (`state_cache` blobs, index, owed blobs, kit info), `missing_plugins`, `presets` (`PresetState`) |
| Media | `media` (`MediaState`: pool, pool import, import progress, browser, drag placement, relink, `ids` = the clip/asset allocators) |
| Devices | `devices` (`DeviceState`: audio inputs, MIDI ports, device-definition registry, external instruments, MIDI map) |
| Project I/O | `io` (`ProjectIoState`: path, save collector, pending load, open token, untitled anchor, autosave timing, reconcile trace, restore echoes, …) |
| Session | `session` (`SessionMetaState`: `dirty`, `revision`, `session_id`, `undo`) |
| Control endpoint | `control` (`ControlEndpointState`: server, per-connection sessions, job board), `control_pending_note_echoes` |
| UI | `ui` (`UiTransientState`: view mode, caches, transport labels, mixer view, interaction), `viewport`, `transport` (also the playhead mirror), `modals`, `banners`, `midi_quantize` |
| Other | `settings` (`AppSettings`), `table_registry` (music-theory tables), `update_depth` (re-entrancy depth of `update`) |

### Construction

`Resonance::new()` opens the real engine (with the render-thread count from
settings), asks it for device lists and a plugin scan, builds the state in
`assemble`, sends the startup default track ("Track 1", id 1) and the first
clip-id grant, and offers a crashed untitled session if one exists. Engine
start failure shows a native dialog and exits; there is no headless mode.
`new_with_control()` also starts the control socket. Test constructors are in
[hermetic-test-app.md](hermetic-test-app.md).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Struct + constructors | `resonance-app/src/lib.rs` | `Resonance`, `new`, `assemble`, `new_with_control`, `revision`, `bump_revision`, `session_id`, `autosave_settings` |
| Sub-state types | `resonance-app/src/state/*.rs` | `DeviceState`, `MediaState`, `UiTransientState`, `SessionMetaState`, `ProjectIoState`, `PluginMirror`, `MasterState`, `ModalState`, `PresetState`, `Banners`, `PluginCatalog`, `TrackRegistry`, … |
| `impl Resonance` beside its data | `state/{arrange,arrange_layout,picks,plugin_index,transport_labels,ui_caches,ids}.rs` | domain helpers, allocators |
| Compose state | `resonance-app/src/compose/` | `ComposeState` |
| Settings | `resonance-app/src/settings.rs` | `AppSettings`, `load`, `persist` |

Invariants:
- `state/` names nothing from `crate::view`, `crate::update` or
  `crate::control_socket` (`arch-invariants::state_module_imports_nothing_from_view_update_or_the_socket`).
- The view layer never reads the engine
  (`arch-invariants::view_layer_never_reads_the_engine`); `view()` is
  read-only, and derived labels are refreshed at the end of `update`.
- Read the revision through `Resonance::revision()` and change it only
  through `bump_revision()`.

## Surfaces

- None of its own. Control reads go through `update/control/view_model`,
  which reads this state.

## How it's tested

| What | Where | Run |
|---|---|---|
| Hermetic construction | `resonance-app/tests/hermetic_construction.rs` | `./scripts/run-tests.py -p resonance-app` |
| Layering of `state/` | `tools/arch-invariants/tests/architecture.rs` | `cargo test -p arch-invariants` |

Not covered: nothing pins the field count; 40 is a design target, not a test.

## Decisions and deviations

- `control` stays a top-level field instead of joining `SessionMetaState`:
  it is a transport endpoint, not edit bookkeeping, and would read as
  `r.session.control.sessions`.
- `SessionMetaState` has no `Default`, because `UndoHistory`'s derived
  `Default` has capacity 0, not 200.
- The A-12 survey's `SongStructureState` (`clips`, `midi_clips`,
  `tempo_events`, `signature_events`, `tempo_map`, `chord_track`, `markers`)
  was deferred until A-13 finished and has not been done.

## Known gaps

- Song-structure fields still sit loose on `Resonance` (the deferred
  `SongStructureState` grouping). No intent written: it is a tidy-up with no
  user-visible effect.
