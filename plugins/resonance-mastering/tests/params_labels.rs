//! Guard test: every `FloatParam` in this crate must be labelled.
//!
//! A `FloatParam` whose builder chain ends without `.with_unit(..)` and
//! `.with_value_to_string(..)` shows up in a host automation lane as a
//! bare number — "0.71" with no indication of what it is. That was
//! finding P8 of the plugin audit (the EQ-stage `q` param, instantiated
//! four bands x two stages).
//!
//! Rather than assert on the one param that was wrong, this test walks
//! the crate's own source and checks *every* `FloatParam::new(..)`
//! builder chain, so the next unlabelled param fails here instead of in
//! a user's host. It is a source scan because the `Param` trait exposes
//! no accessor for the unit or the formatter (adding one is framework
//! work tracked separately in the audit's Wave 1).

use std::fs;
use std::path::{Path, PathBuf};

use resonance_mastering::params::{MasteringParams, PARAM_COUNT};
use resonance_plugin::Param;

/// Methods a `FloatParam` builder chain must contain.
const REQUIRED: [&str; 2] = ["with_unit", "with_value_to_string"];

#[test]
fn every_float_param_declares_a_unit_and_a_formatter() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs_files(&src, &mut files);
    assert!(
        !files.is_empty(),
        "no .rs files found under {} — the scan would pass vacuously",
        src.display()
    );

    let mut checked = 0usize;
    let mut failures = Vec::new();

    for file in &files {
        let text = fs::read(file).expect("read source file");
        let masked = mask_strings_and_comments(&text);
        for site in find_float_param_chains(&masked) {
            checked += 1;
            let missing: Vec<&str> = REQUIRED
                .iter()
                .copied()
                .filter(|m| !site.chain.iter().any(|c| c == m))
                .collect();
            if !missing.is_empty() {
                failures.push(format!(
                    "{}:{}: FloatParam::new(..) chain is missing {} (chain: [{}])",
                    file.display(),
                    line_of(&text, site.start),
                    missing.join(" and "),
                    site.chain.join(", ")
                ));
            }
        }
    }

    assert!(
        checked >= 20,
        "only found {checked} FloatParam::new sites — the scanner is probably broken"
    );
    assert!(
        failures.is_empty(),
        "unlabelled FloatParam(s):\n{}",
        failures.join("\n")
    );
}

/// The other half of the same rule, checked at runtime: whatever a
/// continuous param renders for the host must carry a label, not just
/// digits. This is what the source scan is a proxy for, and it covers
/// the generated params (the EQ `q` exists four bands x two stages)
/// that no single source site can speak for.
#[test]
fn continuous_params_render_a_labelled_value() {
    let params = MasteringParams::default();
    let mut bare = Vec::new();
    for i in 0..PARAM_COUNT {
        let p = params.param_at(i);
        if p.is_stepped() {
            continue;
        }
        let (min, max) = (p.min_plain(), p.max_plain());
        for step in 0..=4 {
            let v = min + (max - min) * f64::from(step) / 4.0;
            let text = p.display(v);
            let labelled = text
                .chars()
                .any(|c| c.is_alphabetic() || c == '%' || c == ':');
            if !labelled {
                bare.push(format!("{} = {:?} (plain {v})", p.id(), text));
            }
        }
    }
    assert!(
        bare.is_empty(),
        "continuous params rendering a bare number:\n{}",
        bare.join("\n")
    );
}

/// The finding that started this: the EQ `q` param, four bands x two
/// stages, used to render as "0.71".
#[test]
fn eq_q_is_labelled() {
    let params = MasteringParams::default();
    for stage in [&params.corrective_eq, &params.tonal_eq] {
        for band in &stage.bands {
            assert_eq!(band.q.display(0.707), "0.71 Q");
            assert_eq!(band.q.parse("0.71 Q"), Some(0.71));
        }
    }
}

