//! Domain types shared between track sources and the Discord consumer.

use std::time::{SystemTime, UNIX_EPOCH};

/// A source observation, timestamped before it enters a worker's mailbox.
#[derive(Debug, Clone)]
pub struct TrackUpdate {
    pub state: State,
    pub track: TrackInfo,
    pub observed_at: i64,
}

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Distributed-notification name Swinsian posts when playback starts.
pub const SWINSIAN_TRACK_PLAYING: &str = "com.swinsian.Swinsian-Track-Playing";
/// Distributed-notification name Swinsian posts when playback stops.
pub const SWINSIAN_TRACK_STOPPED: &str = "com.swinsian.Swinsian-Track-Stopped";
/// Distributed-notification name Swinsian posts when playback pauses.
pub const SWINSIAN_TRACK_PAUSED: &str = "com.swinsian.Swinsian-Track-Paused";

/// Metadata for the currently-playing track.
///
/// `position`/`duration` are populated when available from either notifications
/// or the AppleScript poll. They drive the Discord progress bar when present.
#[derive(Debug, Default, PartialEq, Clone)]
pub struct TrackInfo {
    pub artist: String,
    pub album: String,
    pub title: String,
    pub file_type: Option<String>,
    pub position: Option<f64>,
    pub duration: Option<f64>,
}

/// Playback state derived from a Swinsian notification name.
#[derive(Debug, PartialEq, Clone, Copy, Default)]
pub enum State {
    Playing,
    Stopped,
    Paused,
    #[default]
    Unknown,
}

impl From<&str> for State {
    fn from(notification_name: &str) -> Self {
        match notification_name {
            SWINSIAN_TRACK_PLAYING => State::Playing,
            SWINSIAN_TRACK_STOPPED => State::Stopped,
            SWINSIAN_TRACK_PAUSED => State::Paused,
            _ => State::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_from_known_notification_names() {
        assert_eq!(State::from(SWINSIAN_TRACK_PLAYING), State::Playing);
        assert_eq!(State::from(SWINSIAN_TRACK_STOPPED), State::Stopped);
        assert_eq!(State::from(SWINSIAN_TRACK_PAUSED), State::Paused);
    }

    #[test]
    fn state_from_unknown_name_is_unknown() {
        assert_eq!(State::from("com.example.Something-Else"), State::Unknown);
        assert_eq!(State::from(""), State::Unknown);
    }
}
