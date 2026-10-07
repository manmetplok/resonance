# Hardware I/O

Everything that touches the user's physical setup: the audio devices and
PipeWire graph the engine plays and records through, per-track audio and
MIDI inputs, monitoring and recording, the transport and metronome,
outboard synths driven as external-instrument tracks (with device presets
for their patches and parameters), hardware MIDI in/out and clock, MIDI
Learn for control surfaces, and reading audio and MIDI files from disk.
Editing recorded takes, the pool and the tempo map live in
[../timeline/](../timeline/); the render path behind them in
[../audio-engine/](../audio-engine/). Open work is in
[../../intents/hardware-io/](../../intents/hardware-io/).

| Feature | Doc | One-line summary |
|---|---|---|
| Audio devices and PipeWire | [audio-devices-and-pipewire.md](audio-devices-and-pipewire.md) | Native PipeWire output on the default sink, 48 kHz graph-rate assertion, quantum read at startup, input enumeration, I/O latency readout. |
| Audio input and monitoring | [audio-input-and-monitoring.md](audio-input-and-monitoring.md) | Per-track input device/channel and mono; one shared capture stream; monitoring through the track's chain, bus and master. |
| Recording | [recording.md](recording.md) | Arm and record audio and MIDI with a two-bar count-in, latency-compensated take placement, cycle-record takes, one undo per run. |
| Transport and metronome | [transport.md](transport.md) | Play/pause/stop/seek/loop, starting tempo/meter/key, metronome click, shortcuts and `transport.*` methods. |
| External instrument tracks | [external-instruments.md](external-instruments.md) | Outboard synth as a track: MIDI out + audio return, patch, latency offset and ping, live/recorded playback, realtime bounce. |
| Device presets | [device-presets.md](device-presets.md) | JSON definitions of a synth's CC/NRPN parameters and named patches; patch-by-name and hardware parameter automation. |
| Hardware MIDI I/O and clock | [midi-hardware-io.md](midi-hardware-io.md) | Port enumeration and hotplug, per-track MIDI in/out and Thru, timeline → hardware notes, MIDI clock send/receive. |
| MIDI Learn | [midi-learn.md](midi-learn.md) | Control-surface port, learn by right-click, takeover/relative mapping, controller maps, `midi_map.*` methods. |
| Standard MIDI File import | [smf-import.md](smf-import.md) | Import dialog with track review, placement and tempo reconciliation; `notes.import_midi` for agents. |
| Audio file decoding | [audio-file-decoding.md](audio-file-decoding.md) | symphonia decode to stereo at the project rate, sinc resampling, zero-copy mmap of engine WAVs. |
