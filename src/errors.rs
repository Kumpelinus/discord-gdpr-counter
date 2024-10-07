use thiserror::Error;

#[derive(Error, Debug)]
pub enum MyError {
    #[error("IO error occurred: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON parsing error: {0}")]
    Json(#[from] serde_json::Error),

    #[cfg(feature = "zip")]
    #[error("Failed to process ZIP archive: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("Invalid input path: {0}")]
    InvalidInputPath(String),

    #[cfg(feature = "zip")]
    #[error("Failed to create temporary directory: {0}")]
    TempDir(#[from] tempfile::PersistError),

    #[error("Error in progress bar: {0}")]
    ProgressBar(String),
}
