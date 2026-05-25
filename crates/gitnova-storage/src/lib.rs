pub mod json_export;
pub mod types;
pub mod surrealdb;

pub use surrealdb::{Db, SurrealStore};
pub use types::{FileManifestEntry, StoredEmbedding};

/// Backward-compatible alias
pub type GitnovaStore = SurrealStore;
