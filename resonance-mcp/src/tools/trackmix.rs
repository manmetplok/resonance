//! `track_*` / `mixer_*` — track lifecycle, per-track plugins, and mix
//! parameters. All mutations are normal undoable edits.

use crate::server::ResonanceMcp;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};
use resonance_control::methods::{mixer, track};

#[tool_router(router = router_trackmix, vis = "pub(crate)")]
impl ResonanceMcp {
    #[tool(
        description = "Add a track. kind: instrument | drums | vocal | audio; optional name. \
                       Returns the new track_id — use it in every later track/mixer/clip call. \
                       Give instrument tracks a sound with track_add_instrument.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<track::AddResult>()
    )]
    async fn track_add(
        &self,
        Parameters(params): Parameters<track::AddParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(track::ADD, &params).await
    }

    #[tool(
        description = "Rename a track (track_id from song_summary/song_tracks).",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn track_rename(
        &self,
        Parameters(params): Parameters<track::RenameParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(track::RENAME, &params).await
    }

    #[tool(
        description = "Delete a track and everything on it. Destructive: refused with a \
                       summary of what would be lost until you pass confirm: true.",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn track_delete(
        &self,
        Parameters(params): Parameters<track::DeleteParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(track::DELETE, &params).await
    }

    #[tool(
        description = "Set a track's instrument to a built-in plugin. plugin_id is the plugin's \
                       CLAP id, which always has the form \"com.resonance.<name>\" — the two \
                       built-in instruments are \"com.resonance.wavetable\" (the polyphonic \
                       synth, what an instrument track's generated/authored MIDI plays through) \
                       and \"com.resonance.drums\" (the kit, for a drums track). track_plugins \
                       lists the catalog and a wrong id is rejected with the valid ids. An empty \
                       instrument list means the first-party CLAP bundles were never built \
                       (scripts/bundle.sh), not that the app ships no instruments. Then shape \
                       the sound with track_plugin_params / track_set_plugin_param — the \
                       wavetable synth exposes ~90 parameters and its default patch is only a \
                       starting point.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn track_add_instrument(
        &self,
        Parameters(params): Parameters<track::AddPluginParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(track::ADD_INSTRUMENT, &params).await
    }

    #[tool(
        description = "Append a built-in effect to a track's insert chain. plugin_id is the \
                       plugin's CLAP id, of the form \"com.resonance.<name>\" — e.g. \
                       \"com.resonance.reverb\", \"com.resonance.delay\", \"com.resonance.eq\", \
                       \"com.resonance.compressor\", \"com.resonance.mastering\". track_plugins \
                       lists the catalog and a wrong id is rejected with the valid ids. Each \
                       call APPENDS another instance, so calling it twice with the same id gives \
                       the track two of that effect (address them by occurrence in \
                       track_plugin_params / track_set_plugin_param). Effect parameters are set \
                       the same way as instrument ones, by naming plugin_id.",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn track_add_effect(
        &self,
        Parameters(params): Parameters<track::AddPluginParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(track::ADD_EFFECT, &params).await
    }

    #[tool(
        description = "Route a track's audio: output is either \"master\" (sum straight into \
                       the master output) or {\"bus_id\": N} to send it through a group bus \
                       first, where it is summed with the bus's other members and processed by \
                       the bus's own effects and fader before reaching master. bus_id comes \
                       from bus_create, or from song_summary's entries with kind: \"bus\". A \
                       track's current destination is the output field in song_summary / \
                       song_tracks. Setting the routing it already has is a no-op. Undoable.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn track_set_output(
        &self,
        Parameters(params): Parameters<track::SetOutputParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(track::SET_OUTPUT, &params).await
    }

    #[tool(
        description = "Add an aux SEND: an extra tap from a track into a return bus, on top of \
                       wherever the track's main output goes. Returns the send_id in the reply. \
                       to_bus comes from bus_create; that bus is flagged as a return bus \
                       automatically. \
                       \
                       USE SENDS, NOT INSERTS, FOR REVERB AND DELAY. One reverb fed from \
                       several tracks is what puts those tracks in the same room, and that \
                       shared space is most of what makes a mix cohere. A reverb inserted on \
                       each track puts every instrument in a different building and costs far \
                       more CPU. \
                       \
                       DEPTH is the dimension faders cannot reach, and it is built here: more \
                       wet signal reads as further away, less as closer. pre_fader: false (the \
                       default) taps AFTER the track's fader, so pulling the track down takes \
                       its reverb with it — that is almost always what you want. pre_fader: \
                       true keeps the send level independent of the fader, which is for effect, \
                       not for space. To keep a source forward and intelligible while still \
                       putting it in a room, use 20 ms or more of pre-delay on the reverb \
                       itself; to push it further back, roll off the highs on the return. \
                       \
                       CAVEAT: aux sends are NOT SAVED YET (ba todo #482). A send created here \
                       is lost when the project is saved and reloaded, so treat it as a live \
                       mixing decision and tell the user it will not survive a reload.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<track::AddSendResult>()
    )]
    async fn track_add_send(
        &self,
        Parameters(params): Parameters<track::AddSendParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(track::ADD_SEND, &params).await
    }

    #[tool(
        description = "Change an existing aux send: level_db (how much signal reaches the \
                       return — more wet reads as further away), pre_fader (tap before or \
                       after the track's own fader; post is the usual choice), enabled (silence \
                       the send without losing its routing and level), or to_bus (re-route it \
                       into a different return). Omitted fields keep their current value, and \
                       setting a value it already has is a no-op. send_id comes from \
                       track_add_send or from the sends array in song_tracks. A route that \
                       would feed back is refused. Aux sends are not saved yet (ba todo #482).",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn track_set_send(
        &self,
        Parameters(params): Parameters<track::SetSendParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(track::SET_SEND, &params).await
    }

    #[tool(
        description = "Delete an aux send. The track keeps its main output routing and the \
                       return bus keeps its other feeds; only this one tap goes away. send_id \
                       comes from track_add_send or the sends array in song_tracks. To silence \
                       a send but keep it configured, use track_set_send with enabled: false \
                       instead.",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn track_remove_send(
        &self,
        Parameters(params): Parameters<track::RemoveSendParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(track::REMOVE_SEND, &params).await
    }

    #[tool(
        description = "Remove one effect from a track's insert chain — the counterpart to \
                       track_add_effect, which APPENDS a fresh instance on every call, so \
                       without this a wrong add was permanent. Address the effect EITHER by \
                       slot (the 0-based chain position track_plugin_params reports) OR by \
                       plugin_id plus occurrence (which copy, when the track carries the same \
                       effect twice). Both forms together, or neither, is rejected rather than \
                       guessed at. The track's INSTRUMENT cannot be removed this way — replace \
                       it with track_add_instrument. Slots renumber after a removal, so re-read \
                       track_plugin_params before removing a second one. Undoable.",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn track_remove_effect(
        &self,
        Parameters(params): Parameters<track::RemoveEffectParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(track::REMOVE_EFFECT, &params).await
    }

    #[tool(
        description = "List a track's plugins and every parameter each one exposes — id, name, \
                       current value, min, max and default. Omit plugin_id for all of them. \
                       Plugins are named by the CLAP id song_tracks already shows (its \
                       instrument field or an entry of its effects array); occurrence \
                       disambiguates a track carrying the same plugin twice. Each entry also \
                       carries slot — its 0-based position in the chain, instrument included, \
                       so slot order IS processing order and slot is what track_remove_effect \
                       takes. Read this before track_set_plugin_param to learn the parameter \
                       names and their ranges.",
        annotations(read_only_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<track::PluginParamsView>()
    )]
    async fn track_plugin_params(
        &self,
        Parameters(params): Parameters<track::PluginParamsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(track::PLUGIN_PARAMS, &params).await
    }

    #[tool(
        description = "Set one plugin parameter, so a track can sound like something other than \
                       the plugin's default patch. param takes the parameter's name \
                       (case-insensitive) or its numeric id as a string — both come from \
                       track_plugin_params. plugin_id names the plugin; omitted it targets the \
                       track's instrument, which is the usual case for shaping a synth. A value \
                       outside the parameter's min..=max is rejected with the range rather than \
                       clamped. Repeated sets of the same parameter collapse into one undo \
                       entry. \
                       \
                       The min/max track_plugin_params reports are f64 renderings of f32 plugin \
                       declarations, so a bound often reads with a long tail of digits (a \
                       minimum the plugin calls 0.1 shows as 0.10000000149011612). Send the \
                       clean value — 0.1 is accepted and clamped onto the true bound. There is \
                       no need to copy the long form back.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn track_set_plugin_param(
        &self,
        Parameters(params): Parameters<track::SetPluginParamParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(track::SET_PLUGIN_PARAM, &params).await
    }

    #[tool(
        description = "The built-in plugin catalog: every installed plugin's CLAP id (\
                       \"com.resonance.wavetable\", \"com.resonance.reverb\", ...) with its name \
                       and kind (instrument | effect). These ids are what track_add_instrument / \
                       track_add_effect take, and what track_plugin_params addresses plugins by. \
                       Read-only. An empty result means the first-party CLAP bundles have not \
                       been built.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<track::PluginCatalog>()
    )]
    async fn track_plugins(&self) -> Result<CallToolResult, McpError> {
        self.invoke_structured(track::PLUGINS, &()).await
    }

    #[tool(
        description = "Set a track's fader gain. volume is linear: 1.0 = unity, 0.0 = silence. \
                       For balance work use mixer_set_volume_db instead — the same fader in \
                       decibels, the unit loudness differences are measured in.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn mixer_set_volume(
        &self,
        Parameters(params): Parameters<mixer::SetVolumeParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(mixer::SET_VOLUME, &params).await
    }

    #[tool(
        description = "Set a track's fader in DECIBELS — prefer this over mixer_set_volume for \
                       any balance work. 0 dB is unity (no change), negative attenuates, \
                       positive boosts; the accepted range is -60..=+6 dB, and -60 dB is \
                       silence (the app has no -inf). A value outside that range is rejected \
                       with the range rather than clamped. \
                       \
                       1 LU == 1 dB, so a track measuring 5.2 LU louder than you want is fixed \
                       by subtracting 5.2 from its current volume_db — which song_summary and \
                       song_tracks report per track. That is one subtraction; going through \
                       linear gain means reimplementing new = old * 10^(err/20) every time, \
                       which is where balance arithmetic usually goes wrong. \
                       \
                       Applying the SAME gain change to every fader does NOT change the \
                       balance: it is a scalar and preserves every relationship between tracks \
                       exactly. If a mix is too quiet overall, that is a master-level or \
                       limiting problem, not a per-track one. Undoable like a manual fader \
                       move.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn mixer_set_volume_db(
        &self,
        Parameters(params): Parameters<mixer::SetVolumeDbParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(mixer::SET_VOLUME_DB, &params).await
    }

    #[tool(
        description = "Set a track's stereo pan: -1.0 hard left .. 1.0 hard right, 0 center.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn mixer_set_pan(
        &self,
        Parameters(params): Parameters<mixer::SetPanParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(mixer::SET_PAN, &params).await
    }

    #[tool(
        description = "Mute or unmute a track. Setting the state it is already in is a no-op \
                       (no undo entry). Muting silences the track in playback and in the master \
                       bounce but does NOT shorten the bounce: render_mixdown derives the file's \
                       span from every clip in the project, before mute/solo are applied. \
                       Isolating a track with mute (mute everything else) and with solo therefore \
                       do NOT produce comparably-timed files — see mixer_set_solo.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn mixer_set_mute(
        &self,
        Parameters(params): Parameters<mixer::SetMuteParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(mixer::SET_MUTE, &params).await
    }

    #[tool(
        description = "Solo or unsolo a track: every non-soloed track is silenced. Setting the \
                       state it is already in is a no-op. CAUTION when using this to isolate a \
                       part for inspection: a bounce taken with a track soloed has been observed \
                       to span only the soloed content, while the same isolation done with mutes \
                       bounces the full song — so an offset measured in one file does not \
                       transfer to the other, and neither is a trustworthy source of absolute bar \
                       positions. Read positions from song_tracks (clip start/length) and \
                       song_notes (clip-relative beats) instead of measuring them in audio.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn mixer_set_solo(
        &self,
        Parameters(params): Parameters<mixer::SetSoloParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(mixer::SET_SOLO, &params).await
    }
}
