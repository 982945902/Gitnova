use thiserror::Error;

pub type Result<T> = std::result::Result<T, GitnovaError>;

#[derive(Debug, Error)]
pub enum GitnovaError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("tree-sitter error: {0}")]
    TreeSitter(String),
    #[error("unsupported language for {0}")]
    UnsupportedLanguage(String),
    #[error("{0}")]
    Other(String),
}
