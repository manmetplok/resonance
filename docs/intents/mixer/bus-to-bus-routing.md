# Bus-to-bus routing that always sounds

> Status: **idea**
> Area: mixer
> Source: `resonance-audio/src/mixer/render/routing.rs::pull_bus_aux_sends` doc comment; found while documenting the mixer (2026-10-07)

## Why

Two limits make nested bus structures (drums → drum bus → "music" bus, or
a reverb return feeding a delay return) either impossible or silently
wrong:

- A bus's main output always goes to the master; there is no bus → bus
  output route.
- A bus → bus **aux send** is accepted, passes the cycle check and is
  saved, but it is only audible when the destination bus has a *higher*
  index than the source. A send into a lower-indexed (older) bus has no
  effect at all, with no error. Which bus is "older" is invisible to the
  user and the agent.

## What it would do

- Every accepted bus → bus send is audible, regardless of creation order.
- Optionally, a bus can route its main output into another bus, with the
  same feedback check as sends.

## What exists to build on

- The bus pass already renders busses level by level from the send graph
  (`render/bus_pass.rs::assign_levels`); the restriction is only that
  `pull_bus_aux_sends` scans sources with a lower index than the
  destination.
- `types::aux_send_would_cycle` already walks the bus graph.
- Feature docs: [`../../features/mixer/busses-and-output-routing.md`](../../features/mixer/busses-and-output-routing.md),
  [`../../features/mixer/aux-sends.md`](../../features/mixer/aux-sends.md).

## Open decisions

1. Fix the ordering or refuse the send. **Recommended:** fix it — compute
   levels from the send graph (topological order) and pull from any
   source of a lower *level*, not a lower index. Keep the in-index-order
   summing so output stays bit-identical at any thread count.
2. Add a bus → bus output route. **Recommended:** only after (1), as a
   `TrackOutput`-like field on the bus checked by the same cycle walk; PDC
   would need a stage per bus level (today: track and bus stages only),
   which is the real cost.

## Slices

1. Level assignment over the send graph; render test with a send into an
   older bus; parallel-render parity with `RESONANCE_RENDER_THREADS=8`.
2. (Optional) bus output routing: engine field, app picker in bus
   ROUTING, `bus.set_output`, persistence, PDC.

## How we'd know it works

- `resonance-audio/tests/mixer/aux_send_render.rs` gains a case: bus B
  (created second) sends into bus A (created first) and A's output carries
  it.
- Goldens and `render_block_parity` hold at 1 and 8 render threads.
