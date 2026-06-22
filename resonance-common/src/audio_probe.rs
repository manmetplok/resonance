//! Pure, off-thread helpers the media browser uses to render audio-file
//! rows without touching the realtime engine:
//!
//! * [`probe_audio_file`] — read format, channel count, sample rate and
//!   duration from a file on disk;
//! * [`waveform_thumbnail`] — decode + downsample to a compact min/max
//!   peak set sized for a row thumbnail;
//! * [`scan_audio_folder`] — list the audio files in a folder together
//!   with the metadata above.
//!
//! Everything here is synchronous and self-contained so callers run it
//! on a worker thread. It is built on the same `symphonia` reader as
//! [`crate::wav`]; the two share the probe → decode loop conventions but
//! this module never resamples — thumbnails and metadata describe the
//! file as it is on disk.

use std::path::Path;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// Container/codec family of an audio file, derived from its extension
/// and confirmed decodable by the probe. Kept deliberately small — the
/// media browser only needs to label a row and pick an icon, not to
/// reconstruct every `symphonia` codec variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    Wav,
    Flac,
    Mp3,
    Ogg,
    Aac,
    Mp4,
    /// A decodable file whose extension isn't one of the above.
    Other,
}

impl AudioFormat {
    /// Classify by file extension (case-insensitive). Unknown or absent
    /// extensions map to [`AudioFormat::Other`].
    pub fn from_extension(ext: &str) -> AudioFormat {
        match ext.to_ascii_lowercase().as_str() {
            "wav" | "wave" => AudioFormat::Wav,
            "flac" => AudioFormat::Flac,
            "mp3" => AudioFormat::Mp3,
            "ogg" | "oga" => AudioFormat::Ogg,
            "aac" => AudioFormat::Aac,
            "m4a" | "mp4" => AudioFormat::Mp4,
            _ => AudioFormat::Other,
        }
    }

    /// The extensions the media browser scans for. Every entry maps to a
    /// non-[`AudioFormat::Other`] variant via [`AudioFormat::from_extension`].
    pub const SCANNED_EXTENSIONS: &'static [&'static str] =
        &["wav", "wave", "flac", "mp3", "ogg", "oga", "aac", "m4a", "mp4"];
}

/// Format-level metadata for an audio file, cheap enough to gather for a
/// whole folder of browser rows.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioInfo {
    pub format: AudioFormat,
    pub channels: u16,
    pub sample_rate: u32,
    /// Total frames (per-channel sample count). Zero when the container
    /// declares no frame count and the file body is empty.
    pub frames: u64,
    /// Duration in seconds, `frames / sample_rate`.
    pub duration_secs: f64,
}

/// A compact min/max peak set sized for a single browser-row thumbnail.
/// `min[i]` / `max[i]` bound the (channel-summed) waveform over bucket
/// `i`; rendering a vertical line from `min` to `max` per column draws
/// the familiar filled waveform silhouette.
#[derive(Debug, Clone, PartialEq)]
pub struct WaveformThumbnail {
    pub min: Vec<f32>,
    pub max: Vec<f32>,
    pub channels: u16,
    pub sample_rate: u32,
    pub frames: u64,
}

impl WaveformThumbnail {
    /// Number of buckets (thumbnail columns).
    pub fn len(&self) -> usize {
        self.min.len()
    }

    pub fn is_empty(&self) -> bool {
        self.min.is_empty()
    }
}

/// One row in a media-browser folder listing: an audio file and its
/// probed metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioFileEntry {
    /// Absolute path as a string, matching [`crate::scan_directory`].
    pub path: String,
    pub info: AudioInfo,
}

