pub mod fts;
pub mod json_export;
pub mod migrations;
pub mod sqlite;
pub mod surrealdb;

pub use fts::{FtsStore, FtsSymbol};
pub use sqlite::{FileManifestEntry, StoredEmbedding};
pub use surrealdb::{Db, SurrealStore};

/// Current production store backed by SurrealDB v2.
pub type GitnovaStore = SurrealStore;
