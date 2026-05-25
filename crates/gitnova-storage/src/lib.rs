pub mod json_export;
pub mod sqlite;
pub mod surrealdb;

pub use sqlite::{FileManifestEntry, StoredEmbedding};
pub use surrealdb::{Db, SurrealStore};

/// Backward-compatible alias
pub type GitnovaStore = SurrealStore;
