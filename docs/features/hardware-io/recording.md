# Recording

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: none in `docs/history/specs/`. Leads: doc #260 (findings #2, #4, #17), code review RT-01 / RT-02 / RT-08 / RT-17 / ENG-03 / STATE-02, design doc D-6 §4.2 / §7a, ba doc #292, W3 (cycle-record mode).

Arm one or more tracks and press Record. Audio tracks stream their input
straight to a WAV in the project's `audio/` folder; armed instrument tracks
record the MIDI played into them. Record always counts in two bars. Each
take is placed where the performer heard the mix, and the whole recording
run is one undo step. With the loop on and "Distinct takes per loop pass"
enabled, each pass becomes its own take; editing takes is the timeline
area's job ([../timeline/takes-and-comping.md](../timeline/takes-and-comping.md)).

## Behaviour

### Starting

- The Record button (and `R`) works only when at least one track is armed;
  otherwise it is greyed out. `Shift+R` arms the selected tracks.
- Record is refused while an offline render runs (`TransportRefused`).
- The engine opens the whole session **before** anything rolls
  (`open_recording_session`): the input stream (one source: the first armed
  track that names a device, see
  [audio-input-and-monitoring.md](audio-input-and-monitoring.md)), one take
  file per armed track that captures audio, and the cycle-record cut.
- Each take needs a clip id from the app's grant. If the grant cannot cover
  every capturing track, nothing is captured, the transport still rolls and
  a `Busy` error banner says "no clip ids available". If the input stream
  fails to open, the transport also rolls with an error banner.
- A project directory is required (the startup modal guarantees one).

### Count-in

- The GUI always sends `precount_bars = 2` (`TransportState::precount_bars`
  defaults to 2 and nothing changes it). The engine keeps the playhead where
  Record was pressed, renders two bars of clicks from its own counter
  (whether or not the metronome is on) and the audio thread starts the take
  on the count-in's last frame (`count_in_record_arm`: `ARMED → FIRING →
  FIRED`). Stop or Pause during the count-in discards the empty session.
- The Performance view shows a count-in beat cue (`view/performance/beat_cue.rs`).

### Audio takes

- Files: `audio/clip_<id>.wav`, stereo 32-bit float at the engine rate.
  Capture at another rate is resampled while it is written. The engine
  thread drains a 10-second ring (`RECORDING_RING_SECONDS`, sized in whole
  frames for the stream's channel count) about 60 times a second.
- **Placement.** The first pushed frame latches the playhead
  (`recording_start_latch`). The take starts at that latch minus the graph's
  capture latency, playback latency, and the plugin-delay compensation
  latched at session open (`apply_recording_start_latch`). A take that would
  start before 0 drops its leading input frames.
- **Overflow.** If the ring overflows, whole frames are dropped and one
  `RecordingOverflow` per take raises the banner "Recording overflow: N
  input frames were dropped — this take is missing audio and may be out of
  sync".
- **Disk errors.** A failed write keeps what reached disk: the take is
  finalized up to the failure (or salvaged in memory) and reported, not
  dropped (code review ENG-03).
- A take with zero frames is deleted and makes no clip.
- Clips are named "Recording <n>", numbered per track.

### Loop and cycle record

With the loop enabled and a real range set:

- **Distinct takes per loop pass on** (`loop_record_mode`, project state,
  undoable): the engine cuts audio by input-frame count at each pass
  boundary and MIDI at the playhead wrap, emits `TakeCaptured` per pass and
  starts a new file for the next pass. The trailing pass is filed at Stop.
- **Off:** one clip, no take group. It is trimmed non-destructively to
  the loop range measured from the record start (`trim_start = loop_in −
  start`, `trim_end` at `loop_out − start`), so it shows only the first
  pass through the range; any pre-roll before `loop_in` and all audio from
  later passes stay in the file outside the trim (code reading of
  `finalize_recording` in `recording.rs`; no test pins this). The Settings
  hint says the run "is merged into one clip for the whole loop", which
  overstates it.
- If Record is pressed *inside* the loop (start after `loop_in`), the clip
  is still placed at `loop_in` with `trim_start` saturating to 0, so the
  take would sit early by `start − loop_in` (code reading, unmeasured).
- There is no punch-in/out without looping: the old `punch_*` project
  fields are read as the loop range (`#[serde(alias = "punch_in")]`).

### MIDI recording

- An armed instrument track records live MIDI whenever the transport is
  playing, with Play as well as Record. The first note-on opens a MIDI clip;
  each note is placed at its arrival time (the drain delay is subtracted,
  `handle_record_midi_event`). Held notes are closed at Stop.
- Only notes are recorded. Controllers, pitch bend and aftertouch reach the
  instrument live but are not recorded or sent Thru.
- An armed instrument track opens no take file and does not capture audio.

### Finishing

