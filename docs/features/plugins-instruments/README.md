# Plugins: instruments, content libraries and the plugin platform

This area covers the first-party instruments (the wavetable synth and the
sampled drums), the amp plugin and its NAM inference, the content libraries
they draw from (drum kits, amp models, plugin presets), and the platform
every first-party plugin is built on: the `resonance-plugin` SDK, the egui
editor runtimes for Linux and macOS, the DSP and metering libraries, the
golden-test plumbing and the bundling script. It also describes third-party
CLAP hosting as the user sees it; the host internals are in
[audio-engine](../audio-engine/), and the effect plugins (EQ, compressor,
reverb, mastering, …) are in [plugins-effects](../plugins-effects/).

| Feature | Doc | One-line summary |
|---|---|---|
| Wavetable synth | [wavetable-synth.md](wavetable-synth.md) | 32-voice, 2-oscillator wavetable synth with warp, FM/ring/sync, sub/noise, five filter models, 8-slot mod matrix, FX chain, user wavetables and 28 factory presets. |
| Drums plugin | [drums-plugin.md](drums-plugin.md) | 30-pad multi-mic sampler with velocity layers, round robin, choke, mic banks, disk streaming, seven-port Multi output and a Pads/Mix/Setup editor. |
| Drum kit library | [drum-kit-library.md](drum-kit-library.md) | Shared on-disk kit library with stable slots, marks, import, plok.org downloads, and `drum_kits.list` / `set_marks`. |
| Amp plugin | [amp-plugin.md](amp-plugin.md) | NAM player (WaveNet A1/A2, LSTM) with crossfaded model swaps, pass-through on a missing model, a tuner and parity tests against upstream. |
| NAM model library | [nam-model-library.md](nam-model-library.md) | Shared amp-model library by content hash, Tone3000 sign-in and downloads, and `amp_models.list` / `set_marks`. |
| Plugin preset library | [plugin-preset-library.md](plugin-preset-library.md) | One preset format, browser and marks for every plugin (first- and third-party), in each editor and in the app, with `presets.*` and per-slot preset tools. |
| Plugin SDK | [plugin-sdk.md](plugin-sdk.md) | `ResonancePlugin`, params, state upgrades, `export_clap!`, first-party ids and the private CLAP extensions. |
| Plugin editor runtimes | [plugin-editor-runtimes.md](plugin-editor-runtimes.md) | `plugin-gui-core` plus the Wayland and Cocoa runtimes that open every editor as a floating egui window. |
| DSP building blocks | [dsp-building-blocks.md](dsp-building-blocks.md) | `resonance-dsp`: filters, oversampling, saturation, reverb, granular, pitch, time-stretch and other shared primitives. |
| Metering library | [metering-library.md](metering-library.md) | `resonance-metering`: BS.1770 loudness, true peak, LRA, spectrum, offline mix analysis, decay, probe and detail proxies. |
| DSP goldens and skill keys | [dsp-test-support.md](dsp-test-support.md) | `resonance-dsp-test-support`: golden file IO, `RESONANCE_BLESS`, diffs, and the skill ↔ param-key check. |
| Third-party plugins | [third-party-plugin-hosting.md](third-party-plugin-hosting.md) | Scan folders, additive rescan, generic param panel, native editors, and missing-plugin slots that keep their state. |
| Plugin bundling | [plugin-bundling.md](plugin-bundling.md) | `scripts/bundle.sh` (derived list, cross-checks, macOS bundles) and the checklist for a new plugin crate. |

Open work for this area is in
[`docs/intents/plugins-instruments/`](../../intents/plugins-instruments/).
