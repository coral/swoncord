//! Centralized configuration constants.

use std::time::Duration;

/// Discord application ID this presence runs under.
pub const DISCORD_APP_ID: &str = "1076384656850698240";

/// User-agent reported to the MusicBrainz API (their guidelines require one).
pub const MUSICBRAINZ_USER_AGENT: &str =
    "SwinsianRichPresence/1.0.0 ( https://jonasbengtson.se )";

/// Minimum time between Discord activity writes, to stay under its rate limit.
pub const UPDATE_THROTTLE: Duration = Duration::from_secs(4);

/// How long the consumer waits for a track update before re-evaluating state.
pub const RECV_TIMEOUT: Duration = Duration::from_secs(5);

/// Network timeout for MusicBrainz / CoverArtArchive requests, so a slow
/// upstream can't stall the presence thread.
pub const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// Max length of a Discord rich-presence text field (state/details/large_text).
pub const FIELD_MAX: usize = 128;

/// Menu-bar status icon size, in points.
pub const ICON_SIZE: f64 = 18.0;

/// Distributed-notification name Swinsian posts when playback starts.
pub const SWINSIAN_TRACK_PLAYING: &str = "com.swinsian.Swinsian-Track-Playing";
/// Distributed-notification name Swinsian posts when playback stops.
pub const SWINSIAN_TRACK_STOPPED: &str = "com.swinsian.Swinsian-Track-Stopped";
/// Distributed-notification name Swinsian posts when playback pauses.
pub const SWINSIAN_TRACK_PAUSED: &str = "com.swinsian.Swinsian-Track-Paused";

/// All Swinsian notification names, used when registering observers.
pub const SWINSIAN_NOTIFICATIONS: [&str; 3] = [
    SWINSIAN_TRACK_PLAYING,
    SWINSIAN_TRACK_STOPPED,
    SWINSIAN_TRACK_PAUSED,
];

/// Strips trailing parenthetical/bracketed qualifiers from an album name, e.g.
/// "Album (Deluxe Edition)" or "Album [Remaster] (Bonus)" -> "Album". Used as a
/// last-ditch broadening of the MusicBrainz search.
pub const ALBUM_CLEAN_PATTERN: &str =
    r"\s+[\(\[][^\(\)\[\]]*[\)\]](\s+[\(\[][^\(\)\[\]]*[\)\]])*$";
