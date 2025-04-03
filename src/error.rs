use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("no data in request")]
    NoData,
    #[error("no data in request")]
    IPCError(#[from] Box<dyn std::error::Error>),
}
