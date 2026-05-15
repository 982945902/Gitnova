use anyhow::{anyhow, Context, Result};
use gitnova_core::query::{self, GraphContext, ImpactAnalysis};
use gitnova_core::{CodeGraph, Node, NodeKind, Span};
use gitnova_rank::rank_graph;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

pub const ENV_LLM_API_KEY: &str = "GITNOVA_LLM_API_KEY";
pub const ENV_LLM_BASE_URL: &str = "GITNOVA_LLM_BASE_URL";
pub const ENV_LLM_MODEL: &str = "GITNOVA_LLM_MODEL";
const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

#[derive(Debug, Clone)]
pub struct OpenAiCompatibleProvider {
    api_key: Option<String>,
    base_url: String,
    model: Option<String>,
    timeout: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmAnswer {
    pub schema_version: u32,
    pub query: String,
    pub answer: String,
    pub evidence: Vec<Evidence>,
    pub llm_used: bool,
    pub provider: String,
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<GraphContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impact: Option<ImpactAnalysis>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub node_id: String,
    pub path: String,
    pub span: Option<Span>,
    pub qualified_name: String,
    pub name: String,
    pub kind: NodeKind,
    pub source_snippet: String,
}

impl OpenAiCompatibleProvider {
    pub fn new(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            api_key: Some(api_key.into()),
            base_url: base_url.into(),
            model: Some(model.into()),
            timeout: Duration::from_secs(20),
        }
    }

    pub fn from_env() -> Self {
        Self {
            api_key: non_empty_env(ENV_LLM_API_KEY),
            base_url: non_empty_env(ENV_LLM_BASE_URL).unwrap_or_else(|| DEFAULT_BASE_URL.into()),
            model: non_empty_env(ENV_LLM_MODEL),
            timeout: Duration::from_secs(20),
        }
    }

    pub fn disabled() -> Self {
        Self {
            api_key: None,
            base_url: DEFAULT_BASE_URL.into(),
            model: None,
            timeout: Duration::from_secs(20),
        }
    }

    pub fn enabled(&self) -> bool {
        self.api_key.is_some() && self.model.is_some()
    }

    pub fn model(&self) -> Option<String> {
        self.model.clone()
    }

    pub fn disabled_reason(&self) -> Option<String> {
        match (&self.api_key, &self.model) {
            (None, None) => Some(format!("{ENV_LLM_API_KEY} and {ENV_LLM_MODEL} are not set")),
            (None, Some(_)) => Some(format!("{ENV_LLM_API_KEY} is not set")),
            (Some(_), None) => Some(format!("{ENV_LLM_MODEL} is not set")),
            (Some(_), Some(_)) => None,
        }
    }

    pub fn complete(&self, system: &str, user: &str) -> Result<String> {
        let api_key = self
            .api_key
            .as_deref()
            .ok_or_else(|| anyhow!("{ENV_LLM_API_KEY} is not set"))?;
        let model = self
            .model
            .as_deref()
            .ok_or_else(|| anyhow!("{ENV_LLM_MODEL} is not set"))?;
        let endpoint = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let response: Value = reqwest::blocking::Client::builder()
            .timeout(self.timeout)
            .build()?
            .post(endpoint)
            .bearer_auth(api_key)
            .json(&json!({
                "model": model,
                "temperature": 0.2,
                "messages": [
                    {"role": "system", "content": system},
                    {"role": "user", "content": user}
                ]
            }))
            .send()
            .context("LLM request failed")?
            .error_for_status()
            .context("LLM provider returned an error status")?
            .json()
            .context("LLM provider returned invalid JSON")?;
        let content = response
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("message"))
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|content| !content.is_empty())
            .ok_or_else(|| anyhow!("LLM provider returned an empty answer"))?;
        Ok(content.to_string())
    }
}

pub fn enabled() -> bool {
    OpenAiCompatibleProvider::from_env().enabled()
}

