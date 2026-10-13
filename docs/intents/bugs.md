# Known defects

Behaviour defects that exist in the code today (checked against master
`8eac71b3`, 2026-10-07). Each row is a real, user- or agent-visible fault —
wrong sound, lost data, a control that does nothing, a security leak — not a
missing feature. Missing features are intents in the area folders; when a bug
is part of a larger intent, the Source column links it.

Rows marked *(likely)* were traced by reading code after their fix batch
merged, but not reproduced. Fix any row the usual way: write the failing test
first. When a bug is fixed, delete its row.

Paths: `app/` = `resonance-app/src/`, `audio/` = `resonance-audio/src/`.

## Mixer and routing

| ID | Area | Symptom | Where (file:line) | Source |
|---|---|---|---|---|
| BUG-01 | mixer | Every bus made with the GUI "+ Bus" button is named "Bus 2000000000", "Bus 2000000001", … (its raw id). | `app/update/bus.rs:96` sends `name: None`; `audio/engine/busses.rs:38` formats `Bus {id}`; ids start at `BUS_ID_BASE` (`app/state/ids.rs:131`); `app/engine_events/tracks.rs:371-382` stores the name as-is | area docs 2026-10-07 |
| BUG-02 | mixer | The S button on a sub-track strip lights up, saves and undoes, but changes nothing audible. | `app/view/mixer/track_strip.rs:662-666`; `audio/mixer/render/strategy.rs:56` (`sub_track_silenced` ignores solo); `audio/types/track.rs:611` (`any_top_level_solo` skips sub-tracks) | intent [mixer/sub-track-solo.md](mixer/sub-track-solo.md) |
| BUG-03 | mixer | The reference loudness-match toggle has no audible effect; its readout is always +0.0 dB. | `audio/engine/reference.rs:73` (`offset_db`), only ever set to 0.0 at `:88` | intent [mixer/reference-loudness-match.md](mixer/reference-loudness-match.md) |
| BUG-04 | mixer | A bus→bus send into a lower-indexed (older) bus passes validation and is saved, but is silent. | `audio/mixer/render/bus_pass.rs:152` (`dst_idx > src_idx`); `audio/mixer/render/routing.rs:123-133` | intent [mixer/bus-to-bus-routing.md](mixer/bus-to-bus-routing.md) |
| BUG-05 | mixer | A bus plugin's `FxBypassChanged` echo has no owed-removal guard (tracks have one), so a stale echo can re-apply to a removed/replaced slot. | `app/engine_events/plugins.rs:1148-1152` | review 2026-10-02 FU-A1b2 |

## Timeline and clips

| ID | Area | Symptom | Where (file:line) | Source |
|---|---|---|---|---|
| BUG-10 | timeline | Select a MIDI clip, then press an audio clip and hit Delete: the MIDI clip is deleted, not the audio clip. | `app/update/clips.rs:407,473,587,653` set `selected_clip` without clearing `selected_midi_clip`; `app/commands/resolve.rs:541` and `app/view/timeline/input/keyboard.rs:44` check MIDI first | intent [timeline/clip-multi-select.md](timeline/clip-multi-select.md) (open decision 1) |
| BUG-11 | timeline | "Freeze selected tracks" freezes only the primary track, not the multi-selection. | `app/update/freeze.rs:398-405` reads only `selected_track` (doc comment at 395-397 is stale) | area docs 2026-10-07 |
| BUG-12 | timeline | An audio clip can be dragged onto an instrument or vocal lane, where it plays nothing. | `app/update/clips.rs:429,459-468` (no track-type filter); engine accepts it (`audio/engine/clips.rs:282-299`); instrument tracks never mix clips (`audio/mixer/render/track_pass.rs:570`) | area docs 2026-10-07 |
| BUG-13 | timeline | Clip-inspector gain is limited to −60…+12 dB while drag and API allow +24; a clip above +12 snaps down on any stepper press or typed value. | `app/view/clip_inspector.rs:42-43` (clamps at 267, 279, 292) vs `audio/engine/clips.rs:463-464` | area docs 2026-10-07 |
| BUG-14 | timeline | A clip's fade handle can only be grabbed at the clip corner, not at the fade's real end, once a fade is set; frozen clips are treated as fadeable. | `app/view/timeline/input/hit_test.rs` (`clip_handles` passes fade length 0) | review 2026-10-02 FU-W2d |
| BUG-15 | timeline | The warp-marker strip shadows the clip's trim handle. | `app/view/clip_inspector/warp.rs` / timeline hit-test | review 2026-10-02 FU-W2e · intent [audio-engine/clip-warp-render.md](audio-engine/clip-warp-render.md) |
| BUG-16 | timeline | Undoing a clip relink restores the old link but not the silence of the missing file (the clip keeps playing the relinked audio). | `app/update/relink.rs` (undo of a relink) | review 2026-10-02 FU-W4e |
| BUG-17 | timeline | Typing a BPM in the transport field while the playhead is past a tempo change rewrites the song-start tempo (bar 0), not the tempo shown. | `app/update/transport.rs:265-284` (`CommitBpm` writes `tempo_events[0]`) | review 2026-10-02 FU-A1a5 |

