# Drums plugin (sampler, mic banks, streaming, editor)

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/specs/drums-plugin-rework.md` (built on
> `feat/drums-rework`, merged to master in `0984dfc6`, 2026-10-02; its §12
> "As built" still holds).

`com.resonance.drums` is the first-party sampled drum kit. It plays
multi-mic, multi-velocity, round-robin kits (Drummica and the other plok.org
kits) on 30 GM pads, streams long samples from disk, can split into seven
stereo outputs for per-group mixing, and has an editor with Pads, Mix and
Setup tabs. Choosing and downloading kits is covered in
[drum-kit-library.md](drum-kit-library.md).

## Behaviour

### Pads and kits

- **30 pads**, one per GM note (`resonance-common/src/drum_map.rs`,
  `NUM_PADS`, `GM_PADS`). A kit's pieces map to pads through
  `_meta.pads.<piece>.note`, then the built-in Drummica table, else the pad
  is absent: dimmed in the editor and silent (`pad_map.rs`).
- **Kit selection** is the `kit_select` param: `-2` parked (missing or
  external kit), `-1` the built-in kit, `0..=999` a library slot. It is not
  automatable. Its text is the kit name, so agents can set it by name.
  `kit_load_progress` (read-only, 0..1, text "empty slot" or "failed") says
  when the kit plays. The built-in kit plays only when nothing is selected
  or the selection is missing.
- **State** stores `kit_ref` (`{id, name, rel_path, abs_path}`, resolved in
  that order), the mic bank choices, per-pad mic picks, articulations and
  the stream preload. A missing kit keeps its reference and shows a banner
  with Download, Locate and Choose. Old states are upgraded in order:
  linear levels to dB, missing `output_mode` → Multi, polyphony
  (`params.rs` `upgrade_v1_levels`, `upgrade_output_mode`,
  `upgrade_polyphony`).

### Playing

- **Velocity layers** are picked by measured loudness, with a gain that
  fills the gap to the target level (at least a 24 dB velocity span, up to
  32 layers). `velocity_curve` (±1, 0 is identity) and `velocity_humanize`
  (0–20, fixed seed) shape it.
- **Round robin:** Cycle, or Random that never repeats the last take.
- **Choke:** per pad `-1` Kit default, `0` none, `1–8`. Hats default to
  group 1; a kit can suggest a group in `_meta.pads.<piece>.choke`.
- **Articulations:** primary/alternate pairs from `_meta.articulations`, or
  the Drummica pairs. Switching reloads that pad in place.
- **Per-pad params:** level (−∞ to +6 dB), pan, mute, articulation, close
  mic 1/2, overhead, bleed and room trims (−∞ to +12 dB), choke, output,
  tune (±24 st), hold (0–2000 ms), decay (5–4000 ms, 4000 = off), start
  (0–100 ms). No solo and no per-pad release param; fades are fixed:
  release 25 ms, kit swap 5 ms, steal 3 ms.
- **Voices:** `polyphony` 1–128 (default 128). Stealing order is free voice,
  oldest on the same pad, oldest overall; a hit never steals its own
  voices. Stolen voices fade in one of 32 tail slots. A kit swap fades the
  old kit out (up to 4 retiring at once); a janitor thread frees retired
  kits off the audio thread. `reset()` cuts instantly.

### Mic banks

Every pad gets its close mics (kick and snare two, toms and hats one,
cymbals none) and overhead slot 1. Optional banks: overhead slots 2 and 3,
two bleed banks and one room bank, each with a level, and bleed/room on/off
params. Bank choices are plugin state, not params. Mic labels read
`position · brand mic`.

### Outputs

Seven stereo ports are always declared: Main, Kick, Snare, Toms, Hats,
Cymbals, Overhead. `output_mode` Stereo (default for a new instance) sums all
to Main; Multi sends each pad's close mics to its group port and the
overheads to Overhead, except cymbals, which keep their overhead on their
own port. The app turns Multi into sub-tracks (see
[channel-strips-and-inspector.md](../mixer/channel-strips-and-inspector.md)). `output_mode` and per-pad output are excluded from
plugin presets.

### Disk streaming

Sample heads are preloaded (`stream_preload`: Off, 32k, 64k, 128k frames,
default 32k) and the rest is streamed by a reader pool of
`clamp(cores/4, 2, 4)` threads into per-voice rings of 16384 frames
(`stream/mod.rs`, `stream/reader.rs`). The audio thread never locks,
allocates or blocks; a frame that is not there plays as silence and counts
as an underrun, shown in the editor's status bar. Offline renders wait,
with a bound, for the reader. A process-wide sample cache shares decoded
takes between instances.

### Editor

960×640 by default (780×520 minimum), also laid out for the 1571×856 a
tiling compositor imposes. Header with ★, ◀ kit ▶, `Library…` and load
percentage; tabs Pads (6×5 grid that lights on hits, pad inspector with
audition, waveform of the last take, mics, routing), Mix (Stereo/Multi
switch, port strips and meters, per-pad table, global controls) and Setup
(mic banks, streaming cost and health, kit facts, pad note/output/choke
table, read-only). A status bar shows sample rate, pads, memory, underruns,
last hit and output level.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Plugin + state | `plugins/resonance-drums/src/lib.rs`, `selection.rs` | `ResonanceDrums`, `KitBridge`, `KitSelection`, `output_layout` |
| Params | `src/params.rs`, `level.rs`, `velocity.rs`, `choice.rs` | `DrumParams`, `PadParams` |
| Kit model + pad map | `src/kit.rs`, `pad_map.rs`, `drum_map.rs`, `articulation.rs`, `mic_catalog.rs` | `LoadedPad`, `PAD_MAPPINGS` |
| Loading | `src/kit_loader/{mod,manifest,decode,cache,banks,progress,fallback}.rs`, `reload.rs` | `SampleCache`, bank planning, built-in kit |
| Sampler | `src/dsp/{sampler,voice_pick,janitor}.rs`, `voice.rs` | `DrumSampler`, `MAX_VOICES`, `TAIL_SLOTS` |
| Streaming | `src/stream/{mod,reader}.rs` | `RING_FRAMES`, reader pool |
| Kit info to the host | `src/kit_info_ext.rs` | `com.resonance.kit-info` source |
| Editor | `src/editor/{app,chrome,pad_grid,pad_inspector,mix_tab,setup_tab,controls,missing_kit}.rs` | tabs |
| Host-read keys | `resonance-plugin/src/first_party.rs` | `first_party::drums::{KIT_SELECT, KIT_LOAD_PROGRESS, OUTPUT_MODE}` |
| App side | `resonance-app/src/state/plugin_mirror.rs` (`kit_info`), `compose/drumroll/groups.rs` (`kit_pads_from_info`), `update/compose/drum_groups.rs` | drum-group kit picker |

Invariants: no allocation, lock or blocking call on the audio thread
(`tests/janitor_rt.rs`, `tests/streaming_rt.rs`); streamed and cached
playback are bit-identical; render output is identical at any
`RESONANCE_RENDER_THREADS`.

## Surfaces

- **GUI:** add "Resonance Drums" as an instrument; editor from the plugin slot; the
  compose drum-group picker reads the loaded kit's pads.
- **Control API / MCP:** `track_set_plugin_param` with `kit_select` (slot or
  name) then poll `kit_load_progress` through `track_plugin_params`; per-pad
  params by name (`Pad N Volume` …). Kit listing:
  `drum_kits.list` / `drum_kits.set_marks` (see
  [drum-kit-library.md](drum-kit-library.md)).
- **Agent skills:** `resonance-agent-plugin/skills/drumming` and the
  user-level `resonance-studio` skill.

## How it's tested

| What | Where | Run |
|---|---|---|
| Sampler and DSP | `plugins/resonance-drums/tests/{sampler,round_robin,velocity_layers,choke_groups,voice_steal,release_ms,kit_swap_fade,note_timing,render_block_offsets,tune_decay_start,tune_alias,pad_param_smoothing,levels_db}.rs` | `./scripts/run-tests.py -p resonance-drums` |
| Routing | `tests/{output_mode,port_routing,drummica_routing,group_balance}.rs` | same |
| Kit load and state | `tests/{kit_loader,load_path,kit_handoff,kit_reactivation,kit_pads,articulation_param,mic_banks,state,state_upgrade,janitor_rt}.rs` | same |
| Streaming | `tests/{streaming,streaming_rt}.rs`; three saturation/throughput tests are `#[ignore]` | `cargo test --release -p resonance-drums --test streaming -- --ignored` |
| Editor | `tests/{editor_tabs,editor_layout,narrow_window,editor_honesty,status_readouts,inert_controls,global_controls,audition,rr_display}.rs` | same |
| Golden | `tests/dsp_golden.rs` → `tests/golden/dsp_golden.u32` (word stream with block digests) | `RESONANCE_BLESS=1` or `RESONANCE_BLESS_DSP_GOLDEN=1` |
| App | `resonance-app/tests/compose/drum_kit_picker.rs`, `resonance-app/tests/control/control_drum_kits.rs` | `./scripts/run-tests.py -p resonance-app` |

Not covered: a live session of streaming under real load (see Known gaps);
the editor's look against a real compositor is only checked by layout tests.

## Decisions and deviations

- D2 import copies into the library; D5 a fresh instance is Stereo; D6 dB
  levels; D7 absent pads are dimmed and silent; D8/D9 streaming and mic
  banks were built in the rework instead of deferred.
- Not in the spec's §12: per-pad choke and output gained a "Kit" default
  (`-1` and `0`), `kit_select` ranges `-2..1000`, kits can suggest port and
  choke in `_meta.pads`, and `_meta.mic_kinds` exists.

## Known gaps

- K10 (kit preview, folder-of-WAVs import, editable note map) and the pad
  map over MCP:
  [drums-kit-authoring](../../intents/plugins-instruments/drums-kit-authoring.md),
  [feature-request-2026-10-07-drum-pad-map-mcp](../../intents/control-api/drum-pad-map-mcp.md).
- The live 10-minute streaming session the spec asked for before merge is
  not recorded, and the user's "Drummica Kit" track preset still predates
  the `params` map:
  [drums-live-verification](../../intents/plugins-instruments/drums-live-verification.md).
