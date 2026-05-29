//! HTML-as-index wiki tree storage for agent-generated code understanding.

mod store;
mod types;

pub use store::WikiStore;
pub use types::{ContentFormat, Evidence, PageKind, PatchMode, WikiPage, WikiSchema};
