//! Track sources: producers of `(State, TrackInfo)` updates.
//!
//! A source observes the music player and pushes updates onto a channel that the
//! Discord consumer reads. There are two kinds:
//!
//! - [`notification`]: the live push source, driven by macOS distributed
//!   notifications from Swinsian.
//! - [`applescript`]: a planned pull source that polls Swinsian via JXA (osakit).
//!
//! Both run on the **main thread** — observers and the (future) poll timer hook
//! into the main run loop, and osakit requires the main thread. Only the Discord
//! consumer is backgrounded.

use crate::track::{State, TrackInfo};
use crossbeam::channel::Sender;

pub mod applescript;
pub mod notification;

/// Channel sink a source pushes track updates into.
pub type TrackTx = Sender<(State, TrackInfo)>;

/// A producer of track updates.
///
/// `start` is invoked on the main thread; implementations register run-loop
/// observers or timers rather than spawning their own threads.
pub trait TrackSource {
    fn start(self, tx: TrackTx);
}
