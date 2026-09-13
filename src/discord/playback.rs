//! Playback state and effects. No network access or reads of the system clock.

use crate::track::{State, TrackInfo, TrackUpdate};
use std::time::{Duration, Instant};

const ART_RETRY_INITIAL: Duration = Duration::from_secs(30);
const ART_RETRY_MAX: Duration = Duration::from_secs(300);

pub(super) struct ArtworkRequest {
    pub generation: u64,
    pub track: TrackInfo,
}

pub(super) struct ArtworkResult {
    pub generation: u64,
    pub cover: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Anchor {
    pub start: i64,
    pub end: Option<i64>,
}

pub(super) struct Presence<'a> {
    pub track: &'a TrackInfo,
    pub cover: Option<&'a str>,
    pub anchor: Option<Anchor>,
}

#[derive(Default)]
pub(super) struct Playback {
    current: Option<TrackInfo>,
    last_state: State,
    cover: Option<String>,
    anchor: Option<Anchor>,
    paused_at: Option<i64>,
    generation: u64,
    retry_at: Option<Instant>,
    retry_delay: Option<Duration>,
}

impl Playback {
    /// Apply an observation immediately and schedule work only for a new album.
    pub fn apply(&mut self, update: TrackUpdate) -> Option<ArtworkRequest> {
        let TrackUpdate {
            state,
            track,
            observed_at,
        } = update;
        let previous_state = self.last_state;
        self.last_state = state;
        match state {
            State::Stopped | State::Unknown => {
                self.generation = self.generation.wrapping_add(1);
                self.retry_at = None;
                self.retry_delay = None;
                self.current = None;
                self.cover = None;
                self.anchor = None;
                self.paused_at = None;
                None
            }
            State::Playing | State::Paused => {
                let (position, duration) = (track.position, track.duration);
                let mut request = None;
                if !same_track(self.current.as_ref(), &track) {
                    if album_differs(self.current.as_ref(), &track) {
                        self.retry_at = None;
                        self.retry_delay = None;
                        self.generation = self.generation.wrapping_add(1);
                        self.cover = None;
                        request = Some(ArtworkRequest {
                            generation: self.generation,
                            track: track.clone(),
                        });
                    }
                    self.anchor = None;
                    self.paused_at = None;
                    self.current = Some(track);
                }

                if let Some(pos) = position {
                    self.anchor = Some(progress_anchor(pos, duration, observed_at));
                    self.paused_at = (state == State::Paused).then_some(observed_at);
                } else if state == State::Paused {
                    self.paused_at.get_or_insert(observed_at);
                } else if previous_state == State::Paused {
                    // A position-less resume must not count time spent paused.
                    if let (Some(paused_at), Some(anchor)) =
                        (self.paused_at.take(), &mut self.anchor)
                    {
                        let pause = (observed_at - paused_at).max(0);
                        anchor.start += pause;
                        anchor.end = anchor.end.map(|end| end + pause);
                    }
                }
                request
            }
        }
    }

    /// Results remain valid across songs on the same album, but cannot replace
    /// a later album (including a return to this album) or revive stopped playback.
    pub fn accept_artwork(&mut self, result: ArtworkResult, now: Instant) {
        if self.current.is_some() && result.generation == self.generation {
            self.cover = result.cover;
            if self.cover.is_none() {
                let delay = self.retry_delay.unwrap_or(ART_RETRY_INITIAL);
                self.retry_at = Some(now + delay);
                self.retry_delay = Some((delay * 2).min(ART_RETRY_MAX));
                log::info!(
                    "Artwork retry scheduled in {delay:?}: generation={}",
                    self.generation
                );
            } else {
                self.retry_at = None;
                self.retry_delay = None;
            }
        }
    }

    /// Retry without requiring a track change. Clearing the deadline prevents
    /// duplicate work while the lookup is in flight.
    pub fn retry_artwork(&mut self, now: Instant) -> Option<ArtworkRequest> {
        if self.last_state != State::Playing || !self.retry_at.is_some_and(|at| now >= at) {
            return None;
        }
        self.retry_at = None;
        Some(ArtworkRequest {
            generation: self.generation,
            track: self.current.clone()?,
        })
    }

    pub fn presence(&self, enabled: bool) -> Option<Presence<'_>> {
        if !enabled || self.last_state != State::Playing {
            return None;
        }
        Some(Presence {
            track: self.current.as_ref()?,
            cover: self.cover.as_deref(),
            anchor: self.anchor,
        })
    }
}

fn same_track(current: Option<&TrackInfo>, incoming: &TrackInfo) -> bool {
    current.is_some_and(|c| {
        c.artist == incoming.artist && c.album == incoming.album && c.title == incoming.title
    })
}

fn album_differs(current: Option<&TrackInfo>, incoming: &TrackInfo) -> bool {
    current.is_none_or(|c| c.album != incoming.album || c.artist != incoming.artist)
}