/// Probe `path` for format, channel count, sample rate and duration
/// without decoding the whole file when the container declares its
/// length. Falls back to a full decode only when the frame count is not
/// available up front (e.g. some streamed MP3/Ogg), so the common WAV /
/// FLAC case stays cheap.
pub fn probe_audio_file(path: &Path) -> Result<AudioInfo, String> {
    let format = AudioFormat::from_extension(
        path.extension().and_then(|e| e.to_str()).unwrap_or(""),
    );
    let mut reader = open_reader(path)?;
    let track = reader
        .first_track_known_codec(TrackType::Audio)
        .ok_or_else(|| "no decodable audio track".to_string())?;

    let audio = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| "track missing audio codec parameters".to_string())?;

    let sample_rate = audio
        .sample_rate
        .ok_or_else(|| "missing sample rate".to_string())?;
    let channels = audio
        .channels
        .as_ref()
        .map(|c| c.count())
        .unwrap_or(1)
        .max(1) as u16;

    let track_id = track.id;
    let declared_frames = track.num_frames;

    // Prefer the container's declared frame count; fall back to a full
    // decode-and-count when it's absent.
    let frames = match declared_frames {
        Some(n) if n > 0 => n,
        _ => decode_count_frames(&mut reader, track_id, channels as usize)?,
    };

    let duration_secs = if sample_rate > 0 {
        frames as f64 / sample_rate as f64
    } else {
        0.0
    };

    Ok(AudioInfo {
        format,
        channels,
        sample_rate,
        frames,
        duration_secs,
    })
}

/// Decode `path` and downsample to a `buckets`-wide min/max peak set for
/// a row thumbnail. All channels are summed to mono before bucketing,
/// matching how a single-row waveform silhouette is drawn. `buckets` is
/// clamped to at least 1; a file with fewer frames than buckets still
/// yields exactly `buckets` columns — adjacent buckets then share the
/// same frame, stretching the short waveform across the row.
pub fn waveform_thumbnail(path: &Path, buckets: usize) -> Result<WaveformThumbnail, String> {
    let buckets = buckets.max(1);
    let mut reader = open_reader(path)?;
    let track = reader
        .first_track_known_codec(TrackType::Audio)
        .ok_or_else(|| "no decodable audio track".to_string())?;
    let track_id = track.id;
    let audio = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| "track missing audio codec parameters".to_string())?
        .clone();

    let sample_rate = audio
        .sample_rate
        .ok_or_else(|| "missing sample rate".to_string())?;
    let channels = audio
        .channels
        .as_ref()
        .map(|c| c.count())
        .unwrap_or(1)
        .max(1);

    let mono = decode_to_mono(&mut reader, track_id, &audio)?;
    let frames = mono.len() as u64;

    let (min, max) = bucket_min_max(&mono, buckets);

    Ok(WaveformThumbnail {
        min,
        max,
        channels: channels as u16,
        sample_rate,
        frames,
    })
}

/// Scan `dir` for audio files and return them with probed metadata,
/// sorted by path. Files that fail to probe (corrupt, unsupported, or
/// disappeared mid-scan) are skipped rather than aborting the whole
/// listing — the browser shows the rows it can. An unreadable directory
/// yields an empty list, matching [`crate::scan_directory`].
pub fn scan_audio_folder(dir: &Path) -> Vec<AudioFileEntry> {
    let mut paths = list_audio_paths(dir);
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| {
            let info = probe_audio_file(Path::new(&path)).ok()?;
            Some(AudioFileEntry { path, info })
        })
        .collect()
}

/// List absolute paths of files in `dir` whose extension is one of
/// [`AudioFormat::SCANNED_EXTENSIONS`]. Unsorted; callers sort.
fn list_audio_paths(dir: &Path) -> Vec<String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("scan_audio_folder: {}: {e}", dir.display());
            }
            return Vec::new();
        }
    };
    entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            let ext = path.extension().and_then(|x| x.to_str())?;
            if AudioFormat::SCANNED_EXTENSIONS
                .iter()
                .any(|s| ext.eq_ignore_ascii_case(s))
            {
                Some(path.to_string_lossy().into_owned())
            } else {
                None
            }
        })
        .collect()
}

