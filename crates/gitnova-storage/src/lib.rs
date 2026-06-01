pub mod json_export;
pub mod surrealdb;
pub mod types;

pub use surrealdb::{Db, SurrealStore};
pub use types::{FileManifestEntry, StoredEmbedding};

/// Backward-compatible alias
pub type GitnovaStore = SurrealStore;
