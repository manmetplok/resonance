# Control method catalogue

> Verified against master @ `8eac71b3` on 2026-10-07.
> Source of truth: `resonance_control::methods::capabilities()` and each
> method constant's doc comment in `resonance-control/src/methods/*.rs`.

Every control method the app implements, the MCP tool that exposes it,
and the feature area that owns it. 182 methods, 182 tools. The MCP tool
is always the method name with its first `.` replaced by `_`; in Claude
Code it is `mcp__resonance__<tool>`.

This page lists methods only. What each one does in detail, and its
tests, belong to the owning feature's doc in the linked area folder.
The area column is this area's assignment of a namespace to a feature
area; the sibling areas may refine it.

## Behaviour

- **Kind**
  - `read`: changes nothing.
  - `edit`: changes the open project; one undo entry and one revision
    bump per call.
  - `job`: returns `{job_id}`; see [jobs.md](jobs.md). `meter.*` and
    `master.assist` change nothing but run as jobs.
  - `library`: changes per-user state (preset, model or kit library,
    plugin catalog), not the project. No undo entry, no revision bump,
    no open project needed.
- **Gate:** every method except `control.hello`, `song.*`,
  `plugins.*`, `project.*`, `job.*`, `amp_models.*`, `drum_kits.*`,
  `presets.*` and `track.plugin_params` answers `busy` with no project
  open (see [overview.md](overview.md)).
- Namespaces are listed in `capabilities()` order.

## Catalogue

