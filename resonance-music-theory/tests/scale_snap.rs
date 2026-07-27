use resonance_music_theory::pitch::PitchClass;
use resonance_music_theory::scale::{Mode, Scale};

// Helper to create a scale with root C for simplicity
fn c_scale(mode: Mode) -> Scale {
    Scale::new(PitchClass::C, mode)
}

#[test]
fn strength_zero_is_identity() {
    let scale = c_scale(Mode::Major);
    
    // Test various pitches
    assert_eq!(scale.snap_pitch(60.0, 0.0), 60.0); // C4
    assert_eq!(scale.snap_pitch(61.0, 0.0), 61.0); // C#4 (not in C major)
    assert_eq!(scale.snap_pitch(62.5, 0.0), 62.5); // D4 + 50 cents
    assert_eq!(scale.snap_pitch(45.7, 0.0), 45.7); // A2 + 70 cents
}

#[test]
fn strength_one_lands_on_scale_degree() {
    let scale = c_scale(Mode::Major);
    
    // C major scale: C D E F G A B (0, 2, 4, 5, 7, 9, 11)
    
    // C4 (60.0) is already in scale
    assert_eq!(scale.snap_pitch(60.0, 1.0), 60.0);
    
    // C#4 (61.0) should snap to C (60.0) or D (62.0)? 
    // Distance from 61.0 to 60.0 = 1.0 semitone
    // Distance from 61.0 to 62.0 = 1.0 semitone
    // Tie-breaking: higher degree is chosen, so D (62.0)
    assert_eq!(scale.snap_pitch(61.0, 1.0), 62.0);
    
    // D4 (62.0) is in scale
    assert_eq!(scale.snap_pitch(62.0, 1.0), 62.0);
    
    // D#4 (63.0) should snap to D (62.0) or E (64.0)?
    // Distance to D: 1.0, Distance to E: 1.0 -> tie, higher wins: E (64.0)
    assert_eq!(scale.snap_pitch(63.0, 1.0), 64.0);
    
    // E4 (64.0) is in scale
    assert_eq!(scale.snap_pitch(64.0, 1.0), 64.0);
    
    // F4 (65.0) is in scale
    assert_eq!(scale.snap_pitch(65.0, 1.0), 65.0);
    
    // F#4 (66.0) should snap to F (65.0) or G (67.0)?
    // Distance to F: 1.0, Distance to G: 1.0 -> tie, higher wins: G (67.0)
    assert_eq!(scale.snap_pitch(66.0, 1.0), 67.0);
}

#[test]
fn chromatic_scale_is_no_op() {
    let scale = c_scale(Mode::Chromatic);
    
    // Every pitch should pass through unchanged
    assert_eq!(scale.snap_pitch(60.0, 0.5), 60.0);
    assert_eq!(scale.snap_pitch(61.0, 0.5), 61.0);
    assert_eq!(scale.snap_pitch(61.5, 1.0), 61.5);
    assert_eq!(scale.snap_pitch(62.3, 0.0), 62.3);
    assert_eq!(scale.snap_pitch(45.7, 1.0), 45.7);
}

#[test]
fn intermediate_strength_interpolates() {
    let scale = c_scale(Mode::Major);
    
    // C#4 (61.0) should snap to D4 (62.0) with strength 1.0
    // With strength 0.5, it should be halfway: 61.0 + (62.0 - 61.0) * 0.5 = 61.5
    assert_eq!(scale.snap_pitch(61.0, 0.5), 61.5);
    
    // With strength 0.0, it's 61.0
    assert_eq!(scale.snap_pitch(61.0, 0.0), 61.0);
    // With strength 1.0, it's 62.0
    assert_eq!(scale.snap_pitch(61.0, 1.0), 62.0);
    
    // 60.5 is between C and D: distance to C is 0.5, to D is 1.5 -> snap to C (60.0)
    // Interpolation: 60.5 + (60.0 - 60.5) * 0.5 = 60.5 - 0.25 = 60.25
    assert_eq!(scale.snap_pitch(60.5, 0.5), 60.25);
    
    // 61.5 is between C# and D, distance to D is 0.5, to C is 1.5 -> snap to D (62.0)
    // Interpolation: 61.5 + (62.0 - 61.5) * 0.5 = 61.5 + 0.25 = 61.75
    assert_eq!(scale.snap_pitch(61.5, 0.5), 61.75);
}

#[test]
fn major_scale_snapping() {
    let scale = c_scale(Mode::Major);
    
    // C major: C(0), D(2), E(4), F(5), G(7), A(9), B(11)
    
    // Test that each scale degree snaps to itself
    for &offset in &[0, 2, 4, 5, 7, 9, 11] {
        let note = 60.0 + offset as f32;
        assert_eq!(scale.snap_pitch(note, 1.0), note);
    }
    
    // Test notes between scale degrees
    // Between C(60) and D(62): 61 should snap to C or D?
    // Distance: |61-60|=1, |61-62|=1 -> tie, higher wins: D
    assert_eq!(scale.snap_pitch(61.0, 1.0), 62.0);
    
    // Between D(62) and E(64): 63 should snap to D or E?
    // Distance: |63-62|=1, |63-64|=1 -> tie, higher wins: E
    assert_eq!(scale.snap_pitch(63.0, 1.0), 64.0);
    
    // Between E(64) and F(65): 64.5 is 0.5 from E and 0.5 from F -> tie, higher wins: F
    assert_eq!(scale.snap_pitch(64.5, 1.0), 65.0);
    
    // 64.4 is closer to E (0.4 away) than F (0.6 away)
    assert_eq!(scale.snap_pitch(64.4, 1.0), 64.0);
    
    // 64.6 is closer to F (0.4 away) than E (0.6 away)
    assert_eq!(scale.snap_pitch(64.6, 1.0), 65.0);
}

