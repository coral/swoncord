use crate::error::Error;
use crate::swinsian::{State, TrackInfo};
use crossbeam::channel::Receiver;
use discord_rich_presence::{DiscordIpc, DiscordIpcClient, activity};
use log::error;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
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
    pub fn new(r: Receiver<(State, TrackInfo)>) -> Result<Arc<Mutex<Self>>, Error> {
        let mut client = DiscordIpcClient::new("1076384656850698240")?;
        client.connect()?;

        let cc = Arc::new(Mutex::new(Self {
            client,
            last_updated: Instant::now(),
            state: PresenceState::Cleared,
        }));

        let cc_v = cc.clone();
        let r_v = r.clone();
        std::thread::spawn(move || Self::pump(r_v, cc_v));

        Ok(cc)
    }

    fn pump(r: Receiver<(State, TrackInfo)>, cc: Arc<Mutex<Self>>) {
        let mb_client = AlbumArtRequester::new();

        let mut _last_track: Option<TrackInfo> = None;
        let mut current_track: Option<TrackInfo> = None;
        let mut last_state = State::Unknown;
        let mut cover: Option<String> = None;

        loop {
            match r.recv_timeout(Duration::from_secs(5)) {
                Ok((state, track_info)) => {
                    last_state = state;

                    let clone = track_info.clone();
                    let wrapped = Some(track_info);
                    if current_track != wrapped {
                        if Self::album_changed(&current_track, &wrapped) {
                            match mb_client.get_album_art(&clone) {
                                Ok(v) => {
                                    cover = Some(v);
                                }
                                Err(_) => {
                                    cover = None;
                                }
                            };
                        }
                        _last_track = current_track;
                    }
                    current_track = wrapped;
                }
                Err(_) => {}
            };

            if let Some(track_info) = &current_track {
                let mut dc = cc.lock().unwrap();
                match last_state {
                    State::Playing => {
                        if let Err(e) = dc.update(track_info, cover.clone()) {
                            error!("Error updating Discord status: {:?}", e);
                        }
                    }
                    State::Stopped | State::Paused | State::Unknown => {
                        if let Err(e) = dc.clear() {
                            error!("Error clearing Discord status: {:?}", e);
                        }
                    }
                }
            }
        }
    }

    fn album_changed(last: &Option<TrackInfo>, current: &Option<TrackInfo>) -> bool {
        if last.is_none() && current.is_some() {
            return true;
        }
        if let (Some(last), Some(current)) = (last, current) {
            last.album != current.album
        } else {
            false
        }
    }

    pub fn update(&mut self, t: &TrackInfo, cover: Option<String>) -> Result<(), Error> {
        let state: String = format!("{} ", t.artist).chars().take(128).collect();
        let details: String = t.title.chars().take(128).collect();

        let large_text: String = t.album.chars().take(128).collect();

        let uri = match cover {
            Some(v) => v,
            None => "sw2".to_string(),
        };

        let assets = activity::Assets::new()
            .large_text(large_text.as_str())
            .large_image(&uri)
            .small_text("Listening");

        let payload = activity::Activity::new()
            .state(&state)
            .details(&details)
            .activity_type(activity::ActivityType::Listening)
            .assets(assets.clone());

        // if let Some(v) = data.calculate_POGRESS() {
        //     let timestamp = activity::Timestamps::new().start(v);
        //     payload = payload.timestamps(timestamp);
        // }

        if Instant::now().duration_since(self.last_updated).as_secs() >= 4 {
            match self.client.set_activity(payload) {
                Ok(_) => {
                    self.last_updated = Instant::now();
                }
                Err(e) => {
                    error!("Error setting Discord status: {:?}", e);
                    self.client.reconnect().ok();
                }
            }
        }

        self.state = PresenceState::Active;
        Ok(())
    }

    pub fn clear(&mut self) -> Result<(), Error> {
        match self.state {
            PresenceState::Active => {
                if Instant::now().duration_since(self.last_updated).as_secs() >= 4 {
                    if self.client.clear_activity().is_err() {
                        self.client.reconnect().ok();
                    } else {
                        self.last_updated = Instant::now();
                        self.state = PresenceState::Cleared
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }
}

use musicbrainz_rs::client::MusicBrainzClient;
use musicbrainz_rs::entity::{release_group::ReleaseGroup, release_group::ReleaseGroupSearchQuery};
use musicbrainz_rs::prelude::*;
use reqwest::blocking::Client as HTTPClient;

pub struct AlbumArtRequester {
    client: MusicBrainzClient,
    hc: HTTPClient,
}

impl AlbumArtRequester {
    pub fn new() -> Self {
        let mut mbc = MusicBrainzClient::default();
        mbc.set_user_agent("SwinsianRichPresence/1.0.0 ( https://jonasbengtson.se )")
            .unwrap();
        Self {
            client: mbc,
            hc: HTTPClient::new(),
        }
    }
}
impl AlbumArtRequester {
    pub fn get_album_art(&self, t: &TrackInfo) -> Result<String, Error> {
        let release_id = self.find_release(t)?;

        let cover_art_url = format!(
            "https://coverartarchive.org/release-group/{}/front-250",
            release_id
        );

        let has_art = match self.hc.head(&cover_art_url).send() {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        };

        if has_art {
            Ok(cover_art_url)
        } else {
            Err(Error::NoData)
        }
    }

    fn find_release(&self, t: &TrackInfo) -> Result<String, Error> {
        let query = ReleaseGroupSearchQuery::query_builder()
            .artist(&t.artist)
            .and()
            .release_group(&t.album)
            .build();

        let releases = ReleaseGroup::search(query).execute_with_client(&self.client)?;

        if !releases.entities.is_empty() {
            return Ok(releases.entities[0].id.clone());
        }

        let query = ReleaseGroupSearchQuery::query_builder()
            .and()
            .release_group(&t.album)
            .build();

        let releases = ReleaseGroup::search(query).execute_with_client(&self.client)?;

        if !releases.entities.is_empty() {
            return Ok(releases.entities[0].id.clone());
        }

        Err(Error::NoData)
    }
}