/// Open a file and hand it to symphonia's probe, returning the format
/// reader positioned at the start of the stream.
fn open_reader(path: &Path) -> Result<Box<dyn FormatReader>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| format!("probe {}: {e}", path.display()))
}

/// Decode every packet of `track_id` and return the total frame count
/// without retaining the samples. Used by [`probe_audio_file`] when the
/// container doesn't declare its length.
fn decode_count_frames(
    reader: &mut Box<dyn FormatReader>,
    track_id: u32,
    channels: usize,
) -> Result<u64, String> {
    let audio = reader
        .tracks()
        .iter()
        .find(|t| t.id == track_id)
        .and_then(|t| t.codec_params.as_ref())
        .and_then(|p| p.audio())
        .ok_or_else(|| "track missing audio codec parameters".to_string())?
        .clone();

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&audio, &AudioDecoderOptions::default())
        .map_err(|e| format!("decoder: {e}"))?;

    let channels = channels.max(1);
    let mut samples = 0u64;
    let mut scratch: Vec<f32> = Vec::new();
    loop {
        let packet = match reader.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymphoniaError::IoError(_)) => break,
            Err(e) => return Err(format!("read packet: {e}")),
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(SymphoniaError::IoError(_)) => break,
            Err(e) => return Err(format!("decode: {e}")),
        };
        decoded.copy_to_vec_interleaved(&mut scratch);
        samples += scratch.len() as u64;
    }
    Ok(samples / channels as u64)
}

/// Decode every packet of `track_id`, summing channels to a single mono
/// stream (the channel average) suitable for a one-row thumbnail.
fn decode_to_mono(
    reader: &mut Box<dyn FormatReader>,
    track_id: u32,
    audio: &symphonia::core::codecs::audio::AudioCodecParameters,
) -> Result<Vec<f32>, String> {
    let channels = audio
        .channels
        .as_ref()
        .map(|c| c.count())
        .unwrap_or(1)
        .max(1);

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(audio, &AudioDecoderOptions::default())
        .map_err(|e| format!("decoder: {e}"))?;

    let mut mono: Vec<f32> = Vec::new();
    let mut scratch: Vec<f32> = Vec::new();
    loop {
        let packet = match reader.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymphoniaError::IoError(_)) => break,
            Err(e) => return Err(format!("read packet: {e}")),
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(SymphoniaError::IoError(_)) => break,
            Err(e) => return Err(format!("decode: {e}")),
        };
        decoded.copy_to_vec_interleaved(&mut scratch);
        let frames = scratch.len() / channels;
        mono.reserve(frames);
        for frame in 0..frames {
            let mut sum = 0.0f32;
            for ch in 0..channels {
                sum += scratch[frame * channels + ch];
            }
            mono.push(sum / channels as f32);
        }
    }
    Ok(mono)
}

/// Bucket a mono signal into `buckets` min/max pairs. Each bucket spans a
/// contiguous, near-equal slice of the input; an empty input yields
/// all-zero buckets so callers always get a fixed-width thumbnail.
fn bucket_min_max(mono: &[f32], buckets: usize) -> (Vec<f32>, Vec<f32>) {
    let buckets = buckets.max(1);
    let mut min = vec![0.0f32; buckets];
    let mut max = vec![0.0f32; buckets];
    if mono.is_empty() {
        return (min, max);
    }
    let n = mono.len();
    for (b, (mn, mx)) in min.iter_mut().zip(max.iter_mut()).enumerate() {
        // Span [start, end) of this bucket, distributing the remainder so
        // every sample lands in exactly one bucket.
        let start = b * n / buckets;
        let end = ((b + 1) * n / buckets).max(start + 1).min(n);
        let mut lo = mono[start];
        let mut hi = mono[start];
        for &s in &mono[start..end] {
            if s < lo {
                lo = s;
            }
            if s > hi {
                hi = s;
            }
        }
        *mn = lo;
        *mx = hi;
    }
    (min, max)
}
