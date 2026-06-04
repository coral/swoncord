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
use std::time::Instant;

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

    /// Background loop: receives track updates, refreshes album art on album
    /// change, and reflects playback state into Discord.
    fn pump(rx: Receiver<(State, TrackInfo)>, shared: Arc<Mutex<Self>>) {
        let album_art = AlbumArtRequester::new();

        let mut current: Option<TrackInfo> = None;
        let mut last_state = State::Unknown;
        let mut cover: Option<String> = None;

        loop {
            if let Ok((state, track)) = rx.recv_timeout(consts::RECV_TIMEOUT) {
                last_state = state;
                let incoming = Some(track);
                if current != incoming {
                    if Self::album_changed(&current, &incoming) {
                        cover = incoming
                            .as_ref()
                            .and_then(|t| album_art.get_album_art(t).ok());
                    }
                    current = incoming;
                }
            }

            if let Some(track) = &current {
                let mut discord = shared.lock().expect("discord mutex poisoned");
                match last_state {
                    State::Playing => {
                        if let Err(e) = discord.update(track, cover.clone()) {
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
    }

    fn album_changed(last: &Option<TrackInfo>, current: &Option<TrackInfo>) -> bool {
        match (last, current) {
            (None, Some(_)) => true,
            (Some(last), Some(current)) => last.album != current.album,
            _ => false,
        }
    }

    pub fn update(&mut self, t: &TrackInfo, cover: Option<String>) -> Result<(), Error> {
        let state: String = format!("{} ", t.artist).chars().take(consts::FIELD_MAX).collect();
        let details: String = t.title.chars().take(consts::FIELD_MAX).collect();
        let large_text: String = t.album.chars().take(consts::FIELD_MAX).collect();

        let uri = cover.unwrap_or_else(|| "sw2".to_string());

        let assets = activity::Assets::new()
            .large_text(large_text.as_str())
            .large_image(&uri)
            .small_text("Listening");

        let payload = activity::Activity::new()
            .state(&state)
            .details(&details)
            .activity_type(activity::ActivityType::Listening)
            .assets(assets);
        // TODO(phase 2): when t.position/duration are set (pull source), attach
        // activity::Timestamps to render the Discord progress bar.

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

    pub fn clear(&mut self) -> Result<(), Error> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn track(album: &str) -> TrackInfo {
        TrackInfo {
            album: album.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn album_changed_on_first_track() {
        assert!(Discord::album_changed(&None, &Some(track("A"))));
    }

    #[test]
    fn album_changed_when_album_differs() {
        assert!(Discord::album_changed(&Some(track("A")), &Some(track("B"))));
    }

    #[test]
    fn album_unchanged_when_same_album() {
        assert!(!Discord::album_changed(&Some(track("A")), &Some(track("A"))));
    }

    #[test]
    fn album_unchanged_when_cleared() {
        assert!(!Discord::album_changed(&Some(track("A")), &None));
        assert!(!Discord::album_changed(&None, &None));
    }
}
