//! Display / formatting helpers on `TempoMap`.

use super::map::TempoMap;

impl TempoMap {
    /// Format a sample position as "bar.beat".
    pub fn format_position(&self, sample_pos: u64, sample_rate: u32) -> String {
        let (bar, beat, _) = self.position_to_bars(sample_pos, sample_rate);
        format!("{}.{}", bar, beat)
    }

    /// Format a sample position as "mm:ss.mmm" wall-clock time.
    pub fn format_time(&self, sample_pos: u64, sample_rate: u32) -> String {
        let total_secs = sample_pos as f64 / sample_rate as f64;
        let minutes = (total_secs / 60.0).floor() as u32;
        let seconds = total_secs - (minutes as f64 * 60.0);
        format!("{:02}:{:06.3}", minutes, seconds)
    }
}

impl std::fmt::Display for super::InputDeviceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.description)
    }
}

impl std::fmt::Display for super::ScannedPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.vendor.is_empty() {
            write!(f, "{}", self.name)
        } else {
            write!(f, "{} ({})", self.name, self.vendor)
        }
    }
}
