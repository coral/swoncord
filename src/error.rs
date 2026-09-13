use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("no matching release found")]
    NoData,
    #[error("musicbrainz error")]
    MusicBrainz(#[source] Box<musicbrainz_rs::ApiEndpointError>),
    #[error("cover art request failed: {0}")]
    Http(#[from] reqwest::Error),
}

impl From<musicbrainz_rs::ApiEndpointError> for Error {
    fn from(e: musicbrainz_rs::ApiEndpointError) -> Self {
        Error::MusicBrainz(Box::new(e))
    }
}
