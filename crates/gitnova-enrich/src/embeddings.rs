use gitnova_core::{CodeGraph, NodeKind};
use gitnova_rank::features::tokenize;
use gitnova_storage::StoredEmbedding;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

pub const LOCAL_HASH_PROVIDER: &str = "local-hash";
pub const LOCAL_SEMANTIC_PROVIDER: &str = "local-semantic";
pub const NEURAL_COMMAND_PROVIDER: &str = "neural-command";
const DIMENSIONS: usize = 64;
const SEMANTIC_DIMENSIONS: usize = 96;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingSearchResult {
    pub node_id: String,
    pub qualified_name: String,
    pub path: String,
    pub similarity: f64,
}

pub fn build_local_hash_embeddings(graph: &CodeGraph) -> Vec<StoredEmbedding> {
    build_embeddings_for_provider(graph, LOCAL_HASH_PROVIDER)
}

pub fn build_embeddings(graph: &CodeGraph, provider: &str) -> anyhow::Result<Vec<StoredEmbedding>> {
    build_embeddings_with_command(graph, provider, neural_command_from_env().as_deref())
}

pub fn build_embeddings_with_command(
    graph: &CodeGraph,
    provider: &str,
    command: Option<&Path>,
) -> anyhow::Result<Vec<StoredEmbedding>> {
    match provider {
        LOCAL_HASH_PROVIDER | LOCAL_SEMANTIC_PROVIDER => {
            Ok(build_embeddings_for_provider(graph, provider))
        }
        NEURAL_COMMAND_PROVIDER => build_neural_command_embeddings(graph, command),
        other => anyhow::bail!("unsupported embedding provider: {other}"),
    }
}

fn build_embeddings_for_provider(graph: &CodeGraph, provider: &str) -> Vec<StoredEmbedding> {
    graph
        .nodes
        .iter()
        .filter(|node| !matches!(node.kind, NodeKind::Repository | NodeKind::Import))
        .map(|node| StoredEmbedding {
            node_id: node.id.clone(),
            provider: provider.into(),
            vector: embed_text_for_provider(
                provider,
                &format!(
                    "{} {} {} {:?}",
                    node.qualified_name, node.path, node.text, node.kind
                ),
            )
            .unwrap_or_else(|_| embed_text("")),
        })
        .collect()
}

pub fn similarity_map(
    graph: &CodeGraph,
    vectors: &HashMap<String, Vec<f32>>,
    query: &str,
) -> HashMap<String, f64> {
    similarity_map_for_provider(graph, vectors, query, LOCAL_HASH_PROVIDER).unwrap_or_default()
}

pub fn similarity_map_for_provider(
    graph: &CodeGraph,
    vectors: &HashMap<String, Vec<f32>>,
    query: &str,
    provider: &str,
) -> anyhow::Result<HashMap<String, f64>> {
    similarity_map_for_provider_with_command(
        graph,
        vectors,
        query,
        provider,
        neural_command_from_env().as_deref(),
    )
}

pub fn similarity_map_for_provider_with_command(
    graph: &CodeGraph,
    vectors: &HashMap<String, Vec<f32>>,
    query: &str,
    provider: &str,
    command: Option<&Path>,
) -> anyhow::Result<HashMap<String, f64>> {
    let query_vector = if provider == NEURAL_COMMAND_PROVIDER {
        embed_text_with_neural_command(query, command)?
    } else {
        embed_text_for_provider(provider, query)?
    };
    Ok(graph
        .nodes
        .iter()
        .filter_map(|node| {
            vectors
                .get(&node.id)
                .map(|vector| (node.id.clone(), cosine(&query_vector, vector)))
        })
        .collect())
}

pub fn search_embeddings_with_provider(
    graph: &CodeGraph,
    vectors: &HashMap<String, Vec<f32>>,
    query: &str,
    provider: &str,
    limit: usize,
) -> anyhow::Result<Vec<EmbeddingSearchResult>> {
    search_embeddings_with_command(
        graph,
        vectors,
        query,
        provider,
        neural_command_from_env().as_deref(),
        limit,
    )
}

pub fn search_embeddings_with_command(
    graph: &CodeGraph,
    vectors: &HashMap<String, Vec<f32>>,
    query: &str,
    provider: &str,
    command: Option<&Path>,
    limit: usize,
) -> anyhow::Result<Vec<EmbeddingSearchResult>> {
    let scores =
        similarity_map_for_provider_with_command(graph, vectors, query, provider, command)?;
    Ok(search_from_scores(graph, &scores, limit))
}

pub fn search_embeddings(
    graph: &CodeGraph,
    vectors: &HashMap<String, Vec<f32>>,
    query: &str,
    limit: usize,
) -> Vec<EmbeddingSearchResult> {
    let query_vector = embed_text(query);
    let scores = graph
        .nodes
        .iter()
        .filter_map(|node| {
            vectors
                .get(&node.id)
                .map(|vector| (node.id.clone(), cosine(&query_vector, vector)))
        })
        .collect::<HashMap<_, _>>();
    search_from_scores(graph, &scores, limit)
}

