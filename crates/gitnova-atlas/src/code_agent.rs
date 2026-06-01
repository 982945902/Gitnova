use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InvestigationTask {
    pub task_id: String,
    pub question: String,
    pub repo_path: PathBuf,
    pub scope: InvestigationScope,
    pub expected_outputs: Vec<String>,
    pub budget: InvestigationBudget,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InvestigationScope {
    pub paths: Vec<String>,
    pub symbols: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InvestigationBudget {
    pub max_nodes: usize,
    pub max_depth: usize,
    pub timeout_secs: u64,
}

impl Default for InvestigationBudget {
    fn default() -> Self {
        Self {
            max_nodes: 80,
            max_depth: 4,
            timeout_secs: 120,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InvestigationResult {
    pub task_id: String,
    pub summary_markdown: String,
    pub sources: Vec<InvestigationSource>,
    pub diagram: Option<InvestigationDiagram>,
    pub followups: Vec<String>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InvestigationSource {
    pub file: String,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InvestigationDiagram {
    pub format: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct CodexCliAgent {
    codex_bin: PathBuf,
    model: Option<String>,
    codex_home: Option<PathBuf>,
}

impl CodexCliAgent {
    pub fn new(codex_bin: impl AsRef<Path>) -> Self {
        Self {
            codex_bin: codex_bin.as_ref().to_path_buf(),
            model: None,
            codex_home: None,
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn with_codex_home(mut self, codex_home: impl AsRef<Path>) -> Self {
        self.codex_home = Some(codex_home.as_ref().to_path_buf());
        self
    }

    pub fn investigate(&self, task: &InvestigationTask) -> Result<InvestigationResult> {
        let temp = tempfile::TempDir::new().context("create atlas codex temp dir")?;
        let schema_path = temp.path().join("investigation-result.schema.json");
        let output_path = temp.path().join("investigation-result.json");
        let stdout_path = temp.path().join("codex.stdout.log");
        let stderr_path = temp.path().join("codex.stderr.log");
        std::fs::write(&schema_path, INVESTIGATION_RESULT_SCHEMA)
            .context("write investigation result schema")?;

        let prompt = render_prompt(task)?;
        let mut command = Command::new(&self.codex_bin);
        command
            .arg("exec")
            .arg("--ignore-user-config")
            .arg("--ephemeral")
            .arg("-C")
            .arg(&task.repo_path)
            .arg("-s")
            .arg("read-only")
            .arg("--output-schema")
            .arg(&schema_path)
            .arg("-o")
            .arg(&output_path);
        if let Some(model) = &self.model {
            command.arg("-m").arg(model);
        }
        command.arg("-");
        if let Some(codex_home) = &self.codex_home {
            command.env("CODEX_HOME", codex_home);
        }
        command
            .env("GITNOVA_ATLAS_CHILD", "1")
            .env("GITNOVA_DISABLE_MCP_RECURSION", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::from(File::create(&stdout_path)?))
            .stderr(Stdio::from(File::create(&stderr_path)?));

        let mut child = command
            .spawn()
            .with_context(|| format!("spawn atlas child codex: {}", self.codex_bin.display()))?;
        child
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow!("atlas child codex stdin unavailable"))?
            .write_all(prompt.as_bytes())
            .context("write atlas child codex prompt")?;
        drop(child.stdin.take());

        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().context("poll atlas child codex process")? {
                break status;
            }
            if started.elapsed() >= Duration::from_secs(task.budget.timeout_secs.max(1)) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(anyhow!(
                    "codex exec timed out after {}s for task {}",
                    task.budget.timeout_secs.max(1),
                    task.task_id
                ));
            }
            thread::sleep(Duration::from_millis(50));
        };
        if !status.success() {
            let stderr = std::fs::read_to_string(&stderr_path).unwrap_or_default();
            return Err(anyhow!(
                "codex exec failed with status {}: {}",
                status,
                stderr.trim()
            ));
        }

        let raw = std::fs::read_to_string(&output_path)
            .with_context(|| format!("read atlas result: {}", output_path.display()))?;
        let result: InvestigationResult =
            serde_json::from_str(raw.trim()).context("parse atlas investigation JSON")?;
        if result.task_id != task.task_id {
            return Err(anyhow!(
                "atlas result task_id mismatch: expected {}, got {}",
                task.task_id,
                result.task_id
            ));
        }
        Ok(result)
    }
}

pub fn investigate_many(
    agent: CodexCliAgent,
    tasks: Vec<InvestigationTask>,
    parallelism: usize,
) -> Result<Vec<InvestigationResult>> {
    if tasks.is_empty() {
        return Ok(Vec::new());
    }
    let parallelism = parallelism.max(1);
    let mut results = Vec::with_capacity(tasks.len());
    for chunk in tasks.chunks(parallelism) {
        let handles = chunk
            .iter()
            .cloned()
            .map(|task| {
                let agent = agent.clone();
                thread::spawn(move || agent.investigate(&task))
            })
            .collect::<Vec<_>>();
        for handle in handles {
            let result = handle
                .join()
                .map_err(|_| anyhow!("atlas child investigation thread panicked"))??;
            results.push(result);
        }
    }
    Ok(results)
}

fn render_prompt(task: &InvestigationTask) -> Result<String> {
    let scope = serde_json::to_string_pretty(&task.scope)?;
    let expected_outputs = serde_json::to_string_pretty(&task.expected_outputs)?;
    let source_context = scoped_source_context(task, 48_000)?;
    Ok(format!(
        r#"You are Atlas, a single-task code investigator for Gitnova.

task_id: {task_id}
question: {question}

Scope:
{scope}

Preloaded source context:
{source_context}

Expected outputs:
{expected_outputs}

Budget:
- max_nodes: {max_nodes}
- max_depth: {max_depth}
- timeout_secs: {timeout_secs}

Rules:
- Do not edit files.
- Do not call Gitnova MCP tools.
- Do not call wiki_deepen_page, wiki_expand_tree, or any recursive workflow.
- Investigate only this task.
- Stay inside the scoped paths and symbols unless a nearby reference is essential.
- If the task is too broad for the budget, return a narrow partial answer and put the rest in followups.
- Prefer the preloaded source context when it is sufficient; use shell read/search commands only for missing details.
- Use shell read/search commands as needed.
- Every important claim should have source spans when possible.
- Return only JSON matching the provided schema.
"#,
        task_id = task.task_id,
        question = task.question,
        max_nodes = task.budget.max_nodes,
        max_depth = task.budget.max_depth,
        timeout_secs = task.budget.timeout_secs,
    ))
}

fn scoped_source_context(task: &InvestigationTask, max_chars: usize) -> Result<String> {
    let mut remaining = max_chars;
    let mut out = String::new();
    for path in &task.scope.paths {
        if remaining == 0 {
            break;
        }
        let full_path = task.repo_path.join(path);
        if !full_path.is_file() {
            continue;
        }
        let raw = std::fs::read_to_string(&full_path)
            .with_context(|| format!("read scoped source file: {}", full_path.display()))?;
        let excerpt = truncate_at_char_boundary(&raw, remaining);
        out.push_str("\n--- file: ");
        out.push_str(path);
        out.push_str(" ---\n");
        out.push_str(&numbered_lines(excerpt));
        remaining = remaining.saturating_sub(excerpt.len());
        if raw.len() > excerpt.len() {
            out.push_str("\n--- truncated ---\n");
            remaining = 0;
        }
    }
    if out.is_empty() {
        Ok("(no scoped source files were preloaded)".to_string())
    } else {
        Ok(out)
    }
}

fn truncate_at_char_boundary(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }

    let mut end = 0;
    for (index, _) in value.char_indices() {
        if index > max_bytes {
            break;
        }
        end = index;
    }
    &value[..end]
}

fn numbered_lines(content: &str) -> String {
    content
        .lines()
        .enumerate()
        .map(|(index, line)| format!("{:>5}: {line}", index + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

const INVESTIGATION_RESULT_SCHEMA: &str = r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "additionalProperties": false,
  "required": ["task_id", "summary_markdown", "sources", "diagram", "followups", "confidence"],
  "properties": {
    "task_id": { "type": "string" },
    "summary_markdown": { "type": "string" },
    "sources": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["file", "start_line", "end_line", "note"],
        "properties": {
          "file": { "type": "string" },
          "start_line": { "type": ["integer", "null"], "minimum": 1 },
          "end_line": { "type": ["integer", "null"], "minimum": 1 },
          "note": { "type": ["string", "null"] }
        }
      }
    },
    "diagram": {
      "type": ["object", "null"],
      "additionalProperties": false,
      "required": ["format", "content"],
      "properties": {
        "format": { "type": "string", "enum": ["mermaid", "svg", "none"] },
        "content": { "type": "string" }
      }
    },
    "followups": { "type": "array", "items": { "type": "string" } },
    "confidence": { "type": "number", "minimum": 0, "maximum": 1 }
  }
}"#;