/// Sanity check on the scanner itself: it must actually reject a chain
/// that is missing the calls, and it must not be fooled by parentheses
/// or `.with_unit` mentions that live inside strings and comments.
#[test]
fn scanner_detects_a_missing_unit() {
    let good = br#"
        let a = FloatParam::new("a", "A", 0.0, r)
            .with_unit(" dB")
            .with_value_to_string(v2s_f32_db(1));
    "#;
    let sites = find_float_param_chains(&mask_strings_and_comments(good));
    assert_eq!(sites.len(), 1);
    assert_eq!(sites[0].chain, vec!["with_unit", "with_value_to_string"]);

    let bad = br#"
        // .with_unit(" dB") mentioned in a comment does not count
        let b = FloatParam::new("b", "B (see .with_unit)", 0.0, Range { f: g(-1.0) })
            .with_value_to_string(v2s_f32_rounded(2));
    "#;
    let sites = find_float_param_chains(&mask_strings_and_comments(bad));
    assert_eq!(sites.len(), 1);
    assert_eq!(sites[0].chain, vec!["with_value_to_string"]);

    // A chain that simply ends after the constructor.
    let bare = b"let c = FloatParam::new(\"c\", \"C\", 0.0, r);";
    let sites = find_float_param_chains(&mask_strings_and_comments(bare));
    assert_eq!(sites.len(), 1);
    assert!(sites[0].chain.is_empty());
}

// --- scanner -------------------------------------------------------------

struct ChainSite {
    /// Byte offset of the `FloatParam::new` token.
    start: usize,
    /// Builder methods called on the result, in order.
    chain: Vec<String>,
}

const CTOR: &[u8] = b"FloatParam::new";

