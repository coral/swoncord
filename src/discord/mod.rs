//! Discord rich-presence client and the consumer that drives it.

mod albumart;

use crate::consts;
use crate::error::Error;
use crate::track::{State, TrackInfo};
use albumart::AlbumArtRequester;
use crossbeam::channel::Receiver;
use discord_rich_presence::{DiscordIpc, DiscordIpcClient, activity};
use log::error;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

enum PresenceState {
    Active,
    Cleared,
}

pub struct Discord {
    client: DiscordIpcClient,
    last_updated: Instant,
    state: PresenceState,
}

impl Discord {
    /// Connects to Discord and spawns the background consumer that reads track
    /// updates from `rx`. Returns the shared client so the caller keeps it alive.
    pub fn new(rx: Receiver<(State, TrackInfo)>) -> Result<Arc<Mutex<Self>>, Error> {
        let mut client = DiscordIpcClient::new(consts::DISCORD_APP_ID);
        client.connect()?;

        let shared = Arc::new(Mutex::new(Self {
            client,
            last_updated: Instant::now(),
            state: PresenceState::Cleared,
        }));

        let consumer = shared.clone();
        std::thread::spawn(move || Self::pump(rx, consumer));

        Ok(shared)
    }

    /// Background loop: receives track updates from all sources, refreshes album
    /// art on album change, tracks playback progress, and reflects state into
    /// Discord.
    ///
    /// Updates are deduplicated by track identity so a position-less notification
    /// for the current track doesn't wipe the `position` learned from a poll. The
    /// `anchor` (start/end unix seconds) is (re)computed only when a message
    /// carries position+duration, and reset when the track changes.
    fn pump(rx: Receiver<(State, TrackInfo)>, shared: Arc<Mutex<Self>>) {
        let album_art = AlbumArtRequester::new();

        let mut current: Option<TrackInfo> = None;
        let mut last_state = State::Unknown;
        let mut cover: Option<String> = None;
        let mut anchor: Option<Anchor> = None;

        loop {
            if let Ok((state, track)) = rx.recv_timeout(consts::RECV_TIMEOUT) {
                last_state = state;
                match state {
                    State::Stopped | State::Unknown => {
                        current = None;
                        anchor = None;
                    }
                    State::Playing | State::Paused => {
                        if !same_track(current.as_ref(), &track) {
                            if album_differs(current.as_ref(), &track) {
                                cover = album_art.get_album_art(&track).ok();
                            }
                            anchor = None;
                            current = Some(track.clone());
                        }
                        // Both notifications (currentTime) and the poll
                        // (position + duration) carry a position; (re)anchor
                        // whenever one is present so progress survives a track
                        // switch. Without a duration the bar is open-ended
                        // (elapsed only) until the next poll bounds it.
                        if let Some(pos) = track.position {
                            anchor = Some(progress_anchor(pos, track.duration, unix_now()));
                        }
                    }
                }
            }

            let mut discord = shared.lock().expect("discord mutex poisoned");
            match last_state {
                State::Playing => {
                    if let Some(track) = &current
                        && let Err(e) = discord.update(track, cover.clone(), anchor)
                    {
                        error!("Error updating Discord status: {e:?}");
                    }
                }
                State::Stopped | State::Paused | State::Unknown => {
                    if let Err(e) = discord.clear() {
                        error!("Error clearing Discord status: {e:?}");
                    }
                }
            }
        }
    }

    fn update(
        &mut self,
        t: &TrackInfo,
        cover: Option<String>,
        anchor: Option<Anchor>,
    ) -> Result<(), Error> {
        let state: String = format!("{} ", t.artist).chars().take(consts::FIELD_MAX).collect();
        let details: String = t.title.chars().take(consts::FIELD_MAX).collect();
        let large_text: String = t.album.chars().take(consts::FIELD_MAX).collect();

        let uri = cover.unwrap_or_else(|| "sw2".to_string());

        let assets = activity::Assets::new()
            .large_text(large_text.as_str())
            .large_image(&uri)
            .small_text("Listening");

        let mut payload = activity::Activity::new()
            .state(&state)
            .details(&details)
            .activity_type(activity::ActivityType::Listening)
            .assets(assets);

        // Progress bar: Discord ticks from `start` on its own, so we set absolute
        // timestamps (recomputed from a fresh position by the pump). With an
        // `end` it renders a bounded bar; without one, an open-ended elapsed
        // counter (the case right after a track switch, before the next poll).
        if let Some(anchor) = anchor {
            let mut timestamps = activity::Timestamps::new().start(anchor.start);
            if let Some(end) = anchor.end {
                timestamps = timestamps.end(end);
            }
            payload = payload.timestamps(timestamps);
        }

        if Instant::now().duration_since(self.last_updated) >= consts::UPDATE_THROTTLE {
            match self.client.set_activity(payload) {
                Ok(_) => self.last_updated = Instant::now(),
                Err(e) => {
                    error!("Error setting Discord status: {e:?}");
                    if let Err(e) = self.client.reconnect() {
                        error!("Discord reconnect failed: {e:?}");
                    }
                }
            }
        }

        self.state = PresenceState::Active;
        Ok(())
    }

