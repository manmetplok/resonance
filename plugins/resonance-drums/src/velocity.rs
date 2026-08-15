//! The global velocity curve: how an incoming MIDI velocity is mapped
//! before it picks a velocity layer and sets the hit's gain.
//!
//! The control is bipolar and centred on linear, which is the default
//! and is an exact identity — a project that never touches it renders
//! bit-for-bit what it rendered before the curve existed (ba todo
//! #1326).
//!
//! Positive is **soft**: the same physical hit reads louder, so a light
//! player reaches the top layers. Negative is **hard**: velocity has to
//! be pushed further to reach the same layer. Both are the same power
//! curve, `v^(2^-2c)`, so the ends of the range (0 and 1) are fixed
//! points and the mapping stays monotonic — a harder hit is never
//! quieter than a softer one.

/// Shape `velocity` (0..1) with `curve` (-1..1). `curve == 0` returns
/// the velocity unchanged.
pub fn shape(velocity: f32, curve: f32) -> f32 {
    let v = velocity.clamp(0.0, 1.0);
    if curve == 0.0 {
        // Exact identity, not `v.powf(1.0)` — the default must not be
        // able to perturb an existing project by a rounding step.
        return v;
    }
    let exponent = (-2.0 * curve.clamp(-1.0, 1.0)).exp2();
    v.powf(exponent)
}

/// The curve's name, as the host's parameter display, the editor
/// readout and the control API all show it.
pub fn curve_label(curve: f32) -> String {
    let c = curve.clamp(-1.0, 1.0);
    // Round first so a value that displays as 0% never reads "Soft 0%".
    let percent = (c.abs() * 100.0).round();
    if percent == 0.0 {
        "Linear".to_string()
    } else if c > 0.0 {
        format!("Soft {percent:.0}%")
    } else {
        format!("Hard {percent:.0}%")
    }
}

/// Parse a curve label back to its value, so a host that lets the user
/// type into the automation lane round-trips what it displayed. A plain
/// number is accepted too.
pub fn curve_from_label(text: &str) -> Option<f32> {
    let trimmed = text.trim();
    if trimmed.eq_ignore_ascii_case("linear") {
        return Some(0.0);
    }
    let (sign, rest) = if let Some(rest) = strip_prefix_ignore_case(trimmed, "soft") {
        (1.0, rest)
    } else if let Some(rest) = strip_prefix_ignore_case(trimmed, "hard") {
        (-1.0, rest)
    } else {
        return trimmed.parse::<f32>().ok().map(|v| v.clamp(-1.0, 1.0));
    };
    let percent: f32 = rest.trim().trim_end_matches('%').trim().parse().ok()?;
    Some((sign * percent / 100.0).clamp(-1.0, 1.0))
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    if text.len() >= prefix.len() && text[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&text[prefix.len()..])
    } else {
        None
    }
}