#[test]
fn minor_scale_snapping() {
    let scale = c_scale(Mode::Minor);
    
    // C minor (natural): C(0), D(2), Eb(3), F(5), G(7), Ab(8), Bb(10)
    
    // Test that each scale degree snaps to itself
    for &offset in &[0, 2, 3, 5, 7, 8, 10] {
        let note = 60.0 + offset as f32;
        assert_eq!(scale.snap_pitch(note, 1.0), note);
    }
    
    // Test notes between scale degrees
    // Between C(60) and D(62): 61 should snap to C or D?
    // Distance: |61-60|=1, |61-62|=1 -> tie, higher wins: D
    assert_eq!(scale.snap_pitch(61.0, 1.0), 62.0);
    
    // Between D(62) and Eb(63): 62.5 is 0.5 from D and 0.5 from Eb -> tie, higher wins: Eb
    assert_eq!(scale.snap_pitch(62.5, 1.0), 63.0);
    
    // Between Eb(63) and F(65): 64 should snap to Eb(63) or F(65)?
    // Distance: |64-63|=1, |64-65|=1 -> tie, higher wins: F
    assert_eq!(scale.snap_pitch(64.0, 1.0), 65.0);
}

#[test]
fn different_octaves() {
    let scale = c_scale(Mode::Major);
    
    // Test in different octaves
    // C3 (48.0)
    assert_eq!(scale.snap_pitch(48.0, 1.0), 48.0);
    
    // C5 (72.0)
    assert_eq!(scale.snap_pitch(72.0, 1.0), 72.0);
    
    // C#3 (49.0) should snap to D3 (50.0)
    assert_eq!(scale.snap_pitch(49.0, 1.0), 50.0);
    
    // C#5 (73.0) should snap to D5 (74.0)
    assert_eq!(scale.snap_pitch(73.0, 1.0), 74.0);
}

#[test]
fn non_c_root_scale() {
    // G major scale: G(7), A(9), B(11), C(0+12=12), D(2+12=14), E(4+12=16), F#(6+12=18)
    // MIDI notes: G4=67, A4=69, B4=71, C5=72, D5=74, E5=76, F#5=78
    let scale = Scale::new(PitchClass::G, Mode::Major);
    
    // G4 is MIDI note 67
    assert_eq!(scale.snap_pitch(67.0, 1.0), 67.0);
    
    // A4 is MIDI note 69
    assert_eq!(scale.snap_pitch(69.0, 1.0), 69.0);
    
    // B4 is MIDI note 71
    assert_eq!(scale.snap_pitch(71.0, 1.0), 71.0);
    
    // C5 is MIDI note 72
    assert_eq!(scale.snap_pitch(72.0, 1.0), 72.0);
    
    // D5 is MIDI note 74
    assert_eq!(scale.snap_pitch(74.0, 1.0), 74.0);
    
    // E5 is MIDI note 76
    assert_eq!(scale.snap_pitch(76.0, 1.0), 76.0);
    
    // F#5 is MIDI note 78
    assert_eq!(scale.snap_pitch(78.0, 1.0), 78.0);
    
    // G#4 (68) should snap to G(67) or A(69)?
    // Distance: |68-67|=1, |68-69|=1 -> tie, higher wins: A(69)
    assert_eq!(scale.snap_pitch(68.0, 1.0), 69.0);
}

#[test]
fn cents_precision() {
    let scale = c_scale(Mode::Major);
    
    // Test with cents (1 cent = 0.01 semitone)
    // C4 with 50 cents (60.5)
    // This is 0.5 semitones above C, which should snap to C (60) or D (62)?
    // Distance in semitones: |0.5-0|=0.5, |0.5-2|=1.5 -> nearest is C (0)
    assert_eq!(scale.snap_pitch(60.5, 1.0), 60.0);
    
    // 10 cents above C (60.1)
    assert_eq!(scale.snap_pitch(60.1, 1.0), 60.0);
    
    // 90 cents above C (60.9)
    assert_eq!(scale.snap_pitch(60.9, 1.0), 60.0);
    
    // 110 cents above C (61.1) - this is past C#
    // In absolute terms, 61.1 is closest to 61 (C#) or 62 (D)?
    // But 61 is not in C major. The nearest scale degree is D (62).
    // Distance to D: |61.1-62|=0.9, Distance to C: |61.1-60|=1.1 -> nearest is D
    assert_eq!(scale.snap_pitch(61.1, 1.0), 62.0);
}

#[test]
fn clamp_strength() {
    let scale = c_scale(Mode::Major);
    
    // Strength < 0 should be clamped to 0
    assert_eq!(scale.snap_pitch(61.0, -0.5), 61.0);
    
    // Strength > 1 should be clamped to 1
    assert_eq!(scale.snap_pitch(61.0, 1.5), 62.0);
}
