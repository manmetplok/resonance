# Recording

> Verified against master @ `acac5ea3` on 2026-10-15.
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
  sync". The dropped frames are written back as silence where they were
  lost — after the frames that were queued when the drain learned of the
  drop — so the take keeps wall-clock time and every later cycle-record
  cut stays on its seam. The producers add to a never-reset counter
  (`SharedState::recording_dropped_total`, beside the per-take
  `recording_overflow`); the drain reads how far it moved
  (`RecordingState::track_dropped_frames`, `drain_ring_to_buffers`). The
  drain used to count only popped frames, so after a drop the take came
  up short and every later seam moved by the drop's length (BUG-23).
- **Disk errors.** A failed write keeps what reached disk: the take is
  finalized up to the failure (or salvaged in memory) and reported, not
  dropped (code review ENG-03).
- A take with zero frames is deleted and makes no clip.
- Clips are named "Recording <n>", numbered per track.

### Loop and cycle record

With the loop enabled and a real range set:

- **Distinct takes per loop pass on** (`loop_record_mode`, project state,
  undoable): the engine cuts audio by input-frame count at each pass
  boundary and MIDI at the audio thread's loop wrap, emits `TakeCaptured`
  per pass and starts a new file for the next pass. The trailing pass is
  filed at Stop.
- **MIDI pass boundaries.** The audio callback counts every loop wrap
  (`SharedState::loop_wrap_count`, bumped at the seam after the block's
  live-MIDI pickup) and stamps each live event it forwards with that count
  and the playhead of the picking-up block (`ForwardedLiveMidi`). The
  engine thread files a note into the pass its stamp names: a note stamped
  past the open pass rolls it first, and the engine loop rolls a pass
  nobody played into once it has read a newer count *before* draining the
  forwarded events (`poll_loop_record_seam`), so no note of the old pass
  is left to land in the new one. Several wraps with no note between them
  roll once; an empty pass makes no MIDI take. A later pass's note whose
  rewound press time falls just before `loop_in` is held at `loop_in`
  (BUG-22). It used to roll when the engine's ~16 ms poll saw the playhead
  lower than last time, so boundaries jittered by a tick, a note played
  just after the wrap could land in the old pass, and a backwards seek
  rolled a pass.
- **Seek or loop move mid-run.** A seek, a MIDI-clock song-position
  relocate, or a change to the loop range while a cycle-record run is
  recording ends the current pass there (`restart_cycle_record_pass`): its
  audio and MIDI halves roll together (held notes close at the pre-move
  playhead), filed under the slot they were recorded against. The cut then
  re-arms against the range as it stands (`RecordingState::rearm_cycle`)
  and the start latch is re-armed, so the input callback's next push
  latches the real post-move playhead: the next pass is placed and cut like
  a punch-in pass, and every pass after it is a whole loop of the new
  range. Re-sending the same range is not a move. With the loop switched
  off the rest of the run is one uncut pass, filed under the old slot.
  Frames pushed between the roll and the re-armed latch (at most one push)
  land in the next pass. The session used to ignore all of this and keep
  cutting one loop length from the punch-in against the old range (BUG-20).
- **Off:** one clip, no take group. It is trimmed non-destructively to
  the loop range measured from the record start (`trim_start = loop_in −
  start`, `trim_end` at `loop_out − start`), so it shows only the first
  pass through the range; any pre-roll before `loop_in` and all audio from
  later passes stay in the file outside the trim (`finalize_recording` in
  `recording.rs`). The Settings hint says only the first pass is kept
  (it used to promise a merge, BUG-68).
- The clip is placed at `max(loop_in, start)`: Record pressed *inside* the
  loop has nothing before it to trim and lands on the punch-in (BUG-69; it
  used to be placed at `loop_in`, early by `start − loop_in`).
- Record pressed at or after `loop_out` never wraps (the seam fires only on
  a raw playhead before `loop_out`), so the take gets no loop trim and is
  kept whole. The test is on the raw latched playhead, not the
  latency-compensated start. It used to compute an empty trim range and
  delete the take's file (BUG-74).
- There is no punch-in/out without looping: the old `punch_*` project
  fields are read as the loop range (`#[serde(alias = "punch_in")]`).

### MIDI recording

