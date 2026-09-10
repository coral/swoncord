//! Independent workers for Discord presence and optional album artwork.

mod albumart;
mod playback;
mod transport;

use crate::latest;
use crate::track::TrackUpdate;
use albumart::{AlbumArtRequester, ArtworkLookup};
use crossbeam::channel::{self, Receiver};
use playback::{ArtworkRequest, ArtworkResult, Playback};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use transport::{Clock, Connection, DiscordTransport, SystemClock, Transport};

/// Starts workers without connecting to Discord or making HTTP requests on the
/// main thread. Discord may be absent when Swoncord starts (e.g. at login).
pub fn start(
    rx: latest::Receiver<TrackUpdate>,
    enabled: Arc<AtomicBool>,
    wake: Receiver<()>,
) -> std::io::Result<JoinHandle<()>> {
    let (requests, pending) = latest::channel();
    let (results, artwork) = latest::channel();
    thread::Builder::new()
        .name("swoncord-artwork".into())
        .spawn(move || {
            run_artwork(AlbumArtRequester::new(), pending, results);
        })?;
    thread::Builder::new()
        .name("swoncord-discord".into())
        .spawn(move || {
            let connection = Connection::new(DiscordTransport::new(), SystemClock);
            pump(rx, enabled, wake, requests, artwork, connection);
        })
}

fn run_artwork(
    mut lookup: impl ArtworkLookup,
    requests: latest::Receiver<ArtworkRequest>,
    results: latest::Sender<ArtworkResult>,
) {
    while requests.ready().recv().is_ok() {
        let Some(request) = requests.take() else {
            continue;
        };
        let result = ArtworkResult {
            generation: request.generation,
            cover: lookup.lookup(&request.track),
        };
        if results.send(result).is_err() {
            break;
        }
    }
}

fn pump<T: Transport, C: Clock>(
    rx: latest::Receiver<TrackUpdate>,
    enabled: Arc<AtomicBool>,
    mut wake: Receiver<()>,
    requests: latest::Sender<ArtworkRequest>,
    artwork: latest::Receiver<ArtworkResult>,
    mut connection: Connection<T, C>,
) {
    let mut playback = Playback::default();
    let mut was_enabled = enabled.load(Ordering::Relaxed);
    let mut art_ready = artwork.ready().clone();
    loop {
        // Consume the newest observation before accepting an artwork result.
        // The generation check rejects results for replaced tracks.
        if let Some(update) = rx.take()
            && let Some(request) = playback.apply(update)
        {
            let _ = requests.send(request);
        }
        if let Some(result) = artwork.take() {
            playback.accept_artwork(result);
        }

        let is_enabled = enabled.load(Ordering::Relaxed);
        connection.publish(playback.presence(is_enabled), is_enabled != was_enabled);
        was_enabled = is_enabled;

        channel::select! {
            recv(rx.ready()) -> event => if event.is_err() { break; },
            recv(wake) -> event => if event.is_err() { wake = channel::never(); },
            recv(art_ready) -> event => if event.is_err() { art_ready = channel::never(); },
            default(connection.wait_timeout()) => {},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::{State, TrackInfo};
    use playback::Presence;
    use std::time::Duration;

    struct BlockedLookup {
        started: channel::Sender<String>,
        release: Receiver<()>,
    }
    impl ArtworkLookup for BlockedLookup {
        fn lookup(&mut self, track: &TrackInfo) -> Option<String> {
            self.started.send(track.title.clone()).unwrap();
            self.release.recv().unwrap();
            Some(format!("art:{}", track.title))
        }
    }

    struct RecordingTransport(channel::Sender<Option<String>>);
    impl Transport for RecordingTransport {
        type Error = ();
        fn connect(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
        fn update(&mut self, presence: &Presence<'_>) -> Result<(), Self::Error> {
            self.0.send(Some(presence.track.title.clone())).unwrap();
            Ok(())
        }
        fn clear(&mut self) -> Result<(), Self::Error> {
            self.0.send(None).unwrap();
            Ok(())
        }
        fn disconnect(&mut self) {}
    }

    #[test]
    fn blocked_artwork_does_not_delay_initial_presence_or_disable() {
        let (tx, rx) = latest::channel();
        let (requests, pending) = latest::channel();
        let (results, artwork) = latest::channel();
        let (started_tx, started) = channel::bounded(1);
        let (release, release_rx) = channel::bounded(1);
        let art_thread = thread::spawn(move || {
            run_artwork(
                BlockedLookup {
                    started: started_tx,
                    release: release_rx,
                },
                pending,
                results,
            )
        });
        let enabled = Arc::new(AtomicBool::new(true));
        let (wake_tx, wake) = channel::bounded(1);
        let (writes_tx, writes) = channel::unbounded();
        tx.send(TrackUpdate {
            state: State::Playing,
            track: TrackInfo {
                title: "Song".into(),
                ..Default::default()
            },
            observed_at: 1000,
        })
        .unwrap();
        let worker_enabled = enabled.clone();
        let worker = thread::spawn(move || {
            pump(
                rx,
                worker_enabled,
                wake,
                requests,
                artwork,
                Connection::new(RecordingTransport(writes_tx), SystemClock),
            )
        });
        assert_eq!(
            started.recv_timeout(Duration::from_secs(2)).unwrap(),
            "Song"
        );
        assert_eq!(
            writes.recv_timeout(Duration::from_secs(2)).unwrap(),
            Some("Song".into())
        );
        enabled.store(false, Ordering::Relaxed);
        wake_tx.send(()).unwrap();
        assert_eq!(writes.recv_timeout(Duration::from_secs(2)).unwrap(), None);
        // The lookup remains blocked until explicitly released by the test.
        drop(tx);
        worker.join().unwrap();
        release.send(()).unwrap();
        art_thread.join().unwrap();
    }

    #[test]
    fn artwork_worker_skips_superseded_pending_requests() {
        let (requests, pending) = latest::channel();
        let (results, artwork) = latest::channel();
        let (started_tx, started) = channel::bounded(1);
        let (release, release_rx) = channel::bounded(1);
        let worker = thread::spawn(move || {
            run_artwork(
                BlockedLookup {
                    started: started_tx,
                    release: release_rx,
                },
                pending,
                results,
            )
        });
        let request = |generation, title: &str| ArtworkRequest {
            generation,
            track: TrackInfo {
                title: title.into(),
                ..Default::default()
            },
        };
        requests.send(request(1, "A")).unwrap();
        assert_eq!(started.recv_timeout(Duration::from_secs(2)).unwrap(), "A");
        requests.send(request(2, "B")).unwrap();
        requests.send(request(3, "C")).unwrap();
        release.send(()).unwrap();
        assert_eq!(started.recv_timeout(Duration::from_secs(2)).unwrap(), "C");
        artwork
            .ready()
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert_eq!(artwork.take().unwrap().generation, 1);
        drop(requests);
        release.send(()).unwrap();
        worker.join().unwrap();
        assert_eq!(artwork.take().unwrap().generation, 3);
    }
}
