# Transport and metronome

> Verified against master @ `ef215b08` on 2026-10-15.
> Spec history: none in `docs/history/specs/`; `docs/history/specs/command-palette.md` §5 lists the transport shortcuts. Leads: ba doc #275 P1.4 (tempo re-anchoring), ba doc #286 §2, CTL-01, FU-D6a.

The transport bar plays, pauses, stops, records, loops and seeks, holds the
song's starting tempo, meter and key, and switches the metronome click. The
same controls are on the keyboard, the command palette and the control API.
The full tempo and meter map (changes after bar 1) is edited on the global
tracks; see [../timeline/tempo-and-signature-map.md](../timeline/tempo-and-signature-map.md).

## Behaviour

### Play, pause, stop

- **Play** starts at the playhead. In the Compose view with a section
  selected, Play first loops that section (sets and enables the loop range
  and seeks to its start). A fresh Play starts a new live-MIDI capture run
  for undo purposes.
- **Toggle play** (`Space`) stops and returns to where playback started
  (`play_start`); **Play/Pause** (`Shift+Space`) keeps the position.
- **Pause** keeps the playhead. **Stop** sets it to 0.
- Pause and Stop both finish a recording, release held notes on instrument
  plugins and send all-notes-off to hardware MIDI outputs, rest pitch bend,
  mod wheel and sustain on all 16 channels of both (BUG-24; the loop seam's
  note release leaves controllers alone), close open MIDI
  recordings and send MIDI clock Stop (Stop also sends Song Position 0).
- Seeking while playing releases held notes the same way.
- Live notes the engine thread parked while an instrument's lock was busy
  (`HandlerState::live_note_stash`) predate that release: a released
  instrument has them discarded, one whose lock was still busy keeps only
  its owed note-offs. A parked note-on used to be replayed after the
  release and hang (BUG-36, `panic_all_instrument_plugins` in
  `engine/transport.rs`).
- Skip back/forward buttons move ±5 s. Bar and beat nudges, marker and
  section jumps, Home/End and loop-point keys are listed below.
- Play and Record are refused while an offline render owns the plugins.

### Loop

- The loop toggle enables cycling. With no range set yet, it creates one of
  two bars from the playhead (computed from the flat BPM and numerator).
- The range is dragged on the ruler (snapped to the grid), set with `I` /
  `O` at the playhead, `Cmd+L` from a selection, or `Shift+L` for the
  section at the playhead. Each loop-point edit is one undo entry; the
  range and on/off state are saved in the project.

### Tempo, meter, key

- **Tempo:** typing in the BPM field and pressing Enter commits it (a press
  elsewhere reverts uncommitted text; NaN and out-of-range values are
  rejected by `sanitize_bpm`). It sets the bar-1 tempo event. Every clip
  start, automation point, marker, the loop range and the playhead are
  re-anchored to keep their bar positions; audio clip lengths are not
  stretched.
- **Time signature:** the meter button cycles 4/4 → 3/4 → 6/8 → 5/4 → 7/8 →
  2/4 → 4/4. `transport.set_time_signature` sets any legal meter. Both only
  rewrite the bar-1 signature event.
- **Key:** the transport shows the chord track's song key;
  `transport.set_key` sets it (tonic + mode) through the chord track's
  `SetSongKey`. Per-section scales are a compose feature.

### Metronome

- `K` (or the metronome button) toggles the click; the state is saved in the
  project. Clicks are 20 ms sine bursts, 1500 Hz on the downbeat and 1000 Hz
  otherwise, placed from the tempo map and added after the master FX, before
  master volume (`mixer/click.rs`). They are not delayed by plugin-delay
  compensation, so with latency-adding plugins the click leads the mix.
- The record count-in uses the same click regardless of the toggle
  ([recording.md](recording.md)).
- There is no click volume, sound choice or "click only while recording"
  option.

### MIDI clock

