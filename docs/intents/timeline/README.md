# Timeline — intents

Work that is wanted but not built, or only partly built, in the Timeline area.
What is built is documented in [`../../features/timeline/`](../../features/timeline/README.md).
Each intent follows [`../_template.md`](../_template.md).

| Intent | Status | Why |
|---|---|---|
| [Arrange-side automation editing](arrange-automation-editing.md) | partly built | Lanes can be edited on Arrange but not added, removed or multi-selected there. |
| [Insert / remove bars in the GUI](arrangement-bars-gui.md) | partly built | Insert/remove bars works over the control API but nothing in the GUI sends it. |
| [Automation recording (write / touch / latch)](automation-recording.md) | idea | Automation can only be drawn or scripted, not recorded from moving a control. |
| [Copy, paste and duplicate clips](clip-copy-duplicate.md) | idea | There is no way to copy or duplicate a clip; ⌘D only duplicates section placements. |
| [Multi-clip selection and marquee](clip-multi-select.md) | idea | Only one clip can be selected at a time; no marquee. |
| [Clip state read-back in `song.tracks`](clip-state-readback.md) | idea | `song.tracks` omits trims, fades, gain and warp, so an agent cannot verify clip edits. |
| [Freeze in the control API](freeze-control-api.md) | idea | Agents see `frozen` and get `busy`, but cannot freeze or unfreeze. |
| [Markers in the control API](markers-control-api.md) | idea | Agents cannot see or write arrangement markers. |
| [MIDI clip arrangement edits over the control API](midi-clip-arrange-api.md) | idea | Agents can create and move MIDI clips but not delete, trim, split or re-track them. |
| [Piano-roll note drag as one undo step](piano-roll-drag-undo.md) | idea | A piano-roll note drag records an undo entry per pointer move. |
| [Piano-roll editing controls](piano-roll-editing-controls.md) | idea | Everyday piano-roll controls (velocity lane input and more) are missing. |
| [Pool-backed clip audio](pool-backed-clip-audio.md) | idea | A pooled clip gets its own copied WAV on save instead of playing its asset. |
| [Quantize toolbar remembers its settings](quantize-remember-settings.md) | partly built | The saved quantize settings are never used; the toolbar starts from defaults every session. |
| [Takes in the control API](takes-control-api.md) | idea | Agents can set cycle-record mode but cannot list, solo, comp or delete takes. |
| [Tempo and meter edits keep content on its bars](tempo-edits-keep-content-on-bars.md) | partly built | Only `CommitBpm` keeps content on its bars; other tempo-map edits shift it in time. |
| [Tempo step changes](tempo-step-changes.md) | idea | Tempo always ramps between events; there are no step changes. |
| [Finish track groups](track-groups-completion.md) | partly built | Group level trim is inaudible and other designed group features are missing. |
| [Reorder tracks](track-reordering.md) | idea | Tracks stay in creation order; nothing can reorder them. |
