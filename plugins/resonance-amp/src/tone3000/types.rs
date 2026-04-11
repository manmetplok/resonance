//! Serde types for the Tone3000 REST responses we consume.
//!
//! All fields are modelled defensively: anything the API might omit or
//! rename without warning is wrapped in `Option` so a schema drift
//! shows up as a missing UI field, not a crash.

use serde::{Deserialize, Serialize};

/// Paginated wrapper used by `/tones/search` and `/models`. Only the
/// fields the UI reads are modelled; everything else is ignored.
#[derive(Debug, Clone, Deserialize)]
pub struct PaginatedResponse<T> {
    #[serde(default = "Vec::new")]
    pub data: Vec<T>,
    #[serde(default)]
    pub page: Option<u32>,
    #[serde(default)]
    pub page_size: Option<u32>,
    #[serde(default)]
    pub total: Option<u32>,
}

/// A Tone3000 "tone" entry returned from `/tones/search`. A tone is a
/// user-submitted amp/rig snapshot; one tone may contain several
/// [`Model`]s at different sizes.
#[derive(Debug, Clone, Deserialize)]
pub struct Tone {
    pub id: i64,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub gear: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub models_count: Option<u32>,
    #[serde(default)]
    pub downloads_count: Option<u32>,
    #[serde(default)]
    pub favorites_count: Option<u32>,
    /// Embedded author record. The API nests the username under `user`
    /// rather than exposing it at the top level — decoding it as just
    /// `username` produces "unknown" for every row.
    #[serde(default)]
    pub user: Option<EmbeddedUser>,
}

/// Minimal author fields returned alongside each tone.
#[derive(Debug, Clone, Deserialize)]
pub struct EmbeddedUser {
    #[serde(default)]
    pub username: Option<String>,
}

impl Tone {
    pub fn display_title(&self) -> &str {
        self.title.as_deref().unwrap_or("(untitled)")
    }

    pub fn display_author(&self) -> &str {
        self.user
            .as_ref()
            .and_then(|u| u.username.as_deref())
            .unwrap_or("unknown")
    }
}

/// A single downloadable model attached to a tone.
#[derive(Debug, Clone, Deserialize)]
pub struct Model {
    pub id: i64,
    pub tone_id: i64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub size: Option<String>,
    /// Pre-signed URL we `GET` with Bearer auth to pull the raw .nam
    /// bytes. Only valid while the user is authenticated; the API
    /// rotates it, so we never cache it.
    #[serde(default)]
    pub model_url: Option<String>,
}

impl Model {
    pub fn display_label(&self) -> String {
        match (&self.name, &self.size) {
            (Some(n), Some(s)) => format!("{n} ({s})"),
            (Some(n), None) => n.clone(),
            (None, Some(s)) => format!("model {} ({s})", self.id),
            (None, None) => format!("model {}", self.id),
        }
    }
}

/// Response body of `POST /oauth/token`. Kept in sync with what we
/// persist to disk in [`StoredTokens`].
#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
    #[serde(default)]
    pub scope: Option<String>,
}

/// What we write to `~/.config/resonance/tone3000.json`. Contains the
/// raw tokens plus a unix-timestamp acquisition time so we can decide
/// when to refresh without round-tripping the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredTokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
    /// Unix seconds when [`access_token`] was issued.
    pub acquired_at: i64,
}

impl StoredTokens {
    pub fn from_response(resp: TokenResponse) -> Self {
        let acquired_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Self {
            access_token: resp.access_token,
            refresh_token: resp.refresh_token,
            expires_in: resp.expires_in,
            acquired_at,
        }
    }

    /// Rough "is the access token still usable" check, with a 60-second
    /// safety margin so we refresh before the server would reject us.
    pub fn is_expired(&self) -> bool {
        let Some(ttl) = self.expires_in else {
            return false;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        now >= self.acquired_at + ttl - 60
    }
}