Send and receive are set in Settings → General → MIDI Clock; see
[midi-hardware-io.md](midi-hardware-io.md#midi-clock).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| App state | `resonance-app/src/state/transport.rs` | `TransportState` (`play_start`, `loop_*`, `bpm_editing`, `precount_bars`) |
| App handlers | `resonance-app/src/update/transport.rs`, `update/transport_nav.rs`, `update/tempo_reanchor.rs` | `TransportMessage`, `seek_to`, `set_loop_point`, `musical_anchors`, `reanchor_to_tempo` |
| Engine | `resonance-audio/src/engine/transport.rs`, `engine/loop_range.rs` | `handle_play`, `handle_pause`, `handle_stop`, `handle_seek_to`, `handle_set_loop_range` |
| Click | `resonance-audio/src/mixer/click.rs` | `render_metronome_clicks`, `render_count_in_clicks` |
| View | `resonance-app/src/view/transport.rs` | `record_button`, `loop_toggle_button`, `metronome_toggle_button` |
| Control | `resonance-app/src/update/control/transport.rs` | `resolve_position`, `set_tempo`, `set_key` |
| Shortcuts | `resonance-app/src/commands/bindings.rs` | default + Logic / Pro Tools presets |

`transport.set_tempo` drives the GUI's own two-step text path
(`SetBpmText` + `CommitBpm`) so the undo entry matches a typed edit.

## Surfaces

- **GUI:** transport bar: skip back, stop, play/pause, record, skip forward,
  marker prev/next and overview, position, BPM field, meter button, key,
  loop and metronome toggles, CPU readout.
- **Keyboard (default preset):** `Space` play/stop, `Shift+Space`
  play/pause, `Alt+Space` play from loop start, `R` record, `L` loop, `K`
  metronome, `Home`/`End`, `[` / `]` to loop start/end, `I` / `O` set loop
  points, `←`/`→` bar nudge, `Alt+←/→` beat nudge, `,` / `.` markers,
  `Cmd+J` go to bar. Pro Tools preset: `0`, `3`, `4`, `7`.
- **Control API / MCP:** `transport.play` / `stop` / `pause` / `seek` /
  `loop_set` / `loop_toggle` / `set_tempo` / `set_time_signature` /
  `set_key` / `set_loop_record_mode` → `mcp__resonance__transport_play`,
  `…_stop`, `…_pause`, `…_seek`, `…_loop_set`, `…_loop_toggle`,
  `…_set_tempo`, `…_set_time_signature`, `…_set_key`,
  `…_set_loop_record_mode`. Positions are 1-based bar + beat (the beat is
  the meter's beat unit) or an absolute sample; one resolver serves every
  positional method. No method for Record or the metronome.
- **Agent skills:** `song-structure` and `drumming` warn that
  `transport_set_time_signature` only rewrites bar 1 and point to the
  `global.*` methods for meter changes.

## How it's tested

| What | Where | Run |
|---|---|---|
| Play/stop return, loop points, nudges, shortcuts | `resonance-app/tests/timeline/transport_control.rs` | `./scripts/run-tests.py -p resonance-app` |
| BPM field validation | `resonance-app/tests/timeline/bpm_input_validation.rs` | same |
| Control methods, position resolution | `resonance-app/tests/control/control_transport.rs`, `control_position_beats.rs` | same |
| Engine tempo, seek race, loop snapshot and seam | `resonance-audio/tests/engine/tempo_handlers.rs`, `playhead_seek_race.rs`, `resonance-audio/tests/mixer/loop_range_snapshot.rs`, `loop_seam_edges.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Beat position math | `resonance-audio/tests/types/transport_pos_beats.rs` | same |
| Live-note stash across Stop / Pause / Seek | `resonance-audio/tests/mixer/midi_stash.rs` (`live_stash_across_panic`) | same |
| Panic controller resets (plugin, hardware) | `resonance-audio/tests/clap_host/clap_note_dialect.rs`, `tests/midi_hw/midi_hardware_emit.rs` | same |

Not covered: the click synthesis and its timing have no direct test.

## Decisions and deviations

- Tempo edits keep content on its bars (ba doc #275 P1.4); before, clip
  starts stayed in samples and slid off their bars.
- `transport.set_key` is supported because the app's global key *is* the
  chord track's song key.
- The method doc for `transport.set_tempo` (`resonance-control/src/methods/transport.rs`)
  points to `clip.set_stretch`, which does not exist; stretching is
  `clip.set_warp`.

## Known gaps

- No control method to record or toggle the metronome:
  [record-control-api](../../intents/hardware-io/record-control-api.md).
- Count-in length is fixed:
  [count-in-and-punch](../../intents/hardware-io/count-in-and-punch.md).
