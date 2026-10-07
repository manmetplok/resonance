# Volume, pan, mute and solo

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: none (pan-law change recorded in ba doc #276 "BUG 3"; mute/solo races in code review MIX-05 / MIX-07 / FU-B3a).

Every track and bus has a fader, a pan control and a mute; tracks also
have solo. The master has a fader only. A user rides them on the strips;
an agent sets track faders, pan, mute and solo with `mixer.*` and bus and
master faders with `bus.set_volume` / `master.set_volume`.

## Behaviour

### Fader

- The app stores fader values in dB (`TrackState.volume`,
  `BusState.volume`, `MasterState.volume`) and sends the engine linear
  gain via `util::db_to_gain`. The range is −60 to +6 dB
  (`VOLUME_DB_MIN` / `VOLUME_DB_MAX` in `resonance-control`). −60 dB and
  below is silence (gain 0), not −inf.
- The engine ramps every gain change linearly across one block
  (`mixer::common::ramped_gain`, `sum_to_stereo`), so fader moves, pan
  moves, mutes and automation do not zipper or click. A bus that has never
  rendered starts flat at its target gain instead of ramping up from zero
  (`Bus::rendered_once`, FU-B6a).
- A fader drag coalesces into one undo entry per gesture
  (`CoalesceKey::{TrackVolume, BusVolume, MasterVolume}`).

### Pan

- Pan is `-1.0..=1.0`, 0 = centre, on tracks and busses. The master has
  no pan.
- The law is **stereo balance**, not constant power:
  `resonance_dsp::stereo_balance` gives `(1, 1)` at centre and attenuates
  only the far side when panned (`mixer::common::track_stereo_gains`,
  `bus_stereo_gains`). Every track in this engine is stereo, so the old
  constant-power law cost 3.01 dB at centre for no reason. Centre is
  unity; a hard-panned track is the same as before the change.
- Plugin-internal panning (the wavetable synth, the granular delay) still
  uses `constant_power_pan`; that is the plugin's own business.

### Mute and solo

- One resolution shared by live playback and every offline render
  (`mixer::render::strategy::track_silenced`): a top-level track is
  silenced when it is muted, or when any top-level track is soloed and it
  is not. Busses are never silenced by solo, so a soloed track still
  reaches its bus and its send returns.
- Sub-tracks ignore their own solo flag and follow the parent's
  (`sub_track_silenced`); their own mute still applies.
- Solo state is snapshotted once at the top of each block
  (`snapshot_top_level_solo` / `Track::block_soloed`), so a solo toggle
  landing mid-block cannot silence the whole block.
- A silenced track keeps running its instrument (note-offs still flow),
  fades out over one block, then its output is discarded — including
  pre-fader sends. If a sidechain key is tapped from it, it still renders
  its source and chain up to the key tap and nothing further (code review
  MIX-05; see [sidechain-routing.md](sidechain-routing.md)).
- A mute automation lane is OR-ed with the static mute: an unmute lane
  cannot override a hard mute.
- Track groups add a macro mute/solo that composes with the track's own
  flag: the app always sends the engine the effective value
  (`TrackGroupRegistry::effective_mute` / `effective_solo`). Groups are
  documented under [track-groups.md](../timeline/track-groups.md).
- Offline renders: `to_wav` mixdown honours mute/solo; bounce-in-place of
  a track does not (it is muted afterwards by design).
- `ToggleMuteSelected` / `ToggleSoloSelected` act on the whole selection
  as one undo step: a mixed selection turns the flag on for all, only an
  all-on selection turns it off.

### Mono

The inspector TRACK group's `Mono` toggle (`TrackMessage::ToggleTrackMono`)
is an **input** setting, not a mono sum of the channel: a mono track
monitors and records one input channel onto both sides
(`RenderStrategy::mix_monitor`, `mixer/monitor.rs`). Input routing is
documented under [audio-input-and-monitoring.md](../hardware-io/audio-input-and-monitoring.md).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Pan law | `resonance-dsp/src/pan.rs` | `stereo_balance`, `constant_power_pan` |
| Gains, ramps | `resonance-audio/src/mixer/common.rs` | `track_stereo_gains`, `bus_stereo_gains`, `ramped_gain`, `sum_to_stereo` |
| Mute/solo resolution | `resonance-audio/src/mixer/render/strategy.rs` | `track_silenced`, `sub_track_silenced`, `track_disposition`, `bus_disposition` |
| Solo snapshot | `resonance-audio/src/types/track.rs` | `any_top_level_solo`, `snapshot_top_level_solo` |
| Engine handlers | `resonance-audio/src/engine/tracks.rs`, `engine/busses.rs` | `handle_set_bus_volume`, `handle_set_bus_pan`, `handle_set_bus_mute` |
| App update | `resonance-app/src/update/track.rs`, `update/bus.rs` | `TrackMessage::{SetTrackVolume, SetTrackPan, ToggleMute, ToggleSolo, SetMasterVolume, ToggleTrackMono}`, `BusMessage::{SetBusVolume, SetBusPan, ToggleBusMute}` |
| Control handlers | `resonance-app/src/update/control/track/mixer.rs`, `update/control/bus.rs`, `update/control/master.rs` | `set_volume`, `set_pan`, `set_mute`, `set_solo` |
| View | `resonance-app/src/view/controls.rs`, `view/knob.rs` | `fader_section`, `pan_knob_automated` |

Invariant: any render path that asks "is this track soloed" mid-block must
read `block_soloed()`, never `soloed()`.

## Surfaces

- **GUI:** strip fader, pan knob (centred, value below), M / S buttons;
  MIDI-learnable faders and buttons (see [midi-learn.md](../hardware-io/midi-learn.md)).
  Selection shortcuts for mute/solo (command palette).
- **Control API / MCP:**
  `mixer.set_volume` → `mcp__resonance__mixer_set_volume` (linear),
  `mixer.set_volume_db` → `mcp__resonance__mixer_set_volume_db`,
  `mixer.set_pan` → `mcp__resonance__mixer_set_pan`,
  `mixer.set_mute` → `mcp__resonance__mixer_set_mute`,
  `mixer.set_solo` → `mcp__resonance__mixer_set_solo` (tracks only);
  `bus.set_volume` → `mcp__resonance__bus_set_volume` (dB only);
  `master.set_volume` → `mcp__resonance__master_set_volume` (linear or dB).
- **Agent skills:** `mixing` (balance passes with `mixer_set_volume_db`,
  `mixer_set_pan`).

## How it's tested

| What | Where | Run |
|---|---|---|
| Pan law | `resonance-dsp/tests/stereo.rs` (`balance_is_unity_at_centre_and_fades_the_far_side`) | `./scripts/run-tests.py -p resonance-dsp` |
| Gain ramp, solo predicate and race | `resonance-audio/tests/mixer/{mixer_gain_ramp,solo_predicate,solo_race,bus_first_block_gain,sub_track_parent_fader}.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Control methods | `resonance-app/tests/control/{control_track_mixer,control_mixer_volume_db,control_master}.rs` | `./scripts/run-tests.py -p resonance-app` |
| Group macro mute/solo/level | `resonance-app/tests/mixer/{group_macro_mute,group_macro_solo,group_macro_level}.rs` | same |
| Pan knob drag | `resonance-app/tests/mixer/pan_knob_drag.rs` | same |

Not covered: no test pins that a sub-track's own solo flag is inert in the
app (only the engine predicate is tested).

## Decisions and deviations

- Stereo balance replaced constant-power pan on 2026-08-04 (user-approved
  sound change): centre-panned material came up 3 dB. Any assertion still
  multiplying by `FRAC_1_SQRT_2` is stale.
- `bus.set_volume` takes dB only; `mixer.set_volume` keeps linear for
  compatibility, with `mixer.set_volume_db` beside it.
- Busses have no solo state at all.

## Known gaps

- No `bus.set_pan`, `bus.set_mute` over the control API, and no bus solo —
  [`../../intents/mixer/bus-control-parity.md`](../../intents/mixer/bus-control-parity.md).
- The sub-track strip shows a solo button whose flag the engine ignores —
  [`../../intents/mixer/sub-track-solo.md`](../../intents/mixer/sub-track-solo.md).
