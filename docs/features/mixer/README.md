# Mixer

The Mix tab and the signal routing behind it: channel strips and the
inspector that edits them, faders, pan, mute and solo, busses and track
output routing, aux sends into return busses, sidechain (key) routes, the
master bus, the insert chains on tracks, busses and the master, track
colour, reference-track A/B monitoring, the strip meters, the `meter.*`
measurement API and the mastering assistant's app side (the
`resonance-metering` and `resonance-mastering-assist` crates themselves are
documented in plugins-instruments and plugins-effects). The engine half lives in
`resonance-audio/src/mixer/` and `engine/{busses,chain,sidechain,reference,probe}.rs`;
the app half in `resonance-app/src/{state,update,view/mixer,reference}`.
Open items are in [`../../intents/mixer/`](../../intents/mixer/).

| Feature | Doc | One-line summary |
|---|---|---|
| Channel strips and inspector | [channel-strips-and-inspector.md](channel-strips-and-inspector.md) | Mix tab layout; strips show state, the inspector (CHAIN, SENDS, ROUTING, AUTOMATION, TRACK/BUS/MASTER) edits; inline rename |
| Volume, pan, mute, solo | [volume-pan-mute-solo.md](volume-pan-mute-solo.md) | dB faders with per-block ramps, stereo-balance pan (centre = unity), shared mute/solo resolution, `mixer.*` |
| Busses and output routing | [busses-and-output-routing.md](busses-and-output-routing.md) | Tracks route to master or one bus; busses sum to master; return role; `bus.*`, `track.set_output` |
| Aux sends | [aux-sends.md](aux-sends.md) | Pre/post-fader sends into return busses, cycle check, engine-echo mirror, persistence; `track.*_send` |
| Sidechain routing | [sidechain-routing.md](sidechain-routing.md) | Key a plugin on a track, bus or master from a track or bus; one-block double-buffered taps; control API only |
| Master bus | [master-bus.md](master-bus.md) | Master FX → click → volume → hard clip → meters; master inspector; `master.summary`, `master.set_volume` |
| Plugin chains | [plugin-chains.md](plugin-chains.md) | One `ChainOwner` API for add/move/replace/remove/bypass on all chains; CHAIN rows; generic plugin window |
| Track colour | [track-colour.md](track-colour.md) | Persisted per-track identity colour from a 10-hue palette; sub-tracks follow the parent; `track.set_color` |
| Reference A/B monitoring | [reference-ab-monitoring.md](reference-ab-monitoring.md) | Load references into a rail, flip/hold-B A/B post-master, Mix/Ref/Δ loudness; `reference.load` |
| Strip meters | [metering.md](metering.md) | Post-fader peak meters on every strip (polled, decayed); the `resonance-metering` crate is in [plugins-instruments/metering-library.md](../plugins-instruments/metering-library.md) |
| `meter.*` control API | [meter-control-api.md](meter-control-api.md) | Offline/live loudness, stems, snapshots + loudness-matched compare, insert-chain probe, as jobs |
| Mastering assistant (`master.assist`) | [mastering-assist.md](mastering-assist.md) | App side of `master.assist`: offline render + analysis returning mastering-plugin param suggestions, never applied; the engine crate is in [plugins-effects/mastering-assist.md](../plugins-effects/mastering-assist.md) |
