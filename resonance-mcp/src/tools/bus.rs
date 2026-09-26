//! `bus_*` — group busses: the stage between tracks and master.

use crate::server::ResonanceMcp;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};
use resonance_control::methods::{bus, plugin_preset, track};
use resonance_control::MutationAck;

#[tool_router(router = router_bus, vis = "pub(crate)")]
impl ResonanceMcp {
    #[tool(
        description = "Create a group bus and return its bus_id IN THE REPLY. The bus exists \
                       for track_set_output and track_add_send the moment that reply lands — \
                       no polling, no waiting on the audio engine — so you can route tracks \
                       into it on the very next call. \
                       \
                       A bus sums a group of tracks to one point before master. That buys two \
                       things faders cannot: you can process the group AS ONE — a single \
                       compressor across the whole kit glues it together in a way that \
                       per-track compressors never do — and you can move the group's level \
                       without disturbing the balance inside it. A drum bus is the usual first \
                       one. \
                       \
                       Busses show up in song_summary and song_tracks as entries with \
                       kind: \"bus\", from a distinct id range, so there is no separate list \
                       call. Route tracks in with track_set_output.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<bus::CreateResult>()
    )]
    async fn bus_create(
        &self,
        Parameters(params): Parameters<bus::CreateParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(bus::CREATE, &params).await
    }

    #[tool(
        description = "Delete a bus. Its member tracks are NOT deleted and NOT silenced — they \
                       fall back to routing straight to master — but the bus's own level and \
                       effect chain are gone, so the group will sound different. Refused with a \
                       list of the affected tracks until you pass confirm: true.",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn bus_delete(
        &self,
        Parameters(params): Parameters<bus::DeleteParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(bus::DELETE, &params).await
    }

    #[tool(
        description = "Set a bus's fader, in decibels (0 = unity, range -60..=+6). This is the \
                       lever for moving a whole group against the rest of the mix — pull the \
                       drum bus down 2 dB and every drum track keeps its internal balance \
                       exactly. song_summary reports the current value as the bus entry's \
                       volume_db. Undoable.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn bus_set_volume(
        &self,
        Parameters(params): Parameters<bus::SetVolumeParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(bus::SET_VOLUME, &params).await
    }

    #[tool(
        description = "Insert an effect on a BUS — i.e. on the SUM of everything routed into \
                       it. Returns {plugin_id, occurrence, slot}, the handle \
                       bus_set_plugin_param / bus_remove_effect take. \
                       \
                       THIS IS THE ONLY WAY TO CONTROL A PEAK THAT SEVERAL SOURCES CREATE \
                       TOGETHER. A drum kit can peak near clipping while no individual tap \
                       exceeds -8 dBFS, because the peak is kick plus snare plus crash landing \
                       at the same instant. A compressor inserted on each contributing track \
                       never sees that peak — none of them individually is loud enough to \
                       trigger it. One compressor on the bus does, and it is also what \
                       \"glues\" a group: the whole kit moves as one instrument instead of \
                       several. \
                       \
                       It is likewise what to reach for instead of inserting on a \
                       multi-output instrument's own track. A track's chain only sees that \
                       track's main output, and for com.resonance.drums the main output \
                       carries no audio at all — every pad routes to a group port and lands on \
                       a sub-track. Route those sub-tracks into a bus with track_set_output, \
                       then insert here. \
                       \
                       plugin_id is a CLAP id of the form \"com.resonance.<name>\"; \
                       plugins_catalog lists them. Instruments are refused — a bus is handed \
                       audio, not notes. Each call APPENDS another instance. Unlike aux sends, \
                       bus effects ARE saved with the project. Undoable.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<track::AddPluginResult>()
    )]
    async fn bus_add_effect(
        &self,
        Parameters(params): Parameters<bus::AddEffectParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(bus::ADD_EFFECT, &params).await
    }

    #[tool(
        description = "Remove one effect from a bus's chain. Address it EITHER by slot (the \
                       0-based position bus_plugin_params reports) OR by plugin_id plus \
                       occurrence (which copy, when the bus carries the same effect twice). \
                       Both forms together, or neither, is rejected rather than guessed at — a \
                       wrong guess takes the wrong processor off an entire group. Slots \
                       renumber after a removal, so re-read bus_plugin_params before removing a \
                       second one. Undoable.",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn bus_remove_effect(
        &self,
        Parameters(params): Parameters<bus::RemoveEffectParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(bus::REMOVE_EFFECT, &params).await
    }

    #[tool(
        description = "Move an effect to a different position in a bus's chain. Addressed like \
                       bus_remove_effect, plus to_slot for where it should end up. \
                       \
                       The chain runs front to back over the group sum, so order is audible: a \
                       compressor before an EQ reacts to the un-EQ'd signal, and a limiter \
                       belongs last because anything after it can push the sum back over the \
                       ceiling it exists to hold. Unlike a track, a bus has no instrument \
                       pinned at slot 0 — every position is a valid destination. to_slot past \
                       the end clamps to the end, and moving an effect to where it already sits \
                       is an accepted no-op. Undoable.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn bus_move_effect(
        &self,
        Parameters(params): Parameters<bus::MoveEffectParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(bus::MOVE_EFFECT, &params).await
    }

    #[tool(
        description = "Put a different plugin in one of a bus's chain slots, KEEPING its \
                       position. Addressed like bus_remove_effect — by slot OR by plugin_id \
                       plus occurrence — plus new_plugin_id, a CLAP id from plugins_catalog. \
                       \
                       Use this instead of remove + add: bus_add_effect only APPENDS, so the \
                       pair moves the plugin to the end of the chain, and order over a group \
                       sum is audible. \
                       \
                       It is also how a MISSING plugin is recovered. bus_plugin_params reports \
                       status: \"missing\" for a slot whose plugin is not installed on the \
                       machine running the app: the slot holds its position but nothing is \
                       behind it. Pass the SAME plugin_id as new_plugin_id to RELOCATE it \
                       (after installing it and calling plugins_rescan), which restores the \
                       settings the project saved; pass a different id to SWAP it, keeping the \
                       position but discarding those settings. outcome in the reply says which \
                       happened. Do not remove a missing plugin to tidy up — removal is what \
                       destroys its recoverable settings. Undoable.",
        annotations(destructive_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<track::ReplaceEffectResult>()
    )]
    async fn bus_replace_effect(
        &self,
        Parameters(params): Parameters<bus::ReplaceEffectParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(bus::REPLACE_EFFECT, &params).await
    }

    #[tool(
        description = "Bypass or re-engage a bus's ENTIRE effect chain in one call — the A/B \
                       for \"is this bus processing helping?\". bypassed: true passes the \
                       group through unprocessed; false puts the chain back. This SETS the \
                       state rather than toggling it, so sending the same request twice is \
                       safe: a retry cannot flip the chain back on. Setting the state it is \
                       already in is a no-op. The plugins and their settings are kept either \
                       way. Undoable.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn bus_set_fx_bypass(
        &self,
        Parameters(params): Parameters<bus::SetFxBypassParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(bus::SET_FX_BYPASS, &params).await
    }

    #[tool(
        description = "Bypass or re-engage ONE plugin in a bus's effect chain, leaving the rest of the chain \
                       running — the A/B for \"is this one plugin earning its place?\". \
                       Distinct from bus_set_fx_bypass, which mutes the whole chain: the \
                       two are independent, so a chain-bypassed bus still remembers \
                       which slots were individually bypassed and re-engaging the chain \
                       restores the mix rather than switching everything on. Address the \
                       plugin exactly as bus_set_plugin_param does (plugin_id + \
                       occurrence; omitted means the first plugin). Read bus_plugin_params to see \
                       each slot's current bypassed flag. The engine crossfades over a few \
                       milliseconds, so toggling mid-playback does not click. SETS rather \
                       than toggles, so a retry is safe. Undoable, and saved with the \
                       project.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn bus_set_plugin_bypass(
        &self,
        Parameters(params): Parameters<bus::SetPluginBypassParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(bus::SET_PLUGIN_BYPASS, &params).await
    }

    #[tool(
        description = "List a bus's effects and every parameter each one exposes — id, name, \
                       current value, min, max, default, and what the value MEANS: text (the \
                       plugin's own rendering), unit, module, stepped, choices and hidden, \
                       exactly as track_plugin_params reports them. Omit plugin_id for all of \
                       them. \
                       The entries are the same shape track_plugin_params returns, including \
                       slot (0-based chain position, which IS processing order) and occurrence \
                       (which copy of a repeated effect). Every entry is kind: \"effect\" — a \
                       bus has no instrument. Read this before bus_set_plugin_param to learn \
                       the parameter names and their ranges.",
        annotations(read_only_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<bus::PluginParamsView>()
    )]
    async fn bus_plugin_params(
        &self,
        Parameters(params): Parameters<bus::PluginParamsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(bus::PLUGIN_PARAMS, &params).await
    }

    #[tool(
        description = "Set one parameter on a plugin inserted on a bus — the counterpart to \
                       track_set_plugin_param, and what makes a bus compressor actually useful \
                       rather than just present at its default patch. param takes the \
                       parameter's name (case-insensitive) or its numeric id as a string, both \
                       from bus_plugin_params, or on a com.resonance.* plugin its stable string \
                       key (e.g. \"threshold\"). plugin_id names which effect; omitted it targets \
                       the bus's first, which is unambiguous only on a one-effect chain. A \
                       value outside the parameter's min..=max is rejected with the range \
                       rather than clamped, but a value that rounds onto an f32-declared bound \
                       (0.1 against a reported 0.10000000149011612) is accepted. value also \
                       takes a choice label as a string (\"Low-pass\") for any parameter \
                       bus_plugin_params reports choices for. Repeated sets of the same \
                       parameter collapse into one undo entry.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn bus_set_plugin_param(
        &self,
        Parameters(params): Parameters<bus::SetPluginParamParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(bus::SET_PLUGIN_PARAM, &params).await
    }

    #[tool(
        description = "Route another track's or bus's audio into the external SIDECHAIN KEY of \
                       a plugin ON A BUS — the detector input. This is the tool for the classic \
                       move: a compressor on the bass bus keyed from the kick, so the whole \
                       group ducks on every hit. track_set_sidechain cannot express it (it is \
                       keyed on track_id), and it is the bus, not any single track, that the \
                       group actually exists on. \
                       \
                       The plugin is addressed like bus_set_plugin_param: bus_id plus an \
                       optional plugin_id (+ occurrence). Omitting plugin_id targets the first \
                       plugin on the bus that HAS a key port. Name the key source with EITHER \
                       source_track_id OR source_bus_id; any track, bus or SUB-TRACK works, and \
                       a sub-track (one tap of a multi-output drum kit) is usually the only \
                       address a single kit piece has. Sources are tapped post-FX and PRE-fader, \
                       so a key source can sit at -inf on the mixer and still key. enabled \
                       defaults to true; false keeps the routing configured but stops delivering \
                       the key. \
                       \
                       Only the DETECTOR changes: the key never reaches the output. Two plugins \
                       read a key — resonance-compressor (ducking) and resonance-gate. Routing a \
                       key into a plugin with no key port is REFUSED, naming the ones on that \
                       bus that accept it. A bus may legally key a plugin on itself: the key is \
                       delivered one audio block late by design (~2.7 ms), so it cannot feed \
                       back. Routes are saved with the project. Undoable with edit_undo.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<MutationAck>()
    )]
    async fn bus_set_sidechain(
        &self,
        Parameters(params): Parameters<bus::SetSidechainParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(bus::SET_SIDECHAIN, &params).await
    }

    #[tool(
        description = "Remove a bus plugin's external sidechain key route, so its detector goes \
                       back to reading the bus's own signal. Addressed exactly as \
                       bus_set_sidechain. Clearing a plugin that had no route is not an error.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<MutationAck>()
    )]
    async fn bus_clear_sidechain(
        &self,
        Parameters(params): Parameters<bus::ClearSidechainParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(bus::CLEAR_SIDECHAIN, &params).await
    }

    #[tool(
        description = "List the presets a plugin on a bus can recall, factory and user together. \
                       FACTORY presets are built into the plugin — every install has the same \
                       ones and they are read-only. USER presets are ones saved on this machine, \
                       by this tool or in the plugin's own window; only those can be overwritten \
                       or shadow a factory name. Each entry says which set it is from, and \
                       bus_load_plugin_preset takes the name. \
                       \
                       The bank belongs to the PLUGIN, not to the bus, so a preset saved from a \
                       track shows up here too. Only Resonance's own plugins publish factory \
                       presets to the host; a third-party CLAP reports none rather than a guess. \
                       Addressed exactly as bus_plugin_params, except that omitting plugin_id \
                       targets the bus's FIRST plugin (a bus has no instrument).",
        annotations(read_only_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<plugin_preset::PluginPresetsView>()
    )]
    async fn bus_plugin_presets(
        &self,
        Parameters(params): Parameters<bus::PluginPresetsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(bus::PLUGIN_PRESETS, &params).await
    }

    #[tool(
        description = "Recall a preset onto a plugin on a bus — the fast way to get a whole \
                       sound, instead of setting forty parameters one at a time. The name comes \
                       from bus_plugin_presets. \
                       \
                       source picks which set to take it from; omit it and a user preset wins \
                       over a factory one of the same name. The recall is ONE undo entry, shows \
                       in the plugin's own window immediately, and moves every parameter the \
                       preset names — anything it does not name keeps its current value, so a \
                       preset written for an older build still loads.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<MutationAck>()
    )]
    async fn bus_load_plugin_preset(
        &self,
        Parameters(params): Parameters<bus::LoadPluginPresetParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(bus::LOAD_PLUGIN_PRESET, &params)
            .await
    }

    #[tool(
        description = "Save a bus plugin's current sound as a named user preset — the one worth \
                       keeping is usually the glue compressor that finally sat right on the \
                       group. It can be recalled here, on any track carrying the same plugin, or \
                       picked in the plugin's own window later. \
                       \
                       Overwriting an existing USER preset needs overwrite: true; without it the \
                       call is refused rather than replacing the preset. Factory presets are \
                       never touched — saving under a factory name creates a user preset that \
                       shadows it. \
                       \
                       This answers as soon as the capture is armed, not when the file lands: \
                       the plugin hands its state back a beat later. Read bus_plugin_presets to \
                       see the preset appear.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<MutationAck>()
    )]
    async fn bus_save_plugin_preset(
        &self,
        Parameters(params): Parameters<bus::SavePluginPresetParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(bus::SAVE_PLUGIN_PRESET, &params)
            .await
    }
}
