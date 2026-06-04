//! Domain types shared between track sources and the Discord consumer.

use crate::consts;

/// Metadata for the currently-playing track.
///
/// `position`/`duration` are populated only by sources that can report playback
/// progress (the AppleScript pull source); the notification source leaves them
/// `None`. They drive the Discord progress bar when present.
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
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum State {
    Playing,
    Stopped,
    Paused,
    Unknown,
}

impl From<&str> for State {
    fn from(notification_name: &str) -> Self {
        match notification_name {
            consts::SWINSIAN_TRACK_PLAYING => State::Playing,
            consts::SWINSIAN_TRACK_STOPPED => State::Stopped,
            consts::SWINSIAN_TRACK_PAUSED => State::Paused,
            _ => State::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_from_known_notification_names() {
        assert_eq!(State::from(consts::SWINSIAN_TRACK_PLAYING), State::Playing);
        assert_eq!(State::from(consts::SWINSIAN_TRACK_STOPPED), State::Stopped);
        assert_eq!(State::from(consts::SWINSIAN_TRACK_PAUSED), State::Paused);
    }

    #[test]
    fn state_from_unknown_name_is_unknown() {
        assert_eq!(State::from("com.example.Something-Else"), State::Unknown);
        assert_eq!(State::from(""), State::Unknown);
    }
}
