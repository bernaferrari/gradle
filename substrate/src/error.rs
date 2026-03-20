use thiserror::Error;

#[derive(Error, Debug)]
pub enum SubstrateError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Hash error: {0}")]
    Hash(String),

    #[error("Process error: {0}")]
    Process(String),

    #[error("Cache error: {0}")]
    Cache(String),

    #[error("gRPC error: {0}")]
    Grpc(#[from] tonic::transport::Error),

    #[error("Daemon already running at {0}")]
    AlreadyRunning(String),
}

impl From<SubstrateError> for tonic::Status {
    fn from(err: SubstrateError) -> Self {
        tonic::Status::internal(err.to_string())
    }
}
