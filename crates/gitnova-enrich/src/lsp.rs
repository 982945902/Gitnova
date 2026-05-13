use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspStatus {
    pub tool: String,
    pub available: bool,
}

pub fn detect_language_servers() -> Vec<LspStatus> {
    [
        "rust-analyzer",
        "typescript-language-server",
        "pyright-langserver",
    ]
    .into_iter()
    .map(|tool| LspStatus {
        tool: tool.to_string(),
        available: Command::new(tool).arg("--version").output().is_ok(),
    })
    .collect()
}

pub fn enrich_best_effort() -> Vec<LspStatus> {
    detect_language_servers()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_lsp_tools_are_status_not_errors() {
        let statuses = detect_language_servers();
        assert_eq!(statuses.len(), 3);
    }
}
