# Render graph publishing

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/refactor/refactor-intent.md` (epic B, ARCH-02),
> `docs/history/refactor/arch-migration-plan.md` ("ARCH-02").

The audio callback and the offline renderers read the project (tracks,
busses, master chain, plugins, audio clips, MIDI clips) from one immutable
snapshot that the engine thread republishes after every edit. A reader does
one wait-free load per block and can never be blocked or made to skip a
block by an edit.

## Behaviour

- `RenderGraph` holds six `Arc`'d maps: `midi_clips`, `clips`, `busses`,
  `master`, `tracks`, `plugins`. `RenderGraphSlot` wraps it in an `ArcSwap`.
- Readers call `SharedState::graph.load()` once per block (live) or per chunk
  (offline) and use that graph for the whole block.
- Writers are engine-thread handlers. Each edit goes through one
  `RenderGraphSlot::edit_*` method (`edit_tracks`, `edit_track`,
  `edit_busses`, `edit_bus`, `edit_master`, `edit_clips`, `edit_clip`,
  `edit_midi_clips`, `edit_midi_clip`, `edit_plugins`,
  `edit_tracks_and_busses`). The method copies the element list (`Arc`
  clones), copy-on-writes the changed element with `Arc::make_mut`, and
  publishes a new graph. An internal `edit` mutex serialises
  read-modify-publish; the audio thread never touches it.
- `edit_tracks_and_busses` publishes both maps in one graph, so a bus
  removal and the re-routing of its feeders are never seen apart.
- **Live state is not in the graph's structure.** Faders, pan, mute, solo,
  arm, monitor, meters, last gains, bypass fades, a track's insert chain and
  frozen cache are atomics or `ArcSwap`s shared by every copy of a `Track` /
  `Bus`. Their setters write through and publish nothing.
- **Old graphs are retired, not dropped.** `publish_locked` swaps the graph
  and pushes the old one onto `SharedState::retired`. The engine loop's
  `Retired::sweep` drops every entry whose `strong_count` is 1. So the audio
  thread is never the last owner of a graph, a clip's mmap, a frozen cache or
  a plugin instance.
- **Plugins.** The map holds `Arc<PluginSlot>`; each instance is behind its
  own mutex. `edit_plugins` retires every slot the new map dropped, so a
  `ClapInstance` is destroyed on the engine thread by the sweep.
- **Slot pool growth.** `publish_locked` calls `SlotSupply::ensure` before
  storing a graph with more tracks than the live render-slot pool holds, so
  the audio thread never has to allocate a slot (see
  [render-pool.md](render-pool.md)).
- Other callback-visible tables use the same retire discipline but are
  separate `ArcSwap`s on `SharedState` or the context: tempo map
  (`retire::rcu_tempo`), latency comp, automation snapshot, aux sends,
  sidechain routes, take-comp table, audition and reference sources.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Graph + slot | `resonance-audio/src/engine/render_graph.rs` | `RenderGraph`, `RenderGraphSlot`, `publish_locked` |
| Deferred drop | `resonance-audio/src/engine/retire.rs` | `Retired`, `publish`, `publish_opt`, `rcu_tempo` |
| Shared-state wrappers | `resonance-audio/src/engine/shared_state.rs` | `SharedState::edit_*`, `tracks()`, `plugins()`, `clips()` |
| Engine-thread sweep | `resonance-audio/src/engine/thread/mod.rs` | `ctx.shared.retired.sweep()` once per loop |
| Slot supply | `resonance-audio/src/mixer/render/slots.rs` | `SlotSupply`, `LiveSlots` |

Invariants a change must keep:
- Only the engine thread publishes. Workers post results to
  `SharedState::inbox` instead (see [engine-command-loop.md](engine-command-loop.md)).
- Never `store` a callback-visible `ArcSwap` directly; use `retire::publish`
  so the old value is dropped on the engine thread.
- There is no `RwLock` on any project map in `engine/` or `mixer/`.

## Surfaces

- **GUI / Control API / MCP:** none. This is plumbing under every edit.
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| Epic B "done when" hammer: 500 clips, continuous render, heavy edits; no silent block, zero frees on the render thread, every plugin destroyed on the sweeping thread | `resonance-audio/tests/retire_queue/render_graph_hammer.rs` (standalone binary `tests/retire_queue.rs`, counting allocator) | `./scripts/run-tests.py -p resonance-audio` |
| Retire primitive | `resonance-audio/tests/retire_queue.rs` | same |
| Per-map publishing through real handlers | `tests/engine/render_graph_{busses,clips,midi,tracks}.rs` | same |
| Callback reads | `tests/mixer/render_graph_{clips,plugins,publish}.rs` | same |
| Slot growth | `tests/mixer/render_slot_supply.rs` | same |

Not covered: memory cost of the per-edit list copies on very large
projects (an edit copies `O(n)` `Arc`s).

## Decisions and deviations

- Epic B replaced five `Arc<RwLock<…>>` maps that the callback `try_read`
  and skipped a block on when a writer queued. B-6 deleted the skip path and
  the per-map lock-miss counters with them; `cycle_load` no longer has a
  state-lock line.
- The job schedule for the render pool is not part of the graph; it is
  built per block on the rendering thread (realtime-multithreading.md §11.1).
- A cancelled bounce in place posts `AudioCommand::BounceTargetCancelled`
  so the engine thread removes its target track; no worker writes the graph.

## Known gaps

None known.
