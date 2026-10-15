# Mixer — intents

Work that is wanted but not built, or only partly built, in the Mixer area.
What is built is documented in [`../../features/mixer/`](../../features/mixer/README.md).
Each intent follows [`../_template.md`](../_template.md).

| Intent | Status | Why |
|---|---|---|
| [Bus channel parity (pan, mute, solo, bus-sourced sends)](bus-control-parity.md) | idea | Busses lack pan/mute over the control API and other controls tracks have. |
| [Bus-to-bus routing that always sounds](bus-to-bus-routing.md) | idea | No bus → bus output, and a bus → bus send into an older bus is silently inaudible. |
| ["Show in Arrange" from the inspector's AUTOMATION rows](inspector-automation-jump.md) | idea | Mixer automation rows cannot jump to the lane on the Arrange view. |
| [Sidechain routing in the GUI](sidechain-gui.md) | partly built | Sidechain routing works end to end over the control API but has no GUI. |
| [Sub-track solo that does something](sub-track-solo.md) | idea | Solo on a multi-output sub-track strip does nothing. |
