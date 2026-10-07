# Resonance documentation

Everything about **what Resonance does today** and **what we still want it to
do**, sorted by area. Checked against master `8eac71b3` on 2026-10-07.

Start elsewhere if you want to *use* or *build* it: [README](../README.md)
(setup, running, MCP), [ARCHITECTURE](../ARCHITECTURE.md) (crate layering and
code patterns — enforced by `tools/arch-invariants`), [CLAUDE.md](../CLAUDE.md)
(tests and working rules).

## How this folder is organised

| Folder | Holds | Rule |
|---|---|---|
| [`features/`](#features-built) | Built features, one doc per feature, grouped by area. Each says what it does, **where the logic lives**, how to reach it (GUI / control method / MCP tool / skill), **how it is tested**, deviations from the original design and known gaps. | Describes the code on master, nothing else. Change a feature → update its doc in the same commit and bump its `Verified against master @` line. |
| [`intents/`](#intents-wanted) | Things we want and have not built, or built partly. Plus [`bugs.md`](intents/bugs.md) (known defects) and [`prototypes/`](intents/prototypes/README.md) (HTML mockups of unbuilt work). | One intent per outcome. When it ships, fold it into the feature doc and delete the intent. Fix a bug → delete its row. |
| [`guides/`](#guides) | Rules that always hold. | Kept true; edit when the rule changes. |
| [`history/`](history/README.md) | Archived specs, reviews, audits, refactor plans and prototypes of built features. | Never updated (except the archive banner). Code comments that cite a spec by file name resolve here. |

Templates: [feature doc](features/_template.md) · [intent](intents/_template.md).

## Areas

| Area | Covers | Built | Wanted |
|---|---|---|---|
| **Audio engine** | `resonance-audio`: command loop, render graph, render pool, PipeWire I/O, CLAP hosting, latency, offline render/export, freeze, diagnostics | [17 docs](features/audio-engine/README.md) | [9 intents](intents/audio-engine/README.md) (4 partly built) |
| **Timeline** | Arrange view: clips, fades, warp, tempo map, markers, takes, automation lanes, MIDI editor, pool/import/relink, track groups | [17 docs](features/timeline/README.md) | [18 intents](intents/timeline/README.md) (5 partly built) |
| **Mixer** | Mix tab: strips, inspector, busses, sends, sidechain, master, plugin chains, reference A/B, meters, `meter.*`, `master.assist` | [12 docs](features/mixer/README.md) | [6 intents](intents/mixer/README.md) (2 partly built) |
| **Compose** | Sections, lane generators, chords, drum arrangement, vocals (lyrics, melody, DiffSinger render, voicebank library, pronunciation), music theory | [12 docs](features/compose/README.md) | [15 intents](intents/compose/README.md) (8 partly built) |
| **Hardware I/O** | Audio devices, input/monitoring, recording, transport, external instruments, device presets, MIDI I/O, MIDI learn, SMF import | [10 docs](features/hardware-io/README.md) | [11 intents](intents/hardware-io/README.md) (3 partly built) |
| **Control API** | `resonance-control` protocol, the app control layer, jobs, `resonance-mcp`, the agent plugin, the full method catalogue | [6 docs](features/control-api/README.md) | [4 intents](intents/control-api/README.md) (1 partly built) |
| **UI shell** | Commands, palette, keymap, views, overlays, settings, startup/recovery, media browser, preset browser, plugin windows, performance mode, theme, UI goldens | [13 docs](features/ui-shell/README.md) | [7 intents](intents/ui-shell/README.md) (4 partly built) |
| **Project core** | File format, save/open, autosave, clip storage, templates, undo, Reconcile, ids, app state, update loop, hermetic tests, arch invariants | [12 docs](features/project-core/README.md) | [4 intents](intents/project-core/README.md) (2 partly built) |
| **Plugins — effects** | EQ, compressor, gate, delay, granular delay, reverb, stereo, color, IR, mastering, mastering-assist engine, de-harsh | [12 docs](features/plugins-effects/README.md) | [6 intents](intents/plugins-effects/README.md) (1 partly built) |
| **Plugins — instruments & platform** | Wavetable, drums + kit library, amp + NAM library, preset library, plugin SDK, editor runtimes, DSP/metering crates, 3rd-party hosting, bundling | [13 docs](features/plugins-instruments/README.md) | [9 intents](intents/plugins-instruments/README.md) (6 partly built) |

Totals: **124 feature docs**, **89 intents**, **45 known bugs**, 12 unbuilt prototypes.

## Guides

- [UX guidelines](guides/ux-guidelines.md) — the visual language, tokens and interaction rules for the app (iced) and plugin editors (egui).
- [Control-API conventions](guides/control-api-conventions.md) — how to add a control method: one tool per method, wire type + handler + MCP tool + tests as one vertical slice.

## Features (built)


### Audio engine

| Feature | Summary |
|---|---|
| [Engine command loop](features/audio-engine/engine-command-loop.md) | `AudioEngine` start-up, the engine thread's command/inbox loop, handler rules, id grants. |
| [Engine error taxonomy](features/audio-engine/engine-errors.md) | `EngineError { kind, message }`, typed export errors, job error kinds, the no-`Result<_, String>` invariant. |
| [Render graph publishing](features/audio-engine/render-graph.md) | Immutable `RenderGraph` republished per edit via `ArcSwap`; replaced values freed on the engine thread. |
| [Realtime-safety rules](features/audio-engine/rt-safety.md) | No alloc, lock, log or cold page on the audio path; offline-render gate; limits; how each is enforced. |
| [Mixer render core](features/audio-engine/mixer-render-core.md) | The audio callback's branches and `render_block`: track jobs, ordered reduction, bus levels, live vs bounce strategy. |
| [Parallel render pool](features/audio-engine/render-pool.md) | Realtime worker pool, thread-count resolution, parked idle workers, CLAP thread-pool, bit-identical output. |
| [Audio output stream](features/audio-engine/audio-output-stream.md) | Native PipeWire `pw_stream` output with cpal fallback, graph-rate force, quantum, stream health. |
| [Audio input capture](features/audio-engine/input-capture.md) | Capture streams, monitor ring pacing and lockout, streaming takes to disk with latency-correct placement. |
| [DSP load and stutter diagnostics](features/audio-engine/cycle-load-diagnostics.md) | Per-cycle load meter, missed/dropped cycles, wake timing, lock misses, the `RESONANCE_AUDIO_STATS` report. |
| [CLAP plugin hosting](features/audio-engine/clap-hosting.md) | Scanning, loading, running and removing CLAP plugins; thread roles; host extensions; bypass; editors. |
| [Plugin params, state and presets](features/audio-engine/clap-params-and-presets.md) | Param description and flush, plugin-made edits, state save/load, preset-state and preset-load, discovery. |
| [Latency compensation](features/audio-engine/latency-compensation.md) | Two-stage PDC, live latency changes, offline trim, device I/O latency. |
| [Offline render and export](features/audio-engine/offline-render-and-export.md) | Chunked offline render core, mixdown export (formats, normalise), bounce in place. |
| [Stem export](features/audio-engine/stem-export.md) | Track / bus / master stems over one shared range, sequential with partial-failure reporting. |
| [Offline measurement and chain probe](features/audio-engine/offline-measurement.md) | Render-and-measure loudness and details (incl. depth), measure files/references, probe a cloned chain. |
| [Track freeze (engine side)](features/audio-engine/track-freeze.md) | Freeze-cache render, cache format, rate conversion and playback substitution. |
| [Clip loading and import](features/audio-engine/clip-loading.md) | Pool import, mmapped clip loading with tickets and fences, clip WAV persistence. |

### Timeline

| Feature | Summary |
|---|---|
| [Arrange view](features/timeline/arrange-view.md) | Row layout (tracks, group headers, automation and take sub-rows), track headers, scrolling, zoom (10–1000 px/s), playhead follow, culling and virtualization. |
| [Selection](features/timeline/selection.md) | Multi-track selection, one clip / event / marker at a time, keyboard ownership, and the selection-scoped commands. |
| [Track groups](features/timeline/track-groups.md) | Folder tracks from Cmd/Ctrl+G: header row, colour rails, fold to an overview lane, macro mute/solo; the level trim is stored but not applied to audio. |
| [Clip editing](features/timeline/clip-editing.md) | Place, move, trim, split and delete audio and MIDI clips with grid snap; edits on a still-loading clip are deferred; `clip.*` tools. |
| [Fades, crossfades, clip gain](features/timeline/fades-crossfades-clip-gain.md) | Per-clip fade lengths and curves, clip gain, automatic same-track crossfades and a 2 ms declick, shared by playback and bounce. |
| [Clip inspector](features/timeline/clip-inspector.md) | Floating card for the selected audio clip: fade ms and curves, gain, reset, warp section. |
| [Clip warp and tempo detection](features/timeline/warp-and-tempo-detection.md) | Warp settings and markers are stored, drawn, saved and scriptable but not rendered; tempo detection works. |
| [Tempo and time-signature map](features/timeline/tempo-and-signature-map.md) | Bar-addressed tempo (linear ramps) and meter events on the global shelf and `global.*`. |
| [Markers](features/timeline/markers.md) | Point and region markers in the ruler: add, drag, rename, recolour, jump, loop a region; no control API. |
| [Insert / remove bars](features/timeline/arrangement-bars.md) | One undoable edit that shifts everything after a cut; control API only. |
| [Takes and comping](features/timeline/takes-and-comping.md) | Loop-recording take lanes: solo, split, promote and delete takes (app side). |
| [Track freeze (UI)](features/timeline/track-freeze-ui.md) | Freeze / unfreeze / refreeze, batch freeze, banners and progress, read-only gating; engine side in audio-engine. |
| [Automation lanes](features/timeline/automation-lanes.md) | Breakpoint lanes for volume, pan, mute, plugin and device parameters; edited on the Arrange band and sub-rows, applied per block. |
| [Automation control API](features/timeline/automation-control-api.md) | The seven `automation.*` methods and MCP tools; real units, meter-aware positions, one undo entry per call. |
| [MIDI editor and quantize](features/timeline/midi-editor.md) | Bottom-panel piano roll (draw, move, resize, marquee) with quantize, humanize and groove; `notes.*` tools. |
| [Audio pool and import](features/timeline/audio-pool-and-import.md) | Import wav/flac/mp3/ogg as project-rate stereo WAV, pool usage counts, remove unused; also the MIDI file import dialog. |
| [Missing-media relink](features/timeline/relink.md) | Missing files load offline; locate one by one or search a folder; relinked files are copied into the project. |

### Mixer

| Feature | Summary |
|---|---|
| [Channel strips and inspector](features/mixer/channel-strips-and-inspector.md) | Mix tab layout; strips show state, the inspector (CHAIN, SENDS, ROUTING, AUTOMATION, TRACK/BUS/MASTER) edits; inline rename |
| [Volume, pan, mute, solo](features/mixer/volume-pan-mute-solo.md) | dB faders with per-block ramps, stereo-balance pan (centre = unity), shared mute/solo resolution, `mixer.*` |
| [Busses and output routing](features/mixer/busses-and-output-routing.md) | Tracks route to master or one bus; busses sum to master; return role; `bus.*`, `track.set_output` |
| [Aux sends](features/mixer/aux-sends.md) | Pre/post-fader sends into return busses, cycle check, engine-echo mirror, persistence; `track.*_send` |
| [Sidechain routing](features/mixer/sidechain-routing.md) | Key a plugin on a track, bus or master from a track or bus; one-block double-buffered taps; control API only |
| [Master bus](features/mixer/master-bus.md) | Master FX → click → volume → hard clip → meters; master inspector; `master.summary`, `master.set_volume` |
| [Plugin chains](features/mixer/plugin-chains.md) | One `ChainOwner` API for add/move/replace/remove/bypass on all chains; CHAIN rows; generic plugin window |
| [Track colour](features/mixer/track-colour.md) | Persisted per-track identity colour from a 10-hue palette; sub-tracks follow the parent; `track.set_color` |
| [Reference A/B monitoring](features/mixer/reference-ab-monitoring.md) | Load references into a rail, flip/hold-B A/B post-master, Mix/Ref/Δ loudness; `reference.load` |
| [Strip meters](features/mixer/metering.md) | Post-fader peak meters on every strip (polled, decayed); the `resonance-metering` crate is in [plugins-instruments/metering-library.md](features/plugins-instruments/metering-library.md) |
| [`meter.*` control API](features/mixer/meter-control-api.md) | Offline/live loudness, stems, snapshots + loudness-matched compare, insert-chain probe, as jobs |
| [Mastering assistant (`master.assist`)](features/mixer/mastering-assist.md) | App side of `master.assist`: offline render + analysis returning mastering-plugin param suggestions, never applied; the engine crate is in [plugins-effects/mastering-assist.md](features/plugins-effects/mastering-assist.md) |

### Compose

| Feature | Summary |
|---|---|
| [Sections and placements](features/compose/sections-and-placements.md) | Create, resize, place and delete section definitions; placements never overlap; meter comes from the tempo map. |
| [Lane generators and derived clips](features/compose/lane-generators.md) | Bass/Melody/Pad generators per (section, track) write derived MIDI clips on every placement; `generate.part`, `section.set_lane_generator`. |
| [Section chords and progressions](features/compose/section-chords-and-progressions.md) | Per-section chord grid, scale, Markov style-table and schema progression generators; `harmony.*`, `section.set_scale`. |
| [Chord track and song key](features/compose/chord-track.md) | Song-wide chord regions whose pins constrain regeneration, and the song key set by `transport.set_key`. |
| [Drum patterns and section arrangement](features/compose/drum-arrangement.md) | Project pattern bank of polymetric drum groups, per-section arrangement with repeats and fills, built-in grooves; `generate.drums`. |
| [Vocal lyrics and melody](features/compose/vocal-lyrics-and-melody.md) | Vocal lanes: lyric draft, template lyric generator, style-based vocal melody; `vocal.set_lyrics`, `vocal.set_line`, `vocal.generate`, `song.vocal`. |
| [Vocal rendering](features/compose/vocal-rendering.md) | DiffSinger synthesis via `resonance-svs`, cached partial re-renders, rests and breaths, velocity dynamics, expression curves, per-lane render state; `vocal.render`. |
| [Voicebank library](features/compose/voicebank-library.md) | Installed DiffSinger banks, the plok.org index and installs with their vocoders, the library overlay; `vocal.voicebanks`, `vocal.voicebank_install`. |
| [Lyric pronunciation](features/compose/pronunciation.md) | CMU G2P with a project pronunciation dictionary and voicebank phoneme substitution; `vocal.set_pronunciation`. |
| [Music-theory library](features/compose/music-theory-library.md) | `resonance-music-theory`: pitch, scale, chord, voicing, SATB, progression generators, G2P, fretboard. |
| [Melody and motif engine](features/compose/melody-and-motif-engine.md) | Motif-based melody with phrase forms, sequences, cadence formulas, single climax, embellishing tones and rhythm transforms. |
| [Chord sheet PDF export](features/compose/chord-sheet-export.md) | Printable chord diagrams for guitar, bass and keyboard from the section chords. |

### Hardware I/O

| Feature | Summary |
|---|---|
| [Audio devices and PipeWire](features/hardware-io/audio-devices-and-pipewire.md) | Native PipeWire output on the default sink, 48 kHz graph-rate assertion, quantum read at startup, input enumeration, I/O latency readout. |
| [Audio input and monitoring](features/hardware-io/audio-input-and-monitoring.md) | Per-track input device/channel and mono; one shared capture stream; monitoring through the track's chain, bus and master. |
| [Recording](features/hardware-io/recording.md) | Arm and record audio and MIDI with a two-bar count-in, latency-compensated take placement, cycle-record takes, one undo per run. |
| [Transport and metronome](features/hardware-io/transport.md) | Play/pause/stop/seek/loop, starting tempo/meter/key, metronome click, shortcuts and `transport.*` methods. |
| [External instrument tracks](features/hardware-io/external-instruments.md) | Outboard synth as a track: MIDI out + audio return, patch, latency offset and ping, live/recorded playback, realtime bounce. |
| [Device presets](features/hardware-io/device-presets.md) | JSON definitions of a synth's CC/NRPN parameters and named patches; patch-by-name and hardware parameter automation. |
| [Hardware MIDI I/O and clock](features/hardware-io/midi-hardware-io.md) | Port enumeration and hotplug, per-track MIDI in/out and Thru, timeline → hardware notes, MIDI clock send/receive. |
| [MIDI Learn](features/hardware-io/midi-learn.md) | Control-surface port, learn by right-click, takeover/relative mapping, controller maps, `midi_map.*` methods. |
| [Standard MIDI File import](features/hardware-io/smf-import.md) | Import dialog with track review, placement and tempo reconciliation; `notes.import_midi` for agents. |
| [Audio file decoding](features/hardware-io/audio-file-decoding.md) | symphonia decode to stereo at the project rate, sinc resampling, zero-copy mmap of engine WAVs. |

### Control API

| Feature | Summary |
|---|---|
| [Control API architecture](features/control-api/overview.md) | Socket, threads, dispatch order, mutation gate, one-undo-per-call, revision counter, remote indicator. |
| [Protocol and conventions](features/control-api/protocol.md) | JSON-RPC envelope, framing, error kinds, `PROTOCOL_VERSION` and capabilities, strict params, ids and positions, the vertical-slice rule. |
| [Method catalogue](features/control-api/method-catalogue.md) | All 182 methods with their MCP tool, kind and owning area. |
| [Jobs](features/control-api/jobs.md) | `job.status` / `job.wait`, which calls are jobs, retention, disconnects, MCP-side waiting. |
| [MCP server](features/control-api/mcp-server.md) | `resonance-mcp`: registration, tool naming, schema normalisation, error mapping, connection handling. |
| [Agent plugin](features/control-api/agent-plugin.md) | The six skills, their rules, and the lockstep test that keeps them in step with the tools. |

### UI shell

| Feature | Summary |
|---|---|
| [Command registry and shortcuts](features/ui-shell/command-registry-and-shortcuts.md) | 102 `CommandId`s with gates, scopes and availability. The global key dispatch order and the default keymap. |
| [Command palette](features/ui-shell/command-palette.md) | ⌘K fuzzy command search with recents, suggestions, dimmed unavailable rows, and `:` `@` `#` `+` argument modes. |
| [Keymap editor and presets](features/ui-shell/keymap-editor.md) | Settings › Keyboard: Logic Pro / Pro Tools presets, rebind by pressing a chord, conflict replace, overrides in `settings.json`. |
| [Window chrome and views](features/ui-shell/window-chrome-and-views.md) | The 1440×900 window, view tabs and `--tab`, the chrome row, the transport bar slot, the Remote chip, window file drops. |
| [Overlays, dialogs and status area](features/ui-shell/overlays-and-dialogs.md) | One root overlay in a fixed priority that both renderer and key gate read, Esc dismiss, the quit/switch confirms, AUDIO/AUTOSAVE/ERROR lines. |
| [Settings](features/ui-shell/settings.md) | `settings.json` sections and the General / Keyboard / MIDI overlay (autosave, follow playhead, latency, plugins, MIDI clock). |
| [Startup screen and recovery](features/ui-shell/startup-and-recovery.md) | The modal New session screen, `recent.json`, and the "Recover unsaved work?" prompt on open and at boot. |
| [Media browser panel](features/ui-shell/media-browser-panel.md) | The docked Files / Pool / Presets panel in Arrange with audition transport. Drag-to-timeline is not wired. |
| [Host preset browser and track presets](features/ui-shell/preset-browser.md) | Preset bar, audition-on-arrow overlay with one-undo keep and Esc revert, Media › Presets with drag-to-header, track presets. |
| [Plugin windows](features/ui-shell/plugin-windows.md) | The host-drawn floating generic parameter window, and native CLAP editor open/close mirrored from engine events. |
| [Performance mode](features/ui-shell/performance-mode.md) | A full-bleed chord teleprompter: hero chord, fingering diagram, beat ring and count-in, up-next lane, instrument/capo. |
| [Theme and view performance](features/ui-shell/theme-and-view-performance.md) | `theme.rs` tokens and styles, two-rate tick, cached pick lists, `lazy` regions, Canvas caches, and the arch-invariant checks. |
| [UI snapshot testing](features/ui-shell/ui-snapshot-testing.md) | `iced_test` goldens via `assert_golden`, the absent-golden guard, `RESONANCE_BLESS=1`, `RESONANCE_SKIP_GOLDENS=1`. |

### Project core

| Feature | Summary |
|---|---|
| [Project file format](features/project-core/project-file-format.md) | `.rproj` bundle layout, `ProjectFile` contents, version 2 and additive growth |
| [Save, open, new and quit](features/project-core/save-open-new.md) | async engine-collected save, Save As, versioned backups, open/new over unsaved changes, quit, `project.*` jobs |
| [Autosave and crash recovery](features/project-core/autosave-and-recovery.md) | change-gated 30 s autosave to a side file, session marker, recovery prompt and `recover_autosave` |
| [Clip storage and GC](features/project-core/clip-storage-and-gc.md) | one write-once `clip_<id>.wav` per clip, persisted before undo, reaped after a manual save |
| [Project templates](features/project-core/project-templates.md) | four built-in starters in code, user templates on disk, instantiate untitled; no picker UI yet |
| [Undo and redo](features/project-core/undo-redo.md) | whole-project snapshots, one entry per action / gesture / control call, shared with agents via `edit.*` |
| [Reconcile](features/project-core/reconcile.md) | one 36-domain table restores a project after a disk load (from empty) or an undo (by diff) |
| [Entity ids](features/project-core/entity-ids.md) | app-side allocators per id space, never-reset clip counter, clip-id grant for engine-created clips |
| [App state](features/project-core/app-state.md) | the 40-field `Resonance` struct and its domain sub-states |
| [Update loop](features/project-core/update-loop.md) | `Message` domains, gates → undo → dispatch, the tick, exhaustive engine-event routing, control requests |
| [Hermetic test app](features/project-core/hermetic-test-app.md) | `new_for_test*` constructors with a capturing engine and temp-rooted user dirs |
| [Architecture invariants](features/project-core/architecture-invariants.md) | 31 grep/metadata tests for crate layering, test layout, logging, locking and app boundaries |

### Plugins — effects

| Feature | Summary |
|---|---|
| [EQ](features/plugins-effects/eq.md) | 8-band parametric EQ with M/S bands, Tilt/Lift+Dip/Air types, dynamic bands, auto-gain and a live analyzer |
| [Compressor](features/plugins-effects/compressor.md) | Feed-forward soft-knee compressor with peak/RMS detector, Auto release, sidechain key and HPF |
| [Gate](features/plugins-effects/gate.md) | Noise gate / downward expander with hysteresis, hold, range and an external key |
| [Delay](features/plugins-effects/delay.md) | Tempo-synced stereo delay (Stereo/Ping-Pong/Dual) with Digital/Analog character, freeze, wet gate and duck |
| [Granular delay](features/plugins-effects/granular-delay.md) | Delay buffer read by up to 64 pitched grains, with freeze, shimmer feedback, pitch-sync and quality tiers |
| [Reverb](features/plugins-effects/reverb.md) | Nine algorithmic reverbs (Classic … Shimmer) with a return channel: wet EQ, ducking, ER/tail balance, tempo sync |
| [Stereo](features/plugins-effects/stereo.md) | Mono-safe width tool: M/S width, mono-maker, four wideners, balance, rotation, goniometer |
| [Color](features/plugins-effects/color.md) | Saturation in five voicings (Tube, Tape, Transformer, Console, Warm) with Tape HQ hysteresis and auto-gain |
| [IR](features/plugins-effects/ir.md) | Convolution loader for cab and room IRs with a user-chosen latency mode |
| [Mastering](features/plugins-effects/mastering.md) | Fixed-order mastering chain of ten stages, loudness metering and an assistant panel |
| [Mastering assistant engine](features/plugins-effects/mastering-assist.md) | Analysis + genre/reference decision engine shared by the mastering plugin and `master.assist` |
| [De-harsh](features/plugins-effects/deharsh.md) | STFT resonance suppressor in `resonance-dsp`, used as the mastering chain's de-harsh stage |

### Plugins — instruments & platform

| Feature | Summary |
|---|---|
| [Wavetable synth](features/plugins-instruments/wavetable-synth.md) | 32-voice, 2-oscillator wavetable synth with warp, FM/ring/sync, sub/noise, five filter models, 8-slot mod matrix, FX chain, user wavetables and 28 factory presets. |
| [Drums plugin](features/plugins-instruments/drums-plugin.md) | 30-pad multi-mic sampler with velocity layers, round robin, choke, mic banks, disk streaming, seven-port Multi output and a Pads/Mix/Setup editor. |
| [Drum kit library](features/plugins-instruments/drum-kit-library.md) | Shared on-disk kit library with stable slots, marks, import, plok.org downloads, and `drum_kits.list` / `set_marks`. |
| [Amp plugin](features/plugins-instruments/amp-plugin.md) | NAM player (WaveNet A1/A2, LSTM) with crossfaded model swaps, pass-through on a missing model, a tuner and parity tests against upstream. |
| [NAM model library](features/plugins-instruments/nam-model-library.md) | Shared amp-model library by content hash, Tone3000 sign-in and downloads, and `amp_models.list` / `set_marks`. |
| [Plugin preset library](features/plugins-instruments/plugin-preset-library.md) | One preset format, browser and marks for every plugin (first- and third-party), in each editor and in the app, with `presets.*` and per-slot preset tools. |
| [Plugin SDK](features/plugins-instruments/plugin-sdk.md) | `ResonancePlugin`, params, state upgrades, `export_clap!`, first-party ids and the private CLAP extensions. |
| [Plugin editor runtimes](features/plugins-instruments/plugin-editor-runtimes.md) | `plugin-gui-core` plus the Wayland and Cocoa runtimes that open every editor as a floating egui window. |
| [DSP building blocks](features/plugins-instruments/dsp-building-blocks.md) | `resonance-dsp`: filters, oversampling, saturation, reverb, granular, pitch, time-stretch and other shared primitives. |
| [Metering library](features/plugins-instruments/metering-library.md) | `resonance-metering`: BS.1770 loudness, true peak, LRA, spectrum, offline mix analysis, decay, probe and detail proxies. |
| [DSP goldens and skill keys](features/plugins-instruments/dsp-test-support.md) | `resonance-dsp-test-support`: golden file IO, `RESONANCE_BLESS`, diffs, and the skill ↔ param-key check. |
| [Third-party plugins](features/plugins-instruments/third-party-plugin-hosting.md) | Scan folders, additive rescan, generic param panel, native editors, and missing-plugin slots that keep their state. |
| [Plugin bundling](features/plugins-instruments/plugin-bundling.md) | `scripts/bundle.sh` (derived list, cross-checks, macOS bundles) and the checklist for a new plugin crate. |

## Intents (wanted)

Each area's intent index gives every intent's status (**idea**, **designed**,
**partly built**, **blocked**) and a one-line why — follow the *Wanted* links in
the [Areas](#areas) table. Cross-cutting:

- [Known bugs](intents/bugs.md) — behaviour defects in the code today, with file:line.
- [Open product decisions](intents/project-core/open-decisions.md) — review items that stopped on a product call.
- [Unbuilt prototypes](intents/prototypes/README.md) — HTML mockups and what is left of each.
- Verification debt — checks that need a human, a live session or a Mac:
  [RT live stress run](intents/audio-engine/rt-live-stress-run.md),
  [drums live verification](intents/plugins-instruments/drums-live-verification.md),
  [effects live agent session](intents/plugins-effects/effects-live-agent-session.md),
  [plugin editor visual checks](intents/plugins-effects/plugin-editor-visual-checks.md),
  [macOS open checks](intents/plugins-instruments/macos-port-open-checks.md).
