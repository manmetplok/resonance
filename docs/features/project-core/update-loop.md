# Messages, update loop and engine-event dispatch

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/refactor/arch-migration-plan.md` (ARCH-06 A6-1,
> A6-4; A-10, A-11), `docs/history/reviews/code-review-2026-10-02.md`
> (ARCH2-12). Layering rules: [`ARCHITECTURE.md`](../../../ARCHITECTURE.md)
> → "Update-Handler Pattern" and "Audio Engine Public API".

The app follows iced's Elm architecture: every input — a click, a key, a
timer tick, a finished async task, a control-socket request — becomes a
`Message`, and `Resonance::update` is the only place state changes. Engine
output does not arrive as messages of its own: the periodic `Tick` drains
the engine's event channel and routes each `AudioEvent` to a per-domain
handler.

## Behaviour

### Messages

- `Message` has one variant per domain (`Compose`, `Transport`, `Track`,
  `Clip`, `Plugin`, `ProjectIo`, `Control`, …; 33 domain variants) plus
  `Tick`, `Undo`, `Redo` and `WindowCloseRequested`.
- Each domain enum (`TrackMessage`, `ClipMessage`, …) is declared beside its
  handler in `update/<domain>.rs` and re-exported from `message.rs`
  (`Plugin` and `Ui` still live in `message.rs`). Each also defines an
  exhaustive `undo_action()` (see [undo-redo.md](undo-redo.md)).
- Each handler module exports `handle(r: &mut Resonance, m) -> Task<Message>`.

### One `update` call

`update` → `update_inner` → `dispatch`, in this order:

1. **Gates** (`gates_message`): drop the message if no project is open and it
   is interactive input; if a bounce, stem export or freeze is running and it
   would touch what they render; or if it is a refused plugin move, take
   edit, import confirm, no-op bus rename or a write to a read-only param.
2. **Undo / Redo** are handled directly and return.
3. **Frozen-track gate**: an edit aimed at a frozen track's inputs is dropped
   and the freeze is marked stale.
4. **`record_undo`**: classify, snapshot if needed, mark dirty, bump revision.
5. **`dispatch`**: route to the domain handler.
6. After dispatch: revalidate chords if the meter changed, commit an open
   undo gesture, settle frozen-track params, revalidate frozen content.
7. Outermost call only: settle chain/strip UI and inline renames, grant the
   timeline the keyboard if the delete target changed, refresh the cached
   transport labels (so `view()` never writes state).

Handlers may call `update` re-entrantly (`update_depth` tracks the depth);
control handlers do this on purpose through `run_via_update` so a control
edit takes exactly the path a GUI edit takes.

### Subscriptions

- `Tick` every 16 ms while anything animates, plays, records or a job runs;
  every 200 ms when idle (`tick_interval`).
- Keyboard events go to the command registry (`update/shortcuts.rs`).
- Window close requests, MIDI file drag-and-drop, window size, and the
  control socket's bridge stream (`Message::Control`).

### The tick

`handle_tick` drains every pending `AudioEvent` through
`engine_events::handle_engine_event`, then expires control labels, decays
meters, syncs tempo at the playhead, follows the playhead, refreshes MIDI
devices, polls preset lists, flushes preset-step state, polls engine health
and CPU load, runs the save watchdog and the autosave trigger, and flushes
the command-palette recents.

### Engine-event dispatch

- `route_engine_event` is one exhaustive `match` over `AudioEvent` with no
  `_` arm, classifying each variant into a per-domain router (transport,
  render, control replies, clips, MIDI, plugins, tracks, pool, project I/O,
  …), which hands it to `engine_events/<domain>.rs`.
- After every event, `resume_split_gesture` re-opens a drag that a recording
  split (see undo).
- Event handlers mirror engine state into the app. They are free functions,
  not `impl Resonance` blocks. Only the project I/O handlers return a `Task`
  that starts async work (the save write, the post-`ClearAll` replay).
- Engine-created clips arrive with bare names and are numbered here
  (`number_engine_clip_name`).

### Control requests

The control socket's reader threads parse JSON-RPC and send each request
into the update loop as `Message::Control(ControlMessage::Request)`. The
handler (`update/control/mod.rs::execute`) answers read-only `song.*` views
directly and runs every mutating call inside one compound undo group. Control
handlers never send engine commands themselves; they dispatch domain messages
(`arch-invariants::control_handlers_never_send_engine_commands`).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Message enum | `resonance-app/src/message.rs` | `Message`, `PluginMessage`, `UiMessage`, `RecoveryChoice`, `SwitchChoice` |
| Domain enums + handlers | `resonance-app/src/update/<domain>.rs` | `XMessage`, `handle`, `undo_action` |
| Orchestrator | `resonance-app/src/update.rs` | `update`, `update_inner`, `dispatch`, `subscription`, `TICK_INTERVAL_MS` |
| Gates | `resonance-app/src/update/gates.rs` | `gates_message`, `is_gated_message`, `bounce_blocks_message`, `freeze_blocks_message` |
| Tick | `resonance-app/src/update/tick.rs` | `handle_tick`, `tick_interval`, `TICK_INTERVAL_IDLE_MS` |
| Engine events | `resonance-app/src/engine_events/{mod.rs,dispatch.rs,<domain>.rs}` | `handle_engine_event`, `route_engine_event`, `number_engine_clip_name` |
| Control bridge | `resonance-app/src/control_socket.rs`, `update/control/mod.rs` | `ControlMessage`, `bridge_stream`, `execute`, `run_via_update` |
| Commands / keys | `resonance-app/src/commands/`, `update/shortcuts.rs` | `CommandId`, `key_event_message` |

Invariants: a new `AudioEvent` variant must be routed in
`route_engine_event` (it will not compile otherwise); a new message variant
must decide its `undo_action`; every `AudioCommand` needs an app caller
(`arch-invariants::every_audio_command_has_an_app_caller`); the view layer
never reads the engine.

## Surfaces

- Internal architecture; no user surface of its own.

## How it's tested

| What | Where | Run |
|---|---|---|
| Tick rate gating | `resonance-app/tests/mixer/tick_gating.rs` | `./scripts/run-tests.py -p resonance-app` |
| Startup / offline-render / control gates | `tests/io/offline_render_gate.rs`, `tests/control/control_mutation_gate.rs`, `control_mutation_gate_loading.rs` | same |
| Event mirrors | e.g. `tests/io/engine_events_pool_mirror.rs`, `engine_events_plugin_move_mirror.rs` | same |
| Structural rules | `tools/arch-invariants/tests/architecture.rs` | `cargo test -p arch-invariants` |

Most app tests drive this loop directly (`app.update(Message::…)`, or
`test_apply_engine_event` for engine events) on a hermetic app.

## Decisions and deviations

- `engine_events` and `project_io` used to be large `impl Resonance` blocks;
  both were split into per-domain free functions (ARCHITECTURE.md records
  this).
- The per-domain routers end in an `unreachable!` arm; the exhaustive match
  above them makes it unreachable in fact.

## Known gaps

- `PluginMessage` and `UiMessage` are still declared in `message.rs` rather
  than beside their handlers (left for last in A-11 because of churn).