fn find_float_param_chains(masked: &[u8]) -> Vec<ChainSite> {
    let mut sites = Vec::new();
    let mut i = 0usize;
    while i + CTOR.len() <= masked.len() {
        if &masked[i..i + CTOR.len()] != CTOR {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i + CTOR.len();
        j = skip_trivia(masked, j);
        if masked.get(j) != Some(&b'(') {
            i = start + CTOR.len();
            continue;
        }
        let Some(after_args) = match_paren(masked, j) else {
            i = start + CTOR.len();
            continue;
        };
        let mut chain = Vec::new();
        let mut k = skip_trivia(masked, after_args);
        while masked.get(k) == Some(&b'.') {
            let name_start = skip_trivia(masked, k + 1);
            let mut name_end = name_start;
            while masked
                .get(name_end)
                .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
            {
                name_end += 1;
            }
            if name_end == name_start {
                break;
            }
            let paren = skip_trivia(masked, name_end);
            if masked.get(paren) != Some(&b'(') {
                break;
            }
            let Some(after_call) = match_paren(masked, paren) else {
                break;
            };
            chain.push(String::from_utf8_lossy(&masked[name_start..name_end]).into_owned());
            k = skip_trivia(masked, after_call);
        }
        sites.push(ChainSite { start, chain });
        i = after_args;
    }
    sites
}

/// Index just past the `)` matching the `(` at `open`.
fn match_paren(masked: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open;
    while i < masked.len() {
        match masked[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Skip whitespace. Comments are already blanked by the masker, so they
/// look like whitespace here.
fn skip_trivia(masked: &[u8], mut i: usize) -> usize {
    while masked.get(i).is_some_and(|c| c.is_ascii_whitespace()) {
        i += 1;
    }
    i
}

fn line_of(text: &[u8], offset: usize) -> usize {
    1 + text[..offset.min(text.len())]
        .iter()
        .filter(|c| **c == b'\n')
        .count()
}

/// Replace the body of every string literal, char literal and comment
/// with spaces, preserving byte offsets. Parens and `.with_unit` inside
/// a string or a comment must not be mistaken for code.
fn mask_strings_and_comments(text: &[u8]) -> Vec<u8> {
    let mut out = text.to_vec();
    let mut i = 0usize;
    while i < out.len() {
        match out[i] {
            b'/' if out.get(i + 1) == Some(&b'/') => {
                while i < out.len() && out[i] != b'\n' {
                    out[i] = b' ';
                    i += 1;
                }
            }
            b'/' if out.get(i + 1) == Some(&b'*') => {
                let mut depth = 0usize;
                while i < out.len() {
                    if out[i] == b'/' && out.get(i + 1) == Some(&b'*') {
                        depth += 1;
                        out[i] = b' ';
                        out[i + 1] = b' ';
                        i += 2;
                    } else if out[i] == b'*' && out.get(i + 1) == Some(&b'/') {
                        depth -= 1;
                        out[i] = b' ';
                        out[i + 1] = b' ';
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        if out[i] != b'\n' {
                            out[i] = b' ';
                        }
                        i += 1;
                    }
                }
            }
            b'"' => i = mask_plain_string(&mut out, i),
            b'r' | b'b' => {
                if let Some(next) = raw_string_start(&out, i) {
                    i = mask_raw_string(&mut out, i, next);
                } else if out[i] == b'b' && out.get(i + 1) == Some(&b'"') {
                    i = mask_plain_string(&mut out, i + 1);
                } else {
                    i += 1;
                }
            }
            b'\'' => {
                if let Some(end) = char_literal_end(&out, i) {
                    for c in out.iter_mut().take(end).skip(i + 1) {
                        *c = b' ';
                    }
                    i = end + 1;
                } else {
                    // A lifetime (`'static`), not a char literal.
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    out
}

/// Mask a `"…"` literal starting at the opening quote; returns the index
/// just past the closing quote.
fn mask_plain_string(out: &mut [u8], open: usize) -> usize {
    let mut i = open + 1;
    while i < out.len() {
        match out[i] {
            b'\\' => {
                out[i] = b' ';
                if i + 1 < out.len() && out[i + 1] != b'\n' {
                    out[i + 1] = b' ';
                }
                i += 2;
            }
            b'"' => return i + 1,
            b'\n' => i += 1,
            _ => {
                out[i] = b' ';
                i += 1;
            }
        }
    }
    i
}

/// If a raw string starts at `i` (`r"`, `r#"`, `br##"`, …), return the
/// number of `#` used.
fn raw_string_start(out: &[u8], i: usize) -> Option<usize> {
    let mut j = i;
    if out[j] == b'b' {
        j += 1;
    }
    if out.get(j) != Some(&b'r') {
        return None;
    }
    j += 1;
    let mut hashes = 0usize;
    while out.get(j) == Some(&b'#') {
        hashes += 1;
        j += 1;
    }
    if out.get(j) == Some(&b'"') {
        Some(hashes)
    } else {
        None
    }
}

fn mask_raw_string(out: &mut [u8], start: usize, hashes: usize) -> usize {
    // Find the opening quote, then scan for `"` followed by `hashes` `#`.
    let mut i = start;
    while out[i] != b'"' {
        i += 1;
    }
    i += 1;
    while i < out.len() {
        if out[i] == b'"' {
            let closes = (1..=hashes).all(|h| out.get(i + h) == Some(&b'#'));
            if closes {
                return i + hashes + 1;
            }
        }
        if out[i] != b'\n' {
            out[i] = b' ';
        }
        i += 1;
    }
    i
}

/// Index of the closing `'` if `i` opens a char literal, else `None`
/// (which means it was a lifetime).
fn char_literal_end(out: &[u8], i: usize) -> Option<usize> {
    match out.get(i + 1)? {
        b'\\' => {
            // Escape: scan a short distance for the closing quote.
            let limit = (i + 12).min(out.len());
            out.iter()
                .enumerate()
                .take(limit)
                .skip(i + 2)
                .find(|(_, c)| **c == b'\'')
                .map(|(end, _)| end)
        }
        _ => {
            // A single (possibly multi-byte) char followed by a quote.
            let mut end = i + 2;
            while end < out.len() && (out[end] & 0xC0) == 0x80 {
                end += 1;
            }
            (out.get(end) == Some(&b'\'')).then_some(end)
        }
    }
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}