pub fn answer_with_context(
    graph: &CodeGraph,
    question: &str,
    depth: usize,
    limit: usize,
) -> LlmAnswer {
    let provider = OpenAiCompatibleProvider::from_env();
    answer_with_context_with_provider(graph, question, depth, limit, &provider)
}

pub fn answer_with_context_with_provider(
    graph: &CodeGraph,
    question: &str,
    depth: usize,
    limit: usize,
    provider: &OpenAiCompatibleProvider,
) -> LlmAnswer {
    let context = ranked_context(graph, question, depth, limit);
    let evidence = context
        .nodes
        .iter()
        .filter(|node| !matches!(node.kind, NodeKind::Repository | NodeKind::Import))
        .map(evidence_from_node)
        .collect::<Vec<_>>();
    let fallback = fallback_context_answer(question, &context, &evidence);
    let user_prompt = answer_user_prompt(question, &fallback, &evidence);
    complete_or_fallback(
        LlmAnswerDraft {
            schema_version: graph.schema_version,
            question: question.to_string(),
            evidence,
            context: Some(context),
            impact: None,
            fallback_answer: fallback,
            user_prompt,
        },
        provider,
    )
}

pub fn llm_explain_node(
    graph: &CodeGraph,
    selector: &str,
    depth: usize,
    limit: usize,
) -> LlmAnswer {
    let provider = OpenAiCompatibleProvider::from_env();
    llm_explain_node_with_provider(graph, selector, depth, limit, &provider)
}

pub fn llm_explain_node_with_provider(
    graph: &CodeGraph,
    selector: &str,
    depth: usize,
    limit: usize,
    provider: &OpenAiCompatibleProvider,
) -> LlmAnswer {
    let context = query::graph_context(graph, selector, depth, limit);
    let evidence = context
        .nodes
        .iter()
        .filter(|node| !matches!(node.kind, NodeKind::Repository | NodeKind::Import))
        .map(evidence_from_node)
        .collect::<Vec<_>>();
    let question = format!("Explain {selector}");
    let fallback = fallback_context_answer(&question, &context, &evidence);
    let user_prompt = answer_user_prompt(&question, &fallback, &evidence);
    complete_or_fallback(
        LlmAnswerDraft {
            schema_version: graph.schema_version,
            question,
            evidence,
            context: Some(context),
            impact: None,
            fallback_answer: fallback,
            user_prompt,
        },
        provider,
    )
}

pub fn llm_impact_summary(graph: &CodeGraph, selector: &str, limit: usize) -> LlmAnswer {
    let provider = OpenAiCompatibleProvider::from_env();
    llm_impact_summary_with_provider(graph, selector, limit, &provider)
}

pub fn llm_impact_summary_with_provider(
    graph: &CodeGraph,
    selector: &str,
    limit: usize,
    provider: &OpenAiCompatibleProvider,
) -> LlmAnswer {
    let impact = query::impact_analysis(graph, selector, limit);
    let mut evidence = Vec::new();
    if let Some(symbol) = &impact.symbol {
        if let Some(node) = graph.node(&symbol.id) {
            evidence.push(evidence_from_node(node));
        }
    }
    for node in impact
        .impacted
        .iter()
        .filter_map(|digest| graph.node(&digest.id))
        .filter(|node| !matches!(node.kind, NodeKind::Repository | NodeKind::Import))
    {
        evidence.push(evidence_from_node(node));
    }
    let question = format!("Summarize impact for {selector}");
    let fallback = fallback_impact_answer(&impact, &evidence);
    let user_prompt = answer_user_prompt(&question, &fallback, &evidence);
    complete_or_fallback(
        LlmAnswerDraft {
            schema_version: graph.schema_version,
            question,
            evidence,
            context: None,
            impact: Some(impact),
            fallback_answer: fallback,
            user_prompt,
        },
        provider,
    )
}