fn search_from_scores(
    graph: &CodeGraph,
    scores: &HashMap<String, f64>,
    limit: usize,
) -> Vec<EmbeddingSearchResult> {
    let mut results = graph
        .nodes
        .iter()
        .filter_map(|node| {
            scores.get(&node.id).map(|score| EmbeddingSearchResult {
                node_id: node.id.clone(),
                qualified_name: node.qualified_name.clone(),
                path: node.path.clone(),
                similarity: *score,
            })
        })
        .collect::<Vec<_>>();
    results.sort_by(|a, b| {
        b.similarity
            .partial_cmp(&a.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(limit);
    results
}

pub fn embed_text_for_provider(provider: &str, text: &str) -> anyhow::Result<Vec<f32>> {
    match provider {
        LOCAL_HASH_PROVIDER => Ok(embed_text(text)),
        LOCAL_SEMANTIC_PROVIDER => Ok(embed_semantic_text(text)),
        NEURAL_COMMAND_PROVIDER => {
            embed_text_with_neural_command(text, neural_command_from_env().as_deref())
        }
        other => anyhow::bail!("unsupported embedding provider: {other}"),
    }
}

fn build_neural_command_embeddings(
    graph: &CodeGraph,
    command: Option<&Path>,
) -> anyhow::Result<Vec<StoredEmbedding>> {
    graph
        .nodes
        .iter()
        .filter(|node| !matches!(node.kind, NodeKind::Repository | NodeKind::Import))
        .map(|node| {
            let text = format!(
                "{} {} {} {:?}",
                node.qualified_name, node.path, node.text, node.kind
            );
            Ok(StoredEmbedding {
                node_id: node.id.clone(),
                provider: NEURAL_COMMAND_PROVIDER.into(),
                vector: embed_text_with_neural_command(&text, command)?,
            })
        })
        .collect()
}

fn embed_text_with_neural_command(text: &str, command: Option<&Path>) -> anyhow::Result<Vec<f32>> {
    let command = command.ok_or_else(|| {
        anyhow::anyhow!(
            "neural-command provider requires GITNOVA_EMBEDDING_COMMAND or an explicit command"
        )
    })?;
    let mut child = Command::new(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(serde_json::json!({ "text": text }).to_string().as_bytes())?;
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        anyhow::bail!(
            "neural embedding command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    parse_vector_output(&output.stdout)
}

fn parse_vector_output(bytes: &[u8]) -> anyhow::Result<Vec<f32>> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    let vector_value = value.get("vector").unwrap_or(&value);
    let vector = serde_json::from_value::<Vec<f32>>(vector_value.clone())?;
    if vector.is_empty() {
        anyhow::bail!("neural embedding command returned an empty vector");
    }
    Ok(normalize(vector))
}

fn neural_command_from_env() -> Option<std::path::PathBuf> {
    std::env::var_os("GITNOVA_EMBEDDING_COMMAND").map(std::path::PathBuf::from)
}

pub fn embed_text(text: &str) -> Vec<f32> {
    let mut vector = vec![0.0f32; DIMENSIONS];
    for token in tokenize(text) {
        let digest = Sha256::digest(token.as_bytes());
        let index = digest[0] as usize % DIMENSIONS;
        let sign = if digest[1] % 2 == 0 { 1.0 } else { -1.0 };
        vector[index] += sign;
    }
    normalize(vector)
}

fn embed_semantic_text(text: &str) -> Vec<f32> {
    let mut vector = vec![0.0f32; SEMANTIC_DIMENSIONS];
    for token in tokenize(text) {
        for (dimension, weight) in semantic_dimensions(&token) {
            vector[dimension] += weight;
        }
        let digest = Sha256::digest(token.as_bytes());
        let index = 32 + (digest[0] as usize % (SEMANTIC_DIMENSIONS - 32));
        vector[index] += 0.25;
    }
    normalize(vector)
}

fn semantic_dimensions(token: &str) -> Vec<(usize, f32)> {
    let mut dims = semantic_direct_dimensions(token);
    for expanded in semantic_expansions(token) {
        if *expanded != token {
            dims.extend(
                semantic_direct_dimensions(expanded)
                    .into_iter()
                    .map(|(dim, weight)| (dim, weight * 0.55)),
            );
        }
    }
    if dims.is_empty() {
        let digest = Sha256::digest(token.as_bytes());
        dims.push((8 + (digest[0] as usize % 24), 0.35));
    }
    dims
}

fn semantic_direct_dimensions(token: &str) -> Vec<(usize, f32)> {
    let mut dims = Vec::new();
    if matches!(
        token,
        "auth"
            | "authenticate"
            | "authentication"
            | "authorize"
            | "authorization"
            | "login"
            | "signin"
            | "signon"
            | "session"
            | "token"
            | "credential"
            | "permission"
            | "permissions"
            | "access"
            | "validate"
            | "valid"
            | "user"
            | "account"
    ) {
        dims.push((0, 1.0));
    }
    if matches!(token, "session" | "token" | "cookie" | "jwt") {
        dims.push((1, 0.8));
    }
    if matches!(
        token,
        "date" | "time" | "format" | "calendar" | "timezone" | "timestamp"
    ) {
        dims.push((2, 1.0));
    }
    if matches!(token, "report" | "invoice" | "billing" | "render" | "label") {
        dims.push((3, 1.0));
    }
    if matches!(
        token,
        "config" | "setting" | "option" | "env" | "environment"
    ) {
        dims.push((4, 1.0));
    }
    if matches!(
        token,
        "db" | "database" | "store" | "storage" | "sqlite" | "cache"
    ) {
        dims.push((5, 1.0));
    }
    if matches!(
        token,
        "http" | "api" | "route" | "request" | "response" | "server"
    ) {
        dims.push((6, 1.0));
    }
    if matches!(token, "test" | "spec" | "mock" | "fixture") {
        dims.push((7, 1.0));
    }
    dims
}

fn semantic_expansions(token: &str) -> &'static [&'static str] {
    match token {
        "signin" | "login" | "signon" => &["auth", "session", "validate"],
        "permissions" | "permission" | "access" => &["auth", "authorize", "validate"],
        "validate" | "valid" | "validation" => &["auth", "session", "token"],
        "session" => &["auth", "login", "token"],
        "token" | "jwt" => &["auth", "session", "credential"],
        "credential" | "credentials" => &["auth", "login"],
        "format" => &["date", "time"],
        "invoice" => &["billing", "report"],
        _ => &[],
    }
}

fn normalize(mut vector: Vec<f32>) -> Vec<f32> {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    }
    vector
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(left, right)| *left as f64 * *right as f64)
        .sum::<f64>()
        .max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitnova_core::build_graph;
    use std::fs;

    #[test]
    fn local_hash_embeddings_are_stable() {
        assert_eq!(embed_text("auth validate"), embed_text("auth validate"));
    }

    #[test]
    fn embedding_search_returns_stable_results() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(
            temp.path().join("auth.rs"),
            "pub fn validate_token() -> bool { true }\n",
        )
        .unwrap();
        let graph = build_graph(temp.path()).unwrap();
        let embeddings = build_local_hash_embeddings(&graph);
        let vectors = embeddings
            .into_iter()
            .map(|embedding| (embedding.node_id, embedding.vector))
            .collect();
        let results = search_embeddings(&graph, &vectors, "validate token", 1);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn semantic_embeddings_match_domain_synonyms_without_exact_tokens() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(
            temp.path().join("auth.ts"),
            r#"
export function validateSession(session: Session) {
  return Boolean(session.token);
}

export function formatDate(value: Date) {
  return value.toISOString();
}
"#,
        )
        .unwrap();
        let graph = build_graph(temp.path()).unwrap();
        let embeddings = build_embeddings(&graph, LOCAL_SEMANTIC_PROVIDER).unwrap();
        let vectors = embeddings
            .into_iter()
            .map(|embedding| (embedding.node_id, embedding.vector))
            .collect();
        let results = search_embeddings_with_provider(
            &graph,
            &vectors,
            "signin permissions",
            LOCAL_SEMANTIC_PROVIDER,
            1,
        )
        .unwrap();

        assert_eq!(results[0].qualified_name, "auth.ts::validateSession");
    }

    #[test]
    fn neural_command_provider_uses_external_model_vectors() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(
            temp.path().join("embedder.sh"),
            r#"#!/bin/sh
input=$(cat)
case "$input" in
  *File*) echo '[0.0,0.0,1.0]' ;;
  *validateSession*|*signin*) echo '[1.0,0.0,0.0]' ;;
  *formatDate*|*calendar*) echo '[0.0,1.0,0.0]' ;;
  *) echo '[0.0,0.0,1.0]' ;;
esac
"#,
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let script = temp.path().join("embedder.sh");
            let mut perms = fs::metadata(&script).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script, perms).unwrap();
        }
        fs::write(
            temp.path().join("auth.ts"),
            "export function validateSession() { return true; }\nexport function formatDate() { return ''; }\n",
        )
        .unwrap();
        let graph = build_graph(temp.path()).unwrap();
        let command = temp.path().join("embedder.sh");
        let embeddings =
            build_embeddings_with_command(&graph, NEURAL_COMMAND_PROVIDER, Some(&command)).unwrap();
        let vectors = embeddings
            .into_iter()
            .map(|embedding| (embedding.node_id, embedding.vector))
            .collect();
        let results = search_embeddings_with_command(
            &graph,
            &vectors,
            "signin",
            NEURAL_COMMAND_PROVIDER,
            Some(&command),
            1,
        )
        .unwrap();

        assert_eq!(results[0].qualified_name, "auth.ts::validateSession");
    }
}
