# Bus-to-bus routing that always sounds

> Stage: **intent**
> Status: **partly built** — slice 1 (every bus → bus send audible) shipped; the bus → bus output route has not
> Area: mixer
> Source: `resonance-audio/src/mixer/render/routing.rs::pull_bus_aux_sends` doc comment; found while documenting the mixer (2026-10-07)

## Why

Two limits make nested bus structures (drums → drum bus → "music" bus, or
a reverb return feeding a delay return) either impossible or silently
wrong:

- A bus's main output always goes to the master; there is no bus → bus
  output route.
- ~~A bus → bus aux send into an older bus was inaudible~~ — fixed
  (BUG-04, slice 1): bus levels now come from the send graph; see
  [`../../features/mixer/aux-sends.md`](../../features/mixer/aux-sends.md).

## What it would do

- Every accepted bus → bus send is audible, regardless of creation order.
- Optionally, a bus can route its main output into another bus, with the
  same feedback check as sends.

## What exists to build on

- The bus pass renders busses level by level from the send graph
  (`render/bus_pass.rs::assign_levels`, longest-path levels over every
  enabled bus → bus send); a bus output route would add edges to the same
  level walk.
- `types::aux_send_would_cycle` already walks the bus graph.
- Feature docs: [`../../features/mixer/busses-and-output-routing.md`](../../features/mixer/busses-and-output-routing.md),
  [`../../features/mixer/aux-sends.md`](../../features/mixer/aux-sends.md).

## Open decisions

1. ~~Fix the ordering or refuse the send~~ — decided and built: fixed.
2. Add a bus → bus output route. **Recommended:** only after (1), as a
   `TrackOutput`-like field on the bus checked by the same cycle walk; PDC
   would need a stage per bus level (today: track and bus stages only),
   which is the real cost.

## Slices

1. ~~Level assignment over the send graph~~ — shipped (BUG-04).
2. (Optional) bus output routing: engine field, app picker in bus
   ROUTING, `bus.set_output`, persistence, PDC.

## How we'd know it works

- A bus routes its main output into another bus and the destination's
  output carries it, with PDC holding across the extra bus stage.
- Goldens and `render_block_parity` hold at 1 and 8 render threads.
