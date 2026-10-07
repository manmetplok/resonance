# External instrument tracks

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/prototypes/external-instrument-tracks/index.html` (epic #39, doc #169). Leads: doc #257 (playback source), doc #260 findings #3/#4, ba doc #275 P1.2/P1.3, todo #453/#454.

An external-instrument track plays an outboard synth: its notes go out a
hardware MIDI port and the synth's audio comes back on an input (the
"audio return"). The track can pick a patch, compensate the round-trip
latency (typed or measured with a ping), monitor and record the return, play
back either the live hardware or recorded takes, and bounce the hardware to
a new audio track in real time. Named patches and automatable synth
parameters come from device presets
([device-presets.md](device-presets.md)).

## Behaviour

### Setting one up

- **Create:** Add-track menu → "Ext Instrument" (or the palette's "Add
  External Instrument Track"), or Mix inspector → TRACK → "External
  hardware…" on an existing instrument track, or `track.add {kind:
  "external"}` / `external.enable`.
- Enabling **removes an in-app instrument at slot 0** in the same undo step.
  A leftover synth would run as the first insert and overwrite the return
  with its own silence (ba doc #275 P1.2). Effects stay.
- The MIDI out (device + channel, default channel 1), audio return (device +
  first channel), mono, monitor and arm live on the track itself, as on any
  track. The external config adds bank, program, latency offset and the
  chosen device preset (`ExternalInstrumentState`). Everything is project
  state and undoable, except the offline flags and the ping status.
- **Status** is derived, never stored (`ExternalInstrumentStatus`):
  Unconfigured (no MIDI out), Configuring (no return or monitoring off),
  Live, Offline (a configured device is missing). The inspector shows it as
  a badge and the strip as an overlay.
- Until both halves are set the track makes no sound: nothing renders
  locally.

### Patch

- Bank (14-bit, `MSB<<7 | LSB`) and program (0–127), each optional. Setting
  either sends Bank Select (CC0/CC32) + Program Change.
- The saved patch is re-sent on project load and every time playback starts
  from stopped (`handle_resend_patches`).
- With a device preset chosen, the Patch card lists names from the preset's
  patch list (`SetPatch`); without one it shows numeric pickers.

### Latency

- `latency_offset_samples` delays the rest of the mix so the late return
  lines up (`latency::add_external_offsets` in the PDC table); negative
  values pull it earlier. The slider coalesces into one undo entry per
  track.
- **Detect** (`DetectExternalInstrumentLatency`) needs a stopped transport.
  The engine opens its own capture stream on the return, sends note 69 at
  velocity 127, estimates the noise floor over the first 8 ms, and takes the
  first sample well above it as the onset. It listens up to 750 ms (hard
  wall-clock limit 1250 ms). A result **replaces** the stored offset in
  either direction (ba doc #275 P1.3); a failure (MIDI out offline, silent
  return, nothing heard) is shown in the inspector. The ping is audible.

### Devices going missing

The route is kept when a device is missing. A failed patch send, resend,
ping or the inspector's "Re-scan devices" (`CheckDevices`) re-checks both
endpoints and reports `…MidiOutOffline` / `…ReturnInputOffline`. The alert
offers "Re-scan devices" and a device picker. There is no background
watcher: a device that disappears while idle shows Offline only at the next
check.

### Monitor, record, playback source

- Monitoring and record-arm work as on an audio track; the return is the
  input ([audio-input-and-monitoring.md](audio-input-and-monitoring.md),
  [recording.md](recording.md)).
- **Playback source** (`PlaybackSource`, engine-owned, echoed by
  `TrackPlaybackSourceChanged`): **Live** drives the hardware from the
  timeline's MIDI; **Recorded** plays the recorded takes where they exist.
  Inside a take-covered span the outbound notes are skipped, held notes
  released and the live monitor muted, unless the track is armed. A
  finished take switches the track to Recorded automatically.
- Muting the track also stops its hardware notes.

### Realtime bounce

Bounce in place on an external track opens a dialog (input device, channel,
mono). The engine snapshots every track's mute, mutes all but the source,
creates and arms a target audio track on the chosen input, seeks to the
first MIDI note and records in real time until the last note plus a tail,
then restores mutes, mutes the source and reports `TrackBounceCompleted`.
The captured take is placed earlier by the track's round trip
(`apply_take_shift`). `external.bounce` drives the same dialog as one undo
transaction and refuses when the transport is playing, the track has no
MIDI clip, or the input is unknown.

### Offline renders

Mixdown, stems and freeze render recorded takes at the same relative
position as live playback (the offline PDC table folds in the external
offsets). The hardware itself cannot be rendered offline: an external
track renders only the audio clips on it, so a part that was never
recorded or bounced is silent in a mixdown.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Model | `resonance-common` | `ExternalInstrument`, `PlaybackSource` |
| Engine config | `resonance-audio/src/engine/external_instrument.rs` | `set_external_instrument_in_place`, `handle_set_patch`, `check_external_instrument_devices_in_place`, `handle_resend_patches` |
| Ping | `resonance-audio/src/engine/external_instrument_ping.rs` | `handle_detect_latency`, `poll_pending_latency_ping`, `detect_impulse_onset` |
| PDC offsets | `resonance-audio/src/latency.rs` | `add_external_offsets` |
| Outbound gating | `resonance-audio/src/engine/midi/outbound.rs` | `outbound_track_snapshot`, `OutboundTrack::gate_recorded` |
| Realtime bounce | `resonance-audio/src/engine/bounce_realtime.rs` | `PendingBounce`, `poll_pending_bounce` |
| App state / handlers | `resonance-app/src/state/external_instrument.rs`, `update/external_instrument.rs` | `ExternalInstrumentState`, `ExternalInstrumentStatus`, `ExternalInstrumentMessage`, `enable_external_instrument`, `remove_in_app_instrument` |
| Engine events | `resonance-app/src/engine_events/external_instrument.rs` | offline / latency / params echoes |
| View | `resonance-app/src/view/mixer/inspector/external_instrument.rs`, `view/mixer/track_strip.rs` | inspector TRACK → EXTERNAL INSTRUMENT, strip overlay |
| Control | `resonance-app/src/update/control/external.rs` | `require_external`, `bounce` |

## Surfaces

- **GUI:** Add-track menu; Mix inspector → TRACK → EXTERNAL INSTRUMENT
  (MIDI output, audio return, device preset, patch, latency + Detect,
  monitor, arm, playback source, offline alert); ROUTING shows only the
  output picker for these tracks; strip overlay and bounce glyph.
- **Control API / MCP:** `external.enable` / `disable` / `devices` /
  `status` / `set_midi_out` / `set_return` / `set_patch` / `set_latency` /
  `detect_latency` / `set_monitor` / `set_record_arm` /
  `set_playback_source` / `bounce` → `mcp__resonance__external_enable`,
  `…_disable`, `…_devices`, `…_status`, `…_set_midi_out`, `…_set_return`,
  `…_set_patch`, `…_set_latency`, `…_detect_latency`, `…_set_monitor`,
  `…_set_record_arm`, `…_set_playback_source`, `…_bounce`. Also
  `track.add {kind: "external"}`. `set_patch` takes numbers; the device
  preset and named patches are GUI-only.
- **Agent skills:** the user-level `resonance-studio` skill
  (`references/live-instruments.md`) sets up the user's Moog Muse this way
  and asks before `detect_latency` and `bounce`.

## How it's tested

| What | Where | Run |
|---|---|---|
| Engine handlers, ping detector, recorded playback, source handler | `resonance-audio/tests/engine/external_instrument_handlers.rs`, `external_instrument_ping.rs`, `external_recorded_playback.rs`, `playback_source_handler.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Outbound gating, recorded monitor gate | `resonance-audio/tests/mixer/recorded_outbound_gating.rs`, `recorded_monitor_gating.rs` | same |
| Offline offsets + bounce take shift | `resonance-audio/tests/bounce/bounce_external_offsets.rs` | same |
| App state, persistence, add-track, detect button, patch by name | `resonance-app/tests/plugins/external_instrument_*.rs`, `add_external_instrument_track.rs`, `add_track_menu_external_instrument.rs` | `./scripts/run-tests.py -p resonance-app` |
| Inspector + strip | `resonance-app/tests/mixer/mixer_inspector_external_*.rs`, `mixer_strip_external_instrument.rs` | same |
| Playback source | `resonance-app/tests/performance/playback_source_*.rs`; goldens `external_instrument_playback_source_{live,recorded}-wgpu.png`, `external_instrument_named_patch_picker-wgpu.png` | same |
| Control | `resonance-app/tests/control/control_external_instrument.rs` | same |
| By hand | ping and realtime bounce against a real synth (Moog Muse) | live session |

Not covered: a real MIDI port and audio return; the hardware MIDI timing.

## Decisions and deviations

- The external config never duplicates device/channel/monitor/arm; they stay
  on the track so plain-track commands drive them.
- A measurement replaces the stored offset; it used to be clamped up to
  the old value, so a stale 82 ms survived a 12 ms re-measurement.
- Since the mixer cleanup the external section lives inside TRACK, not in
  place of ROUTING (as the prototype showed).

## Known gaps

- Hardware MIDI is sent from the engine thread in ~16 ms steps:
  [sample-accurate-midi-out](../../intents/hardware-io/sample-accurate-midi-out.md).
- Device presets have no in-app editor:
  [device-definition-editor](../../intents/hardware-io/device-definition-editor.md).
