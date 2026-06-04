//! Pull source: poll Swinsian via JXA (osakit) to supplement notifications.
//!
//! Notifications only fire while both Swinsian and swoncord are running, carry no
//! playback position, and never signal a quit. This source polls once at startup
//! and every [`POLL_INTERVAL_SECS`] to close those gaps: it picks up a track
//! already playing at launch, reports `position`/`duration` to drive the progress
//! bar, and reports "stopped" when Swinsian isn't running (a backstop to the
//! [`super::workspace`] quit observer).
//!
//! osakit must run on the main thread, so the poll is driven by an `NSTimer`
//! scheduled on the main run loop rather than a background thread.

use super::{TrackSource, TrackTx};
use crate::track::{State, TrackInfo};
use block2::RcBlock;
use log::error;
use objc2_foundation::NSTimer;
use osakit::declare_script;
use serde::Deserialize;
use std::ptr::NonNull;

/// How often to poll Swinsian, in seconds (`NSTimer` takes an `NSTimeInterval`).
/// Supplements notifications: catches cold starts, corrects progress, and detects
/// a quit as a backstop.
const POLL_INTERVAL_SECS: f64 = 30.0;

declare_script! {
    #[language(JavaScript)]
    #[source(r#"
    function isRunning(appName) {
        const systemEvents = Application("System Events");
        return systemEvents.processes.name().includes(appName);
    }

    function get() {
        if (!isRunning("Swinsian")) {
            return { state: "stopped" };
        }

        const swinsian = Application("Swinsian");
        const state = swinsian.playerState();

        if (state === "stopped") {
            return { state: "stopped" };
        }

        const track = swinsian.currentTrack();

        return {
            state: state,
            track: {
                format: track.kind(),
                song: track.name(),
                artist: track.artist(),
                album: track.album(),
                pos: swinsian.playerPosition(),
                dur: track.duration()
            }
        };
    }
"#)]
    pub SwinsianState {
        pub fn get() -> PlayerState;
    }
}

#[derive(Deserialize, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackState {
    Playing,
    Paused,
    Stopped,
    Unknown,
}

#[derive(Deserialize, Debug, PartialEq)]
pub struct Track {
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub song: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub pos: f64,
    #[serde(default)]
    pub dur: f64,
}

#[derive(Deserialize, Debug, PartialEq)]
pub struct PlayerState {
    pub state: PlaybackState,
    #[serde(default)]
    pub track: Option<Track>,
}

impl From<PlaybackState> for State {
    fn from(p: PlaybackState) -> Self {
        match p {
            PlaybackState::Playing => State::Playing,
            PlaybackState::Paused => State::Paused,
            PlaybackState::Stopped => State::Stopped,
            PlaybackState::Unknown => State::Unknown,
        }
    }
}

impl PlayerState {
    /// Maps a poll result into the channel's `(State, TrackInfo)` shape. A
    /// missing track (stopped / not running) yields a default `TrackInfo`.
    fn into_update(self) -> (State, TrackInfo) {
        let state = State::from(self.state);
        let track = self
            .track
            .map(|t| TrackInfo {
                artist: t.artist,
                album: t.album,
                title: t.song,
                file_type: Some(t.format),
                position: Some(t.pos),
                duration: Some(t.dur),
            })
            .unwrap_or_default();
        (state, track)
    }
}

/// Polls Swinsian via JXA at startup and on a repeating main-run-loop timer.
pub struct AppleScriptSource;

impl TrackSource for AppleScriptSource {
    fn start(self, tx: TrackTx) {
        let script = match SwinsianState::new() {
            Ok(script) => script,
            Err(e) => {
                // Notifications still work without us; degrade gracefully.
                error!("failed to compile Swinsian poll script: {e}");
                return;
            }
        };

        // Immediate poll: pick up a track already playing at launch.
        poll_once(&script, &tx);

        let block = RcBlock::new(move |_timer: NonNull<NSTimer>| {
            poll_once(&script, &tx);
        });
        // Safe: a scheduled timer fires only on the run loop it's added to (the
        // main run loop here), so the block — and the non-Send osakit `Script`
        // it captures — only ever runs on the main thread, which is exactly
        // where osakit requires it. The timer retains the block.
        unsafe {
            let _timer = NSTimer::scheduledTimerWithTimeInterval_repeats_block(
                POLL_INTERVAL_SECS,
                true,
                &block,
            );
        }
    }
}

/// Runs one poll and forwards the result, logging (not panicking) on failure.
fn poll_once(script: &SwinsianState, tx: &TrackTx) {
    match script.get() {
        Ok(player_state) => {
            if let Err(e) = tx.send(player_state.into_update()) {
                error!("failed to forward poll update: {e}");
            }
        }
        Err(e) => error!("Swinsian poll failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_playing_track_from_jxa_json() {
        let json = r#"{
            "state": "playing",
            "track": {
                "format": "AAC", "song": "Song", "artist": "Artist",
                "album": "Album", "pos": 30.0, "dur": 200.0
            }
        }"#;
        let ps: PlayerState = serde_json::from_str(json).unwrap();
        let (state, track) = ps.into_update();

        assert_eq!(state, State::Playing);
        assert_eq!(track.title, "Song");
        assert_eq!(track.artist, "Artist");
        assert_eq!(track.album, "Album");
        assert_eq!(track.file_type.as_deref(), Some("AAC"));
        assert_eq!(track.position, Some(30.0));
        assert_eq!(track.duration, Some(200.0));
    }

    #[test]
    fn converts_stopped_to_default_track() {
        let ps: PlayerState = serde_json::from_str(r#"{"state":"stopped"}"#).unwrap();
        let (state, track) = ps.into_update();

        assert_eq!(state, State::Stopped);
        assert_eq!(track, TrackInfo::default());
    }
}