## Recording and hardware

| ID | Area | Symptom | Where (file:line) | Source |
|---|---|---|---|---|
| BUG-20 | hardware-io | Seeking or moving the loop range during cycle-record is ignored by the take session; later passes are cut at stale seams. | `audio/engine/transport.rs:672` (`handle_seek_to` ignores `loop_record_session`) | review 2026-10-02 FU-R1b |
| BUG-21 | hardware-io | The last count-in block's tail after the downbeat is silent, so a recording starts with a short gap. | `audio/mixer/callback/count_in.rs:118-121` | review 2026-10-02 FU-R1c (RT-08 remainder) · near [hardware-io/count-in-and-punch.md](hardware-io/count-in-and-punch.md) |
| BUG-22 | hardware-io | MIDI cycle-record passes roll on the engine-thread playhead wrap, so pass boundaries jitter by ~16 ms (audio passes are sample-exact). | `audio/engine/transport.rs:804-818` | review 2026-10-02 FU-R1f |
| BUG-23 | hardware-io | Input-ring overflow drops are reported but not compensated, so every later cycle cut shifts by the dropped frames. | `audio/recording.rs:397-418` | review 2026-10-02 FU-R1a |
| BUG-24 | hardware-io | Panic sends all-notes-off only: pitch bend and mod wheel are not reset on plugins or hardware. | `audio/engine/transport.rs:659-678` | review 2026-10-02 FU-H1f |
| BUG-68 | hardware-io | With distinct takes off, a cycle-recorded run keeps only the first pass (one clip trimmed to `[loop_in, loop_out]`); later passes stay in the WAV outside the trim. The Settings hint says the run "is merged into one clip for the whole loop". | `audio/recording.rs` `finalize_recording` (~:630-645); `audio/engine/transport.rs` (~:472, `cycle_slot` is `None`); hint at `app/view/settings.rs:210` | docs consolidation 2026-10-07 · [timeline/takes-and-comping](../features/timeline/takes-and-comping.md) |
| BUG-69 | hardware-io | *(likely)* Record pressed inside the loop (start after `loop_in`) saturates `trim_start` to 0 but still places the clip at `loop_in`, so the take sits early by `start − loop_in`. | `audio/recording.rs` `finalize_recording` (~:630-645) | docs consolidation 2026-10-07 |

## Plugins and the CLAP host

| ID | Area | Symptom | Where (file:line) | Source |
|---|---|---|---|---|
| BUG-30 | audio-engine | When the engine thread holds a plugin's instance lock, the audio thread skips that plugin for the block: dry audio leaks through an effect, an instrument drops out. Only counted, not concealed. | `audio/clap_host/slot_locking.rs:15-21` | review 2026-10-02 FU-R2a (RT-07 remainder) |
| BUG-31 | audio-engine | A Mastering instance saved bypassed from its own header loads un-bypassed: `own_bypass_sent` starts at −1 and the app ignores `own_bypass_param`. | `audio/clap_host/mod.rs:633,701-711`; `app/engine_events/plugins.rs:1173` | review 2026-10-02 FU-R3c |
| BUG-32 | audio-engine | *(likely)* When a latent plugin first appears on a track that had no delay line, the compensated track restarts from silence (audible gap). | PDC line build, `audio/engine/` latency handling | review 2026-10-02 FU-R3a |
| BUG-33 | audio-engine | *(likely)* Whole-chain bypass on a latent chain crossfades latent wet against undelayed dry for 5 ms (comb/flam on toggle). | `audio/bypass.rs` / chain bypass | review 2026-10-02 FU-R3b |
| BUG-34 | audio-engine | Plugin tails are cut by a fixed 3 s hold after stop; the CLAP tail extension is not asked. | no `CLAP_EXT_TAIL` in `audio/` | review 2026-10-02 FU-R4b |
| BUG-35 | audio-engine | *(likely)* While stopped, busses run without PDC and idle multi-out ports are unrouted. | `audio/mixer/` stopped path | review 2026-10-02 FU-R4c |
| BUG-36 | audio-engine | *(likely)* The held-note release on stop/seek covers timeline notes only, not live-played notes or hardware MIDI out. | `audio/engine/` note release | review 2026-10-02 FU-R4d |
| BUG-37 | audio-engine | Plugin port layouts are not re-read after a plugin restart or rescan (no host `audio_ports` extension). | `audio/clap_host/` (no host-side `audio_ports`) | review 2026-10-02 FU-H1c |
| BUG-38 | plugins-instruments | *(likely)* A save-time state refresh moves an automated parameter's mirror to its value at the playhead. | `app/engine_events/plugins.rs:966-990` | review 2026-10-02 FU-P1b |
| BUG-39 | plugins-instruments | *(likely)* Edge cases: a drums `kit_select` undo races a cache refresh; an in-flight `SavePluginState` is counted as an owed echo. | plugin-state echo bookkeeping | review 2026-10-02 FU-P1c |
| BUG-40 | plugins-effects | The EQ draws a Q slider on LowCut/HighCut bands, but the DSP ignores it (stage Qs come from the slope). | `plugins/resonance-eq/src/band.rs:117` (`uses_q` true for cuts) vs `:326-350` | plugin-audit "smaller items" |
| BUG-41 | plugins-effects | *(likely)* EQ dynamics rows overflow the 350 px strip and the header runs off-window at ≤960 px. | `plugins/resonance-eq/src/editor/app.rs:174`; `factory.rs:70` | review 2026-10-02 FU-P2c |
| BUG-42 | plugins-effects | Mastering's Transformer saturation at partial `sat_mix` dips the sub band (−2.3 dB at 20 Hz); documented, not fixed. | `plugins/resonance-mastering/src/stages/sat_modes.rs:80-92` | review 2026-10-02 FU-D1a2 (DSP2-16 remainder) |
| BUG-43 | plugins-instruments | **Security.** Signing in to Tone3000 appends the PKCE verifier, the auth code, the full token response (access/refresh tokens) and errors to the world-readable fixed path `/tmp/tone3000-auth.log`. | `plugins/resonance-amp/src/tone3000/auth.rs:122-131` (`debug_log`, marked "Temporary"); calls at `:85-92`, `:106-108`, `:306`, `:316` | area docs 2026-10-07 |
| BUG-44 | plugins-instruments | Test gap: the fleet editor checks pin `FLEET.len() == 11` and omit Color and Stereo, so a regression in those two editors' preset bar or palette passes the suite. | `resonance-plugin/tests/fleet_preset_adoption.rs:22,84-90`; `resonance-plugin/tests/fleet_palette.rs:53,139-141` | area docs 2026-10-07 |

