use crate::error::Error;
use crate::swinsian::{State, TrackInfo};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient, activity};
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
    pub fn new() -> Result<Self, Error> {
        let mut client = DiscordIpcClient::new("1076384656850698240")?;
        client.connect()?;
        Ok(Self {
            client,
            last_updated: Instant::now(),
            state: PresenceState::Cleared,
        })
    }

    pub fn update(&mut self, t: TrackInfo) -> Result<(), Error> {
        Ok(())
    }

    // pub fn update_presence(
    //     &mut self,
    //     data: swinsian::SwinsianResponse,
    //     client: &mut impl DiscordIpc,
    // ) -> Result<(), error::SwinsianError> {
    //     let state: String = format!("{} - {}", data.artist(), data.album())
    //         .chars()
    //         .take(128)
    //         .collect();
    //     let details: String = data.song.chars().take(128).collect();
    //     //let large_text: String = format!("Listening to {} with Swinsian", data.format);
    //     let assets = activity::Assets::new()
    //         //.large_text(large_text.as_str())
    //         .large_image("sw2")
    //         .small_text("Listening");

    //     let mut payload = activity::Activity::new()
    //         .state(&state)
    //         .details(&details)
    //         .activity_type(activity::ActivityType::Listening)
    //         .assets(assets.clone());

    //     if let swinsian::State::Playing = data.state {
    //         if let Some(v) = data.calculate_POGRESS() {
    //             let timestamp = activity::Timestamps::new().start(v);
    //             payload = payload.timestamps(timestamp);
    //         }
    //     }

    //     if Instant::now().duration_since(self.last_updated).as_secs() >= 4 {
    //         if client.set_activity(payload).is_err() {
    //             client.reconnect().ok();
    //         } else {
    //             self.last_updated = Instant::now();
    //         }
    //     }

    //     Ok(())
    // }

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
