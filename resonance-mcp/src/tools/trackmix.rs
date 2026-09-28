//! `track_*` / `mixer_*` — track lifecycle, per-track plugins, and mix
//! parameters. All mutations are normal undoable edits.

use crate::server::ResonanceMcp;
use resonance_control::MutationAck;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};
use resonance_control::methods::{mixer, plugin_preset, plugins, track};

#[tool_router(router = router_trackmix, vis = "pub(crate)")]
impl ResonanceMcp {
    #[tool(
        description = "Add a track. kind: instrument | drums | vocal | audio | external; \
                       optional name. Returns the new track_id — use it in every later \
                       track/mixer/clip call. Give instrument tracks a sound with \
                       track_add_instrument. kind \"external\" creates a track driven by \
                       outboard hardware instead of a plugin; wire it up with \
                       external_set_midi_out + external_set_return.",
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
                       and \"com.resonance.drums\" (the kit, for a drums track). plugins_catalog \
                       lists the catalog and a wrong id is rejected with the valid ids. An empty \
                       instrument list means the first-party CLAP bundles were never built \
                       (scripts/bundle.sh), not that the app ships no instruments. Then shape \
                       the sound with track_plugin_params / track_set_plugin_param — the \
                       wavetable synth exposes ~90 parameters and its default patch is only a \
                       starting point. \
                       \
                       This SETS the instrument: a track has exactly one. If it already has \
                       one, that instrument is swapped for plugin_id in the same slot (its \
                       sound settings are discarded; undoable); naming the instrument that is \
                       already loaded changes nothing, so a retry is safe. Effects stay put. \
                       \
                       Returns {plugin_id, occurrence, slot} — pass plugin_id and occurrence \
                       straight to track_set_plugin_param instead of re-reading the chain to \
                       work out which instance is the new one.",
        annotations(destructive_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<track::AddPluginResult>()
    )]
    async fn track_add_instrument(
        &self,
        Parameters(params): Parameters<track::AddPluginParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(track::ADD_INSTRUMENT, &params).await
    }

    #[tool(
        description = "Append a built-in effect to a track's insert chain. plugin_id is the \
                       plugin's CLAP id, of the form \"com.resonance.<name>\" — e.g. \
                       \"com.resonance.reverb\", \"com.resonance.delay\", \"com.resonance.eq\", \
                       \"com.resonance.compressor\", \"com.resonance.mastering\". plugins_catalog \
                       lists the catalog and a wrong id is rejected with the valid ids. Each \
                       call APPENDS another instance, so calling it twice with the same id gives \
                       the track two of that effect. \
                       \
                       Returns {plugin_id, occurrence, slot}: occurrence is WHICH copy you just \
                       made (0 the first time, 1 the second), and it is what \
                       track_set_plugin_param and track_remove_effect take — so pass it \
                       straight on rather than re-reading the chain and guessing which instance \
                       is new. slot is its position in the chain right now, and slots renumber \
                       whenever anything is removed or moved. \
                       \
                       The effect is visible to track_plugin_params immediately, but its \
                       PARAMETER LIST arrives a moment later from the audio engine, so a \
                       set_plugin_param issued in the very next call can report that the plugin \
                       is still initializing — that is a retry, not a failed add. Effect \
                       parameters are then set the same way as instrument ones, by naming \
                       plugin_id.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<track::AddPluginResult>()
    )]
    async fn track_add_effect(
        &self,
        Parameters(params): Parameters<track::AddPluginParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(track::ADD_EFFECT, &params).await
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
                       The send is saved with the project — routing, level and tap point all \
                       come back on reload.",
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
                       would feed back is refused. The edit is saved with the project.",
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
                       instead. The removal is saved with the project, so a deleted send does \
                       not come back on reload.",
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
        description = "Move one effect to a different position in a track's insert chain. \
                       Address it EITHER by slot OR by plugin_id plus occurrence, exactly as in \
                       track_remove_effect, and give to_slot for where it should end up. \
                       \
                       ORDER IS AUDIBLE, and it is not a matter of tidiness. A chain is applied \
                       front to back, so each effect hears what the one before it produced. An \
                       EQ before a compressor changes what the compressor reacts to — cutting \
                       lows first stops a kick from triggering gain reduction on everything \
                       else; the same EQ after the compressor only reshapes what the compressor \
                       already did. A limiter belongs LAST, because anything placed after it \
                       can push the signal back over the ceiling it exists to hold, and a \
                       reverb or delay generally goes after dynamics so the tail is not itself \
                       squashed. Since track_add_effect only ever appends, the order a chain \
                       ends up in is the order it was built in — this is how to correct that \
                       without tearing the chain down and losing every parameter you set. \
                       \
                       Read the current order from track_plugin_params: each entry's slot is \
                       its 0-based position, and slot order IS processing order. to_slot past \
                       the end of the chain clamps to the end rather than failing, and moving \
                       an effect to where it already sits is an accepted no-op. The track's \
                       INSTRUMENT is not a chain-ordered insert: it cannot be moved, and an \
                       effect cannot be moved in front of it. Undoable.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn track_move_effect(
        &self,
        Parameters(params): Parameters<track::MoveEffectParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(track::MOVE_EFFECT, &params).await
    }

    #[tool(
        description = "Put a different plugin in one of a track's chain slots, KEEPING its \
                       position. Address the slot exactly as track_remove_effect does — by slot \
                       OR by plugin_id plus occurrence — and give new_plugin_id, a CLAP id from \
                       plugins_catalog. \
                       \
                       Use this instead of remove + add. track_add_effect only ever APPENDS, so \
                       the pair moves the plugin to the END of the chain, and chain order is \
                       audible: a compressor that was before the reverb ends up after it. \
                       Unlike remove, this also works on the track's INSTRUMENT. \
                       \
                       Its other job is recovering a MISSING plugin. track_plugin_params \
                       reports status: \"missing\" for a slot whose plugin isn't installed on \
                       the machine running the app — the slot is real and holds its position, \
                       but there is nothing behind it, its params list is empty, and \
                       track_set_plugin_param against it changes no sound. Two ways out: pass \
                       the SAME plugin_id as new_plugin_id to RELOCATE it (after installing the \
                       plugin and calling plugins_rescan), which brings the settings the \
                       project saved back with it; or pass a different id to SWAP it, which \
                       keeps the chain position but discards the missing plugin's saved \
                       settings, because they cannot mean anything to another plugin. The reply \
                       says which of the two happened (outcome: relocated / swapped / \
                       already_loaded). Do NOT remove a missing plugin to 'clean up' — removal \
                       is what destroys its recoverable settings. Undoable.",
        annotations(destructive_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<track::ReplaceEffectResult>()
    )]
    async fn track_replace_effect(
        &self,
        Parameters(params): Parameters<track::ReplaceEffectParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(track::REPLACE_EFFECT, &params).await
    }

    #[tool(
        description = "List a track's plugins and every parameter each one exposes — id, name, \
                       current value, min, max, default, and what the value MEANS: text (the \
                       plugin's own rendering, \"40 %\", \"-6.0 dB\", \"Low-pass\"), unit, \
                       module (the group it sits in), stepped, and choices (the names of a \
                       stepped parameter's values, which track_set_plugin_param accepts \
                       directly). Read text before trusting a number: value 2.0 on a 0..=4 \
                       range is meaningless on its own. hidden marks a parameter the plugin \
                       asks not be shown — still readable and writable, but leave it out of a \
                       listing. Omit plugin_id for all of them. \
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
                       track_plugin_params — or on a com.resonance.* plugin its stable string \
                       key. plugin_id names the plugin; omitted it targets the \
                       track's instrument, which is the usual case for shaping a synth. A value \
                       outside the parameter's min..=max is rejected with the range rather than \
                       clamped. Repeated sets of the same parameter collapse into one undo \
                       entry. \
                       \
                       value takes a number, or — for a parameter track_plugin_params reports \
                       with choices — one of those labels as a string, matched \
                       case-insensitively: send \"Low-pass\" instead of working out that it is \
                       3. A label that matches none of them is rejected with the ones that \
                       would have worked. \
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
        description = "Bypass or re-engage a track's ENTIRE insert chain in one call — the A/B \
                       for \"is this track's processing helping?\". bypassed: true passes the \
                       track through unprocessed; false puts the chain back. The mixer strip \
                       has always had this button; this is the wire equivalent. Does not \
                       disturb the per-slot flags — see track_set_plugin_bypass. SETS rather \
                       than toggles, so a retry cannot flip the chain back on; setting the \
                       state it is already in is a no-op. Plugins and their settings are kept \
                       either way. Undoable.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn track_set_fx_bypass(
        &self,
        Parameters(params): Parameters<track::SetFxBypassParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(track::SET_FX_BYPASS, &params).await
    }

    #[tool(
        description = "Bypass or re-engage ONE plugin on a track, leaving the rest of the \
                       chain running — the A/B for \"is this one plugin earning its place?\". \
                       Distinct from track_set_fx_bypass, which mutes the whole chain: the \
                       two are independent, so a chain-bypassed track still remembers which \
                       slots were individually bypassed and re-engaging the chain restores \
                       the mix rather than switching everything on. Address the plugin \
                       exactly as track_set_plugin_param does (plugin_id + occurrence; \
                       omitted means the track's instrument). Read track_plugin_params to \
                       see each slot's current bypassed flag. The engine crossfades over a \
                       few milliseconds, so toggling mid-playback does not click. SETS \
                       rather than toggles, so a retry is safe. Undoable, and saved with \
                       the project.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn track_set_plugin_bypass(
        &self,
        Parameters(params): Parameters<track::SetPluginBypassParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(track::SET_PLUGIN_BYPASS, &params).await
    }

    #[tool(
        description = "Save a track as a reusable preset: its type, mixer settings, instrument \
                       identity and its whole plugin chain INCLUDING each plugin's internal \
                       state, so recalling it restores the sound and not just the plugin names. \
                       This is how a dialled-in track becomes something you can stamp out \
                       again — with track_apply_preset, or from the app's add-track menu. \
                       \
                       name defaults to the track's own name. A name that already exists is \
                       REFUSED unless you pass overwrite: true, because saving replaces the \
                       stored preset. \
                       \
                       The reply means the capture was started, not finished: the plugins' \
                       state blobs come back from the audio engine a moment later and the \
                       preset is written then. Read track_presets to confirm it landed.",
        annotations(destructive_hint = false, idempotent_hint = false, open_world_hint = false)
    )]
    async fn track_save_preset(
        &self,
        Parameters(params): Parameters<track::SavePresetParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(track::SAVE_PRESET, &params).await
    }

    #[tool(
        description = "List the track presets available to stamp new tracks from: the built-ins \
                       that ship with the app and everything saved with track_save_preset. Each \
                       entry carries its name (what track_apply_preset takes), the kind of \
                       track it makes, whether it is builtin, and the CLAP ids of the plugins \
                       it restores — an empty plugins list means the preset carries mixer \
                       settings only and the new track will make no sound on its own. Takes no \
                       arguments.",
        annotations(read_only_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<track::PresetsView>()
    )]
    async fn track_presets(&self) -> Result<CallToolResult, McpError> {
        self.invoke_structured(track::PRESETS, &()).await
    }

    #[tool(
        description = "Create a new track from a preset — the fastest way to get a known sound \
                       onto the timeline, and the counterpart to track_save_preset. preset is a \
                       name from track_presets, matched case-insensitively; name renames the \
                       new track only, leaving the preset's own name alone. Returns the new \
                       track_id. \
                       \
                       This CREATES a track: it never overwrites an existing one, so it needs \
                       no confirmation. The preset's plugin chain is restored a moment after \
                       the track appears (the audio engine loads it), so read track_plugin_params \
                       rather than assuming the chain is there in the same breath.",
        annotations(destructive_hint = false, idempotent_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<track::AddResult>()
    )]
    async fn track_apply_preset(
        &self,
        Parameters(params): Parameters<track::ApplyPresetParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(track::APPLY_PRESET, &params).await
    }

    #[tool(
        description = "The built-in plugin catalog: every installed plugin's CLAP id (\
                       \"com.resonance.wavetable\", \"com.resonance.reverb\", ...) with its name \
                       and kind (instrument | effect). These ids are what track_add_instrument / \
                       track_add_effect take, and what track_plugin_params addresses plugins by. \
                       Takes no arguments. \
                       \
                       This lists what CAN be loaded, app-wide. To see what a particular track \
                       actually carries — its chain, in processing order, with every parameter \
                       and its range — use track_plugin_params instead. \
                       \
                       Read-only and answerable with no project open, so you can decide what to \
                       build with before creating one. An empty result means the first-party \
                       CLAP bundles have not been built (scripts/bundle.sh).",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<plugins::PluginCatalog>()
    )]
    async fn plugins_catalog(&self) -> Result<CallToolResult, McpError> {
        self.invoke_structured(plugins::CATALOG, &()).await
    }

    #[tool(
        description = "Look for plugins installed since the app started, without restarting it. \
                       The catalog is filled by one scan at launch, so a plugin installed while \
                       Resonance is open is invisible to plugins_catalog and to \
                       track_add_instrument / track_add_effect until this runs. Takes no \
                       arguments. \
                       \
                       Safe to call at any time, including mid-playback: the scan is additive, \
                       so plugins already loaded keep running untouched — nothing is unloaded \
                       and no audio is interrupted. The flip side is that a plugin REMOVED from \
                       disk stays in the catalog until the app restarts. \
                       \
                       The refreshed catalog does not come back in this reply (the scan runs on \
                       the audio engine): read plugins_catalog afterwards, which is also where \
                       a bundle that failed to load is reported, under scan_failures.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn plugins_rescan(&self) -> Result<CallToolResult, McpError> {
        self.invoke(plugins::RESCAN, &()).await
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

    #[tool(
        description = "Route another track's or bus's audio into a plugin's external SIDECHAIN \
                       KEY — the detector input. This is what \"duck the pad from the kick\" \
                       needs; it cannot be done with plugin parameters. \
                       \
                       The plugin is addressed like track_set_plugin_param: track_id plus an \
                       optional plugin_id (+ occurrence). Omitting plugin_id targets the first \
                       plugin on the track that HAS a key port other than resonance-reverb and \
                       resonance-eq — in practice the compressor or gate, never the instrument \
                       at slot 0 (a synth has none); the reverb or the EQ is chosen only when \
                       nothing else on the track has a key port — name it with plugin_id \
                       otherwise. Name the key source with EITHER source_track_id OR \
                       source_bus_id; any track, bus or SUB-TRACK works, and a sub-track (one tap \
                       of a multi-output drum kit) is usually the only address a single kit piece \
                       has. Sources are tapped post-FX and PRE-fader, so a key source can sit at \
                       -inf on the mixer and still key. enabled defaults to true; false keeps the \
                       routing configured but stops delivering the key, so the plugin falls back \
                       to keying off its own input. \
                       \
                       Only the DETECTOR changes: the key never reaches the output, so routing a \
                       kick into a pad's compressor makes the pad duck, it does not add kick to \
                       the pad. Four plugins currently read a key — resonance-compressor \
                       (ducking), resonance-gate (open/close from another source), \
                       resonance-reverb (ducks its wet return; set duck_amount > 0, otherwise it \
                       keys nothing) and resonance-eq (a dynamic band detects on the key while \
                       its band{n}_dyn_sc is on). Routing a key into a \
                       plugin that has no key port is REFUSED, naming the plugins on that track \
                       that accept one. \
                       \
                       To verify a route took effect, key from a deliberately SILENT track and \
                       watch the level change — do NOT toggle enabled, because a disabled route \
                       falls back to self-keying and can look identical to a working one. \
                       \
                       The key is delivered one audio block late by design (~2.7 ms at the \
                       default quantum), which keeps the result independent of track order and \
                       makes a track keying off itself legal rather than a feedback loop. That \
                       is below any usable ducker's attack time. Undoable with edit_undo.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<MutationAck>()
    )]
    async fn track_set_sidechain(
        &self,
        Parameters(params): Parameters<track::SetSidechainParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(track::SET_SIDECHAIN, &params).await
    }

    #[tool(
        description = "Remove a plugin's external sidechain key route, so its detector goes back \
                       to reading the plugin's own input. Addressed like track_set_sidechain, \
                       except that omitting plugin_id clears the first plugin on the track that \
                       HAS a route (falling back to track_set_sidechain's default). Clearing a \
                       plugin that had no route is not an error.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<MutationAck>()
    )]
    async fn track_clear_sidechain(
        &self,
        Parameters(params): Parameters<track::ClearSidechainParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(track::CLEAR_SIDECHAIN, &params).await
    }
    #[tool(
        description = "List the presets a plugin on a track can recall, factory and user \
                       together. FACTORY presets are built into the plugin — every install has \
                       the same ones, they are read-only, and they are the set that exists \
                       before anyone has saved anything. USER presets are ones saved on this \
                       machine, by this tool or in the plugin's own window; only those can be \
                       overwritten or shadow a factory name. Each entry carries which set it is \
                       from, and track_load_plugin_preset takes the name. \
                       \
                       Only Resonance's own plugins publish factory presets to the host; a \
                       third-party CLAP reports none rather than a guess. Addressed exactly as \
                       track_plugin_params: omit plugin_id for the track's instrument. \
                       \
                       current is the loaded preset when the app knows it, which it does not \
                       after a knob was turned in the plugin's own window — absent means \
                       unknown, not none.",
        annotations(read_only_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<plugin_preset::PluginPresetsView>()
    )]
    async fn track_plugin_presets(
        &self,
        Parameters(params): Parameters<track::PluginPresetsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(track::PLUGIN_PRESETS, &params).await
    }

    #[tool(
        description = "Recall a preset onto a plugin on a track — the fast way to get a whole \
                       sound, instead of setting forty parameters one at a time. The name comes \
                       from track_plugin_presets. \
                       \
                       source picks which set to take it from; omit it and a user preset wins \
                       over a factory one of the same name, which is what someone who saved over \
                       a factory name meant. The recall is ONE undo entry, is visible in the \
                       plugin's own window immediately, and moves every parameter the preset \
                       names — anything it does not name keeps its current value, so a preset \
                       written for an older build still loads.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<MutationAck>()
    )]
    async fn track_load_plugin_preset(
        &self,
        Parameters(params): Parameters<track::LoadPluginPresetParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(track::LOAD_PLUGIN_PRESET, &params)
            .await
    }

    #[tool(
        description = "Save a plugin's current sound as a named user preset, so it can be \
                       recalled here or picked in the plugin's own window later. \
                       \
                       Overwriting an existing USER preset needs overwrite: true; without it the \
                       call is refused rather than replacing the preset. Factory presets are \
                       never touched — saving under a factory name creates a user preset that \
                       shadows it, which is what the plugin's own window does. \
                       \
                       This answers as soon as the capture is armed, not when the file lands: \
                       the plugin hands its state back a beat later. Read track_plugin_presets \
                       to see the preset appear — the same one-cycle gap track_add_effect has \
                       for its parameter list.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<MutationAck>()
    )]
    async fn track_save_plugin_preset(
        &self,
        Parameters(params): Parameters<track::SavePluginPresetParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(track::SAVE_PLUGIN_PRESET, &params)
            .await
    }
}
