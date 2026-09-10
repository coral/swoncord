//! Track sources: producers of timestamped playback observations.
//!
//! A source observes the music player and pushes updates onto a channel that the
//! Discord consumer reads. There are three kinds:
//!
//! - [`notification`]: the live push source, driven by macOS distributed
//!   notifications from Swinsian.
//! - [`applescript`]: a pull source that polls Swinsian via JXA (osakit) at
//!   startup and every 30s, supplying playback position and catching gaps the
//!   notifications miss.
//! - [`workspace`]: watches for Swinsian quitting and reports `Stopped`.
//!
//! All run on the **main thread** — observers and the poll timer hook into the
//! main run loop, and osakit requires the main thread. Discord and artwork have
//! background workers. Sources share a mailbox that retains the newest update.

use crate::latest::Sender;
use crate::track::TrackUpdate;

pub mod applescript;
pub mod notification;
pub mod workspace;

/// Channel sink a source pushes track updates into.
pub type TrackTx = Sender<TrackUpdate>;

/// A producer of track updates.
///
/// `start` is invoked on the main thread; implementations register run-loop
/// observers or timers rather than spawning their own threads.
pub trait TrackSource {
    fn start(self, tx: TrackTx);
}
