//! Discord rich-presence client and the consumer that drives it.

mod albumart;

use crate::error::Error;
use crate::track::{State, TrackInfo};
use albumart::AlbumArtRequester;
use crossbeam::channel::Receiver;
use discord_rich_presence::{DiscordIpc, DiscordIpcClient, activity};
use log::error;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Discord application ID this presence runs under.
const DISCORD_APP_ID: &str = "1076384656850698240";

/// Minimum time between Discord activity writes, to stay under its rate limit.
const UPDATE_THROTTLE: Duration = Duration::from_secs(4);

/// How long the consumer waits for an event before re-evaluating state.
const RECV_TIMEOUT: Duration = Duration::from_secs(5);

/// Max length of a Discord rich-presence text field (state/details/large_text).
const FIELD_MAX: usize = 128;

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
    pub fn new(
        rx: Receiver<(State, TrackInfo)>,
        enabled: Arc<AtomicBool>,
        wake: Receiver<()>,
    ) -> Result<Arc<Mutex<Self>>, Error> {
        let mut client = DiscordIpcClient::new(DISCORD_APP_ID);
        client.connect()?;

        let shared = Arc::new(Mutex::new(Self {
            client,
            last_updated: Instant::now(),
            state: PresenceState::Cleared,
        }));

        let consumer = shared.clone();
        std::thread::spawn(move || Self::pump(rx, consumer, enabled, wake));

        Ok(shared)
    }

    /// Background loop: waits for a source update, a menu toggle (`wake`), or a
    /// timeout, then reflects the current [`Playback`] into Discord.
    fn pump(
        rx: Receiver<(State, TrackInfo)>,
        shared: Arc<Mutex<Self>>,
        enabled: Arc<AtomicBool>,
        wake: Receiver<()>,
    ) {
        let album_art = AlbumArtRequester::new();
        let mut playback = Playback::default();

        loop {
            // A menu toggle forces an immediate (un-throttled) re-dispatch;
            // source updates and the timeout use the normal cadence.
            let force = crossbeam::channel::select! {
                recv(rx) -> msg => {
                    if let Ok((state, track)) = msg {
                        playback.apply(state, track, &album_art);
                    }
                    false
                }
                recv(wake) -> _ => true,
                default(RECV_TIMEOUT) => false,
            };

            let mut discord = shared.lock().expect("discord mutex poisoned");
            playback.publish(&mut discord, enabled.load(Ordering::Relaxed), force);
        }
    }

    fn update(
        &mut self,
        t: &TrackInfo,
        cover: Option<String>,
        anchor: Option<Anchor>,
        force: bool,
    ) -> Result<(), Error> {
        let state: String = format!("{} ", t.artist).chars().take(FIELD_MAX).collect();
        let details: String = t.title.chars().take(FIELD_MAX).collect();
        let large_text: String = t.album.chars().take(FIELD_MAX).collect();

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

        if force || Instant::now().duration_since(self.last_updated) >= UPDATE_THROTTLE {
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

    fn clear(&mut self, force: bool) -> Result<(), Error> {
        let throttled =
            Instant::now().duration_since(self.last_updated) < UPDATE_THROTTLE;
        if matches!(self.state, PresenceState::Active) && (force || !throttled) {
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

/// The playback state the pump tracks across events, and how it's mirrored into
/// Discord.
///
/// Updates are deduplicated by track identity, so a position-less notification
/// for the current track doesn't wipe the `position` learned from a poll. The
/// `anchor` is (re)computed only when a message carries a position, and reset
/// when the track changes.
#[derive(Default)]
struct Playback {
    current: Option<TrackInfo>,
    last_state: State,
    cover: Option<String>,
    anchor: Option<Anchor>,
}

impl Playback {
    /// Folds a source update into the tracked state.
    fn apply(&mut self, state: State, track: TrackInfo, album_art: &AlbumArtRequester) {
        self.last_state = state;
        match state {
            State::Stopped | State::Unknown => {
                self.current = None;
                self.anchor = None;
            }
            State::Playing | State::Paused => {
                let (position, duration) = (track.position, track.duration);
                if !same_track(self.current.as_ref(), &track) {
                    if album_differs(self.current.as_ref(), &track) {
                        self.cover = album_art.get_album_art(&track).ok();
                    }
                    self.anchor = None;
                    self.current = Some(track);
                }
                // Both notifications (currentTime) and the poll (position +
                // duration) carry a position; (re)anchor whenever one is present
                // so progress survives a track switch. Without a duration the bar
                // is open-ended (elapsed only) until the next poll bounds it.
                if let Some(pos) = position {
                    self.anchor = Some(progress_anchor(pos, duration, unix_now()));
                }
            }
        }
    }

    /// Mirrors the tracked state into Discord. When disabled, presence is cleared
    /// but the state is retained so re-enabling resumes the current track.
    /// `force` bypasses the rate-limit throttle for an instant menu-toggle response.
    fn publish(&self, discord: &mut Discord, enabled: bool, force: bool) {
        let result = match (enabled, self.last_state, &self.current) {
            (true, State::Playing, Some(track)) => {
                discord.update(track, self.cover.clone(), self.anchor, force)
            }
            _ => discord.clear(force),
        };
        if let Err(e) = result {
            error!("Discord presence error: {e:?}");
        }
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
