//! The amp editor's controls against the parameters they edit (ba todo
//! #1283, audit findings F4 and A6).
//!
//! # What had drifted
//!
//! One knob, and it is the one the audit counted: **Output Gain reset to
//! `1.0` while `params.rs` declares `0.5`**, so double-clicking the dial
//! moved the amp +6 dB away from the value a fresh instance — the only
//! other opinion in the crate, since amp ships no factory presets — had
//! been running since it loaded. The declaration wins; nothing in
//! `params.rs` moved to accommodate the knob.
//!
//! Both knobs also passed `logarithmic: true`, so neither declared
//! `FloatRange::Skewed` ever reached a control: that is the *curve* half
//! of the finding, the same one the gate (#1286) turned out to have on
//! its own. Input Gain's range and default agreed with its parameter all
//! along, so it is the second case — nothing could drift there because
//! the two numbers it restated happened to match.
//!
//! # What keeps it fixed
//!
//! * a **source guard** over every file in `src/editor/`, not just the
//!   one that draws the strip today. The compiled-in file list is
//!   checked against the directory on disk, so moving a knob into a new
//!   module fails the guard instead of quietly escaping it;
//! * a **surface guard** across `params.rs`, `lib.rs` and the editor: a
//!   parameter that is declared must be registered with the host, and a
//!   `FloatParam` must either be drawn through the shared helper or be
//!   named here as deliberately not a knob. A new parameter (ba todo
//!   #1316's `model_size`) therefore cannot land half-wired, and cannot
//!   land outside the sweep below;
//! * **behavioural tests** that a control's 0→100 % travel is exactly
//!   the parameter's own `denormalize`, in both directions, with a reset
//!   that writes the declared default verbatim.
//!
//! No GUI is needed: `editor_widgets::float_knob` adds only egui
//! plumbing on top of `normalized_value` / `default_normalized` /
//! `plain_at_normalized` / `set_normalized`, which is the whole contract
//! under test. Plugin editors have no snapshot coverage, so the visual
//! side is a manual check — see the report on ba todo #1283.

use std::collections::BTreeSet;
use std::path::Path;

use resonance_amp::params::AmpParams;
use resonance_amp::ResonanceAmp;
use resonance_plugin::{FloatParam, FloatRange, Param, ResonancePlugin};

/// Every source file under `src/editor/`, compiled in so the guards read
/// exactly what the editor is built from.
///
/// [`the_guard_reads_every_editor_source`] pins this against the
/// directory itself — the list going stale is how a source guard turns
/// vacuous.
const EDITOR_SOURCES: &[(&str, &str)] = &[
    ("actions.rs", include_str!("../src/editor/actions.rs")),
    ("app.rs", include_str!("../src/editor/app.rs")),
    ("controls.rs", include_str!("../src/editor/controls.rs")),
    ("curve_view.rs", include_str!("../src/editor/curve_view.rs")),
    ("factory.rs", include_str!("../src/editor/factory.rs")),
    ("header.rs", include_str!("../src/editor/header.rs")),
    ("meters.rs", include_str!("../src/editor/meters.rs")),
    (
        "missing_banner.rs",
        include_str!("../src/editor/missing_banner.rs"),
    ),
    ("mod.rs", include_str!("../src/editor/mod.rs")),
    ("scope_view.rs", include_str!("../src/editor/scope_view.rs")),
    ("theme.rs", include_str!("../src/editor/theme.rs")),
    (
        "tone3000_panel.rs",
        include_str!("../src/editor/tone3000_panel.rs"),
    ),
    ("tuner_view.rs", include_str!("../src/editor/tuner_view.rs")),
];

const PARAMS_SRC: &str = include_str!("../src/params.rs");
const LIB_SRC: &str = include_str!("../src/lib.rs");

/// The one way a parameter may become a control.
const KNOB_CALL: &str = "editor_widgets::float_knob(";