## Compose and vocals

| ID | Area | Symptom | Where (file:line) | Source |
|---|---|---|---|---|
| BUG-71 | compose | The vocal melody generator writes out-of-scale notes with `stay_in_scale` on: a D-minor chorus (Bbmaj7, F/A, Gm7, A7; style pop_ballad, syllable_mode mixed, seed 7) sang D#4/E♭ on 9 of 29 notes. `vocal_refit_melody` cleans it up. | `resonance-music-theory/src/derive/vocal/` (melody walk / melisma pass) | live MCP session 2026-10-13 (triphop.rproj) |
| BUG-72 | compose | Chord symbols are spelled with sharps regardless of key: `harmony_apply_progression` with "Bbmaj7" in D minor reads back as "A#maj7" in `song_sections` (and the chord track). | chord symbol formatting in `resonance-music-theory` | live MCP session 2026-10-13 |
| BUG-51 | compose | Drum group Swing, Humanize and "Fills (last bar)" sliders change nothing in the generated drums. | sliders `app/view/compose/lane_inspector/drums/rhythm.rs:44-97`; stored at `app/update/compose/drum_groups.rs:315-324`; never read | intent [compose/drum-groove-feel.md](compose/drum-groove-feel.md) |

## Project, save and export

| ID | Area | Symptom | Where (file:line) | Source |
|---|---|---|---|---|
| BUG-60 | project-core | Save As does not copy clip WAVs that only the undo history references; undoing back to them in the new project plays silence. | `app/update/project_io/mod.rs:660-672` (engine saves live clips only) | review 2026-10-02 FU-U1a |
| BUG-61 | project-core | Save replies are untagged (`SaveCollector` has no round-trip id), so a late reply from an earlier save can complete a newer one. | `app/update/project_io/mod.rs:661-672` | review 2026-10-02 FU-S2c |
| BUG-62 | project-core | *(likely)* Save As over another existing `.rproj` leaves that project's `backups/` behind, and clip GC keeps their clips alive. | `app/project/clip_gc.rs:102` seeds the keep-set from `backups/`; nothing clears it on Save As | review 2026-10-02 STATE2-10 |
| BUG-63 | ui-shell | The "preset already exists" error banner has a run of ~14 spaces in the middle of the sentence. | `app/update/track.rs:879` (spaces inside the literal) | area docs 2026-10-07 |
| BUG-64 | ui-shell | Stem export's overwrite mode "Ask each" behaves like "Add suffix"; there is no prompt. | `app/update/export.rs:205,234` | review 2026-10-02 FU-A1a7 · intent [audio-engine/stem-export-options.md](audio-engine/stem-export-options.md) |
| BUG-66 | audio-engine | `ClearAll` (project load) does not clear `midi_recording`; a live MIDI capture across a load targets a clip that no longer exists. | `audio/engine/tracks.rs` `handle_clear_all` (~:497) | refactor FU-D7d · intent [project-core/finish-app-owned-ids.md](project-core/finish-app-owned-ids.md) |
| BUG-67 | audio-engine | Frozen tracks may apply their aux sends twice (baked into the cache and sent again). | freeze playback substitution | field report 2026-10-06 §3 · intent [audio-engine/freeze-aux-send-double-apply.md](audio-engine/freeze-aux-send-double-apply.md) |