fn ranked_context(graph: &CodeGraph, question: &str, depth: usize, limit: usize) -> GraphContext {
    let selector = rank_graph(graph, question, 1)
        .results
        .into_iter()
        .next()
        .map(|result| result.node.id)
        .unwrap_or_else(|| question.to_string());
    query::graph_context(graph, &selector, depth, limit)
}

struct LlmAnswerDraft {
    schema_version: u32,
    question: String,
    evidence: Vec<Evidence>,
    context: Option<GraphContext>,
    impact: Option<ImpactAnalysis>,
    fallback_answer: String,
    user_prompt: String,
}

fn complete_or_fallback(draft: LlmAnswerDraft, provider: &OpenAiCompatibleProvider) -> LlmAnswer {
    if draft.evidence.is_empty() {
        return LlmAnswer {
            schema_version: draft.schema_version,
            query: draft.question,
            answer: draft.fallback_answer,
            evidence: draft.evidence,
            llm_used: false,
            provider: "deterministic-fallback".into(),
            model: provider.model(),
            fallback_reason: Some("no code evidence found".into()),
            context: draft.context,
            impact: draft.impact,
        };
    }
    let disabled_reason = provider.disabled_reason();
    if !provider.enabled() {
        return LlmAnswer {
            schema_version: draft.schema_version,
            query: draft.question,
            answer: draft.fallback_answer,
            evidence: draft.evidence,
            llm_used: false,
            provider: "deterministic-fallback".into(),
            model: provider.model(),
            fallback_reason: disabled_reason,
            context: draft.context,
            impact: draft.impact,
        };
    }
    match provider.complete(answer_system_prompt(), &draft.user_prompt) {
        Ok(answer) => LlmAnswer {
            schema_version: draft.schema_version,
            query: draft.question,
            answer,
            evidence: draft.evidence,
            llm_used: true,
            provider: "openai-compatible".into(),
            model: provider.model(),
            fallback_reason: None,
            context: draft.context,
            impact: draft.impact,
        },
        Err(error) => LlmAnswer {
            schema_version: draft.schema_version,
            query: draft.question,
            answer: draft.fallback_answer,
            evidence: draft.evidence,
            llm_used: false,
            provider: "deterministic-fallback".into(),
            model: provider.model(),
            fallback_reason: Some(error.to_string()),
            context: draft.context,
            impact: draft.impact,
        },
    }
}

fn answer_system_prompt() -> &'static str {
    "You explain code using only the provided Gitnova evidence. Keep the answer concise. Do not mention files, functions, or behavior that are not in the evidence."
}

fn answer_user_prompt(question: &str, fallback_answer: &str, evidence: &[Evidence]) -> String {
    json!({
        "question": question,
        "fallback_answer": fallback_answer,
        "evidence": evidence,
        "instruction": "Write a short answer grounded only in fallback_answer and evidence. Do not invent facts. The API response will attach structured evidence separately."
    })
    .to_string()
}

fn fallback_context_answer(
    question: &str,
    context: &GraphContext,
    evidence: &[Evidence],
) -> String {
    let Some(target) = &context.target else {
        return format!("No code evidence was found for `{question}`.");
    };
    let mut parts = vec![format!(
        "The strongest evidence for `{question}` is `{}` in `{}`.",
        target.qualified_name, target.path
    )];
    if let Some(summary) = &context.summary {
        parts.push(summary.clone());
    }
    let related = evidence
        .iter()
        .filter(|item| item.node_id != target.id)
        .take(3)
        .map(|item| format!("`{}`", item.qualified_name))
        .collect::<Vec<_>>();
    if !related.is_empty() {
        parts.push(format!("Related evidence includes {}.", related.join(", ")));
    }
    parts.join(" ")
}

