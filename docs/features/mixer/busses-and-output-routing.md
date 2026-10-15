# Busses and track output routing

> Verified against master @ `acac5ea3` on 2026-10-15.
> Spec history: none as a spec; control-API side came from ba doc #273 (todos #1221, #1226–#1228, #1237, #1238).

A bus is a summing point between tracks and the master with its own fader,
pan, mute and insert chain. Each track's post-fader output goes either
straight to the master or into one bus; each bus sums into the master. A
user adds a bus with `+ Bus` and picks it in a track's ROUTING output
picker; an agent uses `bus.create` and `track.set_output`. A bus can also
be flagged as an aux return, the destination of sends (see
[aux-sends.md](aux-sends.md)).

## Behaviour

- **Output choices.** A track (or sub-track) routes to `Master` or to any
  bus (`TrackOutput::{Master, Bus}`). The picker lists Master plus every
  bus, plain or return (`state::picks::output_choices_for`).
- **A bus always sums into the master.** There is no bus → bus output
  route; a bus's ROUTING group shows "Output · Master" as a fixed fact.
  Bus → bus signal flow exists only as an aux send.
- **Ordering.** Busses render level by level: a bus fed by sends from
  other busses renders after them (realtime-multithreading.md §4.5). Into
  the master, busses are summed in index order by one serial reduction, so
  output is bit-identical at any render-thread count.
- **Fallback.** A track routed to a bus that no longer exists (or is past
  `active_busses`) falls back to the master rather than going silent
  (`render::routing::route_post_fader`). Freeze capture forces the master
  route so bus FX are not baked into the freeze cache.
- **Delete.** Removing a bus re-routes every track that fed it to the
  master and unpublishes the bus in one render-graph edit, so no block sees
  a feeder pointing at a missing bus. The app also drops every send that
  started or ended on the bus (and tells the engine), every key route
  sourced from it, its automation lanes and its selection.
- **Limits.** At most `MAX_BUSSES` (32) busses; `AddBus` past that is
  refused with an engine `busy` error.
- **Ids.** Bus ids are app-allocated from `BUS_ID_BASE` (2 000 000 000) so
  they never collide with a live track id in `song.summary` / `song.tracks`,
  which list tracks and busses in one id-addressed sequence
  (`kind: "bus"`). The engine refuses a duplicate id instead of replacing
  the live bus.
- **Return role.** `BusState::is_return` marks a bus as an aux return. It
  changes no summing; it decides which busses the SENDS pickers offer. Set
  from the bus inspector's `AUX RETURN` toggle, by "New FX return…" in a
  track's SENDS picker, and saved as `ProjectBus::is_return`.
- **Bus inspector.** CHAIN (effects), ROUTING (members routed in, sends
  landing here, output Master, `AUX RETURN`), AUTOMATION, BUS
  (**Delete bus**). Bus strips have mute but no solo.
- **Multi-output instruments.** Sub-tracks honour `TrackOutput::Bus`. The
  supported way to process a drum kit's taps as a group is to route them to
  a bus and insert there; inserts on the parent track only touch port 0
  (ba doc #274 / todo #1233, option (b)).
- **Persistence and undo.** `ProjectBus` stores id, name, order, volume,
  pan, mute, chain bypass, plugins and `is_return`; `ProjectTrack::output_bus`
  stores the route. Bus adds, removes, renames, mute, chain edits and
  return-role changes are one undo step each; volume and pan drags
  coalesce.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Engine types | `resonance-audio/src/types/mod.rs`, `types/track.rs` | `TrackOutput`, `Bus`, `BusRuntime` |
| Engine handlers | `resonance-audio/src/engine/busses.rs` | `handle_add_bus`, `handle_remove_bus`, `handle_set_track_output`, `handle_set_bus_role` |
| Render | `resonance-audio/src/mixer/render/{bus_pass,routing}.rs` | `render_bus_pass`, `assign_levels`, `route_post_fader` |
| App state | `resonance-app/src/state/tracks.rs` | `BusState`, `TrackRegistry::allocate_bus_id`, `with_bus_mut` |
| Id ranges | `resonance-app/src/state/ids.rs` | `BUS_ID_BASE` |
| App update | `resonance-app/src/update/bus.rs`, `update/track.rs` | `BusMessage`, `TrackMessage::SetTrackOutput` |
| Removal mirror | `resonance-app/src/engine_events/tracks.rs` | `bus_added`, `bus_removed` |
| Persistence | `resonance-app/src/project/model.rs`, `update/project_io/reconcile/routing.rs` | `ProjectBus`, `ProjectTrack::output_bus` |
| Control | `resonance-app/src/update/control/bus.rs`, `update/control/track/output.rs` | `bus.*`, `track.set_output` |
| View | `resonance-app/src/view/mixer/{bus_strip.rs,inspector/bus.rs,inspector/io.rs}` | `view_bus_strip`, `routing_group`, `output_block` |

## Surfaces

- **GUI:** Mix tab bottom lane; `+ Bus`; track inspector ROUTING output
  picker; bus inspector.
- **Control API / MCP:**
  `bus.create` → `mcp__resonance__bus_create` (returns `bus_id` at once),
  `bus.delete` → `mcp__resonance__bus_delete` (`confirm: true` required),
  `bus.rename` → `mcp__resonance__bus_rename`,
  `bus.set_volume` → `mcp__resonance__bus_set_volume`,
  `track.set_output` → `mcp__resonance__track_set_output`.
  Bus chain methods are in [plugin-chains.md](plugin-chains.md); busses are
  listed by `song.summary` / `song.tracks`.
- **Agent skills:** `mixing` (bus creation and routing), `spatial`.

## How it's tested

| What | Where | Run |
|---|---|---|
| Engine bus graph, duplicate ids | `resonance-audio/tests/engine/{render_graph_busses,bus_id_duplicate_rejected}.rs` | `./scripts/run-tests.py -p resonance-audio` |
| First-block gain, stems through a bus | `resonance-audio/tests/mixer/{bus_first_block_gain,bus_first_block_loop_seam,stem_bus_sub_track_render}.rs` | same |
| Bus inspector | `resonance-app/tests/mixer/{mixer_inspector_bus,mixer_inspector_bus_snapshot}.rs` | `./scripts/run-tests.py -p resonance-app` |
| Control | `resonance-app/tests/control/{control_bus,control_bus_create_commit,control_song_routing}.rs` | same |
| Id allocation, undo shape | `resonance-app/tests/io/{id_allocation,undo_diff_shape,reconcile_order}.rs` | same |
| Goldens | `mixer_inspector_bus-wgpu.png`, `mixer_inspector_bus_members-wgpu.png`, `mixer_rename_bus_strip-wgpu.png` | `RESONANCE_BLESS=1 cargo test -p resonance-app --test mixer` |

Not covered: the name a GUI `+ Bus` gives a new bus (see Known gaps).

## Decisions and deviations

- Busses and tracks share one wire id space on purpose; the 2e9 base is
  an app/control convention, not an engine range (ARCH-04 D-3).
- Return role is a flag, not a different kind of bus.
- Multi-out group processing goes through a bus rather than an implicit
  per-instrument stage, because PDC has exactly two compensation stages
  (track and bus) (doc #274).

## Known gaps

- No bus → bus output route (bus → bus aux sends do work, in either
  creation order) —
  [`../../intents/mixer/bus-to-bus-routing.md`](../../intents/mixer/bus-to-bus-routing.md).
- No bus pan / mute / solo over the control API, no bus solo at all —
  [`../../intents/mixer/bus-control-parity.md`](../../intents/mixer/bus-control-parity.md).
