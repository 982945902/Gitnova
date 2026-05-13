pub mod error;
pub mod extract;
pub mod graph;
pub mod language;
pub mod model;
pub mod parser;
pub mod query;
pub mod scan;

pub use error::{GitnovaError, Result};
pub use graph::{build_graph, build_graph_from_entries};
pub use model::*;
pub use scan::{scan_repository, SourceFile};

use std::path::Path;

pub fn index_repository(root: impl AsRef<Path>) -> Result<CodeGraph> {
    let root = root.as_ref();
    let files = scan_repository(root)?;
    build_graph_from_entries(root, &files)
}
