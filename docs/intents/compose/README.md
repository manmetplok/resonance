# Compose — intents

Work that is wanted but not built, or only partly built, in the Compose area.
What is built is documented in [`../../features/compose/`](../../features/compose/README.md).
Each intent follows [`../_template.md`](../_template.md).

| Intent | Status | Why |
|---|---|---|
| [Chord track editing surface](chord-track-editing.md) | partly built | Only tonic add/delete/pin is reachable; pinned harmony is mostly unusable and agents have no access. |
| [Separate climaxes across simultaneous melodies](cross-lane-climax-separation.md) | idea | Simultaneous lanes place their climaxes independently, so they pile up on the same beat. |
| [Drum groove feel: swing, humanize, fills, step velocity](drum-groove-feel.md) | partly built | Swing, Humanize and Fills sliders change nothing, and step velocities are read as on/off. |
| [Drum material per drum track](drum-material-per-track.md) | idea | Two drum tracks in one project get the same derived drum clip. |
| [EmbellishmentStyle control in the melody inspector](embellishment-style-ui.md) | partly built | `MelodyParams::embellishment` changes the melody style but has no control in the melody inspector. |
| [Share the phrase plan between harmony and melody](harmony-melody-phrase-plan.md) | idea | Melody and harmony each plan phrases on their own instead of sharing one plan. |
| [SoulX-Singer as a second singing backend](soulx-singer-backend.md) | idea | The only open 2026 English score-to-singing model with permissive weights; worth a sidecar experiment against DiffSinger. |
| [Theme-driven lyric generation](lyric-theme-generation.md) | partly built | The lyric generator ignores the theme brief, point of view and "avoid clichés"; it samples a fixed corpus. |
| [Make motif augmentation audible (or drop it)](motif-augment-washout.md) | idea | `Augment` sounds the same as `Identity` because durations are normalised to fill each chord. |
| [Pop/jazz cadence colour](pop-jazz-cadence-color.md) | idea | Generated endings are always classical cadences, whatever the style. |
| [Pronunciation editing, dictionary and persistence](pronunciation-editing.md) | partly built | Pronunciation fixes exist only over the control API and are lost on reload and undo. |
| [Section placement editing (fill on place, move, place-again)](section-placement-editing.md) | partly built | Placing a section with generated parts gives a silent, empty placement. |
| [Vocals: what is left after the MCP-control and quality work](vocal-followups.md) | partly built | The vocal spec is built; which banks join the plok.org index (blind listen), publishing the index, a live MCP session, Lilia's licence and a few small gaps are not. |
| [Vocal harmony and doubling generator](vocal-harmony-doubling.md) | designed | A single synthesized lead sounds thin; there is no harmony or double generator. |
| [Vocal pitch editor (graphical tuning of recorded vocals)](vocal-pitch-editor.md) | partly built | Pitch detection and retune render exist, but no editor view opens, so tuning is unreachable. |
