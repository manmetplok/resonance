# Aux sends and return busses

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: design doc #172 and ba doc #273 (todos #475–#478, #1229, #1269, #1310); no file in `docs/history/specs/`.

An aux send taps a track's signal into a return bus — typically a shared
reverb or delay — at its own level, before or after the track's fader,
without changing where the track's main output goes. A track can have
several sends. A user manages them in the track inspector's SENDS group;
an agent uses `track.add_send` / `track.set_send` / `track.remove_send`.

## Behaviour

- **The send.** `AuxSend { id, source, dest, level_db, pre_fader, enabled }`.
  The source is `SendSource::Track` or `SendSource::Bus`; the destination is
  always a bus.
- **Tap point.** Post-fader (default) follows the source's fader, pan and
  mute ramp. Pre-fader takes the post-plugin signal at the send level only,
  ignoring the fader. A silenced (muted / solo-suppressed) track stops
  feeding all its sends once its one-block fade ends, pre-fader ones too.
- **Level.** The engine clamps to −120…+24 dB and turns a non-finite level
  into 0 dB (`engine/busses.rs::clamp_send_level`). The GUI slider spans
  −60…+6 dB; a value set outside that over the control API shows verbatim
  in the readout while the handle parks at the end.
- **Disabled sends** keep routing and level, pass nothing, and still count
  in the cycle check.
- **Cycles.** Only bus → bus sends can form a loop. The engine refuses a
  send that would close one (`types::aux_send_would_cycle`) and reports
  `AuxSendRejected`; the SENDS group shows the reason under the picker.
- **Sub-tracks.** A multi-output instrument's sub-tracks can have sends;
  they follow the same rules, with the parent's group trim in the ramp
  (field report 2026-10-06 §3).
- **Render.** Track sends are added into the destination's summing buffer
  during the track pass, so a track → return send is sample-correct
  whatever the bus order. Bus-sourced sends are *pulled* by the
  destination bus from lower-indexed busses only (`pull_bus_aux_sends`).
- **Ids.** Send ids are app-allocated (`AuxSendState::allocate_send_id`,
  ARCH-04 D-2); the engine refuses to reuse a live id.
- **Mirror.** The app never edits the send list directly: every
  `MixerMessage` becomes an engine command, and the `AuxSendChanged` /
  `AuxSendRemoved` / `BusRoleChanged` echoes update `AuxSendState`. An edit
  re-sends the whole send built from the mirror (`upsert_send`); several
  fields changed at once must go as one `MixerMessage::SetSend`. The one
  non-echo writer is project-load replay, which seeds the saved graph.
- **Endpoint removal.** The engine does not prune sends when a track or bus
  is removed, so the app drops them from the mirror
  (`drop_sends_touching_track` / `_bus`) **and** sends `RemoveAuxSend` for
  each.
- **GUI flow.** SENDS lists one slot per send (destination picker, level
  slider with dB readout, PRE/POST, enable, remove). `+ Add send` offers
  every return bus the track does not already feed, then "New FX return…",
  which creates a bus named `FX Return N`, flags it a return and adds the
  send in one gesture (`CreateReturnFromSend`). Re-route offers return
  busses only (plus the current destination if it is a plain bus).
- **Persistence and undo.** `ProjectFile::sends` (`ProjectSend`) holds the
  graph at project scope, with the source as a `("track"|"bus", id)` tag;
  an unknown tag drops the send rather than guessing. Saved ids are reused
  on load. Each send edit is one undo step; a level drag coalesces
  (`CoalesceKey::SendLevel`). The graph rides the `ProjectFile` undo
  snapshot since ba todo #1269.
- **No per-send meters** (cancelled todo #481): a send row shows routing,
  not level.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Model + cycle check | `resonance-audio/src/types/aux_send.rs` | `AuxSend`, `SendSource`, `aux_send_would_cycle` |
| Engine handlers | `resonance-audio/src/engine/busses.rs` | `handle_add_aux_send`, `handle_set_aux_send`, `handle_remove_aux_send`, `publish_aux_sends` |
| Render | `resonance-audio/src/mixer/render/routing.rs` | `apply_track_aux_sends`, `pull_bus_aux_sends` |
| App mirror | `resonance-app/src/state/aux_sends.rs` | `AuxSendState`, `AuxSendRejection` |
| App update | `resonance-app/src/update/mixer.rs` | `MixerMessage`, `upsert_send`, `next_return_bus_name` |
| Echo handling | `resonance-app/src/engine_events/aux_sends.rs` | — |
| Persistence | `resonance-app/src/project/model.rs`, `update/project_io/serialize.rs` | `ProjectSend`, `send_source_tag`, `send_source_from_tag` |
| Control | `resonance-app/src/update/control/track/sends.rs` | `track.add_send` / `set_send` / `remove_send` |
| View | `resonance-app/src/view/mixer/inspector/sends.rs`, `state/picks.rs` | `sends_group`, `add_send_choices`, `send_dest_choices` |

Invariant: the engine is the only writer of live sends; the mirror shows
only what the engine accepted.

## Surfaces

- **GUI:** track inspector → SENDS; bus inspector → ROUTING ("Sends in",
  `AUX RETURN` toggle).
- **Control API / MCP:** `track.add_send` → `mcp__resonance__track_add_send`
  (flags the destination as a return, returns `send_id`),
  `track.set_send` → `mcp__resonance__track_set_send`,
  `track.remove_send` → `mcp__resonance__track_remove_send`. Sends are
  reported in `song.tracks`.
- **Agent skills:** `spatial` (reverb/delay returns for depth).

## How it's tested

| What | Where | Run |
|---|---|---|
| Model, cycle check | `resonance-audio/tests/types/aux_send_model.rs`, `resonance-audio/tests/engine/{aux_send_cycle,aux_send_id_duplicate_rejected}.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Render (pre/post, sub-tracks) | `resonance-audio/tests/mixer/{aux_send_render,sub_track_aux_sends}.rs` | same |
| Handlers, mirror, persistence | `resonance-app/tests/mixer/{aux_send_handlers,aux_send_mirror,aux_send_persistence}.rs` | `./scripts/run-tests.py -p resonance-app` |
| SENDS group | `resonance-app/tests/mixer/{mixer_inspector_sends,mixer_inspector_sends_snapshot}.rs` | same |
| Control | `resonance-app/tests/control/control_sends.rs` | same |
| Goldens | `mixer_inspector_sends_populated-wgpu.png`, `mixer_inspector_send_rejected-wgpu.png` | `RESONANCE_BLESS=1 cargo test -p resonance-app --test mixer` |

Not covered: bus-sourced sends through any app surface (none exists).

## Decisions and deviations

- Sends live at project scope, not on the source track, because a send is
  an edge between two entities.
- The add/re-route pickers offer return busses only; a plain bus is
  reached through the output picker. The control API will still point a
  send at a plain bus, and the GUI shows it.
- Per-send metering was deliberately dropped.

## Known gaps

- Bus-sourced sends exist in the engine and the file format, but neither
  the GUI nor the control API can create one —
  [`../../intents/mixer/bus-control-parity.md`](../../intents/mixer/bus-control-parity.md).
- A bus → bus send into a lower-indexed bus is accepted and inaudible —
  [`../../intents/mixer/bus-to-bus-routing.md`](../../intents/mixer/bus-to-bus-routing.md).
