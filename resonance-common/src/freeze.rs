//! Freeze data model for track freezing.
//!
//! This module provides the shared data types for track freeze functionality,
//! used by both the engine and the app.

use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// The status of a freeze cache reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FreezeCacheStatus {
    /// The freeze cache is valid and up-to-date.
    Frozen,
    /// The freeze cache exists but the source has changed (stale).
    Stale,
    /// The freeze operation failed or the cache is missing.
    Failed,
}

/// A reference to a freeze cache file.
///
/// Contains metadata about a frozen track's cached audio file,
/// including a fingerprint of the inputs that were frozen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreezeCacheRef {
    /// The filename of the cache WAV file (relative to project's freeze cache dir).
    pub cache_filename: String,
    /// The sample rate of the frozen audio.
    pub sample_rate: u32,
    /// The bit depth of the frozen audio.
    pub bit_depth: u16,
    /// A fingerprint hash of the frozen inputs (notes, lyrics, plugin params, instrument selection).
    pub render_fingerprint: u64,
    /// The current status of this cache reference.
    pub status: FreezeCacheStatus,
}

impl FreezeCacheRef {
    /// Creates a new freeze cache reference.
    pub fn new(
        cache_filename: String,
        sample_rate: u32,
        bit_depth: u16,
        render_fingerprint: u64,
        status: FreezeCacheStatus,
    ) -> Self {
        Self {
            cache_filename,
            sample_rate,
            bit_depth,
            render_fingerprint,
            status,
        }
    }

    /// Returns true if the cache is valid and can be used for playback.
    pub fn is_valid(&self) -> bool {
        self.status == FreezeCacheStatus::Frozen
    }

    /// Returns true if the cache is stale and needs re-freezing.
    pub fn is_stale(&self) -> bool {
        self.status == FreezeCacheStatus::Stale
    }

    /// Returns true if the cache failed or is missing.
    pub fn is_failed(&self) -> bool {
        self.status == FreezeCacheStatus::Failed
    }
}

/// The freeze state for a single track.
///
/// Wraps the per-track freeze status along with an optional reference
/// to the freeze cache.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackFreezeState {
    /// Whether the track is currently frozen (has an active cache).
    pub is_frozen: bool,
    /// The reference to the freeze cache, if one exists.
    pub cache_ref: Option<FreezeCacheRef>,
}

impl TrackFreezeState {
    /// Creates a new track freeze state.
    pub fn new(cache_ref: Option<FreezeCacheRef>) -> Self {
        Self {
            is_frozen: cache_ref.is_some(),
            cache_ref,
        }
    }

    /// Creates a new unfrozen track state.
    pub fn unfrozen() -> Self {
        Self {
            is_frozen: false,
            cache_ref: None,
        }
    }

    /// Creates a new frozen track state with the given cache reference.
    pub fn frozen(cache_ref: FreezeCacheRef) -> Self {
        Self {
            is_frozen: true,
            cache_ref: Some(cache_ref),
        }
    }

    /// Returns the cache reference if the track is frozen.
    pub fn as_ref(&self) -> Option<&FreezeCacheRef> {
        self.cache_ref.as_ref()
    }

    /// Returns true if the track is frozen with a valid cache.
    pub fn is_validly_frozen(&self) -> bool {
        self.is_frozen
            && self
                .cache_ref
                .as_ref()
                .is_some_and(|ref_| ref_.is_valid())
    }

    /// Returns true if the track is frozen but the cache is stale.
    pub fn is_stale(&self) -> bool {
        self.is_frozen
            && self
                .cache_ref
                .as_ref()
                .is_some_and(|ref_| ref_.is_stale())
    }
}

impl Default for TrackFreezeState {
    fn default() -> Self {
        Self::unfrozen()
    }
}

/// Inputs that are used to compute the freeze fingerprint.
///
/// This struct captures all the data that affects the freeze render output.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FreezeFingerprintInputs {
    /// The notes data as a byte representation (serialized).
    pub notes: Vec<u8>,
    /// The lyrics data as a byte representation (serialized).
    pub lyrics: Vec<u8>,
    /// The plugin parameters as a byte representation (serialized).
    pub plugin_params: Vec<u8>,
    /// The instrument identifier.
    pub instrument_id: String,
}

/// Computes a stable fingerprint hash over the freeze inputs.
///
/// This fingerprint is used to detect when a frozen track's inputs have changed,
/// requiring a re-freeze. The same inputs will always produce the same fingerprint,
/// and different inputs will (with extremely high probability) produce different fingerprints.
pub fn compute_fingerprint(inputs: &FreezeFingerprintInputs) -> u64 {
    let mut hasher = DefaultHasher::new();
    inputs.hash(&mut hasher);
    hasher.finish()
}

/// Builder for creating freeze fingerprint inputs.
///
/// Provides a convenient API for constructing the inputs incrementally.
#[derive(Debug, Default)]
pub struct FreezeFingerprintBuilder {
    notes: Vec<u8>,
    lyrics: Vec<u8>,
    plugin_params: Vec<u8>,
    instrument_id: String,
}

impl FreezeFingerprintBuilder {
    /// Creates a new builder with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the notes data.
    pub fn with_notes(mut self, notes: Vec<u8>) -> Self {
        self.notes = notes;
        self
    }

    /// Sets the lyrics data.
    pub fn with_lyrics(mut self, lyrics: Vec<u8>) -> Self {
        self.lyrics = lyrics;
        self
    }

    /// Sets the plugin parameters data.
    pub fn with_plugin_params(mut self, plugin_params: Vec<u8>) -> Self {
        self.plugin_params = plugin_params;
        self
    }

    /// Sets the instrument identifier.
    pub fn with_instrument_id(mut self, instrument_id: impl Into<String>) -> Self {
        self.instrument_id = instrument_id.into();
        self
    }

    /// Builds the final fingerprint inputs.
    pub fn build(self) -> FreezeFingerprintInputs {
        FreezeFingerprintInputs {
            notes: self.notes,
            lyrics: self.lyrics,
            plugin_params: self.plugin_params,
            instrument_id: self.instrument_id,
        }
    }
}
