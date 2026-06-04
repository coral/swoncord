//! Pull source (Phase 2, not yet wired): poll Swinsian via JXA using osakit.
//!
//! This parks the script bindings and response types for the planned pull-based
//! source. Unlike the notification source it can report playback position and
//! duration, which drive the Discord progress bar. osakit must run on the main
//! thread, so a future implementation will poll from a main-run-loop timer and
//! push `(State, TrackInfo)` updates onto the channel like any other source.

#![allow(dead_code)] // Phase 2: wired up in a follow-up pass.

use osakit::declare_script;
use serde::{Deserialize, Serialize};

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

#[derive(Deserialize, Serialize, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackState {
    Playing,
    Paused,
    Stopped,
    Unknown,
}

#[derive(Deserialize, Serialize, Debug, PartialEq)]
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

#[derive(Deserialize, Serialize, Debug, PartialEq)]
pub struct PlayerState {
    pub state: PlaybackState,
    #[serde(default)]
    pub track: Option<Track>,
}
