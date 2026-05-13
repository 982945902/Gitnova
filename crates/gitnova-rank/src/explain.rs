use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreExplanation {
    pub strong_signals: Vec<String>,
    pub weak_signals: Vec<String>,
    pub penalties: Vec<String>,
}

impl ScoreExplanation {
    pub fn new() -> Self {
        Self {
            strong_signals: Vec::new(),
            weak_signals: Vec::new(),
            penalties: Vec::new(),
        }
    }
}

impl Default for ScoreExplanation {
    fn default() -> Self {
        Self::new()
    }
}
