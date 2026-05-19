//! Reference-track loading via `symphonia`.
//!
//! The user supplies a path to a stereo audio file (WAV, FLAC, MP3,
//! AAC, OGG/Vorbis, MP4 / M4A). The loader opens the file, decodes
//! every packet through symphonia's default codec registry, converts
//! each audio frame to stereo `f32`, and runs the same offline analysis
//! the assistant uses on the captured live buffer. The resulting
//! [`ReferenceTrack`] can then be supplied to the decision engine as
//! an ad-hoc target: the reference's LTAS becomes the target spectral
//! shape and the reference's integrated LUFS becomes the target
//! loudness.
//!
//! Runs synchronously on the UI thread — loading a few minutes of MP3
//! takes well under a second, and a background thread adds complexity
//! we don't need for the first pass.

use std::fs::File;
use std::path::Path;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use super::analyze::{self, AnalysisResult};

/// Maximum number of samples we'll decode from a reference file.
/// Ten minutes at 96 kHz stereo (~115 MB of f32 samples total) is
/// plenty of headroom for any mastering reference; longer inputs are
/// truncated so we don't allocate gigabytes on a bad file pick.
const MAX_SAMPLES_PER_CHANNEL: usize = 96_000 * 60 * 10;

#[derive(Debug, Clone)]
pub struct ReferenceTrack {
    pub display_name: String,
    pub sample_rate: f32,
    pub analysis: AnalysisResult,
}

/// Decode a file at `path` and run the full offline analysis. Returns
/// an error string (suitable for UI display) on any failure.
pub fn load_from_path(path: &str) -> Result<ReferenceTrack, String> {
    let path_obj = Path::new(path);
    let display_name = path_obj
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
        .to_string();

    let file = File::open(path).map_err(|e| format!("open '{path}': {e}"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path_obj.extension().and_then(|s| s.to_str()) {
        hint.with_extension(ext);
    }

    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| format!("probe: {e}"))?;

    let track = format
        .first_track_known_codec(TrackType::Audio)
        .ok_or_else(|| "no decodable audio track in file".to_string())?;
    let track_id = track.id;
    let audio_params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| "track missing audio codec parameters".to_string())?
        .clone();

    let sample_rate = audio_params
        .sample_rate
        .map(|sr| sr as f32)
        .unwrap_or(48_000.0);
    let channels = audio_params
        .channels
        .as_ref()
        .map(|c| c.count())
        .unwrap_or(2)
        .max(1);

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&audio_params, &AudioDecoderOptions::default())
        .map_err(|e| format!("decoder: {e}"))?;

    let mut left: Vec<f32> = Vec::new();
    let mut right: Vec<f32> = Vec::new();
    let mut interleaved: Vec<f32> = Vec::new();

    'decode: loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break 'decode,
            Err(SymphoniaError::IoError(_)) => break 'decode,
            Err(e) => return Err(format!("read packet: {e}")),
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(SymphoniaError::IoError(_)) => break 'decode,
            Err(e) => return Err(format!("decode: {e}")),
        };

        interleaved.clear();
        decoded.copy_to_vec_interleaved(&mut interleaved);
        append_frames(&interleaved, channels, &mut left, &mut right);

        if left.len() >= MAX_SAMPLES_PER_CHANNEL {
            break 'decode;
        }
    }

    if left.is_empty() {
        return Err("decoded 0 samples".to_string());
    }

    let analysis = analyze::run(sample_rate, &left, &right);
    Ok(ReferenceTrack {
        display_name,
        sample_rate,
        analysis,
    })
}

/// Copy one decoded packet's interleaved samples into the running
/// left/right accumulators. Mono sources are duplicated to both
/// channels; anything ≥2 channels uses channels 0 and 1 as L and R.
fn append_frames(samples: &[f32], channels: usize, left: &mut Vec<f32>, right: &mut Vec<f32>) {
    if channels == 1 {
        for &s in samples {
            left.push(s);
            right.push(s);
        }
    } else {
        for frame in samples.chunks_exact(channels) {
            left.push(frame[0]);
            right.push(frame[1]);
        }
    }
}
