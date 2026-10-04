//! `midi_map_*` — the project's MIDI Learn bindings: which knob, fader
//! or pad on the user's control surface drives which control in the mix.
//! One tool per `midi_map.*` control method.

use crate::server::ResonanceMcp;
use resonance_control::methods::midi_map;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};

#[tool_router(router = router_midi_map, vis = "pub(crate)")]
impl ResonanceMcp {
    #[tool(
        description = "Read the project's MIDI Learn bindings: which hardware control on the \
                       user's control surface (source: kind \"cc\" or \"note\", channel 1-16, \
                       number 0-127, mode \"absolute\" / \"relative\" for a CC) drives which \
                       target — a track's volume / pan / mute / solo, one of its sends, a \
                       plugin parameter on it, or a transport action (play, stop, record, \
                       loop). Each target reads back with the same fields midi_map_learn \
                       takes plus a label. Also reports learning (the target learn is armed \
                       for, if any) and control_surface_input (the MIDI port the bindings are \
                       played from; null means none is chosen, so nothing reaches a binding — \
                       the user picks it in Settings › MIDI). Read-only.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<midi_map::BindingsResult>()
    )]
    async fn midi_map_bindings(&self) -> Result<CallToolResult, McpError> {
        self.invoke_structured(midi_map::BINDINGS, &()).await
    }

    #[tool(
        description = "Arm MIDI Learn for one target: the next knob, fader or pad the USER \
                       moves on their control surface is bound to it (replacing whatever that \
                       control drove before), as one undoable edit. You cannot move the \
                       control yourself — tell the user which control to move, then confirm \
                       with midi_map_bindings (learning goes back to null once it is \
                       captured). Target: track_id with control (\"volume\" | \"pan\" | \
                       \"mute\" | \"solo\"), send_id, or param (a plugin parameter by name or \
                       numeric id, with plugin_id / occurrence when it is not the track's \
                       instrument, as track_set_plugin_param takes it); or transport \
                       (\"play\" | \"stop\" | \"record\" | \"loop\") alone. Busses and the \
                       master have no learnable fader. Arming replaces an earlier arm; arming \
                       the armed target again is a no-op. Needs a control surface port \
                       (control_surface_input in midi_map_bindings).",
        annotations(idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<midi_map::LearnResult>()
    )]
    async fn midi_map_learn(
        &self,
        Parameters(params): Parameters<midi_map::LearnParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(midi_map::LEARN, &params).await
    }

    #[tool(
        description = "Disarm MIDI Learn without binding anything. cancelled says whether \
                       learn was armed; disarming when nothing is armed is a clean no-op.",
        annotations(idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<midi_map::CancelLearnResult>()
    )]
    async fn midi_map_cancel_learn(&self) -> Result<CallToolResult, McpError> {
        self.invoke_structured(midi_map::CANCEL_LEARN, &()).await
    }

    #[tool(
        description = "Remove one MIDI Learn binding by id (from midi_map_bindings). The \
                       hardware control stops driving its target; one undo entry \
                       (edit_undo puts it back). An unknown id is not_found.",
        annotations(destructive_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<midi_map::ClearResult>()
    )]
    async fn midi_map_clear(
        &self,
        Parameters(params): Parameters<midi_map::ClearParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(midi_map::CLEAR, &params).await
    }

    #[tool(
        description = "Remove EVERY MIDI Learn binding in the project — the user's whole \
                       controller setup for it — as one undo entry. cleared counts them; \
                       with none, a no-op. Ask before doing this: the user built these \
                       bindings by hand.",
        annotations(destructive_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<midi_map::ClearResult>()
    )]
    async fn midi_map_clear_all(&self) -> Result<CallToolResult, McpError> {
        self.invoke_structured(midi_map::CLEAR_ALL, &()).await
    }
}
