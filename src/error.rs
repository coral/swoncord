use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("no matching release found")]
    NoData,
    #[error("discord IPC error")]
    Ipc(#[from] discord_rich_presence::error::Error),
    // Boxed: `ApiEndpointError` is large, and boxing keeps `Result<_, Error>`
    // small (avoids clippy::result_large_err).
    #[error("musicbrainz error")]
    MusicBrainz(#[source] Box<musicbrainz_rs::ApiEndpointError>),
}

impl From<musicbrainz_rs::ApiEndpointError> for Error {
    fn from(e: musicbrainz_rs::ApiEndpointError) -> Self {
        Error::MusicBrainz(Box::new(e))
    }
}
