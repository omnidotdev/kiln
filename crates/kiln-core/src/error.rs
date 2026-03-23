use std::path::PathBuf;

/// Errors that can occur during detection, planning, or Dockerfile generation.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no provider detected for project at {0}")]
    NoProviderDetected(PathBuf),

    #[error("failed to read {path}: {source}")]
    ReadFile { path: PathBuf, source: std::io::Error },

    #[error("failed to parse {path}: {message}")]
    Parse { path: PathBuf, message: String },

    #[error("{0}")]
    Provider(String),
}

pub type Result<T> = std::result::Result<T, Error>;
