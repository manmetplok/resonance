//! `external_*` — external-instrument tracks: outboard synths played
//! over MIDI whose audio returns on an input.

use crate::server::ResonanceMcp;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};
use resonance_control::methods::external;

#[tool_router(router = router_external, vis = "pub(crate)")]
impl ResonanceMcp {
    #[tool(
        description = "List the hardware this machine currently offers: MIDI output ports to \
                       play a synth through, and audio inputs its sound can come back on. The \
                       names are exactly what external_set_midi_out and external_set_return \
                       take — do not invent or abbreviate them, and re-read after the user \
                       plugs something in.",
        annotations(read_only_hint = true, destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<external::DevicesView>()
    )]
    async fn external_devices(&self) -> Result<CallToolResult, McpError> {
        self.invoke_structured(external::DEVICES, &serde_json::json!({}))
            .await
    }

    #[tool(
        description = "Read every external-instrument track's full wiring: MIDI out port and \
                       channel, audio return device and port, patch, latency offset, \
                       monitoring, record-arm, playback source and how many takes are \
                       recorded. Pass track_id for one track. \
                       \
                       READ THIS BEFORE CONCLUDING AN EXTERNAL TRACK IS BROKEN. The `status` \
                       field says which half is missing: \"unconfigured\" = no MIDI out, so \
                       the synth is never played; \"configuring\" = no audio return or \
                       monitoring off, so nothing can be heard; \"live\" = fully wired; \
                       \"offline\" = a configured device was unplugged (the route is kept, a \
                       replug restores it). An external track makes no sound of its own — it \
                       is not a plugin — so silence with status \"unconfigured\" is missing \
                       setup, not a bug.",
        annotations(read_only_hint = true, destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<external::StatusView>()
    )]
    async fn external_status(
        &self,
        Parameters(params): Parameters<external::StatusParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(external::STATUS, &params).await
    }

    #[tool(
        description = "Put an existing instrument track into external-instrument mode, so it \
                       drives outboard hardware instead of a plugin. Its MIDI clips are then \
                       played out a hardware port rather than to a synth plugin, and its sound \
                       has to arrive back on an audio input. \
                       \
                       To create one from scratch, prefer track_add with kind \"external\" — \
                       one call instead of two. After enabling, wire both halves \
                       (external_set_midi_out AND external_set_return) or the track stays \
                       silent. \
                       \
                       If the track already carried an in-app INSTRUMENT it is removed here: on \
                       an external track every plugin is an insert, so a leftover synth runs \
                       first and overwrites the incoming hardware audio with its own silence \
                       (audible in the app, -120 dBFS in every offline render). Effects on the \
                       track are kept. Undoable.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn external_enable(
        &self,
        Parameters(params): Parameters<external::TrackParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(external::ENABLE, &params).await
    }

    #[tool(
        description = "Take a track out of external-instrument mode, dropping its hardware \
                       route, patch and latency offset. Recorded takes on the track are NOT \
                       deleted, but with the route gone the track has no sound source until \
                       you give it an instrument plugin. Undoable.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn external_disable(
        &self,
        Parameters(params): Parameters<external::TrackParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(external::DISABLE, &params).await
    }

    #[tool(
        description = "Pick the hardware MIDI output port and/or channel the track plays its \
                       notes to — the first half of an external instrument's route. device \
                       must be a name from external_devices (an unknown name is rejected \
                       rather than stored, because a wrong one silently swallows every note); \
                       explicit null disconnects. channel is 1..=16, matching what the app \
                       shows. Omit a field to leave it alone. Undoable.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn external_set_midi_out(
        &self,
        Parameters(params): Parameters<external::SetMidiOutParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(external::SET_MIDI_OUT, &params).await
    }

    #[tool(
        description = "Pick the audio input the synth's sound comes back on — the second half \
                       of the route, and the one that is usually missing when an external \
                       track is inaudible. device is a name from external_devices; port is the \
                       0-based first channel on it (a stereo pair uses port and port+1). \
                       Explicit null device clears the return. Without this the track can \
                       neither be heard nor recorded. Undoable.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn external_set_return(
        &self,
        Parameters(params): Parameters<external::SetReturnParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(external::SET_RETURN, &params).await
    }

    #[tool(
        description = "Select a sound on the synth by sending Bank Select + Program Change. \
                       program is 0..=127; bank is the combined 14-bit value (MSB << 7 | LSB). \
                       Sending both in one call fires one patch change — set them separately \
                       and the synth briefly lands on a patch nobody asked for. Explicit null \
                       clears that half (no message sent for it). The change reaches the \
                       hardware immediately and is re-sent when the project reopens. Undoable.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn external_set_patch(
        &self,
        Parameters(params): Parameters<external::SetPatchParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(external::SET_PATCH, &params).await
    }

    #[tool(
        description = "Set the manual round-trip latency offset, in samples, that aligns the \
                       synth's returning audio with the timeline. Positive delays the return. \
                       Hardware round trips are real and audible — the rest of the mix is \
                       delayed to meet this figure — so guessing is worse than measuring: \
                       prefer external_detect_latency. Undoable.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn external_set_latency(
        &self,
        Parameters(params): Parameters<external::SetLatencyParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(external::SET_LATENCY, &params).await
    }

    #[tool(
        description = "Measure the round-trip latency: the engine fires a MIDI impulse at the \
                       synth and times how long the audio takes to come back, then stores the \
                       offset. Needs both halves of the route wired and the transport stopped; \
                       it makes an audible click. The measurement lands asynchronously — read \
                       external_status back for latency_offset_samples, or \
                       latency_detect_error if it failed (no return, silent synth, nothing \
                       heard in the listen window). \
                       \
                       A measurement REPLACES the stored offset, larger or smaller, so \
                       re-running always reports what it just heard. (It used to clamp up to \
                       the stored value, which made a re-measurement return the previous \
                       reading verbatim.)",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn external_detect_latency(
        &self,
        Parameters(params): Parameters<external::TrackParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(external::DETECT_LATENCY, &params).await
    }

    #[tool(
        description = "Hear, or stop hearing, the synth's audio return through the mix. An \
                       external track with monitoring off is silent no matter how well it is \
                       wired — this is the switch that makes a live external instrument \
                       audible. Declarative: enabled: true on an already-monitoring track is \
                       a no-op, not a flip. Undoable.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn external_set_monitor(
        &self,
        Parameters(params): Parameters<external::SetMonitorParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(external::SET_MONITOR, &params).await
    }

    #[tool(
        description = "Arm the track so a record pass captures the synth's audio return to the \
                       timeline. Arming alone records nothing — the user still has to run the \
                       transport in record — and an armed track keeps monitoring live even \
                       where takes exist, so a punch-in works. To capture a part without a \
                       manual record pass, use external_bounce instead. Declarative, undoable.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn external_set_record_arm(
        &self,
        Parameters(params): Parameters<external::SetRecordArmParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(external::SET_RECORD_ARM, &params).await
    }

    #[tool(
        description = "Choose what the track plays: \"live\" re-drives the hardware from the \
                       track's MIDI on every pass, \"recorded\" plays the recorded takes over \
                       the spans they cover (live in the gaps). \
                       \
                       THIS IS WHAT DECIDES WHETHER AN EXTERNAL PART EXISTS IN A MIXDOWN. An \
                       offline render cannot play a synth that lives outside the computer, so \
                       a \"live\" track renders silence — capture it first (external_bounce, \
                       or a record pass) and set \"recorded\". The app switches to \
                       \"recorded\" by itself when a take finishes recording. Undoable.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn external_set_playback_source(
        &self,
        Parameters(params): Parameters<external::SetPlaybackSourceParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(external::SET_PLAYBACK_SOURCE, &params).await
    }

    #[tool(
        description = "Capture the synth to audio, in real time: the app plays the track's \
                       MIDI to the hardware and records what comes back onto a fresh audio \
                       track, muting everything else for the pass. This is how an external \
                       part becomes something that renders, exports and survives the synth \
                       being unplugged. \
                       \
                       device and port default to the track's own audio return, which is \
                       normally right. Needs the transport stopped and MIDI on the track — \
                       there is nothing to play a silent track to. THE REPLY MEANS STARTED, \
                       NOT FINISHED: the capture takes as long as the part does, and the new \
                       track appears in song_tracks when it lands. It makes real sound in the \
                       room while it runs.",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn external_bounce(
        &self,
        Parameters(params): Parameters<external::BounceParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(external::BOUNCE, &params).await
    }
}
