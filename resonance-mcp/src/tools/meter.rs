//! `meter_*` — measure the mix instead of bouncing and analysing it.

use crate::server::ResonanceMcp;
use resonance_control::job::JobStatus;
use resonance_control::methods::meter;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};

/// A measurement renders the slice offline, so it costs about what a
/// bounce of the same length costs; wait generously.
const MEASURE_WAIT_MS: u64 = 300_000;

#[tool_router(router = router_meter, vis = "pub(crate)")]
impl ResonanceMcp {
    #[tool(
        description = "Measure one slice of the mix and get real numbers back — loudness, peak, \
                       dynamics, phase and tonal balance. Nothing is written and nothing is \
                       changed. Use this instead of bouncing a WAV and analysing it: it is the \
                       only way to hear anything about this mix. \
                       \
                       target: \"master\" (default), {track_id: N} or {bus_id: N}, ids from \
                       song_summary. A track target includes its sub-tracks, so a multi-output \
                       instrument is measured as one whole thing. range defaults to the whole \
                       song and is clamped to it. source defaults to \"render\" (an offline \
                       render, deterministic, needs a stopped transport and no bounce in \
                       flight); \"live\" instead reads the master meter as it plays, master \
                       only. Runs as a job — waits up to 5 minutes and returns the final status. \
                       \
                       READING THE RESULT. 1 LU == 1 dB, so differences in LUFS are differences \
                       in dB. lufs_integrated is GATED: a part that plays in 2 of 9 sections \
                       reports how loud it is WHILE IT PLAYS, not an average watered down by \
                       its silence — that is what makes it usable for balance. A healthy MIX \
                       sits around -23..-18 LUFS with peaks well under 0 dBFS. -14 LUFS is a \
                       MASTERING target for a finished, limited master; treating it as a mix \
                       target is a documented field error that leads to squashing the mix. \
                       true_peak_db: -1 dBTP is Spotify's recommendation, -2 is safer. \
                       crest_db under 8 dB is squashed, over 20 dB is essentially \
                       uncompressed. correlation near +1 is mono-identical, near 0 is wide, \
                       negative means anti-phase content a mono listener loses; check \
                       mono_penalty_db alongside it. clipped_samples above 0 on the master is \
                       audible clipping. bands (low/mid/high/air) are RAW ENERGY SHARES summing \
                       to 1.0 — compare them against each other or against a reference \
                       measurement, never against an absolute rule. \
                       \
                       A null field means the number does not exist for this measurement, never \
                       zero: either the range was silent or too short for that meter's window, \
                       or source was \"live\", where the streaming tap supplies none of \
                       sample_peak_db, clipped_samples, crest_db, correlation, mono_penalty_db, \
                       bands or measured_seconds, and reports its current windows as \
                       lufs_short_term_now / lufs_momentary_now instead of maxima. So crest and \
                       phase can only be judged with source \"render\". The three figures that \
                       DO arrive on \"live\" — lufs_integrated, lra, true_peak_db — are \
                       SESSION-CUMULATIVE: they cover everything played since the app's audio \
                       engine started, not a range you asked for, so use \"live\" for \"how is \
                       this session going\" and \"render\" for any figure about a passage. Check \
                       `source` before trusting a field.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<JobStatus>()
    )]
    async fn meter_measure(
        &self,
        Parameters(params): Parameters<meter::MeasureParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_job(meter::MEASURE, &params, MEASURE_WAIT_MS)
            .await
    }

    #[tool(
        description = "Measure EVERY track plus the master in ONE pass — a whole balance pass in \
                       one call. This replaces bouncing one stem per track and analysing the \
                       files; that loop cost about ten minutes per iteration and, because it \
                       worked by muting the other tracks, routinely left a whole drum kit \
                       bleeding into every stem and produced a plausible but completely wrong \
                       table. Nothing is written and nothing is changed. \
                       \
                       Every entry is measured in ONE pass over ONE SHARED range — one command, \
                       one set of results — so the numbers are directly comparable and \
                       measured_seconds is identical on all of them. Inside that pass the engine \
                       renders each target in turn, so the COST scales with track count: a \
                       20-track project is 20 full-length renders. On a large project that can \
                       exceed this tool's 5-minute wait, after which it returns a still-running \
                       job and you poll job_status or block with job_wait. range defaults to the \
                       whole song. include_busses (default \
                       false) adds each group/return bus; a bus and its member tracks then both \
                       appear, describing the same audio before and after the bus chain — they \
                       overlap, so never add them together. \
                       \
                       The result is {master, tracks[]}. Each track entry carries track_id, \
                       name and the same fields meter_measure returns — see that tool for what \
                       they mean and how to read them. A SUB-TRACK NEVER GETS ITS OWN ENTRY: \
                       the extra output ports of a multi-output instrument carry no material of \
                       their own, so they are measured as part of their parent and listed in \
                       that entry's includes_track_ids. The kit is therefore counted exactly \
                       once, on the parent track. \
                       \
                       Read balance off lufs_integrated differences (1 LU == 1 dB). A useful \
                       starting convention, validated in the field, is drums 0 LU as the \
                       reference, bass -2, lead -4, rhythm guitar -6, texture -11, pad -13, fx \
                       -18. That is a convention and not physics — its value is catching a \
                       track that drifted far from where you meant to put it, not dictating the \
                       arrangement.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<JobStatus>()
    )]
    async fn meter_stems(
        &self,
        Parameters(params): Parameters<meter::StemsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_job(meter::STEMS, &params, MEASURE_WAIT_MS)
            .await
    }
}
