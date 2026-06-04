//! Track sources: producers of `(State, TrackInfo)` updates.
//!
//! A source observes the music player and pushes updates onto a channel that the
//! Discord consumer reads. There are two kinds:
//!
//! - [`notification`]: the live push source, driven by macOS distributed
//!   notifications from Swinsian.
//! - [`applescript`]: a pull source that polls Swinsian via JXA (osakit) at
//!   startup and every 30s, supplying playback position and catching gaps the
//!   notifications miss.
//! - [`workspace`]: watches for Swinsian quitting and reports `Stopped`.
//!
//! All run on the **main thread** — observers and the poll timer hook into the
//! main run loop, and osakit requires the main thread. Only the Discord
//! consumer is backgrounded. Sources share the channel; the `Sender` is cloned.

use crate::track::{State, TrackInfo};
use crossbeam::channel::Sender;

pub mod applescript;
pub mod notification;
pub mod workspace;

/// Channel sink a source pushes track updates into.
pub type TrackTx = Sender<(State, TrackInfo)>;

/// A producer of track updates.
///
/// `start` is invoked on the main thread; implementations register run-loop
/// observers or timers rather than spawning their own threads.
pub trait TrackSource {
    fn start(self, tx: TrackTx);
}
