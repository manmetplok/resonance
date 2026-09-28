//! `master_*` — the master bus: the final summing stage every track and
//! bus feeds into.

use crate::server::ResonanceMcp;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};
use resonance_control::methods::{master, plugin_preset, track};
use resonance_control::MutationAck;

#[tool_router(router = router_master, vis = "pub(crate)")]
impl ResonanceMcp {
    #[tool(
        description = "The master bus: its fader (linear volume and volume_db), whether its FX \
                       chain is bypassed, and every plugin inserted on it with its slot, CLAP \
                       id and name. The master is the FINAL summing stage — every track and \
                       every bus lands here, after their own faders and effects — so anything \
                       inserted here acts on the whole mix at once. Read this before deciding \
                       what to put on the master: an empty plugins array means the mix is \
                       going out completely unprocessed. Identity only, though — for what a \
                       master plugin is actually SET to, read master_plugin_params. Read-only.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<master::MasterSummary>()
    )]
    async fn master_summary(&self) -> Result<CallToolResult, McpError> {
        self.invoke_structured(master::SUMMARY, &()).await
    }

    #[tool(
        description = "Set the master fader. Give EXACTLY ONE of volume (linear gain, 1.0 = \
                       unity) or volume_db (decibels, 0 = unity); both or neither is rejected. \
                       Accepted range is -60..=+6 dB (linear 0.0..=~1.995). \
                       \
                       This is a single scalar applied after everything else, so it does NOT \
                       change the balance between tracks — every relationship is preserved \
                       exactly. It also cannot make a mix meaningfully louder: raising it \
                       raises the loudest transient with everything else, so the peak hits \
                       full scale and clips long before the average level catches up. Getting \
                       a mix louder without clipping needs dynamics processing on the master \
                       chain (a limiter), not a bigger master fader. Undoable, and reflected \
                       in the GUI's master strip.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn master_set_volume(
        &self,
        Parameters(params): Parameters<master::SetMasterVolumeParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(master::SET_VOLUME, &params).await
    }

    #[tool(
        description = "Append an effect to the MASTER insert chain, where it processes the \
                       whole summed mix (after every track and bus, before the master fader). \
                       plugin_id is a CLAP id of the form \"com.resonance.<name>\"; \
                       \"com.resonance.mastering\" is the one to reach for here. Instrument \
                       plugins are refused — the master has no notes to play. Each call \
                       APPENDS, so calling it twice gives the master two instances (address \
                       them by occurrence in master_remove_effect). Returns the slot and \
                       occurrence it landed on. Undoable. \
                       \
                       Adding is only half the job: a plugin sits at its defaults until \
                       master_set_plugin_param configures it, and the mastering plugin's stages \
                       all default to OFF, so an unconfigured master chain measures identically \
                       to an empty one. \
                       \
                       WHY THIS MATTERS: a finished mix typically sits around -23..-18 LUFS \
                       with headroom deliberately left for mastering, and it CANNOT be made \
                       louder by raising faders — the loudest transient hits full scale first. \
                       A limiter on the master is what buys level beyond that transient. Do \
                       NOT chase loudness by pulling the drums down: that trades a level \
                       problem for a balance problem, and the balance problem is the one \
                       listeners hear. \
                       \
                       TARGETS: keep true peak at or below -1 dBTP (Spotify's recommendation), \
                       or -2 dBTP to stay safe through lossy codecs, which push peaks up. \
                       Streaming platforms normalise, so mastering louder than about -14 LUFS \
                       integrated buys nothing — it is turned back down on playback and you \
                       keep only the squashed dynamics.",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn master_add_effect(
        &self,
        Parameters(params): Parameters<master::AddEffectParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(master::ADD_EFFECT, &params).await
    }

    #[tool(
        description = "Remove one effect from the master insert chain. Address it EITHER by \
                       slot (the 0-based position master_summary reports) OR by plugin_id plus \
                       occurrence when the same effect is on the master more than once — both \
                       forms together, or neither, is rejected rather than guessed at, because \
                       a guess here strips a processor off the entire mix. Remaining plugins \
                       renumber, so re-read master_summary before a second removal. Undoable.",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn master_remove_effect(
        &self,
        Parameters(params): Parameters<master::RemoveEffectParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(master::REMOVE_EFFECT, &params).await
    }

    #[tool(
        description = "Move an effect to a different position in the master chain. Addressed \
                       like master_remove_effect — by slot OR by plugin_id plus occurrence — \
                       plus to_slot for where it should end up. \
                       \
                       The chain runs front to back over the finished mix, so order is audible, \
                       and on the master it decides whether the chain works at all: a limiter \
                       holding a ceiling must be LAST, because any processor after it can push \
                       the sum back over the ceiling it exists to hold. Like a bus and unlike a \
                       track, the master has no instrument pinned at slot 0 — every position is \
                       a valid destination. to_slot past the end clamps to the end, and moving \
                       an effect to where it already sits is an accepted no-op. Undoable.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn master_move_effect(
        &self,
        Parameters(params): Parameters<master::MoveEffectParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(master::MOVE_EFFECT, &params).await
    }

    #[tool(
        description = "Put a different plugin in one of the master chain's slots, KEEPING its \
                       position. Addressed like master_remove_effect — by slot OR by plugin_id \
                       plus occurrence — plus new_plugin_id, a CLAP id from plugins_catalog. \
                       \
                       Position matters more here than anywhere: the master chain IS its order, \
                       and a limiter that must sit last has to still sit last after the EQ in \
                       front of it is swapped. remove + add cannot do that — master_add_effect \
                       only appends. \
                       \
                       It is also how a MISSING plugin is recovered. master_plugin_params \
                       reports status: \"missing\" for a slot whose plugin is not installed on \
                       the machine running the app: the slot holds its position but nothing is \
                       behind it, so the mix is going out unprocessed at that stage. Pass the \
                       SAME plugin_id as new_plugin_id to RELOCATE it (after installing it and \
                       calling plugins_rescan), which restores the settings the project saved; \
                       pass a different id to SWAP it, keeping the position but discarding \
                       those settings. outcome in the reply says which happened. Do not remove \
                       a missing plugin to tidy up — removal is what destroys its recoverable \
                       settings. Undoable.",
        annotations(destructive_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<track::ReplaceEffectResult>()
    )]
    async fn master_replace_effect(
        &self,
        Parameters(params): Parameters<master::ReplaceEffectParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(master::REPLACE_EFFECT, &params).await
    }

    #[tool(
        description = "List the master chain's effects and every parameter each one exposes — \
                       id, name, current value, min, max, default, and what the value MEANS: \
                       text (the plugin's own rendering, so a stage reads \"-1.0 dB\" rather \
                       than a bare number), unit, module — which stage of the mastering chain a \
                       parameter belongs to — plus stepped, choices and hidden. Omit plugin_id \
                       for all of them. The entries are the same shape track_plugin_params and \
                       bus_plugin_params return, including slot (0-based chain position, which \
                       IS processing order) and occurrence (which copy of a repeated effect). \
                       Every entry is kind: \"effect\" — the master has no instrument. \
                       \
                       master_summary reports identity only, so this is the ONLY way to see \
                       what a master plugin is actually set to. Read it before \
                       master_set_plugin_param to learn the parameter names and their ranges. \
                       A plugin whose params array is empty was added moments ago and is still \
                       initializing — read again.",
        annotations(read_only_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<master::PluginParamsView>()
    )]
    async fn master_plugin_params(
        &self,
        Parameters(params): Parameters<master::PluginParamsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(master::PLUGIN_PARAMS, &params).await
    }

    #[tool(
        description = "Set one parameter on a plugin inserted on the master — what turns a \
                       limiter that is merely present into one that actually holds a ceiling. \
                       param takes the parameter's name (case-insensitive) or its numeric id as \
                       a string, both from master_plugin_params, or on a com.resonance.* plugin \
                       its stable string key (e.g. \"lim_on\"). plugin_id names which effect; \
                       omitted it targets the master's first, which is unambiguous only on a \
                       one-effect chain. A value outside the parameter's min..=max is rejected \
                       with the range rather than clamped, but a value that rounds onto an \
                       f32-declared bound (0.1 against a reported 0.10000000149011612) is \
                       accepted. value also takes a choice label as a string for any parameter \
                       master_plugin_params reports choices for. Repeated sets of the same \
                       parameter collapse into one undo entry, and a set applies even while the \
                       transport is stopped. \
                       \
                       \"com.resonance.mastering\" is a chain of stages that each default to \
                       OFF, so at its defaults it passes the mix through untouched — a plugin \
                       added and never configured measures the same as no plugin at all. To \
                       bring a finished mix up to a release level: set \"Limiter On\" to 1, set \
                       \"Ceiling\" to -1 (dBTP), then raise \"Input Trim\" in dB until \
                       meter_measure reports the integrated loudness you want. Trim drives the \
                       signal INTO the limiter; the master fader is post-FX and cannot do this.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn master_set_plugin_param(
        &self,
        Parameters(params): Parameters<master::SetPluginParamParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(master::SET_PLUGIN_PARAM, &params).await
    }

    #[tool(
        description = "Bypass or re-engage the ENTIRE master effect chain in one call. This \
                       SETS the state rather than toggling it, so it is safe to repeat and \
                       cannot end up inverted; setting the state it is already in is a no-op. \
                       Bypassing is the A/B test for whether the master processing is helping: \
                       bounce with bypassed: true and with false and compare the measurements, \
                       rather than judging the chain by its settings. master_summary reports \
                       the current value as fx_bypassed. Master volume is NOT affected — only \
                       the plugins.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn master_set_fx_bypass(
        &self,
        Parameters(params): Parameters<master::SetFxBypassParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(master::SET_FX_BYPASS, &params).await
    }

    #[tool(
        description = "Bypass or re-engage ONE plugin in the master chain, leaving the rest of the chain \
                       running — the A/B for \"is this one plugin earning its place?\". \
                       Distinct from master_set_fx_bypass, which mutes the whole chain: the \
                       two are independent, so a chain-bypassed master chain still remembers \
                       which slots were individually bypassed and re-engaging the chain \
                       restores the mix rather than switching everything on. Address the \
                       plugin exactly as master_set_plugin_param does (plugin_id + \
                       occurrence; omitted means the first plugin). Read master_plugin_params to see \
                       each slot's current bypassed flag. The engine crossfades over a few \
                       milliseconds, so toggling mid-playback does not click. SETS rather \
                       than toggles, so a retry is safe. Undoable, and saved with the \
                       project.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn master_set_plugin_bypass(
        &self,
        Parameters(params): Parameters<master::SetPluginBypassParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(master::SET_PLUGIN_BYPASS, &params).await
    }

    #[tool(
        description = "Route a track's or bus's audio into the external SIDECHAIN KEY of a \
                       plugin ON THE MASTER CHAIN — the detector input. The mastering use is \
                       narrow but real: a bus compressor on the master keyed from the kick, so \
                       the mix breathes with the rhythm instead of with whichever transient \
                       happens to be loudest. There is no master_id — there is one master. \
                       \
                       Addressed like master_set_plugin_param: an optional plugin_id (+ \
                       occurrence); omitted targets the first plugin on the chain that HAS a \
                       key port other than resonance-reverb and resonance-eq (in practice the \
                       compressor); the reverb or the EQ only when nothing else has one. Name \
                       the key source with EITHER source_track_id OR source_bus_id. \
                       Sources are tapped post-FX and PRE-fader, so a key source can sit at -inf \
                       and still key. enabled defaults to true; false keeps the routing \
                       configured but stops delivering the key. \
                       \
                       Only the DETECTOR changes: the key never reaches the output. Routing a \
                       key into a plugin with no key port is REFUSED, naming the ones on the \
                       master that accept it — note that \"com.resonance.mastering\" does NOT \
                       take a key; use \"com.resonance.compressor\". The key is delivered one \
                       audio block late by design (~2.7 ms), so no routing can feed back. Routes \
                       are saved with the project. Undoable with edit_undo.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<MutationAck>()
    )]
    async fn master_set_sidechain(
        &self,
        Parameters(params): Parameters<master::SetSidechainParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(master::SET_SIDECHAIN, &params).await
    }

    #[tool(
        description = "Remove a master plugin's external sidechain key route, so its detector \
                       goes back to reading the mix itself. Addressed like \
                       master_set_sidechain, except that omitting plugin_id clears the first \
                       plugin on the chain that HAS a route (falling back to \
                       master_set_sidechain's default). Clearing a plugin that had no route is \
                       not an error.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<MutationAck>()
    )]
    async fn master_clear_sidechain(
        &self,
        Parameters(params): Parameters<master::ClearSidechainParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(master::CLEAR_SIDECHAIN, &params)
            .await
    }

    #[tool(
        description = "List the presets a plugin on the master chain can recall, factory and \
                       user together. FACTORY presets are built into the plugin — every install \
                       has the same ones and they are read-only. USER presets are ones saved on \
                       this machine, by this tool or in the plugin's own window; only those can \
                       be overwritten or shadow a factory name. Each entry says which set it is \
                       from, and master_load_plugin_preset takes the name. \
                       \
                       The bank belongs to the PLUGIN, not to the master, so a preset saved from \
                       a track or bus shows up here too. Only Resonance's own plugins publish \
                       factory presets to the host; a third-party CLAP reports none rather than \
                       a guess. Omitting plugin_id targets the FIRST plugin on the chain.",
        annotations(read_only_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<plugin_preset::PluginPresetsView>()
    )]
    async fn master_plugin_presets(
        &self,
        Parameters(params): Parameters<master::PluginPresetsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(master::PLUGIN_PRESETS, &params).await
    }

    #[tool(
        description = "Recall a preset onto a plugin on the master chain. A mastering chain is \
                       exactly where a starting point earns its keep — a limiter's factory \
                       preset gets a sane ceiling and release in one call instead of a dozen. \
                       The name comes from master_plugin_presets. \
                       \
                       source picks which set to take it from; omit it and a user preset wins \
                       over a factory one of the same name. The recall is ONE undo entry, shows \
                       in the plugin's own window immediately, and moves every parameter the \
                       preset names — anything it does not name keeps its current value.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<MutationAck>()
    )]
    async fn master_load_plugin_preset(
        &self,
        Parameters(params): Parameters<master::LoadPluginPresetParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(master::LOAD_PLUGIN_PRESET, &params)
            .await
    }

    #[tool(
        description = "Save a master plugin's current sound as a named user preset, so the \
                       mastering settings that worked on this mix can be recalled on the next \
                       one — here, on any track or bus carrying the same plugin, or in the \
                       plugin's own window. \
                       \
                       Overwriting an existing USER preset needs overwrite: true; without it the \
                       call is refused rather than replacing the preset. Factory presets are \
                       never touched — saving under a factory name creates a user preset that \
                       shadows it. \
                       \
                       This answers as soon as the capture is armed, not when the file lands: \
                       the plugin hands its state back a beat later. Read master_plugin_presets \
                       to see the preset appear.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<MutationAck>()
    )]
    async fn master_save_plugin_preset(
        &self,
        Parameters(params): Parameters<master::SavePluginPresetParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(master::SAVE_PLUGIN_PRESET, &params)
            .await
    }

    #[tool(
        description = "Run the mastering assistant over the master mix and get its suggestions \
                       for com.resonance.mastering back — NOTHING IS APPLIED. It renders the \
                       master offline over range (default the whole song; needs a stopped \
                       transport, like meter_measure), analyses it exactly as the plugin's \
                       Assistant panel does, and compares it against a target: \
                       {mode: \"genre\", genre: rock|indie|acoustic|jazz|pop} — a built-in \
                       target BAND per 1/3 octave (a Pestana-style -4.5..-5 dB/oct slope with \
                       genre low-end/top offsets and a tolerance), or {mode: \"reference\", \
                       pool_asset_id} — a reference track from pool_list (import it with \
                       pool_import first), whose own spectrum and loudness become the target. \
                       Runs as a job and returns the final status. \
                       \
                       The result: target {mode, genre | pool_asset_id, label, target_lufs}; \
                       measured {lufs_integrated, true_peak_db, crest_db, correlation, \
                       measured_seconds} — the master as it is NOW, after the master chain, \
                       including any mastering plugin already on it; suggestions[], stage by \
                       stage (input_trim, tonal_low_shelf, tonal_high_shelf, glue, imager, \
                       limiter, target_lufs, diagnostic), each {stage, rationale[], \
                       params[{key, value}]} where params are EXACTLY the mastering-plugin writes \
                       the panel's Apply would make (keys like input_trim_db, tone_b0_gain, \
                       glue_ratio, img_width, lim_ceiling; bools 0/1, choices by index) and an \
                       empty params list means that stage needs no change; deviations[], 31 \
                       bands 20 Hz..20 kHz of {hz, lo_db, hi_db, measured_db, deviation_db} — \
                       the master's spectral SHAPE (midrange aligned, never level) against the \
                       band: deviation 0 is inside it, positive is too much there, negative too \
                       little. Shelves act only on what lies OUTSIDE the band. plugin_id and \
                       master_slot say where to apply: master_slot null means no \
                       com.resonance.mastering is on the master yet — master_add_effect it first. \
                       \
                       Apply the parts you agree with through master_set_plugin_param (plugin_id \
                       com.resonance.mastering, param = key, value = value), then re-measure at \
                       matched loudness (meter_compare) before accepting a move. Read-only.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<resonance_control::job::JobStatus>()
    )]
    async fn master_assist(
        &self,
        Parameters(params): Parameters<master::AssistParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_job(master::ASSIST, &params, ASSIST_WAIT_MS)
            .await
    }
}

/// `master.assist` renders the master range offline, like a
/// `meter.measure`; wait as generously.
const ASSIST_WAIT_MS: u64 = 300_000;
