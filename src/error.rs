use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("no release in request")]
    NoData,
    #[error("no data in request")]
    IPCError(#[from] Box<dyn std::error::Error>),
    #[error("musicbrainz error")]
    MusicBrainzError(#[from] musicbrainz_rs::Error),
}