fn fallback_impact_answer(impact: &ImpactAnalysis, evidence: &[Evidence]) -> String {
    let Some(symbol) = &impact.symbol else {
        return "No impact evidence was found for the requested symbol.".into();
    };
    let impacted = impact
        .impacted
        .iter()
        .take(5)
        .map(|node| format!("`{}`", node.qualified_name))
        .collect::<Vec<_>>();
    if impacted.is_empty() {
        return format!(
            "`{}` has no reverse-dependency impact in the indexed graph.",
            symbol.qualified_name
        );
    }
    format!(
        "Changing `{}` may affect {}. The response includes {} evidence item(s).",
        symbol.qualified_name,
        impacted.join(", "),
        evidence.len()
    )
}

fn evidence_from_node(node: &Node) -> Evidence {
    Evidence {
        node_id: node.id.clone(),
        path: node.path.clone(),
        span: node.span,
        qualified_name: node.qualified_name.clone(),
        name: node.name.clone(),
        kind: node.kind.clone(),
        source_snippet: node.text.clone(),
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitnova_core::build_graph;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn answer_with_context_falls_back_with_evidence_without_provider() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(
            temp.path().join("auth.ts"),
            "export interface Session { userId: string; expiresAt: Date; }\nexport function validateSession(session: Session): boolean { return session.userId.length > 0 && session.expiresAt > new Date(); }\n",
        )
        .unwrap();
        let graph = build_graph(temp.path()).unwrap();
        let answer = answer_with_context_with_provider(
            &graph,
            "Where is auth session validated?",
            1,
            20,
            &OpenAiCompatibleProvider::disabled(),
        );
        assert!(!answer.llm_used);
        assert!(answer.fallback_reason.is_some());
        assert!(answer
            .evidence
            .iter()
            .any(|item| item.qualified_name.contains("validateSession")));
        assert!(answer.answer.contains("validateSession"));
    }

    #[test]
    fn impact_summary_prefers_real_symbol_evidence() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(
            temp.path().join("utils.ts"),
            "export function formatDate(value: Date): string { return value.toISOString(); }\n",
        )
        .unwrap();
        fs::write(
            temp.path().join("report.ts"),
            "import { formatDate } from './utils';\nexport function renderReport(date: Date): string { return formatDate(date); }\n",
        )
        .unwrap();
        let graph = build_graph(temp.path()).unwrap();
        let answer = llm_impact_summary_with_provider(
            &graph,
            "formatDate",
            5,
            &OpenAiCompatibleProvider::disabled(),
        );
        assert_eq!(answer.evidence[0].qualified_name, "utils.ts::formatDate");
        assert!(answer
            .impact
            .as_ref()
            .unwrap()
            .symbol
            .as_ref()
            .unwrap()
            .qualified_name
            .contains("formatDate"));
    }

    #[test]
    fn provider_completes_against_openai_compatible_chat_endpoint() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            assert!(request.contains("POST /chat/completions HTTP/1.1"));
            assert!(request.contains("authorization: Bearer test-key"));
            assert!(request.contains("\"model\":\"test-model\""));
            let body = json!({
                "choices": [
                    {"message": {"content": "Evidence-backed answer"}}
                ]
            })
            .to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        let provider =
            OpenAiCompatibleProvider::new("test-key", format!("http://{address}"), "test-model");
        assert!(provider.enabled());
        let answer = provider.complete("system", "user").unwrap();
        handle.join().unwrap();
        assert_eq!(answer, "Evidence-backed answer");
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> String {
        let mut buffer = Vec::new();
        let mut temp = [0u8; 512];
        loop {
            let read = stream.read(&mut temp).unwrap();
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&temp[..read]);
            let request = String::from_utf8_lossy(&buffer);
            let Some(header_end) = request.find("\r\n\r\n") else {
                continue;
            };
            let content_length = request
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            let body_start = header_end + 4;
            if buffer.len().saturating_sub(body_start) >= content_length {
                return request.into_owned();
            }
        }
        String::from_utf8(buffer).unwrap()
    }
}