| Method | MCP tool | Kind | Area | What it does |
|---|---|---|---|---|
| `control.hello` | `control_hello` | read | [control-api](protocol.md) | handshake: version check and the `capabilities` list |
| `song.summary` | `song_summary` | read | [control-api](overview.md) | whole-song overview |
| `song.sections` | `song_sections` | read | [control-api](../compose/sections-and-placements.md) | section definitions + placements |
| `song.tracks` | `song_tracks` | read | [control-api](overview.md) | per-track detail, including clips |
| `song.notes` | `song_notes` | read | [control-api](../timeline/midi-editor.md) | notes of one MIDI clip |
| `song.vocal` | `song_vocal` | read | [compose](../compose/vocal-lyrics-and-melody.md) | lyrics/phonemes of a vocal track |
| `project.new` | `project_new` | job | [project-core](../project-core/save-open-new.md) | start a fresh project |
| `project.open` | `project_open` | job | [project-core](../project-core/save-open-new.md) | open a project from disk |
| `project.save` | `project_save` | job | [project-core](../project-core/save-open-new.md) | save in place, or to `path` on first save |
| `project.save_as` | `project_save_as` | job | [project-core](../project-core/save-open-new.md) | save to a new path |
| `transport.play` | `transport_play` | edit | [timeline](../hardware-io/transport.md) | start playback at the playhead |
| `transport.stop` | `transport_stop` | edit | [timeline](../hardware-io/transport.md) | stop and return to the stop position |
| `transport.pause` | `transport_pause` | edit | [timeline](../hardware-io/transport.md) | pause at the current position |
| `transport.seek` | `transport_seek` | edit | [timeline](../hardware-io/transport.md) | move the playhead |
| `transport.loop_set` | `transport_loop_set` | edit | [timeline](../hardware-io/transport.md) | set the loop region |
| `transport.loop_toggle` | `transport_loop_toggle` | edit | [timeline](../hardware-io/transport.md) | toggle looping |
| `transport.set_tempo` | `transport_set_tempo` | edit | [timeline](../timeline/tempo-and-signature-map.md) | set the song tempo; the arrangement keeps its musical positions |
| `transport.set_time_signature` | `transport_set_time_signature` | edit | [timeline](../timeline/tempo-and-signature-map.md) | set the time signature |
| `transport.set_key` | `transport_set_key` | edit | [timeline](../hardware-io/transport.md) | set the global key where the app exposes one; otherwise key lives per-section (`section.set_scale` |
| `transport.set_loop_record_mode` | `transport_set_loop_record_mode` | edit | [timeline](../hardware-io/recording.md) | set cycle-record mode (a take per loop pass, or one clip trimmed to the first pass) |
| `global.list_events` | `global_list_events` | read | [timeline](../timeline/tempo-and-signature-map.md) | read both global tracks: every tempo change and every meter change in the song, with the bar each takes effect at |
| `global.add_tempo_event` | `global_add_tempo_event` | edit | [timeline](../timeline/tempo-and-signature-map.md) | put a tempo change on the tempo track at a bar |
| `global.add_signature_event` | `global_add_signature_event` | edit | [timeline](../timeline/tempo-and-signature-map.md) | put a meter change on the signature track at a bar |
| `global.edit_tempo_event` | `global_edit_tempo_event` | edit | [timeline](../timeline/tempo-and-signature-map.md) | change the tempo event that already sits on a bar |
| `global.edit_signature_event` | `global_edit_signature_event` | edit | [timeline](../timeline/tempo-and-signature-map.md) | change the meter event that already sits on a bar |
| `global.remove_tempo_event` | `global_remove_tempo_event` | edit | [timeline](../timeline/tempo-and-signature-map.md) | take the tempo event off a bar |
| `global.remove_signature_event` | `global_remove_signature_event` | edit | [timeline](../timeline/tempo-and-signature-map.md) | take the meter event off a bar |
| `track.add` | `track_add` | edit | [mixer](../mixer/channel-strips-and-inspector.md) | add a track |
| `track.rename` | `track_rename` | edit | [mixer](../mixer/channel-strips-and-inspector.md) | rename a track; generated section clips re-derive their names but keep their ids |
| `track.set_color` | `track_set_color` | edit | [mixer](../mixer/track-colour.md) | set a track's identity colour |
| `track.delete` | `track_delete` | edit | [mixer](../mixer/channel-strips-and-inspector.md) | delete a track; destructive, requires `"confirm": true` |
| `track.add_instrument` | `track_add_instrument` | edit | [plugins-instruments](../mixer/plugin-chains.md) | set the instrument by stable plugin id (set semantics: replaces an existing instrument) |
| `track.add_effect` | `track_add_effect` | edit | [mixer](../mixer/plugin-chains.md) | append an effect to the insert chain (instrument plugins are refused) |
| `track.remove_effect` | `track_remove_effect` | edit | [mixer](../mixer/plugin-chains.md) | take an effect off the insert chain |
| `track.move_effect` | `track_move_effect` | edit | [mixer](../mixer/plugin-chains.md) | reorder the insert chain |
| `track.replace_effect` | `track_replace_effect` | edit | [mixer](../mixer/plugin-chains.md) | put a different plugin in a chain slot, keeping its position |
| `track.set_output` | `track_set_output` | edit | [mixer](../mixer/busses-and-output-routing.md) | route a track to master or into a bus |
| `track.add_send` | `track_add_send` | edit | [mixer](../mixer/aux-sends.md) | tap a track into a return bus |
| `track.set_send` | `track_set_send` | edit | [mixer](../mixer/aux-sends.md) | edit an existing send |
| `track.remove_send` | `track_remove_send` | edit | [mixer](../mixer/aux-sends.md) | delete a send |
| `track.plugin_params` | `track_plugin_params` | read | [mixer](../mixer/plugin-chains.md) | a track's plugins and their parameters |
| `track.set_plugin_param` | `track_set_plugin_param` | edit | [mixer](../mixer/plugin-chains.md) | set one plugin parameter |
| `track.set_fx_bypass` | `track_set_fx_bypass` | edit | [mixer](../mixer/plugin-chains.md) | bypass/unbypass a track's whole insert chain |
| `track.set_plugin_bypass` | `track_set_plugin_bypass` | edit | [mixer](../mixer/plugin-chains.md) | bypass/unbypass ONE slot in a track's chain |
| `track.plugin_presets` | `track_plugin_presets` | read | [plugins-effects](../plugins-instruments/plugin-preset-library.md) | a plugin's factory and user presets, and which one is loaded |
| `track.load_plugin_preset` | `track_load_plugin_preset` | edit | [plugins-effects](../plugins-instruments/plugin-preset-library.md) | recall a preset onto a plugin |
| `track.save_plugin_preset` | `track_save_plugin_preset` | edit | [plugins-effects](../plugins-instruments/plugin-preset-library.md) | save a plugin's current settings as a named user preset |
| `track.set_sidechain` | `track_set_sidechain` | edit | [mixer](../mixer/sidechain-routing.md) | route another track's or bus's audio into a plugin's sidechain key |
| `track.clear_sidechain` | `track_clear_sidechain` | edit | [mixer](../mixer/sidechain-routing.md) | remove a plugin's key route |
| `track.save_preset` | `track_save_preset` | edit | [mixer](../ui-shell/preset-browser.md) | capture a dialled-in track as a reusable preset |
| `track.presets` | `track_presets` | read | [mixer](../ui-shell/preset-browser.md) | the track-preset library, built-in and user-saved |
| `track.apply_preset` | `track_apply_preset` | edit | [mixer](../ui-shell/preset-browser.md) | stamp out a new track from a preset |
| `external.enable` | `external_enable` | edit | [hardware-io](../hardware-io/external-instruments.md) | put an existing track into external-instrument mode (removes an in-app instrument at slot 0) |
| `external.disable` | `external_disable` | edit | [hardware-io](../hardware-io/external-instruments.md) | take a track out of external-instrument mode |
| `external.devices` | `external_devices` | read | [hardware-io](../hardware-io/external-instruments.md) | MIDI outputs and audio inputs available on this machine |
| `external.status` | `external_status` | read | [hardware-io](../hardware-io/external-instruments.md) | full external-instrument config + live device status per track |
| `external.set_midi_out` | `external_set_midi_out` | edit | [hardware-io](../hardware-io/external-instruments.md) | pick the hardware MIDI output device and channel |
| `external.set_return` | `external_set_return` | edit | [hardware-io](../hardware-io/external-instruments.md) | pick the audio-return input device and port |
| `external.set_patch` | `external_set_patch` | edit | [hardware-io](../hardware-io/external-instruments.md) | select bank/program on the synth |
| `external.set_latency` | `external_set_latency` | edit | [hardware-io](../hardware-io/external-instruments.md) | manual round-trip latency offset |
| `external.detect_latency` | `external_detect_latency` | edit | [hardware-io](../hardware-io/external-instruments.md) | measure round-trip latency with a MIDI impulse; the result lands asynchronously, read it with `external.status` |
| `external.set_monitor` | `external_set_monitor` | edit | [hardware-io](../hardware-io/external-instruments.md) | hear (or stop hearing) the audio return |
| `external.set_record_arm` | `external_set_record_arm` | edit | [hardware-io](../hardware-io/external-instruments.md) | arm the track so a record pass captures the return |
| `external.set_playback_source` | `external_set_playback_source` | edit | [hardware-io](../hardware-io/external-instruments.md) | play the hardware live, or play the recorded takes |
| `external.bounce` | `external_bounce` | edit | [hardware-io](../hardware-io/external-instruments.md) | capture the hardware to a new audio track in real time |
| `plugins.catalog` | `plugins_catalog` | read | [plugins-effects](../plugins-instruments/third-party-plugin-hosting.md) | every installed plugin |
| `plugins.rescan` | `plugins_rescan` | library | [plugins-effects](../plugins-instruments/third-party-plugin-hosting.md) | look for plugins installed since the app started |
| `mixer.set_volume` | `mixer_set_volume` | edit | [mixer](../mixer/volume-pan-mute-solo.md) | set fader gain |
| `mixer.set_volume_db` | `mixer_set_volume_db` | edit | [mixer](../mixer/volume-pan-mute-solo.md) | set the same fader in dB |
| `mixer.set_pan` | `mixer_set_pan` | edit | [mixer](../mixer/volume-pan-mute-solo.md) | set stereo pan |
| `mixer.set_mute` | `mixer_set_mute` | edit | [mixer](../mixer/volume-pan-mute-solo.md) | mute/unmute |
| `mixer.set_solo` | `mixer_set_solo` | edit | [mixer](../mixer/volume-pan-mute-solo.md) | solo/unsolo |
| `master.summary` | `master_summary` | read | [mixer](../mixer/master-bus.md) | master volume, FX bypass and insert chain |
| `master.set_volume` | `master_set_volume` | edit | [mixer](../mixer/master-bus.md) | set the master fader |
| `master.add_effect` | `master_add_effect` | edit | [mixer](../mixer/plugin-chains.md) | append an effect to the master insert chain |
| `master.remove_effect` | `master_remove_effect` | edit | [mixer](../mixer/plugin-chains.md) | take an effect off the master chain |
| `master.move_effect` | `master_move_effect` | edit | [mixer](../mixer/plugin-chains.md) | reorder the master chain |
| `master.replace_effect` | `master_replace_effect` | edit | [mixer](../mixer/plugin-chains.md) | put a different plugin in a chain slot, keeping its position |
| `master.set_fx_bypass` | `master_set_fx_bypass` | edit | [mixer](../mixer/plugin-chains.md) | bypass/unbypass the whole master chain |
| `master.plugin_params` | `master_plugin_params` | read | [mixer](../mixer/plugin-chains.md) | the master chain's plugins and their parameters |
| `master.set_plugin_param` | `master_set_plugin_param` | edit | [mixer](../mixer/plugin-chains.md) | set one parameter on a master plugin |
| `master.set_plugin_bypass` | `master_set_plugin_bypass` | edit | [mixer](../mixer/plugin-chains.md) | bypass/unbypass ONE slot in the master chain |
| `master.plugin_presets` | `master_plugin_presets` | read | [plugins-effects](../plugins-instruments/plugin-preset-library.md) | a master plugin's factory and user presets |
| `master.load_plugin_preset` | `master_load_plugin_preset` | edit | [plugins-effects](../plugins-instruments/plugin-preset-library.md) | recall a preset onto a master plugin |
| `master.save_plugin_preset` | `master_save_plugin_preset` | edit | [plugins-effects](../plugins-instruments/plugin-preset-library.md) | save a master plugin's current settings as a named user preset |
| `master.set_sidechain` | `master_set_sidechain` | edit | [mixer](../mixer/sidechain-routing.md) | key a plugin on the master chain from a track or bus |
| `master.clear_sidechain` | `master_clear_sidechain` | edit | [mixer](../mixer/sidechain-routing.md) | remove a master plugin's key route |
| `master.assist` | `master_assist` | job | [mixer](../mixer/mastering-assist.md) | run the mastering assistant offline and return suggestions without applying them |
| `bus.create` | `bus_create` | edit | [mixer](../mixer/busses-and-output-routing.md) | add a group bus |
| `bus.delete` | `bus_delete` | edit | [mixer](../mixer/busses-and-output-routing.md) | remove a bus; destructive, requires `"confirm": true` |
| `bus.rename` | `bus_rename` | edit | [mixer](../mixer/busses-and-output-routing.md) | rename a bus |
| `bus.set_volume` | `bus_set_volume` | edit | [mixer](../mixer/busses-and-output-routing.md) | set a bus fader in dB |
| `bus.add_effect` | `bus_add_effect` | edit | [mixer](../mixer/plugin-chains.md) | append an effect to a bus's insert chain |
| `bus.remove_effect` | `bus_remove_effect` | edit | [mixer](../mixer/plugin-chains.md) | take an effect off a bus's chain |
| `bus.move_effect` | `bus_move_effect` | edit | [mixer](../mixer/plugin-chains.md) | reorder a bus's chain |
| `bus.replace_effect` | `bus_replace_effect` | edit | [mixer](../mixer/plugin-chains.md) | put a different plugin in a chain slot, keeping its position |
| `bus.set_fx_bypass` | `bus_set_fx_bypass` | edit | [mixer](../mixer/plugin-chains.md) | bypass/unbypass a bus's whole chain |
| `bus.plugin_params` | `bus_plugin_params` | read | [mixer](../mixer/plugin-chains.md) | a bus's plugins and their parameters |
| `bus.set_plugin_param` | `bus_set_plugin_param` | edit | [mixer](../mixer/plugin-chains.md) | set one parameter on a bus plugin |
| `bus.set_plugin_bypass` | `bus_set_plugin_bypass` | edit | [mixer](../mixer/plugin-chains.md) | bypass/unbypass ONE slot in a bus's chain |
| `bus.plugin_presets` | `bus_plugin_presets` | read | [plugins-effects](../plugins-instruments/plugin-preset-library.md) | a bus plugin's factory and user presets |
| `bus.load_plugin_preset` | `bus_load_plugin_preset` | edit | [plugins-effects](../plugins-instruments/plugin-preset-library.md) | recall a preset onto a bus plugin |
| `bus.save_plugin_preset` | `bus_save_plugin_preset` | edit | [plugins-effects](../plugins-instruments/plugin-preset-library.md) | save a bus plugin's current settings as a named user preset |
| `bus.set_sidechain` | `bus_set_sidechain` | edit | [mixer](../mixer/sidechain-routing.md) | key a plugin on a bus's chain from another track or bus |
| `bus.clear_sidechain` | `bus_clear_sidechain` | edit | [mixer](../mixer/sidechain-routing.md) | remove a bus plugin's key route |
| `edit.undo` | `edit_undo` | edit | [project-core](../project-core/undo-redo.md) | undo the newest edit |
| `edit.redo` | `edit_redo` | edit | [project-core](../project-core/undo-redo.md) | redo the newest undone edit |
| `edit.status` | `edit_status` | read | [project-core](../project-core/undo-redo.md) | what undo/redo would do, without doing it |
| `section.create` | `section_create` | edit | [compose](../compose/sections-and-placements.md) | create a definition; also places it after the last placement unless `place: false` |
| `section.rename` | `section_rename` | edit | [compose](../compose/sections-and-placements.md) | rename a definition |
| `section.resize` | `section_resize` | edit | [compose](../compose/sections-and-placements.md) | change a definition's length |
| `section.delete` | `section_delete` | edit | [compose](../compose/sections-and-placements.md) | delete a definition and its placements; destructive, requires `"confirm": true` |
| `section.place` | `section_place` | edit | [compose](../compose/sections-and-placements.md) | place a definition in the arrangement |
| `section.remove_placement` | `section_remove_placement` | edit | [compose](../compose/sections-and-placements.md) | remove one placement (the definition survives |
| `section.set_scale` | `section_set_scale` | edit | [compose](../compose/sections-and-placements.md) | set a definition's key/scale |
| `section.set_lane_generator` | `section_set_lane_generator` | edit | [compose](../compose/lane-generators.md) | configure (or clear) the generator on a `(section definition, track)` lane |
| `harmony.add_chord` | `harmony_add_chord` | edit | [compose](../compose/section-chords-and-progressions.md) | add a chord |
| `harmony.edit_chord` | `harmony_edit_chord` | edit | [compose](../compose/section-chords-and-progressions.md) | change symbol/position/length of a chord |
| `harmony.delete_chord` | `harmony_delete_chord` | edit | [compose](../compose/section-chords-and-progressions.md) | remove a chord |
| `harmony.apply_progression` | `harmony_apply_progression` | edit | [compose](../compose/section-chords-and-progressions.md) | render a whole progression onto a section's grid |
| `generate.part` | `generate_part` | edit | [compose](../compose/lane-generators.md) | generate a pad/bass/lead part from the section's chords |
| `generate.drums` | `generate_drums` | edit | [compose](../compose/drum-arrangement.md) | generate a drum pattern for a section |
| `notes.insert` | `notes_insert` | edit | [compose](../timeline/midi-editor.md) | insert one note |
| `notes.edit` | `notes_edit` | edit | [compose](../timeline/midi-editor.md) | edit one note in place |
| `notes.delete` | `notes_delete` | edit | [compose](../timeline/midi-editor.md) | delete one note |
| `notes.create_clip` | `notes_create_clip` | edit | [compose](../timeline/midi-editor.md) | create an empty MIDI clip on a track |
| `notes.move_clip` | `notes_move_clip` | edit | [compose](../timeline/midi-editor.md) | reposition a MIDI clip on the timeline |
| `notes.insert_many` | `notes_insert_many` | edit | [compose](../timeline/midi-editor.md) | insert a batch of notes as ONE undoable edit |
| `notes.replace_all` | `notes_replace_all` | edit | [compose](../timeline/midi-editor.md) | replace a clip's whole note list as ONE undoable edit; destructive on a non-empty clip, requires `"confirm": true` there |
| `notes.import_midi` | `notes_import_midi` | edit | [compose](../hardware-io/smf-import.md) | import a Standard MIDI File |
| `pool.list` | `pool_list` | read | [timeline](../timeline/audio-pool-and-import.md) | the project's imported audio assets |
| `pool.import` | `pool_import` | job | [timeline](../timeline/audio-pool-and-import.md) | import audio files into the pool without placing them |
| `pool.remove_unused` | `pool_remove_unused` | edit | [timeline](../timeline/audio-pool-and-import.md) | drop every asset no clip plays; needs `confirm`, one undo restores them |
| `clip.place` | `clip_place` | job | [timeline](../timeline/clip-editing.md) | put a sample on a track |
| `clip.move` | `clip_move` | edit | [timeline](../timeline/clip-editing.md) | reposition a clip, optionally to another track |
| `clip.trim` | `clip_trim` | edit | [timeline](../timeline/clip-editing.md) | change which part of the source a clip plays |
| `clip.delete` | `clip_delete` | edit | [timeline](../timeline/clip-editing.md) | remove a clip from the timeline; destructive, requires `"confirm": true` |
| `clip.split` | `clip_split` | edit | [timeline](../timeline/clip-editing.md) | cut one audio clip in two at a timeline position (non-destructive trims) |
| `clip.set_gain` | `clip_set_gain` | edit | [timeline](../timeline/fades-crossfades-clip-gain.md) | per-clip gain in dB |
| `clip.set_fade` | `clip_set_fade` | edit | [timeline](../timeline/fades-crossfades-clip-gain.md) | fade-in/out lengths and curves |
| `clip.set_warp` | `clip_set_warp` | edit | [timeline](../timeline/warp-and-tempo-detection.md) | switch warp ("follow tempo") on or off and set the source tempo, transpose and algorithm |
| `clip.set_warp_markers` | `clip_set_warp_markers` | edit | [timeline](../timeline/warp-and-tempo-detection.md) | replace a clip's warp-marker set |
| `clip.detect_tempo` | `clip_detect_tempo` | job | [timeline](../timeline/warp-and-tempo-detection.md) | estimate the tempo of a clip's source audio (analysis only) |
| `reference.load` | `reference_load` | edit | [mixer](../mixer/reference-ab-monitoring.md) | add a pooled asset to the project's reference list Undoable, like loading one in the GUI |
| `arrangement.insert_bars` | `arrangement_insert_bars` | edit | [timeline](../timeline/arrangement-bars.md) | open a gap, pushing everything at or after `at_bar` later |
| `arrangement.remove_bars` | `arrangement_remove_bars` | edit | [timeline](../timeline/arrangement-bars.md) | close a gap, pulling later content earlier; needs `confirm` when content starts inside the span |
| `vocal.set_lyrics` | `vocal_set_lyrics` | edit | [compose](../compose/vocal-lyrics-and-melody.md) | replace a vocal track's full lyric text |
| `vocal.set_line` | `vocal_set_line` | edit | [compose](../compose/vocal-lyrics-and-melody.md) | replace one lyric line |
| `vocal.set_pronunciation` | `vocal_set_pronunciation` | edit | [compose](../compose/pronunciation.md) | set a per-word pronunciation override |
| `vocal.clear_pronunciation` | `vocal_clear_pronunciation` | edit | [compose](../compose/pronunciation.md) | remove a per-word override |
| `vocal.render` | `vocal_render` | job | [compose](../compose/vocal-rendering.md) | kick off an SVS render |
| `vocal.generate` | `vocal_generate` | edit | [compose](../compose/vocal-lyrics-and-melody.md) | generate a vocal lane's melody (and lyrics) into its derived clip |
| `render.mixdown` | `render_mixdown` | job | [audio-engine](../audio-engine/offline-render-and-export.md) | bounce the master mix to a WAV file |
| `meter.measure` | `meter_measure` | job | [mixer](../mixer/meter-control-api.md) | measure one slice of the mix |
| `meter.stems` | `meter_stems` | job | [mixer](../mixer/meter-control-api.md) | measure every track plus the master in one pass |
| `meter.snapshot` | `meter_snapshot` | job | [mixer](../mixer/meter-control-api.md) | measure one slice and keep the numbers for a later `meter.compare` |
| `meter.compare` | `meter_compare` | job | [mixer](../mixer/meter-control-api.md) | loudness-matched deltas between two measurements |
| `meter.probe` | `meter_probe` | job | [mixer](../mixer/meter-control-api.md) | harmonic signature of an insert chain |
| `automation.lanes` | `automation_lanes` | read | [timeline](../timeline/automation-control-api.md) | read every lane, or a filtered subset |
| `automation.set_lane` | `automation_set_lane` | edit | [timeline](../timeline/automation-control-api.md) | create a lane or replace ALL of its points |
| `automation.add_points` | `automation_add_points` | edit | [timeline](../timeline/automation-control-api.md) | insert points, upserting an occupied frame |
| `automation.delete_points` | `automation_delete_points` | edit | [timeline](../timeline/automation-control-api.md) | delete by index or by range |
| `automation.set_enabled` | `automation_set_enabled` | edit | [timeline](../timeline/automation-control-api.md) | the lane's Read flag, declaratively |
| `automation.remove_lane` | `automation_remove_lane` | edit | [timeline](../timeline/automation-control-api.md) | delete the whole lane |
| `automation.shape` | `automation_shape` | edit | [timeline](../timeline/automation-control-api.md) | generate a ramp / curve / LFO over a range |
| `midi_map.bindings` | `midi_map_bindings` | read | [hardware-io](../hardware-io/midi-learn.md) | every binding, the armed learn target and the control-surface port |
| `midi_map.learn` | `midi_map_learn` | edit | [hardware-io](../hardware-io/midi-learn.md) | arm learn for one target: the next control the user moves is bound to it |
| `midi_map.cancel_learn` | `midi_map_cancel_learn` | edit | [hardware-io](../hardware-io/midi-learn.md) | disarm learn without binding anything |
| `midi_map.clear` | `midi_map_clear` | edit | [hardware-io](../hardware-io/midi-learn.md) | remove one binding by id |
| `midi_map.clear_all` | `midi_map_clear_all` | edit | [hardware-io](../hardware-io/midi-learn.md) | remove every binding |
| `amp_models.list` | `amp_models_list` | read | [plugins-effects](../plugins-instruments/nam-model-library.md) | the installed models |
| `amp_models.set_marks` | `amp_models_set_marks` | library | [plugins-effects](../plugins-instruments/nam-model-library.md) | favourite or tag one NAM model (per-user, no undo, no revision bump) |
| `drum_kits.list` | `drum_kits_list` | read | [plugins-instruments](../plugins-instruments/drum-kit-library.md) | the installed kits |
| `drum_kits.set_marks` | `drum_kits_set_marks` | library | [plugins-instruments](../plugins-instruments/drum-kit-library.md) | favourite or tag one kit (per-user, no undo, no revision bump) |
| `presets.search` | `presets_search` | read | [plugins-effects](../plugins-instruments/plugin-preset-library.md) | find presets across every plugin, or one, before a plugin is on a track |
| `presets.rename` | `presets_rename` | library | [plugins-effects](../plugins-instruments/plugin-preset-library.md) | rename a user preset; its id does not change |
| `presets.delete` | `presets_delete` | library | [plugins-effects](../plugins-instruments/plugin-preset-library.md) | move a user preset to the trash (recoverable for 30 days); needs `confirm` |
| `presets.set_marks` | `presets_set_marks` | library | [plugins-effects](../plugins-instruments/plugin-preset-library.md) | favourite or tag one preset, factory ones included (per-user, no undo, no revision bump) |
| `presets.update_meta` | `presets_update_meta` | library | [plugins-effects](../plugins-instruments/plugin-preset-library.md) | edit a user preset's own metadata (refused on factory presets) |
| `presets.vocabulary` | `presets_vocabulary` | read | [plugins-effects](../plugins-instruments/plugin-preset-library.md) | the seeded facet values plus the ones in use, for consistent tagging |
| `job.status` | `job_status` | read | [control-api](jobs.md) | read a job's current state |
| `job.wait` | `job_wait` | read | [control-api](jobs.md) | block until the job is terminal or the timeout elapses; returns the then-current status |

### Defined but not listed

| Method | Status |
|---|---|
| `render.stems` | Wire types (`StemsParams`, `StemsResult`) and a handler that answers `unsupported` exist; kept out of `capabilities` and has no MCP tool. See [render-stems-control](../../intents/control-api/render-range-and-stems.md). |

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Method names and types | `resonance-control/src/methods/<namespace>.rs` | `pub const`, `METHODS` |
| Capability list | `resonance-control/src/methods/mod.rs` | `capabilities()` |
| App handlers | `resonance-app/src/update/control/<namespace>.rs` (and `track/`, `automation/`, `clip/` subfolders) | `try_handle` per namespace |
| MCP tools | `resonance-mcp/src/tools/*.rs` | `async fn <tool>` |

## Surfaces

- **Control API / MCP:** the table above. `control_hello` returns the
  same list as dotted names for the running build.

## How it's tested

| What | Where | Run |
|---|---|---|
| Every capability has exactly one tool, aliases excluded | `resonance-mcp/tests/translation.rs::combined_router_exposes_every_control_method` | `./scripts/run-tests.py -p resonance-mcp` |
| Every gated method is refused `busy` with no project | `resonance-app/tests/control/control_mutation_gate.rs` | `./scripts/run-tests.py -p resonance-app` |
| Every `track.*`/`mixer.*` method reaches a handler | `resonance-app/tests/control/control_track_dispatch.rs` | same |
| Every mutating method replies with the same ack shape | `resonance-app/tests/control/control_reply_contract.rs` | same |
| Per-namespace behaviour | `resonance-app/tests/control/control_<namespace>*.rs` | same |

Not covered: nothing checks that a method outside `track.*`/`mixer.*` is
routed (an arm dropped from another namespace's dispatch would answer
"method not implemented yet").

## Decisions and deviations

- `kind` and `area` here are derived from the doc comments and the gate
  lists, not from a field in the code.
- `track.plugin_params` is answered by `update/control/song.rs` above the
  mutation gate, unlike its `bus`/`master` twins, which are gated.
- The doc comment on `track::PLUGIN_PRESETS` in
  `resonance-control/src/methods/track.rs` begins with text that belongs
  to `SET_SIDECHAIN` (the sidechain description was pasted above the
  wrong constant), and `SET_SIDECHAIN` has none.

## Known gaps

- `render.stems` (above).
- Drum pad map:
  [drum-pad-map-mcp](../../intents/control-api/drum-pad-map-mcp.md).
