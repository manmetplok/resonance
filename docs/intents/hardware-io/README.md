# Hardware I/O — intents

Work that is wanted but not built, or only partly built, in the Hardware I/O area.
What is built is documented in [`../../features/hardware-io/`](../../features/hardware-io/README.md).
Each intent follows [`../_template.md`](../_template.md).

| Intent | Status | Why |
|---|---|---|
| [Audio device settings](audio-device-settings.md) | idea | Output device and buffer size cannot be chosen in the app. |
| [Audio input routing beyond one device](audio-input-routing.md) | idea | One capture stream per session: tracks cannot record from different devices at once. |
| [Count-in setting and punch recording](count-in-and-punch.md) | idea | Record always counts in two bars and there is no punch without cycling. |
| [Device definition editor and portability](device-definition-editor.md) | partly built | Only the Moog Muse definition ships and definitions cannot be edited in the app. |
| [MIDI controller data: record, Thru, import](midi-controller-data.md) | idea | Sustain, mod wheel, bends and aftertouch play live but are not recorded, sent Thru or imported. |
| [MIDI Learn: binding editing and more targets](midi-learn-binding-editing.md) | partly built | Bindings support ranges, inversion and pickup, but nothing edits them, and few targets are learnable. |
| [Opening a project at a different sample rate](project-sample-rate-mismatch.md) | idea | Sample-denominated positions are not rescaled when a project opens at another engine rate. |
| [Recording over the control API](record-control-api.md) | idea | An agent cannot set input, monitor or arm on audio tracks, or start a take. |
| [Recording and MIDI Learn: live checks and leftovers](recording-and-midi-learn-leftovers.md) | idea | The recording and MIDI-learn work is proven by offline tests; several checks need real hardware, and a few code leftovers remain. |
| [Sample-accurate hardware MIDI output](sample-accurate-midi-out.md) | idea | Hardware MIDI leaves from the ~16 ms engine thread, so timing jitters. |
| [SMF import leftovers](smf-import-leftovers.md) | partly built | SMF import works, but previews, signature adoption and Format 0/2 handling are missing. |
