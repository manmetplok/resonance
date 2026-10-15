# Plugins — instruments and libraries — intents

Work that is wanted but not built, or only partly built, in the Plugins — instruments and libraries area.
What is built is documented in [`../../features/plugins-instruments/`](../../features/plugins-instruments/README.md).
Each intent follows [`../_template.md`](../_template.md).

| Intent | Status | Why |
|---|---|---|
| [Cocoa editor runtime: test and teardown leftovers](cocoa-runtime-leftovers.md) | idea | Items in `cocoa-plugin-gui` that are not in [`macos-port-open-checks.md`](macos-port-open-checks.md) (which covers the embedded editors, macOS goldens, FU-H4a and re-verification). |
| [Drum kit preview, folder-of-WAVs import and an editable note map](drums-kit-authoring.md) | designed | Only kits with a ready `drum_samples.json` can be used; no preview, WAV-folder import or note-map editing. |
| [Drums rework: the live checks and outside-the-repo follow-ups](drums-live-verification.md) | partly built | The drums rework merged with live-session and outside-repo checks still open. |
| [macOS port: open checks and embedded editors](macos-port-open-checks.md) | partly built | Cocoa-side changes since 2026-09-29 are unverified on a Mac; embedded editors are not built. |
| [NAM A2-Lite: run slimmable models at reduced size](nam-a2-lite-sizes.md) | partly built | Slimmable NAM models always run at full size; reduced sizes would save CPU. |
| [NAM: load models with a top-level post-stack head](nam-post-stack-head.md) | idea | Some NAM WaveNet exports end in a top-level head after the layer stacks. |
| [Orchestral instruments from open sample libraries (an SFZ sampler)](orchestral-sample-library.md) | idea | No sampled acoustic instruments; strings, winds and brass are faked on the wavetable synth. |
| [Preset library: what round 2 left out](preset-library-remaining.md) | partly built | Preset library round 2 left P9 partly built and P10 unbuilt. |
| [Third-party plugin hosting: catalog UI, scan control and isolation](third-party-hosting-gaps.md) | partly built | Users cannot see or steer the plugin scan; no catalog UI, scan folders or crash isolation. |
| [Wavetable synth: Mod Wheel and Aftertouch mod sources](wavetable-mod-wheel-aftertouch.md) | partly built | Mod Wheel and Aftertouch are listed as wavetable mod sources but do nothing. |
