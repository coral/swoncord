//! Playback state and effects. No network access or reads of the system clock.

use crate::track::{State, TrackInfo, TrackUpdate};

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
}

impl Playback {
    /// Apply an observation immediately and return any artwork work to schedule.
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
                    self.generation = self.generation.wrapping_add(1);
                    if album_differs(self.current.as_ref(), &track) {
                        self.cover = None;
                    }
                    if self.cover.is_none() {
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

    /// A result for an earlier track (including an earlier play of this track)
    /// cannot replace the current artwork or bring stopped playback back.
    pub fn accept_artwork(&mut self, result: ArtworkResult) {
        if self.current.is_some() && result.generation == self.generation {
            self.cover = result.cover;
        }
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
    current.is_none_or(|c| c.album != incoming.album)
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
        playback.accept_artwork(ArtworkResult {
            generation: request.generation,
            cover: Some("art".into()),
        });
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
    fn old_artwork_is_rejected_even_when_the_same_track_plays_again() {
        let mut playback = Playback::default();
        let first = playback
            .apply(update(State::Playing, "A", None, 1000))
            .unwrap();
        playback.apply(update(State::Playing, "B", None, 1001));
        let current = playback
            .apply(update(State::Playing, "A", None, 1002))
            .unwrap();
        playback.accept_artwork(ArtworkResult {
            generation: first.generation,
            cover: Some("stale".into()),
        });
        assert!(playback.presence(true).unwrap().cover.is_none());
        playback.accept_artwork(ArtworkResult {
            generation: current.generation,
            cover: Some("current".into()),
        });
        assert_eq!(playback.presence(true).unwrap().cover, Some("current"));
    }

    #[test]
    fn stopped_playback_ignores_inflight_artwork() {
        let mut playback = Playback::default();
        let request = playback
            .apply(update(State::Playing, "Song", None, 1000))
            .unwrap();
        playback.apply(update(State::Stopped, "", None, 1001));
        playback.accept_artwork(ArtworkResult {
            generation: request.generation,
            cover: Some("old".into()),
        });
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
        playback.accept_artwork(ArtworkResult {
            generation: request.generation,
            cover: Some("art".into()),
        });
        assert!(
            playback
                .apply(update(State::Playing, "B", None, 1001))
                .is_none()
        );
        assert_eq!(playback.presence(true).unwrap().cover, Some("art"));
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
