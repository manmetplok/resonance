# Wavetable synth: Mod Wheel and Aftertouch mod sources

> Stage: **intent**
> Status: **partly built**
> Area: plugins-instruments
> Source: ba todos #1295 (built) and #1301 (open), noted in
> `plugins/resonance-wavetable/src/dsp/modulation.rs`.
> Register items covered: plugin audit W1.

## Why

The wavetable's mod matrix lists Mod Wheel and Aftertouch as sources, but
they are flagged unavailable and do nothing
(`ModSource::unavailable_reason`: "No MIDI CC or aftertouch reaches plugins
yet"). A player with a controller cannot use the wheel or pressure for
vibrato, filter opens or swells, which is how most synth patches are played.

## What it would do

Mod Wheel (CC 1) and channel/poly aftertouch drive the mod matrix like any
other source, smoothed, per voice where polyphonic pressure is used. The
"unavailable" arm in `ModSource` is deleted and the editor stops greying
the two sources out.

## What exists to build on

- The plugin side of #1295 is built: the CLAP bridge declares the MIDI
  dialect and decodes CC, channel and poly pressure and pitch bend into
  `ControlEvent` (`resonance-plugin/src/clap_bridge/midi.rs`,
  `resonance-plugin/src/plugin.rs`).
- The host forwards live MIDI controllers as `CLAP_EVENT_MIDI`
  (`ClapInstance::queue_midi`, `resonance-audio/src/mixer/live_midi.rs`).
- The wavetable does not read `ControlEvent` anywhere yet.
- `docs/features/plugins-instruments/wavetable-synth.md`.

## Open decisions

1. Whether recorded MIDI clips carry CC and pressure to the plugin, or only
   live input does. Recommended: check the clip path in the engine first;
   if clips drop CCs, that is a separate engine slice.
2. Pitch bend: add a bend-range param in the same change. Recommended: yes,
   it arrives through the same events.

## Slices

1. Read `ControlEvent` in the engine, keep per-channel wheel and pressure
   values, expose them as mod sources; remove the unavailable arm; tests in
   `plugins/resonance-wavetable/tests/mod_sources.rs`.
2. Pitch bend with a range param.

## How we'd know it works

A test feeding CC 1 and pressure events changes the modulated destination
by the expected amount; `mod_availability.rs` no longer lists them as
unavailable; a live check with a controller.
