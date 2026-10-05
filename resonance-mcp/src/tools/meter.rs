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
                       instrument is measured as one whole thing. {reference: N} measures a \
                       REFERENCE TRACK loaded with reference_load instead — whole (no range, \
                       no live), from the same pooled file a clip of it would play, with every \
                       field and detail block a mix slice gets — so compare a reference's \
                       tilt, per-band width and PLR with the master's directly. range defaults to the whole \
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
                       \"spectrum\", \"stereo\", \"dynamics\", \"depth\", \"decay\" and adds \
                       one object per name (depth is described on meter_stems, where it is \
                       meant to be used). \
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
                       excess over the local trend of the spectrum around them, strongest \
                       first; at most 1/3 octave wide, so a broad hump is never listed. \
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
                       decay measures how long a tail really is, by number: the reverb time \
                       read off the LAST stop in the range, where a sustained level (within 2 \
                       dB of its last half second) starts to fall, up to the next onset (a 6 \
                       dB rise) or the range end, by Schroeder integration of the energy after \
                       the stop. Use it on the TRACK that sends to the reverb, over a range \
                       that ends in silence after a stop: its stem carries its sends' returns \
                       back, while a {bus_id: N} target hears only tracks routed INTO that bus, \
                       never sends, so a send-fed return measured by bus_id is silent (a reverb \
                       inserted on a group bus is measured on that bus). For the song's last \
                       note use the default range \
                       (an explicit range is clamped to the song end and cuts the tail, the \
                       default renders 2 s past the last clip). Fields: found (a stop exists; \
                       false means everything else is null and note says why); clean (falls \
                       far enough for T30: 40 dB to a floor, 45 when cut off); stop (song \
                       position of the stop, null for a reference), stop_seconds, \
                       length_seconds; ends: floor (silence, or a quiet part playing on), \
                       onset (new signal came in) or range_end (still falling); \
                       dynamic_range_db (level before the stop minus the lowest level after, \
                       100 = digital silence); edt_seconds (first 10 dB, x6: the perceived \
                       length), t20_seconds (-5..-25 dB) and t30_seconds (-5..-35 dB, what a \
                       reverb's decay knob means), each null when the decay does not fall far \
                       enough (EDT 15 dB, T20 30, T30 40; 5 dB more when cut off); \
                       tail_20db_seconds (from the stop to 20 dB down: compare stop_seconds + \
                       tail_20db_seconds with the next downbeat to see whether the tail \
                       clears it); bands, 7 octaves 125 Hz..8 kHz of {center_hz, t30_seconds}; \
                       note when not clean. A decay overlapped by new notes is not a decay: on \
                       a mix whose tail other parts cover there is nothing to read. A \
                       self-decaying note (piano, pluck) adds its own decay to EDT, the track's \
                       dry signal stopping makes EDT short against T30 (which still reads the \
                       room, within about 10 % of the reverb's decay knob), and tails over \
                       about 15 s are not detected. \
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
                       costs one spectral analysis per entry on top of the render. detail \
                       [\"decay\"] reads every track's tail length in one pass (see meter_measure \
                       for its fields). \
                       \
                       DEPTH: detail [\"depth\"] adds a depth object per entry for judging \
                       front-to-back staging: hf_tilt_db (energy 6-16 kHz over 1-4 kHz; falls \
                       from front to back as sources darken), and for each track \
                       drr_db_estimate, dry_only, layer_hint and sends. drr_db_estimate is an \
                       ESTIMATE of the direct-to-reverberant ratio, not a measurement: per send, \
                       minus (send level + the return's gain), summed in power over the track's \
                       own sends, a pre-fader send adding the track's fader. The return's gain \
                       (each sends entry's return_gain_db, with bus_id, send_level_db and \
                       pre_fader) is measured in the same pass: every return a track sends to \
                       is rendered once from its feeders' sends alone, against their dry \
                       signal, so it covers the return's reverb and its fader. Send levels are \
                       their current static values (send automation is not read); everything \
                       rendered honours automation. A track with no sends is dry_only: true \
                       with a null drr_db_estimate, and ranks as the driest. layer_hint is \
                       front, middle or back from the DRR tertiles of the tracks in this pass, \
                       so it is relative, never absolute. Check ORDERING (lead drier than \
                       backing drier than pads); rough targets front +10 or more, middle +3 to \
                       +8, back 0 or less. Only a track's own sends count: a track feeding a \
                       bus that sends to a reverb reads dry. Cost: one extra render per return \
                       and one per sending track. \
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

    #[tool(
        description = "Measure one slice of the mix (like meter_measure, render only) and KEEP \
                       the numbers, so a later meter_compare can tell you what a change did. \
                       Returns {snapshot_id, measurement}. Take one BEFORE a move (EQ, \
                       saturation, width, a send), make the move, then meter_compare {a: \
                       snapshot_id} against the current state. \
                       \
                       target and range as in meter_measure; the resolved sample range is \
                       stored, and the \"current\" side of a compare re-renders exactly that \
                       range. detail defaults to ALL of spectrum, stereo and dynamics, since a \
                       detail the snapshot lacks has no delta; pass a narrower list to save \
                       space. Snapshots live in the running app's memory for this session \
                       only: not saved with the project, gone after a restart, and the least \
                       recently used is evicted past 32. They survive opening another project, \
                       so one song can be compared against another. Nothing is changed and it \
                       is not an undo step.",
        annotations(read_only_hint = true, idempotent_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<JobStatus>()
    )]
    async fn meter_snapshot(
        &self,
        Parameters(params): Parameters<meter::SnapshotParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_job(meter::SNAPSHOT, &params, MEASURE_WAIT_MS)
            .await
    }

    #[tool(
        description = "Loudness-matched A/B: the deltas (B minus A) of every measured proxy \
                       between two measurements. This is THE tool for judging a warmth, width \
                       or tone move, because louder always reads as warmer and better: with \
                       match \"lufs\" (the default) B is gain-matched to A's integrated \
                       loudness first, so a delta is a change in character, not in level. \
                       \
                       a and b are each \"current\" (render the project now) or a snapshot_id \
                       from meter_snapshot; b defaults to \"current\". Both sides always \
                       describe the same audio slice: a snapshot fixes the target and sample \
                       range, and \"current\" re-renders exactly that range, so omit target \
                       and range unless both sides are \"current\". Two snapshots compare \
                       instantly with no render. match \"none\" compares as measured. \
                       \
                       Result: {target, measured_seconds, a, b (each {side, \
                       lufs_integrated}), match, matched, match_gain_db, deltas}. \
                       match_gain_db is the gain applied to B (a B that is 3 dB louder reads \
                       about -3); matched is false with match \"none\" or when either side is \
                       silent. Matching is exact arithmetic on the stored numbers: level \
                       figures (lufs_integrated, lufs_short_max, lufs_momentary_max, \
                       true_peak_db, sample_peak_db, the third_octave bands) move by the gain; \
                       shape figures (lra, crest_db, correlation, mono_penalty_db, bands, \
                       tilt_db_per_oct, centroid_hz, lowmid_presence_db, \
                       presence_peakiness_db, air_ratio_db, plr_db, psr_db and the whole \
                       stereo block) cannot change with a pure gain. clipped_samples is the \
                       one delta reported AS MEASURED. deltas.spectrum carries third_octave \
                       (per band), tilt_db_per_oct, centroid_hz, centroid_pct, \
                       lowmid_presence_db, presence_peakiness_db, air_ratio_db; deltas.stereo \
                       carries bands of {lo_hz, hi_hz, correlation, side_mid_db, \
                       mono_loss_db}, balance_db, pct_below_0_3 and worst_window_correlation; \
                       deltas.dynamics carries plr_db and psr_db. A delta is null when either \
                       side lacks the number. Identical states compare to all zeros. \
                       \
                       Reading it for warmth: tilt_db_per_oct more negative, lowmid_presence_db \
                       up 1-2, presence_peakiness_db down, centroid_pct down 5-15, and crest_db \
                       down no more than 2 with psr_db staying at 8 or more in absolute terms.",
        annotations(read_only_hint = true, idempotent_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<JobStatus>()
    )]
    async fn meter_compare(
        &self,
        Parameters(params): Parameters<meter::CompareParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_job(meter::COMPARE, &params, MEASURE_WAIT_MS)
            .await
    }

    #[tool(
        description = "Measure the harmonic signature of an insert chain: run a sine through it \
                       offline and read THD, the harmonics and the aliasing floor. Use it to set \
                       a saturator's drive to a THD target instead of guessing from the knob, \
                       and to check that a character stage is even-dominant (\"warm\"). \
                       \
                       target: \"master\" (default), {track_id: N} or {bus_id: N}; the chain is \
                       that owner's inserts in order (a track's instrument is not part of it; \
                       bypassed and missing plugins are left out and listed in skipped). \
                       freq_hz defaults to 1000 and is snapped to the analysis grid (the result \
                       echoes the exact value); probe at 5000 to expose aliasing, since \
                       harmonics past Nyquist fold back. level_dbfs (default -18, -80..0) is \
                       the tone's peak; -18 is the level the colour presets' THD targets are \
                       voiced at. Distortion depends on level, so probe again at what the \
                       chain really peaks at to see the loudest moments. imd: true adds the \
                       SMPTE 60 Hz + 7 kHz 4:1 pair and \
                       imd_pct. \
                       \
                       Runs any time, including while the transport rolls: the probe builds a \
                       fresh CLONE of each plugin from the live plugin's current saved state \
                       and drives the clones on a worker thread. The live plugins are only read \
                       (one state save each), never processed, reset or reloaded, so \
                       automation, undo and every plugin's running state are untouched. The \
                       state save holds the live plugin for its duration, so during playback \
                       that plugin may skip one audio block, exactly as when the project is \
                       saved; probe a busy chain while stopped if that matters. The clone gets \
                       no automation (it probes current values) and a \
                       sidechain key hears silence. stages lists what was probed, with \
                       state_copied false for a plugin that has no state extension (probed at \
                       its defaults). \
                       \
                       Result: {target, freq_hz, level_dbfs, stages, skipped, gain_db, thd_pct, \
                       h, h2_h3_db, decay_db_per_order, aliasing_floor_dbc, imd_pct, \
                       latency_samples}. h is H2..H9 in dBc (h[0] is H2), floored at -160, \
                       null for a harmonic above Nyquist. Targets: thd_pct 0.1-1 on the \
                       master, 0.5-3 on a bus, 3-10 on a single track; h2_h3_db above 0 is \
                       even-dominant (warm), below 0 odd-dominant (harder, edgier); \
                       decay_db_per_order of 6 or more; aliasing_floor_dbc of -90 or lower. \
                       gain_db is the chain's level change at the probe frequency. Runs as a \
                       job.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<JobStatus>()
    )]
    async fn meter_probe(
        &self,
        Parameters(params): Parameters<meter::ProbeParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_job(meter::PROBE, &params, MEASURE_WAIT_MS)
            .await
    }
}
