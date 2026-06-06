//! Album art lookup via MusicBrainz + CoverArtArchive.

use crate::error::Error;
use crate::track::TrackInfo;
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

/// Resolves an album's front cover-art URL from its metadata.
pub struct AlbumArtRequester {
    client: MusicBrainzClient,
    http: HttpClient,
    /// Strips trailing "(Deluxe)"/"[Remaster]"-style qualifiers from albums.
    album_filter: Regex,
}

impl AlbumArtRequester {
    pub fn new() -> Self {
        let client = MusicBrainzClient::new("SwinsianRichPresence/1.0.0 ( https://jonasbengtson.se )");

        // MusicBrainzClient manages its own (ureq-based) HTTP client internally;
        // we only control the CoverArtArchive request, so we give that one a
        // timeout to keep a slow upstream from stalling the consumer.
        let http = HttpClient::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("HTTP client builds from a static config");

        // Constant pattern verified by the unit tests below.
        let album_filter =
            Regex::new(ALBUM_CLEAN_PATTERN).expect("album-clean regex is valid");

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

        let has_art = match self.http.head(&cover_art_url).send() {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        };

        if has_art {
            Ok(cover_art_url)
        } else {
            Err(Error::NoData)
        }
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

        for query in queries {
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
