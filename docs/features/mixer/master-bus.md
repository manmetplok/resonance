# Master bus and master summary

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/specs/mixer-cleanup.md` §3.3 / §5 (master strip and inspector); control side from ba doc #273 (todos #1226 / #1227).

The master is the last stage: every track and bus sums into it, then it
runs its own insert chain (a limiter, the mastering plugin), its fader and
a final safety clip. A user selects the master strip to edit its chain,
automation and Bounce in the inspector; an agent reads it with
`master.summary` and edits it with the `master.*` methods.

## Behaviour

- **Order of the master passes, per callback** (`callback/master_pass.rs`):
  1. master insert chain over the summed buffer (chain bypass and per-slot
     bypass both crossfade over 5 ms);
  2. metronome click layered in (so the click never goes through master
     FX);
  3. master volume, ramped per sample from the previous block's value,
     then a **hard clip to ±1.0**, with non-finite samples replaced by 0;
  4. master peak meters and the A/B "mix" meter, both post-FX,
     post-volume.
- **Master volume** is stored in dB (`MasterState.volume`), same −60…+6 dB
  range and silence floor as a track fader. A master-gain automation lane
  overrides the static fader for the block without touching it; it is
  evaluated at the latency-compensated position, so a drawn move lands on
  the audio it was drawn against.
- **No pan, mute or solo** on the master.
- **Offline renders** run their own copy of this chain (`bounce::render`);
  `meter.measure` with `target: "master"` and `render.mixdown` see the same
  master FX, volume and clip.
- **Reference A/B** replaces the entire output after the master, so the
  reference bypasses the master chain (see
  [reference-ab-monitoring.md](reference-ab-monitoring.md)).
- **Inspector.** Selecting the master strip shows CHAIN, AUTOMATION and a
  MASTER group holding **Bounce** (the project-to-WAV render; it reads
  "Bouncing…" while one runs). The strip has the FX switch, slot lines and
  fader.
- **Persistence and undo.** `ProjectFile::{master_volume, master_plugins,
  master_fx_bypassed}`. Every master chain edit and the chain bypass are
  one undo step; a fader drag coalesces (`CoalesceKey::MasterVolume`).
- **`master.summary`** returns linear and dB volume, chain bypass, the
  insert chain in processing order (slot, CLAP id, name — no per-slot
  bypass; `master.plugin_params` has that) and the master's automation
  lanes in compact form. It needs an open project.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Master passes | `resonance-audio/src/mixer/callback/master_pass.rs` | `run_master_passes`, `MasterTail` |
| Master FX, volume, clip, peaks | `resonance-audio/src/mixer/master.rs` | `apply_master_fx_chain`, `apply_master_volume_and_peaks`, `sanitize_sample` |
| Engine model | `resonance-audio/src/types/track.rs` | `MasterBus` |
| App state | `resonance-app/src/state/master.rs` | `MasterState` |
| App update | `resonance-app/src/update/master.rs`, `update/track.rs` | `MasterMessage`, `TrackMessage::SetMasterVolume` |
| Control | `resonance-app/src/update/control/master.rs` | `master.*` handlers |
| View | `resonance-app/src/view/mixer/master_strip.rs`, `view/mixer/inspector/master.rs` | `view_master_strip`, `master_group` |

## Surfaces

- **GUI:** master strip (right of the track lane); master inspector.
- **Control API / MCP:**
  `master.summary` → `mcp__resonance__master_summary`,
  `master.set_volume` → `mcp__resonance__master_set_volume` (`volume` linear
  or `volume_db`, exactly one).
  Chain methods (`master.add_effect`, `remove_effect`, `move_effect`,
  `replace_effect`, `set_fx_bypass`, `plugin_params`, `set_plugin_param`,
  `set_plugin_bypass`, `plugin_presets`, `load_plugin_preset`,
  `save_plugin_preset`) are in [plugin-chains.md](plugin-chains.md); key
  routes in [sidechain-routing.md](sidechain-routing.md);
  `master.assist` in [mastering-assist.md](mastering-assist.md).
- **Agent skills:** `mastering`, `mixing` (`master_summary`,
  `master_add_effect`).

## How it's tested

| What | Where | Run |
|---|---|---|
| Control | `resonance-app/tests/control/{control_master,control_master_params}.rs` | `./scripts/run-tests.py -p resonance-app` |
| Master chain moves | `resonance-audio/tests/engine/master_plugin_move.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Master tail / latency in renders | `resonance-audio/tests/bounce/bounce_tail_and_master_latency.rs` | same |
| Live vs offline parity | `resonance-audio/tests/mixer/{mix_audio_parity,render_block_parity}.rs` | same |
| Master inspector golden | `resonance-app/tests/snapshots/mixer_inspector_master-wgpu.png` | `RESONANCE_BLESS=1 cargo test -p resonance-app --test mixer` |

Not covered: no test asserts the hard clip or the non-finite guard on the
live master pass directly.

## Decisions and deviations

- Bounce moved off the master strip into the master inspector
  (mixer-cleanup.md §3.3).
- The final hard clip is a safety net, not a limiter: anything over 0 dBFS
  is cut, so a limiter belongs in the master chain.

## Known gaps

- None specific to the master beyond the shared plugin-chain gaps.