- Stop or Pause finalizes: remaining frames are drained, files closed and
  memory-mapped, clips published to the mixer in one graph swap, and
  `RecordingFinished` / `TakeCaptured` sent to the app.
- The app records each recording run as one undo entry
  (`record_recording_edit`), marks the project dirty and bumps the control
  revision.
- On an external-instrument track a finished take switches the playback
  source to Recorded (no undo entry of its own).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Record / count-in / session | `resonance-audio/src/engine/transport.rs` | `handle_record_with`, `open_recording_session`, `poll_precount`, `settle_count_in_arm`, `apply_recording_start_latch`, `poll_recording_capture` |
| Take files, drain, finalize | `resonance-audio/src/recording.rs` | `RecordingState`, `create_track_buf`, `drain_ring_to_buffers`, `finalize_recording`, `roll_audio_pass`, `apply_take_shift` |
| Ring size | `resonance-audio/src/engine/recording_ring.rs` | `recording_ring_len` |
| Count-in clicks | `resonance-audio/src/mixer/click.rs` | `render_count_in_clicks` |
| MIDI capture | `resonance-audio/src/engine/midi/live.rs` | `handle_record_midi_event`, `close_open_recordings`, `capture_loop_record_midi_pass` |
| App transport | `resonance-app/src/update/transport.rs`, `state/transport.rs` | `TransportMessage::Record`, `record_pending`, `loop_record_mode` |
| App events | `resonance-app/src/engine_events/transport.rs`, `engine_events/clips.rs` | `recording_started`, `recording_overflow`, `recording_finished` |

Invariants: the audio thread only flips atomics and pushes whole frames;
file I/O and resampling stay on the engine thread. Clip ids come only from
the app's grant.

## Surfaces

- **GUI:** transport Record button (red when any track is armed), arm
  buttons on track headers and strips, Settings → General → Recording →
  "Distinct takes per loop pass". Shortcuts: `R` record, `Shift+R` arm
  selected (Pro Tools preset: `3`).
- **Control API / MCP:** `transport.set_loop_record_mode` →
  `mcp__resonance__transport_set_loop_record_mode` (`distinct_takes`).
  `external.set_record_arm` → `mcp__resonance__external_set_record_arm`
  arms external-instrument tracks only. There is no method to start
  recording or to arm an audio/instrument track:
  [record-control-api](../../intents/hardware-io/record-control-api.md).
- **Agent skills:** the user-level `resonance-studio` skill tells the user to
  set input, mono, monitor and arm by hand for this reason.

## How it's tested

| What | Where | Run |
|---|---|---|
| Take placement, count-in hand-off, cycle cuts | `resonance-audio/tests/engine/recording_alignment.rs`, `loop_record_takes.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Start latch, drain, overflow event | `resonance-audio/tests/mixer/recording_start_latch.rs`, `recording_drain.rs`, `recording_overflow_event.rs`, `recording_whole_frame_push.rs` | same |
| Disk-full salvage (own binary, rlimit) | `resonance-audio/tests/recording_write_failure.rs` | same |
| Live MIDI record / stop | `resonance-audio/tests/midi_hw/live_record_stop.rs`, `live_arrival_offset.rs` | same |
| Undo + dirty per run | `resonance-app/tests/timeline/recording_undo.rs` | `./scripts/run-tests.py -p resonance-app` |
| Overflow banner | `resonance-app/tests/mixer/recording_overflow_banner.rs` | same |
| Loop-record mode persistence | `resonance-app/tests/io/loop_record_mode_persistence.rs` | same |
| By hand | record from a real interface and check the take against the grid | live session |

Not covered: a real capture device end to end, the non-cycle loop trim, the
count-in click sound, and the two-bar count-in default.

## Decisions and deviations

- The session opens before the transport rolls. It used to flip `playing`
  first, so the input-open delay (up to 500 ms) landed inside every take
  (doc #260 finding #2).
- The take starts on the count-in's last frame on the audio thread (RT-08);
  before, it waited for the engine thread and an input rebuild.
- Cycle passes are cut by sample count, not by watching the playhead wrap,
  so a late engine tick cannot move a cut (RT-02).
- Punch was folded into the loop range; there is no separate punch region.

## Known gaps

- With "Distinct takes" off, a looped recording keeps only the first pass
  and a start inside the loop looks misplaced (see Loop and cycle record
  above); the Settings hint promises a merge.
- Fixed two-bar count-in, no punch without cycling:
  [count-in-and-punch](../../intents/hardware-io/count-in-and-punch.md).
- No control-API recording: [record-control-api](../../intents/hardware-io/record-control-api.md).
- MIDI controllers are not recorded:
  [midi-controller-data](../../intents/hardware-io/midi-controller-data.md).
- One capture device per session:
  [audio-input-routing](../../intents/hardware-io/audio-input-routing.md).
