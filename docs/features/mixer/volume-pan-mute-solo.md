# Volume, pan, mute and solo

> Verified against master @ `acac5ea3` on 2026-10-15.
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
  (`mixer::render::strategy::{solo_role, track_silenced,
  sub_track_silenced}`): a track is silenced when it is muted, or when
  any track — a sub-track included — is soloed and neither it nor any of
  its sub-tracks is. Busses are never silenced by solo, so a soloed track
  still reaches its bus and its send returns.
- **Sub-track (kit tap) solo** (BUG-02). Soloing a tap plays that tap,
  plus anything else soloed, and nothing more. Its parent cannot simply
  be silenced — the tap's audio is one of its instrument's ports — so it
  renders as a *fan-out driver* (`SoloRole::FanOutDriver`, the
  `discard_own_output` disposition stems use): the instrument runs, its
  own main output (port 0) is silent, and so is every tap that is not
  soloed itself (`TrackDisposition::solo_taps_only`). Live, port 0 fades
  out over one block like a mute before it is discarded.
- A soloed parent carries every tap whatever their own solo; a tap's own
  mute still applies; a muted parent silences all its taps, a soloed one
  included (mute wins over solo, as on a top-level track).
- A frozen kit's taps are baked into its one cache, so soloing a tap of a
  frozen kit plays the whole frozen kit (still as a solo: everything else
  drops out).
- Solo state is snapshotted once at the top of each block, for every
  track and sub-track (`snapshot_solo` / `Track::block_soloed`), so a solo
  toggle landing mid-block cannot silence the whole block.
- A silenced track keeps running its instrument (note-offs still flow),
  fades out over one block, then its output is discarded — including
  pre-fader sends. If a sidechain key is tapped from it, it still renders
  its source and chain up to the key tap and nothing further (code review
  MIX-05; see [sidechain-routing.md](sidechain-routing.md)). A fan-out
  driver that is keyed does the same, and its soloed taps still play.
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
| Mute/solo resolution | `resonance-audio/src/mixer/render/strategy.rs` | `SoloRole`, `solo_role`, `track_silenced`, `sub_track_silenced`, `track_disposition`, `sub_track_disposition`, `bus_disposition` |
| Fan-out driver | `resonance-audio/src/mixer/render/track_pass.rs`, `sub_track.rs` | `render_one_track`, `fan_out_to_sub_tracks` |
| Solo snapshot | `resonance-audio/src/types/track.rs` | `any_track_soloed`, `snapshot_solo` |
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
  `mixer.set_solo` → `mcp__resonance__mixer_set_solo` (tracks and
  sub-tracks; not busses);
  `bus.set_volume` → `mcp__resonance__bus_set_volume` (dB only);
  `master.set_volume` → `mcp__resonance__master_set_volume` (linear or dB).
- **Agent skills:** `mixing` (balance passes with `mixer_set_volume_db`,
  `mixer_set_pan`).

## How it's tested

| What | Where | Run |
|---|---|---|
| Pan law | `resonance-dsp/tests/stereo.rs` (`balance_is_unity_at_centre_and_fades_the_far_side`) | `./scripts/run-tests.py -p resonance-dsp` |
| Gain ramp, solo predicate and race | `resonance-audio/tests/mixer/{mixer_gain_ramp,solo_predicate,solo_race,bus_first_block_gain,sub_track_parent_fader}.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Sub-track solo, live and offline master render | `resonance-audio/tests/mixer/sub_track_solo.rs` | same |
| Control methods (incl. `mixer.set_solo` on a sub-track) | `resonance-app/tests/control/{control_track_mixer,control_mixer_volume_db,control_master}.rs` | `./scripts/run-tests.py -p resonance-app` |
| Sub-track S button sends the tap's flag | `resonance-app/tests/mixer/sub_track_solo.rs` | same |
| Group macro mute/solo/level | `resonance-app/tests/mixer/{group_macro_mute,group_macro_solo,group_macro_level}.rs` | same |
| Pan knob drag | `resonance-app/tests/mixer/pan_knob_drag.rs` | same |

Not covered: a keyed fan-out driver (the key path is the MIX-05 key-only
one, exercised for silenced tracks by the sidechain tests).

## Decisions and deviations

- Stereo balance replaced constant-power pan on 2026-08-04 (user-approved
  sound change): centre-panned material came up 3 dB. Any assertion still
  multiplying by `FRAC_1_SQRT_2` is stale.
- `bus.set_volume` takes dB only; `mixer.set_volume` keeps linear for
  compatibility, with `mixer.set_volume_db` beside it.
- Busses have no solo state at all.
- Sub-track solo (BUG-02) was fixed rather than the S button hidden on
  sub-strips; it reuses the stems' fan-out-driver disposition instead of
  a new render path, and `solo_predicate.rs`'s old
  `sub_track_solo_alone_is_false` was flipped on purpose.

## Known gaps

- No `bus.set_pan`, `bus.set_mute` over the control API, and no bus solo —
  [`../../intents/mixer/bus-control-parity.md`](../../intents/mixer/bus-control-parity.md).
