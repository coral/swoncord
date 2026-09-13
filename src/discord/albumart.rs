//! Album art lookup via MusicBrainz + CoverArtArchive.

use crate::error::Error;
use crate::track::TrackInfo;
use musicbrainz_rs::api_bindium::ureq::{Agent, config::Config};
use musicbrainz_rs::client::MusicBrainzClient;
use musicbrainz_rs::entity::release_group::{ReleaseGroup, ReleaseGroupSearchQuery};
use musicbrainz_rs::prelude::*;
use regex::Regex;
use reqwest::blocking::Client as HttpClient;
use std::time::Duration;

/// Strips trailing parenthetical/bracketed qualifiers from an album name, e.g.
/// "Album (Deluxe Edition)" or "Album [Remaster] (Bonus)" -> "Album". Used as a
/// last-ditch broadening of the MusicBrainz search.
const ALBUM_CLEAN_PATTERN: &str = r"\s+[\(\[][^\(\)\[\]]*[\)\]](\s+[\(\[][^\(\)\[\]]*[\)\]])*$";

pub(super) trait ArtworkLookup {
    fn lookup(&mut self, track: &TrackInfo, cancelled: &dyn Fn() -> bool) -> Option<String>;
}

/// Resolves an album's front cover-art URL from its metadata.
pub struct AlbumArtRequester {
    client: MusicBrainzClient,
    http: HttpClient,
    /// Strips trailing "(Deluxe)"/"[Remaster]"-style qualifiers from albums.
    album_filter: Regex,
}

impl AlbumArtRequester {
    pub fn new() -> Self {
        let mut client = MusicBrainzClient::default();
        client.api_client.agent = Agent::new_with_config(
            Config::builder()
                .user_agent("SwinsianRichPresence/1.0.0 ( https://jonasbengtson.se )")
                .timeout_global(Some(Duration::from_secs(10)))
                .build(),
        );
        // This is an attempt count, not additional retries. Disable internal
        // retry sleeps so stale artwork cannot occupy the worker indefinitely.
        client.api_client.max_retries = 1;

        // Both upstreams have deadlines; only the artwork worker waits on them.
        let http = HttpClient::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("HTTP client builds from a static config");

        // Constant pattern verified by the unit tests below.
        let album_filter = Regex::new(ALBUM_CLEAN_PATTERN).expect("album-clean regex is valid");

        Self {
            client,
            http,
            album_filter,
        }
    }

    /// Returns the front cover-art URL for `t`'s album, or [`Error::NoData`] if
    /// no release matches or the art is missing.
    pub fn get_album_art(&self, t: &TrackInfo) -> Result<String, Error> {
        let release_id = self.find_release(t)?;
        let cover_art_url = format!(
            "https://coverartarchive.org/release-group/{}/front-250",
            release_id
        );

        self.check_cover(&cover_art_url)?;
        Ok(cover_art_url)
    }

    fn check_cover(&self, url: &str) -> Result<(), Error> {
        let response = self.http.head(url).send()?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::NoData);
        }
        response.error_for_status()?;
        Ok(())
    }

    /// Searches MusicBrainz from most to least specific, returning the first
    /// matching release-group id: artist+album, then album, then cleaned album.
    fn find_release(&self, t: &TrackInfo) -> Result<String, Error> {
        let cleaned_album = self.clean_album_name(&t.album);
        let queries = [
            ReleaseGroupSearchQuery::query_builder()
                .artist(&t.artist)
                .and()
                .release_group(&t.album)
                .build(),
            ReleaseGroupSearchQuery::query_builder()
                .and()
                .release_group(&t.album)
                .build(),
            ReleaseGroupSearchQuery::query_builder()
                .and()
                .release_group(&cleaned_album)
                .build(),
        ];

        for (index, query) in queries.into_iter().enumerate() {
            // The synchronous MusicBrainz client has no built-in rate limiter.
            if index > 0 {
                std::thread::sleep(Duration::from_secs(1));
            }
            let results = ReleaseGroup::search(query).execute_with_client(&self.client)?;
            if let Some(first) = results.entities.into_iter().next() {
                return Ok(first.id);
            }
        }

        Err(Error::NoData)
    }

    fn clean_album_name(&self, album: &str) -> String {
        self.album_filter.replace(album, "").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    #[test]
    fn http_failure_is_distinct_from_missing_art_and_later_requests_recover() {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/front-250", server.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            for status in ["503 Service Unavailable", "404 Not Found", "200 OK"] {
                let (mut stream, _) = server.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = BufReader::new(&mut stream);
                let mut line = String::new();
                while request.read_line(&mut line).unwrap() > 0 {
                    if line == "\r\n" {
                        break;
                    }
                    line.clear();
                }
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
            }
        });
        let mut requester = AlbumArtRequester::new();
        requester.http = HttpClient::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        assert!(
            matches!(requester.check_cover(&url), Err(Error::Http(error)) if error.status() == Some(reqwest::StatusCode::SERVICE_UNAVAILABLE))
        );
        assert!(matches!(requester.check_cover(&url), Err(Error::NoData)));
        assert!(requester.check_cover(&url).is_ok());
        worker.join().unwrap();
    }

    #[test]
    fn clean_album_name_strips_trailing_qualifiers() {
        let r = AlbumArtRequester::new();
        assert_eq!(r.clean_album_name("Album (Deluxe Edition)"), "Album");
        assert_eq!(r.clean_album_name("Album [Remaster]"), "Album");
        assert_eq!(r.clean_album_name("Album [Remaster] (Bonus)"), "Album");
    }

    #[test]
    fn clean_album_name_leaves_plain_names_untouched() {
        let r = AlbumArtRequester::new();
        assert_eq!(r.clean_album_name("Plain Album"), "Plain Album");
        // Parentheses that are part of the title (not trailing-separated) stay.
        assert_eq!(r.clean_album_name("Album(NoSpace)"), "Album(NoSpace)");
    }
}
