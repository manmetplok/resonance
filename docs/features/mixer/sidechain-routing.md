# Sidechain (key) routing

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: ba docs #157 / #159 / #279, todos #1311 (persistence, bus/master slices); prototype `docs/history/prototypes/external-sidechain-routing/index.html`.

A keyed plugin — a compressor or gate ducking from a kick, say — can take
its detection signal from another track or bus instead of its own input.
The key only drives the plugin's detector; it never reaches the output.
Routes can target a plugin on a track, a bus or the master chain. Today
this is reachable **only over the control API**; the GUI shows nothing.

## Behaviour

- **One route per plugin.** `SidechainRoute { plugin, source, enabled }`
  keyed by `PluginInstanceId`; a second route onto the same plugin replaces
  the first. The source is a track (including a sub-track / drum tap) or a
  bus.
- **Which plugins take a key.** Plugins that declare a CLAP key port
  (`resonance-plugin`'s sidechain input): the compressor and gate are the
  primary ones; the reverb (ducks its wet return) and the EQ (dynamic
  bands) have secondary key ports (`first_party::SECONDARY_KEY_PLUGINS`).
  The engine stores a route onto a plugin with no key port inertly; the
  control layer refuses it up front and names the keyable plugins on the
  chain.
- **One block late, by design.** `SidechainTaps` double-buffers: each
  block writes sources into one bank and keys read the other, i.e. the
  previous block. The result does not depend on track order, cycles are
  harmless (a track may key off itself), and the cost is one block of
  detector latency (2.7 ms at 128 frames). No cycle validation exists or is
  needed.
- **Tap point.** Post-FX, pre-fader: riding the source's fader does not
  change how hard it ducks. A muted or solo-suppressed source still renders
  its source and chain up to the key tap (`TrackDisposition::key_only`) so
  the duck keeps working while the source is silent.
- **Source cap.** At most `MAX_SIDECHAIN_SOURCES` (8) distinct sources are
  active at once; tap buffers are pre-allocated. Routes naming a ninth
  source are silently ignored (nothing reports it, despite the type's doc
  comment saying the engine does).
- **Default targeting (control API).** Exactly one of `source_track_id` /
  `source_bus_id`. With no `plugin_id`, `*.set_sidechain` targets the first
  keyable plugin on the chain, skipping the secondary-key plugins while a
  primary one exists; `*.clear_sidechain` targets the plugin that actually
  has a route.
- **Lifecycle.** The engine drops a route when its plugin is removed (any
  owner) and when its source track or bus is removed, so a recycled id
  never inherits a key. The app mirror (`SidechainState`) follows the same
  rules and is what gets saved.
- **Persistence and undo.** `ProjectFile::sidechain_routes`
  (`ProjectSidechainRoute`), at project scope like sends, sorted by target
  id on save; an unknown source tag drops the route. Instance ids survive a
  reload because plugins are re-created with their saved id. Set/clear is
  one undo step (`PluginMessage::SetPluginSidechain`); the route is
  replayed by the project-load and undo-diff paths
  (`reconcile/routing.rs`).
- **Mirror timing.** The app mirrors a route optimistically before the
  engine echo (`SidechainRouteChanged`), so a save taken in between still
  writes it.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Model, taps | `resonance-audio/src/types/sidechain.rs` | `SidechainRoute`, `SidechainTaps`, `active_sources`, `MAX_SIDECHAIN_SOURCES` |
| Engine handlers | `resonance-audio/src/engine/sidechain.rs` | `handle_set`, `handle_clear`, `drop_plugin_route`, `drop_source_routes` |
| Render | `resonance-audio/src/mixer/render/{context,track_pass,bus_pass}.rs`, `mixer/master.rs` | `key_consumed`, `run_fx_chain`, `apply_master_fx_chain` |
| App mirror | `resonance-app/src/state/sidechain.rs` | `SidechainState` |
| App update | `resonance-app/src/update/plugin.rs` | `PluginMessage::SetPluginSidechain` |
| Shared control rules | `resonance-app/src/update/control/sidechain.rs` | `resolve_key_source`, `require_key_port`, `first_keyable`, `clear_target` |
| Per-namespace handlers | `resonance-app/src/update/control/{track/sidechain.rs,bus.rs,master.rs}` | `set_sidechain`, `clear_sidechain` |
| Persistence | `resonance-app/src/project/model.rs`, `update/project_io/serialize.rs`, `update/project_io/reconcile/routing.rs` | `ProjectSidechainRoute` |

## Surfaces

- **GUI:** none.
- **Control API / MCP:**
  `track.set_sidechain` → `mcp__resonance__track_set_sidechain`,
  `track.clear_sidechain` → `mcp__resonance__track_clear_sidechain`,
  `bus.set_sidechain` → `mcp__resonance__bus_set_sidechain`,
  `bus.clear_sidechain` → `mcp__resonance__bus_clear_sidechain`,
  `master.set_sidechain` → `mcp__resonance__master_set_sidechain`,
  `master.clear_sidechain` → `mcp__resonance__master_clear_sidechain`.
- **Agent skills:** `mixing` (`track_set_sidechain`), `spatial`
  (`bus_set_sidechain`, ducking a reverb return).

## How it's tested

| What | Where | Run |
|---|---|---|
| Tap double-buffer | `resonance-audio/tests/sidechain_taps.rs` (standalone: process-global state) | `./scripts/run-tests.py -p resonance-audio` |
| Key delivery in the mixer | `resonance-audio/tests/mixer/sidechain_key_delivery.rs` | same |
| Control (all three namespaces) | `resonance-app/tests/control/control_sidechain.rs` | `./scripts/run-tests.py -p resonance-app` |
| Persistence and undo | `resonance-app/tests/mixer/sidechain_persistence.rs` | same |
| Plugin side (key port behaviour) | `plugins/resonance-compressor/tests/{sidechain_key,key_meter}.rs` | `./scripts/run-tests.py -p resonance-compressor` |

Not covered: the eight-source cap (no test names `MAX_SIDECHAIN_SOURCES`).

## Decisions and deviations

- The double-buffered tap was kept over a cyclic-validation design that
  once existed on a branch (now dangling): cycles are harmless here and
  the latency is below any usable attack time (ba doc #279).
- Routes are keyed by plugin instance and source, so bus and master targets
  never needed engine work; only the wire was track-shaped until #1311.
- Not built from the earlier design (doc #157): per-route key gain,
  detector HPF on the key, and "audition the key".

## Known gaps

- No GUI at all: no key-source picker, no indication on the strip or in
  CHAIN that a plugin is keyed, no rejection message —
  [`../../intents/mixer/sidechain-gui.md`](../../intents/mixer/sidechain-gui.md).
- A ninth distinct key source is ignored without any report.