- An armed instrument track records live MIDI whenever the transport is
  playing, with Play as well as Record. The first note-on opens a MIDI clip;
  each note is placed at its arrival time: the playhead of the audio block
  that picked it up, minus the time from the MIDI callback to that pickup
  (`ForwardedLiveMidi::press_sample`, `handle_record_midi_event`). Held
  notes are closed at Stop.
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
| Record / count-in / session | `resonance-audio/src/engine/transport.rs` | `handle_record_with`, `open_recording_session`, `poll_precount`, `settle_count_in_arm`, `apply_recording_start_latch`, `poll_recording_capture`, `restart_cycle_record_pass` |
| Take files, drain, finalize | `resonance-audio/src/recording.rs` | `RecordingState`, `create_track_buf`, `drain_ring_to_buffers`, `finalize_recording`, `roll_audio_pass`, `apply_take_shift` |
| Ring size | `resonance-audio/src/engine/recording_ring.rs` | `recording_ring_len` |
| Count-in clicks | `resonance-audio/src/mixer/click.rs` | `render_count_in_clicks` |
| MIDI capture | `resonance-audio/src/engine/midi/live.rs`, `engine/transport.rs`, `mixer/live_midi.rs` | `handle_record_midi_event`, `close_open_recordings`, `capture_loop_record_midi_pass`, `poll_loop_record_seam`, `loop_record_midi_pass_for`, `ForwardedLiveMidi` (stamped in `pickup_live_midi`) |
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
| Take placement, count-in hand-off, cycle cuts, non-cycle loop trim | `resonance-audio/tests/engine/recording_alignment.rs`, `loop_record_takes.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Start latch, drain, overflow event | `resonance-audio/tests/mixer/recording_start_latch.rs`, `recording_drain.rs`, `recording_overflow_event.rs`, `recording_whole_frame_push.rs` | same |
| Disk-full salvage (own binary, rlimit) | `resonance-audio/tests/recording_write_failure.rs` | same |
| Live MIDI record / stop | `resonance-audio/tests/midi_hw/live_record_stop.rs`, `live_arrival_offset.rs` | same |
| Cycle-record MIDI passes by wrap count and pickup stamp | `resonance-audio/tests/engine/loop_record_midi_seam.rs` | same |
| Seek / loop move during cycle-record | `resonance-audio/tests/engine/recording_alignment.rs` (BUG-20 section) | same |
| Overflow gaps filled with silence, cuts kept | `resonance-audio/tests/engine/recording_alignment.rs` (BUG-23 section) | same |
| Undo + dirty per run | `resonance-app/tests/timeline/recording_undo.rs` | `./scripts/run-tests.py -p resonance-app` |
| Overflow banner | `resonance-app/tests/mixer/recording_overflow_banner.rs` | same |
| Loop-record mode persistence | `resonance-app/tests/io/loop_record_mode_persistence.rs` | same |
| By hand | record from a real interface and check the take against the grid | live session |

Not covered: a real capture device end to end, the
count-in click sound, and the two-bar count-in default.

## Decisions and deviations

- The session opens before the transport rolls. It used to flip `playing`
  first, so the input-open delay (up to 500 ms) landed inside every take
  (doc #260 finding #2).
- The take starts on the count-in's last frame on the audio thread (RT-08);
  before, it waited for the engine thread and an input rebuild.
- Cycle passes are cut by sample count, not by watching the playhead wrap,
  so a late engine tick cannot move a cut (RT-02). MIDI passes follow the
  audio thread's wrap count for the same reason (BUG-22); the two halves
  of a pass can disagree by up to a block about where the seam fell.
- Punch was folded into the loop range; there is no separate punch region.

## Known gaps

- With "Distinct takes" off, a looped recording keeps only the first pass
  (see Loop and cycle record above).
- Fixed two-bar count-in, no punch without cycling:
  [count-in-and-punch](../../intents/hardware-io/count-in-and-punch.md).
- No control-API recording: [record-control-api](../../intents/hardware-io/record-control-api.md).
- MIDI controllers are not recorded:
  [midi-controller-data](../../intents/hardware-io/midi-controller-data.md).
- One capture device per session:
  [audio-input-routing](../../intents/hardware-io/audio-input-routing.md).
