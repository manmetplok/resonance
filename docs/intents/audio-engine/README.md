# Audio engine — intents

Work that is wanted but not built, or only partly built, in the Audio engine area.
What is built is documented in [`../../features/audio-engine/`](../../features/audio-engine/README.md).
Each intent follows [`../_template.md`](../_template.md).

| Intent | Status | Why |
|---|---|---|
| [Render clip warp (time-stretch) in playback and bounce](clip-warp-render.md) | partly built | Warp settings are saved, shown and scriptable, but no render path reads them — nothing stretches. |
| [Expose export formats, sample rate, normalisation and metadata](export-formats-and-metadata.md) | partly built | The engine writes FLAC/MP3/Opus, resamples and normalises, but no GUI or control path can choose any of it. |
| [Wire up MIDI project export](midi-project-export.md) | partly built | The export dialog's MIDI tab is disabled although a tested SMF writer exists. |
| [Engine and CLAP host: review leftovers](review-leftovers-engine.md) | idea | Code-quality, robustness and test items in `resonance-audio` and the plugin bridge that the review campaigns left open. |
| [Live-path robustness: lock misses, PDC transitions, tails, stopped mix, port rescans](live-path-robustness.md) | idea | Seven live-only engine defects (former BUG-21/30/32-35/37) that need realtime design work, not a quick fix. |
| [Live stress run of the parallel render pool](rt-live-stress-run.md) | designed | The parallel render pool is proven offline only; the spec's 10-minute live run was never done. |
| [Show render-pool status, stutter counters and the thread setting in the app](rt-pool-status-ui.md) | idea | Stutter causes (serial fallback, critical track, missed cycles) are visible only in the log. |
| [RTKit fallback for render-pool realtime priority](rt-rtkit-fallback.md) | idea | Without `RLIMIT_RTPRIO` the render workers get no realtime priority; RTKit could grant it. |
| [Stem export options, overwrite prompt and retry](stem-export-options.md) | partly built | The Audio stems tab of the export dialog works (track / bus / master checklist, whole project or loop range, destination, progress, cancel, partial-failure report), but the prototype's option row neve |