    fn clear(&mut self) -> Result<(), Error> {
        let throttled =
            Instant::now().duration_since(self.last_updated) < consts::UPDATE_THROTTLE;
        if matches!(self.state, PresenceState::Active) && !throttled {
            if self.client.clear_activity().is_err() {
                if let Err(e) = self.client.reconnect() {
                    error!("Discord reconnect failed: {e:?}");
                }
            } else {
                self.last_updated = Instant::now();
                self.state = PresenceState::Cleared;
            }
        }
        Ok(())
    }
}

/// Whether `incoming` is the same track as `current` (ignoring position, so a
/// poll-vs-notification difference doesn't count as a new track).
fn same_track(current: Option<&TrackInfo>, incoming: &TrackInfo) -> bool {
    match current {
        Some(c) => {
            c.artist == incoming.artist && c.album == incoming.album && c.title == incoming.title
        }
        None => false,
    }
}

/// Whether `incoming`'s album differs from `current`'s (i.e. cover art must be
/// refetched). A `None` current counts as a change.
fn album_differs(current: Option<&TrackInfo>, incoming: &TrackInfo) -> bool {
    current.is_none_or(|c| c.album != incoming.album)
}

/// Absolute timestamps for the Discord progress bar. `end` is unknown until a
/// poll reports the track duration, so it's optional (open-ended elapsed bar).
#[derive(Clone, Copy)]
struct Anchor {
    start: i64,
    end: Option<i64>,
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Anchor for the Discord progress bar, derived from the current playback `pos`
/// and (when known) track `dur`, both in seconds.
fn progress_anchor(pos: f64, dur: Option<f64>, now: i64) -> Anchor {
    let start = now - pos as i64;
    Anchor {
        start,
        end: dur.map(|d| start + d as i64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(album: &str) -> TrackInfo {
        TrackInfo {
            album: album.to_string(),
            ..Default::default()
        }
    }

    fn full_track(artist: &str, album: &str, title: &str) -> TrackInfo {
        TrackInfo {
            artist: artist.to_string(),
            album: album.to_string(),
            title: title.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn album_differs_on_first_track() {
        assert!(album_differs(None, &track("A")));
    }

    #[test]
    fn album_differs_when_album_changes() {
        assert!(album_differs(Some(&track("A")), &track("B")));
    }

    #[test]
    fn album_same_when_album_unchanged() {
        assert!(!album_differs(Some(&track("A")), &track("A")));
    }

    #[test]
    fn same_track_matches_identity_ignoring_position() {
        let mut a = full_track("Artist", "Album", "Song");
        let mut b = full_track("Artist", "Album", "Song");
        a.position = None;
        b.position = Some(42.0); // a poll-vs-notification difference
        assert!(same_track(Some(&a), &b));
    }

    #[test]
    fn same_track_distinguishes_different_song() {
        let a = full_track("Artist", "Album", "Song One");
        let b = full_track("Artist", "Album", "Song Two");
        assert!(!same_track(Some(&a), &b));
        assert!(!same_track(None, &b));
    }

    #[test]
    fn progress_anchor_with_duration_bounds_end() {
        let a = progress_anchor(30.0, Some(200.0), 1000);
        assert_eq!(a.start, 970);
        assert_eq!(a.end, Some(1170));
    }

    #[test]
    fn progress_anchor_without_duration_is_open_ended() {
        let a = progress_anchor(30.0, None, 1000);
        assert_eq!(a.start, 970);
        assert_eq!(a.end, None);
    }
}