fn progress_anchor(pos: f64, dur: Option<f64>, observed_at: i64) -> Anchor {
    let start = observed_at - pos as i64;
    Anchor {
        start,
        end: dur.map(|d| start + d as i64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(state: State, title: &str, pos: Option<f64>, observed_at: i64) -> TrackUpdate {
        TrackUpdate {
            state,
            track: TrackInfo {
                artist: "Artist".into(),
                album: "Album".into(),
                title: title.into(),
                position: pos,
                duration: Some(200.0),
                ..Default::default()
            },
            observed_at,
        }
    }

    #[test]
    fn delayed_processing_and_artwork_do_not_move_observed_progress() {
        let mut playback = Playback::default();
        let request = playback
            .apply(update(State::Playing, "Song", Some(30.0), 1000))
            .unwrap();
        let before = playback.presence(true).unwrap();
        assert!(before.cover.is_none());
        assert_eq!(
            before.anchor,
            Some(Anchor {
                start: 970,
                end: Some(1170)
            })
        );
        playback.accept_artwork(
            ArtworkResult {
                generation: request.generation,
                cover: Some("art".into()),
            },
            Instant::now(),
        );
        let after = playback.presence(true).unwrap();
        assert_eq!(after.cover, Some("art"));
        assert_eq!(
            after.anchor,
            Some(Anchor {
                start: 970,
                end: Some(1170)
            })
        );
    }

    #[test]
    fn old_artwork_is_rejected_after_switching_away_and_back_to_an_album() {
        let mut playback = Playback::default();
        let first = playback
            .apply(update(State::Playing, "A", None, 1000))
            .unwrap();
        let mut other = update(State::Playing, "B", None, 1001);
        other.track.album = "Other album".into();
        playback.apply(other);
        let current = playback
            .apply(update(State::Playing, "A", None, 1002))
            .unwrap();
        playback.accept_artwork(
            ArtworkResult {
                generation: first.generation,
                cover: Some("stale".into()),
            },
            Instant::now(),
        );
        assert!(playback.presence(true).unwrap().cover.is_none());
        playback.accept_artwork(
            ArtworkResult {
                generation: current.generation,
                cover: Some("current".into()),
            },
            Instant::now(),
        );
        assert_eq!(playback.presence(true).unwrap().cover, Some("current"));
    }

    #[test]
    fn stopped_playback_ignores_inflight_artwork() {
        let mut playback = Playback::default();
        let request = playback
            .apply(update(State::Playing, "Song", None, 1000))
            .unwrap();
        playback.apply(update(State::Stopped, "", None, 1001));
        playback.accept_artwork(
            ArtworkResult {
                generation: request.generation,
                cover: Some("old".into()),
            },
            Instant::now(),
        );
        assert!(playback.presence(true).is_none());
        assert!(playback.cover.is_none());
    }

    #[test]
    fn pause_resume_with_positions_reanchors_and_disable_retains_playback() {
        let mut playback = Playback::default();
        playback.apply(update(State::Playing, "Song", Some(30.0), 1000));
        playback.apply(update(State::Paused, "Song", Some(40.0), 1010));
        assert!(playback.presence(true).is_none());
        playback.apply(update(State::Playing, "Song", Some(40.0), 1060));
        assert!(playback.presence(false).is_none());
        assert_eq!(
            playback.presence(true).unwrap().anchor,
            Some(Anchor {
                start: 1020,
                end: Some(1220)
            })
        );
    }

    #[test]
    fn positionless_pause_resume_excludes_paused_time() {
        let mut playback = Playback::default();
        playback.apply(update(State::Playing, "Song", Some(30.0), 1000));
        playback.apply(update(State::Paused, "Song", None, 1010));
        playback.apply(update(State::Paused, "Song", None, 1020));
        playback.apply(update(State::Playing, "Song", None, 1060));
        assert_eq!(
            playback.presence(true).unwrap().anchor,
            Some(Anchor {
                start: 1020,
                end: Some(1220)
            })
        );
    }

    #[test]
    fn positionless_notification_keeps_anchor_and_does_not_restart_artwork() {
        let mut playback = Playback::default();
        playback.apply(update(State::Playing, "Song", Some(30.0), 1000));
        assert!(
            playback
                .apply(update(State::Playing, "Song", None, 1005))
                .is_none()
        );
        assert_eq!(playback.presence(true).unwrap().anchor.unwrap().start, 970);
    }

    #[test]
    fn same_album_reuses_existing_artwork() {
        let mut playback = Playback::default();
        let request = playback
            .apply(update(State::Playing, "A", None, 1000))
            .unwrap();
        playback.accept_artwork(
            ArtworkResult {
                generation: request.generation,
                cover: Some("art".into()),
            },
            Instant::now(),
        );
        assert!(
            playback
                .apply(update(State::Playing, "B", None, 1001))
                .is_none()
        );
        assert_eq!(playback.presence(true).unwrap().cover, Some("art"));
    }

    #[test]
    fn missing_art_retries_without_track_change_and_recovers() {
        let mut playback = Playback::default();
        let now = Instant::now();
        let first = playback
            .apply(update(State::Playing, "Song", Some(30.0), 1000))
            .unwrap();
        playback.accept_artwork(
            ArtworkResult {
                generation: first.generation,
                cover: None,
            },
            now,
        );
        // Restarting the same track doesn't duplicate the pending retry.
        assert!(
            playback
                .apply(update(State::Playing, "Song", Some(0.0), 1001))
                .is_none()
        );
        assert!(
            playback
                .retry_artwork(now + Duration::from_secs(29))
                .is_none()
        );
        let retry = playback
            .retry_artwork(now + Duration::from_secs(30))
            .unwrap();
        assert_eq!(retry.generation, first.generation);
        assert!(
            playback
                .retry_artwork(now + Duration::from_secs(90))
                .is_none()
        );
        playback.accept_artwork(
            ArtworkResult {
                generation: retry.generation,
                cover: Some("art".into()),
            },
            now,
        );
        assert_eq!(playback.presence(true).unwrap().cover, Some("art"));
        assert!(
            playback
                .retry_artwork(now + Duration::from_secs(3600))
                .is_none()
        );
    }

    #[test]
    fn failures_back_off_and_stale_results_cannot_schedule_retries() {
        let mut playback = Playback::default();
        let mut now = Instant::now();
        let first = playback
            .apply(update(State::Playing, "A", None, 1000))
            .unwrap();
        for seconds in [30, 60, 120, 240, 300, 300] {
            playback.accept_artwork(
                ArtworkResult {
                    generation: first.generation,
                    cover: None,
                },
                now,
            );
            assert!(
                playback
                    .retry_artwork(now + Duration::from_secs(seconds - 1))
                    .is_none()
            );
            now += Duration::from_secs(seconds);
            assert!(playback.retry_artwork(now).is_some());
        }
        let mut other = update(State::Playing, "B", None, 1001);
        other.track.album = "Other album".into();
        let current = playback.apply(other).unwrap();
        playback.accept_artwork(
            ArtworkResult {
                generation: first.generation,
                cover: None,
            },
            now,
        );
        assert!(
            playback
                .retry_artwork(now + Duration::from_secs(3600))
                .is_none()
        );
        playback.accept_artwork(
            ArtworkResult {
                generation: current.generation,
                cover: None,
            },
            now,
        );
        playback.apply(update(State::Stopped, "", None, 1002));
        assert!(
            playback
                .retry_artwork(now + Duration::from_secs(3600))
                .is_none()
        );
    }

    #[test]
    fn same_album_keeps_inflight_artwork_and_accepts_it_for_the_new_song() {
        let mut playback = Playback::default();
        let first = playback
            .apply(update(State::Playing, "A", Some(30.0), 1000))
            .unwrap();
        assert!(
            playback
                .apply(update(State::Playing, "B", Some(0.0), 1010))
                .is_none()
        );
        assert!(
            playback
                .apply(update(State::Playing, "C", Some(0.0), 1020))
                .is_none()
        );
        playback.accept_artwork(
            ArtworkResult {
                generation: first.generation,
                cover: Some("album art".into()),
            },
            Instant::now(),
        );
        let presence = playback.presence(true).unwrap();
        assert_eq!(presence.track.title, "C");
        assert_eq!(presence.cover, Some("album art"));
        assert_eq!(presence.anchor.unwrap().start, 1020);
    }

    #[test]
    fn same_album_keeps_retry_backoff_but_a_new_album_requests_immediately() {
        let mut playback = Playback::default();
        let now = Instant::now();
        let first = playback
            .apply(update(State::Playing, "A", None, 1000))
            .unwrap();
        playback.accept_artwork(
            ArtworkResult {
                generation: first.generation,
                cover: None,
            },
            now,
        );
        assert!(
            playback
                .apply(update(State::Playing, "B", None, 1001))
                .is_none()
        );
        assert!(
            playback
                .retry_artwork(now + Duration::from_secs(29))
                .is_none()
        );
        let retry = playback
            .retry_artwork(now + Duration::from_secs(30))
            .unwrap();
        assert_eq!(retry.track.title, "B");
        assert_eq!(retry.generation, first.generation);
        let mut other = update(State::Playing, "C", None, 1002);
        other.track.album = "Other album".into();
        assert!(playback.apply(other).is_some());
    }

    #[test]
    fn artists_with_identically_named_albums_do_not_share_art() {
        let mut playback = Playback::default();
        let first = playback
            .apply(update(State::Playing, "A", None, 1000))
            .unwrap();
        playback.accept_artwork(
            ArtworkResult {
                generation: first.generation,
                cover: Some("old".into()),
            },
            Instant::now(),
        );
        let mut next = update(State::Playing, "B", None, 1001);
        next.track.artist = "Different artist".into();
        assert!(playback.apply(next).is_some());
        assert!(playback.presence(true).unwrap().cover.is_none());
    }

    #[test]
    fn progress_without_duration_is_open_ended() {
        assert_eq!(
            progress_anchor(30.0, None, 1000),
            Anchor {
                start: 970,
                end: None
            }
        );
    }
}
