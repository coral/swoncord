//! Discord I/O and retry scheduling, owned exclusively by the presence worker.

use super::playback::Presence;
use discord_rich_presence::{DiscordIpc, DiscordIpcClient, activity};
use log::warn;
use std::fmt::Debug;
use std::time::{Duration, Instant};

const DISCORD_APP_ID: &str = "1076384656850698240";
const FIELD_MAX: usize = 128;
const UPDATE_THROTTLE: Duration = Duration::from_secs(4);
const RECV_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY_INITIAL: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(30);

pub(super) trait Clock {
    fn now(&self) -> Instant;
}

pub(super) struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

pub(super) trait Transport {
    type Error: Debug;
    fn connect(&mut self) -> Result<(), Self::Error>;
    fn update(&mut self, presence: &Presence<'_>) -> Result<(), Self::Error>;
    fn clear(&mut self) -> Result<(), Self::Error>;
    fn disconnect(&mut self);
}

pub(super) struct DiscordTransport {
    client: DiscordIpcClient,
}

impl DiscordTransport {
    pub fn new() -> Self {
        Self {
            client: DiscordIpcClient::new(DISCORD_APP_ID),
        }
    }
}

impl Transport for DiscordTransport {
    type Error = discord_rich_presence::error::Error;

    fn connect(&mut self) -> Result<(), Self::Error> {
        // Recreate the client so recovery also works after an initial failure.
        // The library's reconnect() first closes an existing socket.
        self.client = DiscordIpcClient::new(DISCORD_APP_ID);
        self.client.connect()
    }

    fn disconnect(&mut self) {
        self.client = DiscordIpcClient::new(DISCORD_APP_ID);
    }

    fn update(&mut self, presence: &Presence<'_>) -> Result<(), Self::Error> {
        let state = presence_field(&presence.track.artist);
        let details = presence_field(&presence.track.title);
        let large_text = presence_field(&presence.track.album);
        let mut assets = activity::Assets::new()
            .large_image(presence.cover.unwrap_or("sw2"))
            .small_text("Listening");
        if let Some(text) = &large_text {
            assets = assets.large_text(text);
        }
        let mut payload = activity::Activity::new()
            .activity_type(activity::ActivityType::Listening)
            .assets(assets);
        if let Some(state) = &state {
            payload = payload.state(state);
        }
        if let Some(details) = &details {
            payload = payload.details(details);
        }
        if let Some(anchor) = presence.anchor {
            let mut timestamps = activity::Timestamps::new().start(anchor.start);
            if let Some(end) = anchor.end {
                timestamps = timestamps.end(end);
            }
            payload = payload.timestamps(timestamps);
        }
        self.client.set_activity(payload)
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        self.client.clear_activity()
    }
}

pub(super) struct Connection<T, C> {
    transport: T,
    clock: C,
    connected: bool,
    active: bool,
    last_updated: Option<Instant>,
    retry_at: Option<Instant>,
    retry_delay: Duration,
}

impl<T: Transport, C: Clock> Connection<T, C> {
    pub fn new(transport: T, clock: C) -> Self {
        Self {
            transport,
            clock,
            connected: false,
            active: false,
            last_updated: None,
            retry_at: None,
            retry_delay: RETRY_INITIAL,
        }
    }

    pub fn publish(&mut self, presence: Option<Presence<'_>>, force: bool) {
        let now = self.clock.now();
        if !self.connected {
            if self.retry_at.is_some_and(|retry_at| now < retry_at) {
                return;
            }
            if let Err(e) = self.transport.connect() {
                warn!("Discord connection failed: {e:?}");
                self.failed();
                return;
            }
            self.connected = true;
            // Synchronize even when disabled/paused: don't assume the remote
            // state after reconnecting, and don't throttle the first write.
            self.active = true;
            self.last_updated = None;
            self.retry_at = None;
        }

        if !force
            && self
                .last_updated
                .is_some_and(|last| now.duration_since(last) < UPDATE_THROTTLE)
        {
            return;
        }
        let active = presence.is_some();
        let result = match presence {
            Some(presence) => self.transport.update(&presence),
            None if self.active => self.transport.clear(),
            None => return,
        };
        match result {
            Ok(()) => {
                self.active = active;
                self.last_updated = Some(self.clock.now());
                self.retry_delay = RETRY_INITIAL;
            }
            Err(e) => {
                warn!("Discord presence write failed: {e:?}");
                self.failed();
            }
        }
    }

    fn failed(&mut self) {
        self.connected = false;
        self.transport.disconnect();
        self.retry_at = Some(self.clock.now() + self.retry_delay);
        self.retry_delay = (self.retry_delay * 2).min(RETRY_MAX);
    }

