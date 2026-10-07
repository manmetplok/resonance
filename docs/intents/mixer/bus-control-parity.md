# Bus channel parity (pan, mute, solo, bus-sourced sends)

> Status: **idea**
> Area: mixer
> Source: gaps found while documenting the mixer (2026-10-07); ba doc #273 scoped busses to create/delete/rename/volume/chain/sidechain only

## Why

A bus is a channel like a track, but several of its controls only half
exist:

- Over the control API a bus has `bus.set_volume` but no pan or mute,
  although the app has `BusMessage::SetBusPan` / `ToggleBusMute` and the
  GUI strip has both. An agent can balance a drum bus but not pan or mute
  it.
- Busses have no solo state anywhere (engine, app, file). "Solo the drum
  bus" means soloing each member track.
- The engine and project format support sends *from* a bus
  (`SendSource::Bus`, `ProjectSend.source_kind = "bus"`), but neither the
  GUI (SENDS is a track-only group) nor the control API (`track.add_send`
  takes a `track_id`) can create one.

## What it would do

- `bus.set_pan`, `bus.set_mute` control methods (+ MCP tools), same shape
  as `mixer.set_pan` / `mixer.set_mute`.
- A bus solo with defined semantics against track solo.
- A SENDS group in the bus inspector and a bus-sourced send over the
  control API (`bus.add_send` or a `source_bus_id` on `track.add_send`).

## What exists to build on

- `resonance-app/src/update/bus.rs` (`SetBusPan`, `ToggleBusMute`, undo
  classification already in place).
- `resonance-app/src/update/mixer.rs` (`MixerMessage::AddSend { source }`
  already takes any `SendSource`); `view/mixer/inspector/sends.rs`
  (`sends_block` keyed on a track today).
- Engine bus-sourced sends: `mixer/render/routing.rs::pull_bus_aux_sends`
  and the cycle check `types::aux_send_would_cycle`.
- Feature docs: [`../../features/mixer/busses-and-output-routing.md`](../../features/mixer/busses-and-output-routing.md),
  [`../../features/mixer/aux-sends.md`](../../features/mixer/aux-sends.md),
  [`../../features/mixer/volume-pan-mute-solo.md`](../../features/mixer/volume-pan-mute-solo.md).

## Open decisions

1. Bus solo semantics. **Recommended:** soloing a bus solos every track
   routed into it (and keeps that bus and returns audible), implemented as
   an effective-solo fan-out in the app like group macro solo, so the
   engine's solo predicate does not change.
2. Bus-sourced send surface: a `source_bus_id` on `track.add_send`, or a
   new `bus.add_send`. **Recommended:** `bus.add_send`, keeping each
   namespace about its own channel; `track.set_send` / `track.remove_send`
   already address by `send_id` and work for any source.
3. Whether to land bus sends before [bus-to-bus-routing.md](bus-to-bus-routing.md)
   fixes the index-order rule. **Recommended:** fix ordering first, or
   bus → bus sends can be created that do nothing.

## Slices

1. `bus.set_pan` + `bus.set_mute` (wire, handler, MCP tool, control tests).
2. Bus solo: app state, strip button, persistence, `bus.set_solo`.
3. Bus SENDS group + `bus.add_send`.

## How we'd know it works

- Control tests per method (undoable, revision bumps once, validation).
- Render test: a bus-sourced send into a return is audible, pre and post.
- Solo test: soloing a bus silences tracks not routed into it.
