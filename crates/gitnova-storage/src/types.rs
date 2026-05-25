use gitnova_core::model::Language;
use serde::{Deserialize, Serialize};

/// File manifest entry retained for backward compatibility with the SurrealStore API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileManifestEntry {
    pub path: String,
    pub content_hash: String,
    pub language: Language,
    pub indexed_at_unix: u64,
}

/// Serialized embedding stored for semantic search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredEmbedding {
    pub node_id: String,
    pub provider: String,
    pub vector: Vec<f32>,
}