    pub fn wait_timeout(&self) -> Duration {
        let now = self.clock.now();
        if let Some(retry_at) = self.retry_at {
            return retry_at.saturating_duration_since(now).min(RECV_TIMEOUT);
        }
        if let Some(last) = self.last_updated {
            let remaining = (last + UPDATE_THROTTLE).saturating_duration_since(now);
            if !remaining.is_zero() {
                return remaining.min(RECV_TIMEOUT);
            }
        }
        RECV_TIMEOUT
    }
}

/// Discord text fields accept 2–128 characters; omit empty values.
fn presence_field(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut field: String = trimmed.chars().take(FIELD_MAX).collect();
    while field.chars().count() < 2 {
        field.push(' ');
    }
    Some(field)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::TrackInfo;
    use std::cell::Cell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    #[derive(Clone)]
    struct FakeClock(Rc<Cell<Instant>>);
    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            self.0.get()
        }
    }
    impl FakeClock {
        fn advance(&self, seconds: u64) {
            self.0.set(self.0.get() + Duration::from_secs(seconds));
        }
    }

    #[derive(Default)]
    struct FakeTransport {
        connects: usize,
        connect_results: VecDeque<Result<(), &'static str>>,
        writes: Vec<Option<String>>,
        fail_write: bool,
    }
    impl Transport for FakeTransport {
        type Error = &'static str;
        fn connect(&mut self) -> Result<(), Self::Error> {
            self.connects += 1;
            self.connect_results.pop_front().unwrap_or(Ok(()))
        }
        fn update(&mut self, presence: &Presence<'_>) -> Result<(), Self::Error> {
            self.writes.push(Some(presence.track.title.clone()));
            if std::mem::take(&mut self.fail_write) {
                Err("disconnected")
            } else {
                Ok(())
            }
        }
        fn clear(&mut self) -> Result<(), Self::Error> {
            self.writes.push(None);
            if std::mem::take(&mut self.fail_write) {
                Err("disconnected")
            } else {
                Ok(())
            }
        }
        fn disconnect(&mut self) {}
    }

    fn presence(track: &TrackInfo) -> Option<Presence<'_>> {
        Some(Presence {
            track,
            cover: None,
            anchor: None,
        })
    }

    #[test]
    fn discord_can_start_later_and_receives_the_latest_track() {
        let clock = FakeClock(Rc::new(Cell::new(Instant::now())));
        let transport = FakeTransport {
            connect_results: [Err("absent"), Err("absent"), Ok(())].into(),
            ..Default::default()
        };
        let mut connection = Connection::new(transport, clock.clone());
        let old = TrackInfo {
            title: "Old".into(),
            ..Default::default()
        };
        let current = TrackInfo {
            title: "Current".into(),
            ..Default::default()
        };
        connection.publish(presence(&old), false);
        assert_eq!(connection.wait_timeout(), Duration::from_secs(1));
        connection.publish(presence(&current), true);
        assert_eq!(connection.transport.connects, 1); // toggles don't bypass backoff
        clock.advance(1);
        connection.publish(presence(&current), false);
        assert_eq!(connection.wait_timeout(), Duration::from_secs(2));
        clock.advance(2);
        connection.publish(presence(&current), false);
        assert_eq!(connection.transport.writes, vec![Some("Current".into())]);
    }

    #[test]
    fn reconnect_republishes_after_a_write_failure() {
        let clock = FakeClock(Rc::new(Cell::new(Instant::now())));
        let mut connection = Connection::new(FakeTransport::default(), clock.clone());
        let track = TrackInfo::default();
        connection.publish(presence(&track), false);
        clock.advance(4);
        connection.transport.fail_write = true;
        connection.publish(presence(&track), false);
        assert!(!connection.connected);
        clock.advance(1);
        connection.publish(presence(&track), false);
        assert_eq!(connection.transport.connects, 2);
        assert!(connection.connected);
        assert_eq!(connection.transport.writes.len(), 3);
    }

    #[test]
    fn failed_clear_is_retried_and_does_not_restore_disabled_presence() {
        let clock = FakeClock(Rc::new(Cell::new(Instant::now())));
        let mut connection = Connection::new(FakeTransport::default(), clock.clone());
        let track = TrackInfo::default();
        connection.publish(presence(&track), false);
        connection.transport.fail_write = true;
        connection.publish(None, true);
        clock.advance(1);
        connection.publish(None, false);
        assert_eq!(
            connection.transport.writes,
            vec![Some("".into()), None, None]
        );
        assert!(!connection.active);
    }

    #[test]
    fn backoff_is_capped_and_resets_after_recovery() {
        let clock = FakeClock(Rc::new(Cell::new(Instant::now())));
        let transport = FakeTransport {
            connect_results: vec![Err("absent"); 10].into(),
            ..Default::default()
        };
        let mut connection = Connection::new(transport, clock.clone());
        for delay in [1, 2, 4, 8, 16, 30, 30, 30, 30, 30] {
            connection.publish(None, false);
            assert_eq!(
                connection.retry_at.unwrap().duration_since(clock.now()),
                Duration::from_secs(delay)
            );
            clock.advance(delay);
        }
        connection.publish(None, false);
        assert!(connection.connected);
        assert_eq!(connection.retry_delay, RETRY_INITIAL);
    }

    #[test]
    fn forced_disable_bypasses_write_throttle() {
        let clock = FakeClock(Rc::new(Cell::new(Instant::now())));
        let mut connection = Connection::new(FakeTransport::default(), clock);
        let track = TrackInfo::default();
        connection.publish(presence(&track), false);
        connection.publish(None, false);
        assert_eq!(connection.transport.writes.len(), 1);
        connection.publish(None, true);
        assert_eq!(connection.transport.writes, vec![Some("".into()), None]);
    }

    #[test]
    fn presence_fields_pad_omit_and_truncate_unicode_safely() {
        assert_eq!(presence_field("J").as_deref(), Some("J "));
        assert_eq!(presence_field("   "), None);
        assert_eq!(
            presence_field(" Normal Title ").as_deref(),
            Some("Normal Title")
        );
        assert_eq!(
            presence_field(&"🎵".repeat(150)).unwrap().chars().count(),
            FIELD_MAX
        );
    }
}
