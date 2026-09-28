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
                       SOLO: a track or bus target ignores solo, the master target HONOURS it. \
                       A master result therefore carries soloed_track_ids whenever anything is \
                       soloed — non-empty means those numbers describe only those tracks, not \
                       the mix, even though every per-track entry still reads correct. \
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
                       DETAIL (opt-in, source \"render\" only; omitted from the result unless \
                       asked for, so the default reply stays small): detail takes any of \
                       \"spectrum\", \"stereo\", \"dynamics\" and adds one object per name. \
                       \
                       spectrum holds the warmth and harshness proxies. third_octave is 31 \
                       ISO 1/3-octave band levels, 20 Hz..20 kHz, in dB where a full-scale sine \
                       reads 0 (pink noise reads flat). tilt_db_per_oct is the slope of the \
                       power density over 100 Hz..10 kHz: pink noise -3.0, white 0, commercial \
                       pop about -4.5 to -5; more negative is darker/warmer. centroid_hz is the \
                       power-weighted mean frequency. lowmid_presence_db is energy 150-500 Hz \
                       over 2-5 kHz (up = warmer or muddier, down = harsher). \
                       presence_peakiness_db is the 1/6-octave crest inside 2-5 kHz (0 = even, \
                       high = a harsh resonance). air_ratio_db is energy 8-16 kHz over the \
                       total. peaks lists up to 5 narrow resonances as {freq_hz, excess_db}, \
                       excess over the smoothed spectrum around them, strongest first. \
                       \
                       stereo holds width and mono safety. bands is 8 bands (edges 20, 60, 150, \
                       400, 1k, 2.5k, 5k, 10k, 20k Hz) of {lo_hz, hi_hz, correlation, \
                       side_mid_db, mono_loss_db}: healthy is correlation >= +0.9 below 150 Hz, \
                       >= +0.5 to 1 kHz, >= 0 above; side_mid_db (side over mid power) -60 is \
                       mono, 0 is hard-panned or uncorrelated, and with equal L/R energy \
                       correlation = (1 - rho)/(1 + rho), rho = 10^(side_mid_db/10), so reason \
                       about width in side_mid_db and use correlation as the fault detector; \
                       mono_loss_db is the band's level change folded to mono (0 mono, -3 \
                       uncorrelated, -60 anti-phase). correlation_windows summarises 400 ms \
                       windows as {windows, pct_below_0_3, worst, worst_at_seconds} (seconds \
                       from the start of the range); warn when pct_below_0_3 is over 10. \
                       balance_db is left over right energy (positive leans left). one_sided \
                       true means one channel is silent, i.e. hard-panned mono: every 0/0 \
                       correlation is then null, and the top-level correlation must not be \
                       read as width. haas_lag_ms is a static L/R delay of 1-35 ms (positive: \
                       right is late), a mono comb risk; null when there is none. \
                       \
                       dynamics holds plr_db (true_peak_db minus lufs_integrated; 8-12 is \
                       typical for a master) and psr_db (true_peak_db minus lufs_short_max; \
                       keep it at 8 or more). Compare detail numbers between two states only at \
                       matched loudness. \
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
                       detail (e.g. [\"spectrum\", \"stereo\"]) works exactly as on \
                       meter_measure and adds its objects to the master and to every entry; it \
                       costs one spectral \
                       analysis per entry on top of the render. \
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
