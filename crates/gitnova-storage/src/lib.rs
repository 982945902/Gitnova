pub mod json_export;
pub mod sqlite;
pub mod surrealdb;

<<<<<<< HEAD
pub use sqlite::{FileManifestEntry, StoredEmbedding};
pub use surrealdb::{Db, SurrealStore};

/// Backward-compatible alias
=======
pub use fts::{FtsStore, FtsSymbol};
pub use sqlite::{FileManifestEntry, StoredEmbedding};
pub use surrealdb::{Db, SurrealStore};

/// Current production store backed by SurrealDB v2.
>>>>>>> origin/worktree-agent-a617ffcd
pub type GitnovaStore = SurrealStore;
