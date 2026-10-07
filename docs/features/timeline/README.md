# Timeline (Arrange view)

The timeline area is the Arrange tab and everything edited on it: the track
lanes and headers, audio and MIDI clips with their fades, gain and warp, take
lanes from loop recording, the global tempo/signature map, markers, automation
lanes, the MIDI piano roll, and the media side (pool, import, relink; the media browser panel is in
[ui-shell/media-browser-panel.md](../ui-shell/media-browser-panel.md))
that feeds clips onto the lanes. Most of the code lives in `resonance-app`
(`state/`, `update/`, `view/timeline/`, `view/track_header/`,
`view/midi_editor/`, `view/clip_inspector*`, `view/browser/`), with shared
model types in `resonance-common` and playback in `resonance-audio` (see
[../audio-engine/](../audio-engine/)). Open work is in
[../../intents/timeline/](../../intents/timeline/).

| Feature | Doc | One-line summary |
|---|---|---|
| Arrange view | [arrange-view.md](arrange-view.md) | Row layout (tracks, group headers, automation and take sub-rows), track headers, scrolling, zoom (10–1000 px/s), playhead follow, culling and virtualization. |
| Selection | [selection.md](selection.md) | Multi-track selection, one clip / event / marker at a time, keyboard ownership, and the selection-scoped commands. |
| Track groups | [track-groups.md](track-groups.md) | Folder tracks from Cmd/Ctrl+G: header row, colour rails, fold to an overview lane, macro mute/solo; the level trim is stored but not applied to audio. |
| Clip editing | [clip-editing.md](clip-editing.md) | Place, move, trim, split and delete audio and MIDI clips with grid snap; edits on a still-loading clip are deferred; `clip.*` tools. |
| Fades, crossfades, clip gain | [fades-crossfades-clip-gain.md](fades-crossfades-clip-gain.md) | Per-clip fade lengths and curves, clip gain, automatic same-track crossfades and a 2 ms declick, shared by playback and bounce. |
| Clip inspector | [clip-inspector.md](clip-inspector.md) | Floating card for the selected audio clip: fade ms and curves, gain, reset, warp section. |
| Clip warp and tempo detection | [warp-and-tempo-detection.md](warp-and-tempo-detection.md) | Warp settings and markers are stored, drawn, saved and scriptable but not rendered; tempo detection works. |
| Tempo and time-signature map | [tempo-and-signature-map.md](tempo-and-signature-map.md) | Bar-addressed tempo (linear ramps) and meter events on the global shelf and `global.*`. |
| Markers | [markers.md](markers.md) | Point and region markers in the ruler: add, drag, rename, recolour, jump, loop a region; no control API. |
| Insert / remove bars | [arrangement-bars.md](arrangement-bars.md) | One undoable edit that shifts everything after a cut; control API only. |
| Takes and comping | [takes-and-comping.md](takes-and-comping.md) | Loop-recording take lanes: solo, split, promote and delete takes (app side). |
| Track freeze (UI) | [track-freeze-ui.md](track-freeze-ui.md) | Freeze / unfreeze / refreeze, batch freeze, banners and progress, read-only gating; engine side in audio-engine. |
| Automation lanes | [automation-lanes.md](automation-lanes.md) | Breakpoint lanes for volume, pan, mute, plugin and device parameters; edited on the Arrange band and sub-rows, applied per block. |
| Automation control API | [automation-control-api.md](automation-control-api.md) | The seven `automation.*` methods and MCP tools; real units, meter-aware positions, one undo entry per call. |
| MIDI editor and quantize | [midi-editor.md](midi-editor.md) | Bottom-panel piano roll (draw, move, resize, marquee) with quantize, humanize and groove; `notes.*` tools. |
| Audio pool and import | [audio-pool-and-import.md](audio-pool-and-import.md) | Import wav/flac/mp3/ogg as project-rate stereo WAV, pool usage counts, remove unused; also the MIDI file import dialog. |
| Missing-media relink | [relink.md](relink.md) | Missing files load offline; locate one by one or search a folder; relinked files are copied into the project. |
