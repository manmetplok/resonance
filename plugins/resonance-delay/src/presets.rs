pub struct PresetEntry {
    pub name: &'static str,
    pub json: &'static str,
}

pub const PRESETS: &[PresetEntry] = &[
    PresetEntry {
        name: "Quarter Note",
        json: r#"{"params":{"sync":1.0,"division":4.0,"time_ms":375.0,"feedback":0.35,"mix":0.35,"character":0.0,"routing":0.0,"stereo_offset":0.0,"hi_cut":8000.0,"lo_cut":120.0,"drive":0.1,"mod_rate":0.4,"mod_depth":0.05,"freeze":0.0}}"#,
    },
    PresetEntry {
        name: "Dotted Eighth",
        json: r#"{"params":{"sync":1.0,"division":8.0,"time_ms":375.0,"feedback":0.40,"mix":0.35,"character":0.0,"routing":0.0,"stereo_offset":0.0,"hi_cut":8000.0,"lo_cut":120.0,"drive":0.1,"mod_rate":0.4,"mod_depth":0.05,"freeze":0.0}}"#,
    },
    PresetEntry {
        name: "Slapback",
        json: r#"{"params":{"sync":0.0,"division":4.0,"time_ms":80.0,"feedback":0.15,"mix":0.50,"character":0.0,"routing":0.0,"stereo_offset":0.0,"hi_cut":10000.0,"lo_cut":80.0,"drive":0.05,"mod_rate":0.3,"mod_depth":0.02,"freeze":0.0}}"#,
    },
    PresetEntry {
        name: "Dub",
        json: r#"{"params":{"sync":1.0,"division":4.0,"time_ms":375.0,"feedback":0.65,"mix":0.40,"character":1.0,"routing":0.0,"stereo_offset":0.0,"hi_cut":3000.0,"lo_cut":200.0,"drive":0.35,"mod_rate":0.5,"mod_depth":0.15,"freeze":0.0}}"#,
    },
    PresetEntry {
        name: "Ping-Pong Eighth",
        json: r#"{"params":{"sync":1.0,"division":7.0,"time_ms":375.0,"feedback":0.45,"mix":0.40,"character":0.0,"routing":1.0,"stereo_offset":0.0,"hi_cut":8000.0,"lo_cut":120.0,"drive":0.1,"mod_rate":0.4,"mod_depth":0.05,"freeze":0.0}}"#,
    },
    PresetEntry {
        name: "Lo-Fi Tape",
        json: r#"{"params":{"sync":0.0,"division":4.0,"time_ms":350.0,"feedback":0.55,"mix":0.40,"character":1.0,"routing":0.0,"stereo_offset":0.1,"hi_cut":2500.0,"lo_cut":180.0,"drive":0.25,"mod_rate":0.6,"mod_depth":0.30,"freeze":0.0}}"#,
    },
];