/// Parameters that are deliberately not knobs.
///
/// `file_select` is an index into a directory listing whose length is
/// only known at runtime; the header draws it as ◀/▶ browse buttons
/// against `file_list`, which is why that call site is allowed to read
/// the list's length without restating anything from `params.rs`.
const NOT_A_KNOB: &[&str] = &["file_select"];

/// Every `FloatParam` the amp declares, paired with its field name.
///
/// [`the_sweep_covers_every_declared_float_param`] checks this against
/// `params.rs`, so a parameter cannot be added without being swept.
fn float_params(p: &AmpParams) -> Vec<(&'static str, &FloatParam)> {
    vec![
        ("input_gain", &p.input_gain),
        ("output_gain", &p.output_gain),
    ]
}

// ---------------------------------------------------------------------------
// Source parsing helpers
// ---------------------------------------------------------------------------

/// A source file with its comment lines removed, so a guard that bans a
/// pattern is not tripped by prose *about* that pattern.
fn code_only(src: &str) -> String {
    src.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The text between a call's parentheses, given the source that starts
/// just after the opening one. String literals are skipped so a `)` or a
/// `,` inside one cannot be mistaken for syntax.
fn balanced(rest: &str) -> Option<&str> {
    let mut depth = 1usize;
    let mut in_str = false;
    let mut escaped = false;
    for (i, b) in rest.bytes().enumerate() {
        if in_str {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&rest[..i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Split an argument list on its top-level commas.
fn split_args(inner: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut depth = 0usize;
    let mut in_str = false;
    let mut escaped = false;
    let mut start = 0usize;
    for (i, b) in inner.bytes().enumerate() {
        if in_str {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => {
                args.push(inner[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = inner[start..].trim();
    if !last.is_empty() {
        args.push(last.to_string());
    }
    args
}

/// Every `float_knob` call in the editor, as (file, arguments). The
/// arguments are normalized to one line so a call broken across lines
/// reads the same as an inline one.
fn knob_calls() -> Vec<(&'static str, Vec<String>)> {
    let mut calls = Vec::new();
    for (file, src) in EDITOR_SOURCES {
        let code = code_only(src);
        let mut from = 0usize;
        while let Some(at) = code[from..].find(KNOB_CALL) {
            let open = from + at + KNOB_CALL.len();
            let inner = balanced(&code[open..])
                .unwrap_or_else(|| panic!("{file}: unbalanced `{KNOB_CALL}` call"));
            let args = split_args(inner)
                .into_iter()
                .map(|a| a.split_whitespace().collect::<Vec<_>>().join(" "))
                .collect();
            calls.push((*file, args));
            from = open + inner.len();
        }
    }
    calls
}

/// The parameter fields `params.rs` declares, as (name, type).
fn declared_params() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in code_only(PARAMS_SRC).lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("pub ") else {
            continue;
        };
        let Some((name, ty)) = rest.split_once(':') else {
            continue;
        };
        let ty = ty.trim().trim_end_matches(',');
        if ty == "FloatParam" || ty == "IntParam" || ty == "BoolParam" {
            out.push((name.trim().to_string(), ty.to_string()));
        }
    }
    out
}

/// Every parameter ID `params.rs` declares, read from the first argument
/// of each `*Param::new(...)` call.
///
/// IDs rather than field names on purpose: the id is what the host, the
/// automation lane and `track.plugin_params` address the parameter by, so
/// it is the name that has to line up with what `param()` hands out. A
/// field renamed without its id (or vice versa) is exactly the drift this
/// file exists to catch, and comparing field names would miss it.
fn declared_param_ids() -> BTreeSet<String> {
    let src = code_only(PARAMS_SRC);
    let mut out = BTreeSet::new();
    for (i, _) in src.match_indices("Param::new(") {
        let rest = &src[i + "Param::new(".len()..];
        let Some(open) = rest.find('"') else { continue };
        let Some(close) = rest[open + 1..].find('"') else {
            continue;
        };
        out.insert(rest[open + 1..open + 1 + close].to_string());
    }
    out
}

/// The body of a function in `lib.rs`, by the start of its signature.
fn lib_fn_body(signature: &str) -> String {
    let body = code_only(LIB_SRC);
    let at = body
        .find(signature)
        .unwrap_or_else(|| panic!("lib.rs must define `{signature}`"));
    let open = body[at..]
        .find('{')
        .unwrap_or_else(|| panic!("`{signature}` must have a body"));
    balanced(&body[at + open + 1..])
        .unwrap_or_else(|| panic!("`{signature}`'s body is unbalanced"))
        .to_string()
}

// ---------------------------------------------------------------------------
// Source guard: the knob call sites restate nothing
// ---------------------------------------------------------------------------

/// The guard is only as good as its file list. A knob moved into a
/// module nobody added here would be invisible to every test below —
/// which is the failure mode that nearly slipped past the gate's
/// migration (#1286), where the guard read a single hardcoded file.
#[test]
fn the_guard_reads_every_editor_source() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/editor");
    let mut on_disk = BTreeSet::new();
    let mut stack = vec![dir.clone()];
    while let Some(path) = stack.pop() {
        for entry in std::fs::read_dir(&path).expect("src/editor must be readable") {
            let entry = entry.expect("readable dir entry");
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e == "rs") {
                on_disk.insert(
                    p.strip_prefix(&dir)
                        .expect("under src/editor")
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }

    let compiled_in: BTreeSet<String> = EDITOR_SOURCES.iter().map(|(f, _)| f.to_string()).collect();
    assert_eq!(
        compiled_in, on_disk,
        "EDITOR_SOURCES has drifted from src/editor — add the new file(s) here, \
         otherwise the knob guards below silently stop covering them"
    );
}

#[test]
fn params_are_drawn_only_through_the_param_bound_helper() {
    for (file, src) in EDITOR_SOURCES {
        let code = code_only(src);
        for raw in [
            // Takes its range, default and a linear/log flag as
            // arguments — that is finding F4's drift, one layer down.
            "widgets::knob(",
            "widgets::slider(",
            // egui's own controls, likewise.
            "egui::Slider",
            "egui::DragValue",
        ] {
            assert!(
                !code.contains(raw),
                "{file}: `{raw}` draws a control from arguments instead of from its \
                 FloatParam; use `{KNOB_CALL}`"
            );
        }
    }
    assert!(
        !knob_calls().is_empty(),
        "no knob call sites found at all — the guard has gone vacuous"
    );
}

#[test]
fn every_knob_call_passes_the_param_and_two_captions() {
    for (file, args) in knob_calls() {
        assert_eq!(
            args.len(),
            4,
            "{file}: float_knob takes ui, the param and two captions — nothing else. \
             A range, a default or a readout among these is the drift finding F4 is \
             about. Got: {args:?}"
        );
        assert_eq!(args[0], "ui", "{file}: {args:?}");
        let param = &args[1];
        assert!(
            param.starts_with("&params.") || param.starts_with("&app.params."),
            "{file}: the knob must be handed a declared parameter, not a locally built \
             one — got `{param}`"
        );
        assert!(
            !param.contains('(') && !param.contains('"'),
            "{file}: `{param}` is not a plain field path"
        );
    }
}

#[test]
fn no_caption_restates_the_range_the_unit_or_the_readout() {
    for (file, args) in knob_calls() {
        for caption in args.iter().skip(2) {
            for restated in [
                "..=",
                "format!",
                ".value()",
                ".display(",
                "min_plain",
                "max_plain",
                "default_plain",
                "get_plain",
                "logarithmic",
            ] {
                assert!(
                    !caption.contains(restated),
                    "{file}: caption `{caption}` contains `{restated}` — the helper reads \
                     all of that off the parameter"
                );
            }
            for unit in [" dB", " Hz", " ms", " %", "%\""] {
                assert!(
                    !caption.contains(unit),
                    "{file}: caption `{caption}` repeats the unit `{unit}` that params.rs \
                     declares and the readout already prints"
                );
            }
            assert!(
                !caption.bytes().any(|b| b.is_ascii_digit()),
                "{file}: caption `{caption}` names a number; the parameter's own range and \
                 readout are the only place values belong"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Surface guard: declared, registered, drawn, swept
// ---------------------------------------------------------------------------

/// A parameter that `params.rs` declares but that never comes back out of
/// the host-facing `param()` exists for the DSP and the editor and for
/// nobody else: no host automation, no `track.plugin_params`, no plugin
/// state. The reverse — a registered field that no longer exists — will
/// not compile, so this is the direction that can rot.
///
/// Asked of the RUNTIME surface, not of the source. An earlier version
/// scraped `lib.rs`'s `param()` body for `&self.params.<field>` and broke
/// the moment that function was refactored to delegate
/// (`self.params.param_at(index)`) — it then found nothing, reported an
/// empty surface, and failed a plugin whose registration was perfectly
/// fine. Walking `0..param_count()` and asking each parameter for its own
/// id is what "registered with the host" actually means, and it cannot
/// care how the delegation is spelled.
#[test]
fn every_declared_param_is_registered_with_the_host() {
    let declared: BTreeSet<String> = declared_param_ids();

    let plugin = ResonanceAmp::new();
    let registered: BTreeSet<String> = (0..plugin.param_count())
        .map(|i| plugin.param(i).id().to_string())
        .collect();

    assert_eq!(
        declared, registered,
        "params.rs declares one parameter surface and `param()` hands the \
         host another"
    );
    assert_eq!(
        plugin.param_count(),
        declared.len(),
        "param_count reports {} for {} declared parameters — an index in \
         range that maps to no distinct parameter is a silently duplicated \
         or unreachable control",
        plugin.param_count(),
        declared.len()
    );
}

#[test]
fn every_float_param_is_drawn_as_a_knob() {
    let bound: BTreeSet<String> = knob_calls()
        .into_iter()
        .map(|(_, args)| {
            args[1]
                .rsplit('.')
                .next()
                .expect("a field path")
                .to_string()
        })
        .collect();

    for (name, ty) in declared_params() {
        if ty != "FloatParam" {
            continue;
        }
        assert!(
            bound.contains(&name) || NOT_A_KNOB.contains(&name.as_str()),
            "`{name}` is a FloatParam that no knob edits — draw it through {KNOB_CALL} or \
             record here why it is not a knob"
        );
    }
    for name in &bound {
        assert!(
            declared_params().iter().any(|(n, _)| n == name),
            "a knob is bound to `{name}`, which params.rs does not declare"
        );
    }
}

/// The sweeps below run over a hand-written list, so the list is what
/// decides whether they cover anything.
#[test]
fn the_sweep_covers_every_declared_float_param() {
    let params = AmpParams::default();
    let swept: BTreeSet<&str> = float_params(&params).into_iter().map(|(n, _)| n).collect();
    let declared: BTreeSet<String> = declared_params()
        .into_iter()
        .filter(|(_, ty)| ty == "FloatParam")
        .map(|(n, _)| n)
        .collect();
    let swept: BTreeSet<String> = swept.into_iter().map(str::to_string).collect();
    assert_eq!(
        swept, declared,
        "add the new FloatParam to `float_params` so the travel tests cover it"
    );
}

// ---------------------------------------------------------------------------
// Behaviour: the arc is the parameter's own range
// ---------------------------------------------------------------------------

#[test]
fn knob_travel_is_exactly_the_declared_range() {
    let params = AmpParams::default();
    for (field, param) in float_params(&params) {
        let range = param.range();
        assert_eq!(
            param.plain_at_normalized(0.0),
            range.min(),
            "{field}: 0 % travel must be the declared minimum"
        );
        assert_eq!(
            param.plain_at_normalized(1.0),
            range.max(),
            "{field}: 100 % travel must be the declared maximum"
        );

        let mut previous = f32::NEG_INFINITY;
        for step in 0..=100 {
            let t = step as f32 / 100.0;
            let plain = param.plain_at_normalized(t);
            assert_eq!(
                plain,
                range.denormalize(t),
                "{field}: the knob must sweep the param's own denormalize"
            );
            assert!(
                plain >= previous,
                "{field}: travel must move the value monotonically (t = {t})"
            );
            assert!(
                plain >= range.min() && plain <= range.max(),
                "{field}: {plain} at t = {t} escapes the declared range"
            );
            previous = plain;
        }
    }
}

#[test]
fn the_knob_and_the_parameter_agree_in_both_directions() {
    let params = AmpParams::default();
    for (field, param) in float_params(&params) {
        let range = param.range();
        for step in 0..=20 {
            // Where the host puts the value, the knob must point.
            let plain = range.denormalize(step as f32 / 20.0);
            param.set_value(plain);
            let travel = param.normalized_value();
            let round_tripped = param.plain_at_normalized(travel);
            let tolerance = (range.max() - range.min()).abs() * 1e-4;
            assert!(
                (round_tripped - plain).abs() <= tolerance,
                "{field}: {plain} reads back as {round_tripped} through the arc"
            );

            // And where the user drags the knob, the parameter follows.
            param.set_normalized(travel);
            assert!(
                (param.value() - plain).abs() <= tolerance,
                "{field}: dragging to {travel} left the param at {}",
                param.value()
            );
        }
    }
}

#[test]
fn a_reset_lands_on_the_declared_default_verbatim() {
    let params = AmpParams::default();
    for (field, param) in float_params(&params) {
        param.set_value(param.range().max());
        // Double-click-to-reset drops the knob on default_normalized.
        param.set_normalized(param.default_normalized());
        assert_eq!(
            param.value(),
            param.default_value(),
            "{field}: reset must write the declared default exactly, not the curve's round trip"
        );
    }
}

#[test]
fn no_readout_doubles_up_the_declared_unit() {
    let params = AmpParams::default();
    for (field, param) in float_params(&params) {
        let unit = param.unit();
        if unit.is_empty() {
            continue;
        }
        for step in 0..=10 {
            let value = param.plain_at_normalized(step as f32 / 10.0);
            let text = param.display(value as f64);
            assert_eq!(
                text.matches(unit.trim()).count(),
                1,
                "{field}: `{text}` repeats the unit `{unit}`"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The one drifted knob, pinned
// ---------------------------------------------------------------------------

/// Finding F4's amp entry. The editor reset Output Gain to `1.0` (0 dB);
/// `params.rs` declares `0.5` (−6.02 dB), which is what `initialize()`
/// primes the smoother with and what the host is told the default is.
/// The crate ships no factory presets, so there was no third opinion —
/// the knob was simply wrong, and the declaration is untouched.
#[test]
fn output_gain_resets_to_the_declared_minus_six_db() {
    let p = AmpParams::default();
    assert_eq!(
        p.output_gain.default_value(),
        0.5,
        "params.rs declares 0.5; the old call site said 1.0"
    );
    assert_eq!(p.output_gain.display(0.5), "-6.02 dB");
    assert_eq!(p.output_gain.value(), 0.5, "a fresh param starts at 0.5");

    // Input Gain is the other case: its call site restated 0.01..=4.0
    // and 1.0, which is exactly what the parameter declares, so nothing
    // there *could* have drifted in value.
    assert_eq!(p.input_gain.default_value(), 1.0);
    assert_eq!(p.input_gain.range().min(), 0.01);
    assert_eq!(p.input_gain.range().max(), 4.0);
}

/// Every parameter a host or the control API can reach reports the
/// default the plugin actually starts from — the property the drift
/// above broke for one control, checked here across the whole surface.
#[test]
fn a_fresh_instance_starts_on_every_declared_default() {
    let plugin = ResonanceAmp::new();
    for param in plugin.params() {
        assert_eq!(
            param.get_plain(),
            param.default_plain(),
            "`{}` starts somewhere other than its declared default",
            param.id()
        );
    }
}

/// Both gain knobs were drawn on a hardcoded logarithmic arc, so their
/// declared `Skewed` ranges reached nothing. They do now — which also
/// makes ba todo #1349 visible: `gain_skew_factor` encodes nothing about
/// where unity sits, so neither dial has 0 dB at its centre. The numbers
/// are recorded rather than corrected, because moving them is a change
/// to `params.rs` and #1349's own job.
#[test]
fn the_declared_skew_reaches_the_arc_and_unity_is_off_centre() {
    let p = AmpParams::default();
    for (field, param, unity_travel) in [
        ("input_gain", &p.input_gain, 0.619_f32),
        ("output_gain", &p.output_gain, 0.646_f32),
    ] {
        assert!(
            matches!(param.range(), FloatRange::Skewed { .. }),
            "{field} must stay Skewed for the arc to be worth testing"
        );
        let travel = param.range().normalize(1.0);
        assert!(
            (travel - unity_travel).abs() < 0.005,
            "{field}: unity sits at {travel} of the dial, recorded as {unity_travel}"
        );
        assert!(
            travel > 0.55,
            "{field}: unity is above dial centre — that is ba todo #1349, not a regression"
        );

        // Half travel is around −6 dB on both dials, well below the
        // arithmetic middle of the linear gain range: the skew is live.
        let arc_mid = param.plain_at_normalized(0.5);
        let linear_mid = (param.range().min() + param.range().max()) / 2.0;
        assert!(
            arc_mid < linear_mid * 0.3,
            "{field}: half travel is {arc_mid} against a linear middle of {linear_mid} — \
             the declared skew is not reaching the control"
        );
    }
}

// ---------------------------------------------------------------------------
// Finding A6: file_select is visible
// ---------------------------------------------------------------------------

/// Model switching is the most consequential thing this plugin does, and
/// `.hidden()` kept it out of the host's parameter list, the generic
/// parameter panel and the automation-lane picker.
#[test]
fn no_parameter_is_hidden_from_the_host() {
    let plugin = ResonanceAmp::new();
    for param in plugin.params() {
        assert!(
            !param.is_hidden(),
            "`{}` is hidden; resonance-ir's identical file_select is not, and ba todo \
             #1290 made `hidden` a display hint rather than a storage switch",
            param.id()
        );
    }
    assert!(
        !code_only(PARAMS_SRC).contains(".hidden()"),
        "params.rs declares a hidden parameter again — finding A6"
    );
}

/// Visibility is not what made the value survive: plugin state has
/// always been written from the full parameter list, and since ba todo
/// #1290 the engine reports hidden params to the app flagged rather than
/// dropping them. Unhiding must therefore change the param list a reader
/// sees and nothing about what is stored.
#[test]
fn the_selected_model_index_round_trips_through_plugin_state() {
    let plugin = ResonanceAmp::new();
    let params = plugin.params();
    let file_select = params
        .iter()
        .find(|p| p.id() == "file_select")
        .expect("file_select must be registered");
    file_select.set_plain(7.0);

    let state = resonance_plugin::state::params_to_json(&params);
    file_select.set_plain(0.0);
    assert!(resonance_plugin::state::load_params_from_json(
        &params, &state
    ));
    assert_eq!(
        file_select.get_plain(),
        7.0,
        "the selected model index must survive a state round trip"
    );
}

/// `file_select` is stepped and 1000 values wide. The engine's
/// choice-label walk (`MAX_CHOICE_STEPS`, 64) declines anything that
/// large, so making the parameter visible does not put a 1000-call label
/// enumeration behind every `track.plugin_params`.
#[test]
fn the_model_selector_is_too_wide_for_the_choice_label_walk() {
    let p = AmpParams::default();
    let steps = p.file_select.range().max() - p.file_select.range().min() + 1;
    assert_eq!(steps, 1000);
    assert!(
        steps > 64,
        "file_select now fits the choice walk; check what {steps} labels cost per query"
    );
}
