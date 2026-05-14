use crate::{resources, tools};
use rmcp::model::{
    Annotated, CallToolRequestParams, CallToolResult, Content, JsonObject, ListResourcesResult,
    ListToolsResult, PaginatedRequestParams, RawResource, ReadResourceRequestParams,
    ReadResourceResult, Resource, ResourceContents, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};
use serde_json::{json, Value};
use std::borrow::Cow;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const TOOL_DEFINITIONS: &[(&str, &str)] = &[
    ("index_project", "Index a repository into Gitnova storage"),
    ("rank_context", "Rank code context by salience"),
    (
        "explain_symbol",
        "Explain a symbol and its graph neighborhood",
    ),
    ("impact_analysis", "Find reverse dependencies for a symbol"),
    ("architecture_map", "Summarize repository architecture"),
    ("watch_project", "Start a managed repository watcher"),
    ("watch_status", "Report a managed watcher's current state"),
    ("stop_watch", "Stop a managed watcher by watch_id"),
    (
        "diff_context",
        "Rank context from git diff metadata and hunks",
    ),
    ("search_embeddings", "Search persisted embeddings"),
];

const RESOURCE_DEFINITIONS: &[(&str, &str)] = &[
    ("gitnova://graph/summary", "Graph summary"),
    ("gitnova://graph/nodes", "Graph nodes"),
    ("gitnova://graph/edges", "Graph edges"),
    ("gitnova://graph/hubs", "High-degree graph hubs"),
    ("gitnova://graph/recent-churn", "Recently changed nodes"),
    (
        "gitnova://graph/dashboard-url",
        "Default local dashboard URL",
    ),
];

#[derive(Debug, Clone)]
pub struct GitnovaRmcpServer {
    repo_state: Arc<Mutex<PathBuf>>,
}

impl GitnovaRmcpServer {
    pub fn new(repo_root: PathBuf) -> Self {
        Self {
            repo_state: Arc::new(Mutex::new(repo_root)),
        }
    }

    pub fn tool_names(&self) -> Vec<String> {
        TOOL_DEFINITIONS
            .iter()
            .map(|(name, _)| (*name).to_string())
            .collect()
    }
}

pub async fn serve_stdio(repo_root: PathBuf) -> anyhow::Result<()> {
    let service = GitnovaRmcpServer::new(repo_root)
        .serve(rmcp::transport::stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}

impl ServerHandler for GitnovaRmcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("Local-first code intelligence for Git repositories".into()),
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
            ..Default::default()
        }
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        std::future::ready(Ok(ListToolsResult::with_all_items(
            TOOL_DEFINITIONS
                .iter()
                .map(|(name, description)| tool(name, description))
                .collect(),
        )))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResult, McpError>> + Send + '_ {
        let repo_state = self.repo_state.clone();
        async move {
            let args = request
                .arguments
                .map(Value::Object)
                .unwrap_or_else(|| json!({}));
            let output = {
                let mut repo_state = repo_state.lock().unwrap();
                tools::call_tool(request.name.as_ref(), &args, &mut repo_state)
            }
            .map_err(to_mcp_error)?;
            let text = serde_json::to_string(&output).map_err(to_mcp_error)?;
            Ok(CallToolResult::success(vec![Content::text(text)]))
        }
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        std::future::ready(Ok(ListResourcesResult::with_all_items(
            RESOURCE_DEFINITIONS
                .iter()
                .map(|(uri, name)| resource(uri, name))
                .collect(),
        )))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResult, McpError>> + Send + '_ {
        let repo_state = self.repo_state.clone();
        async move {
            let repo = repo_state.lock().unwrap().clone();
            let value = resources::read_resource(repo, &request.uri).map_err(to_mcp_error)?;
            let text = serde_json::to_string(&value).map_err(to_mcp_error)?;
            Ok(ReadResourceResult {
                contents: vec![ResourceContents::text(text, request.uri)],
            })
        }
    }
}

fn tool(name: &'static str, description: &'static str) -> Tool {
    Tool {
        name: Cow::Borrowed(name),
        title: None,
        description: Some(Cow::Borrowed(description)),
        input_schema: Arc::new(object_schema()),
        output_schema: None,
        annotations: None,
        execution: None,
        icons: None,
        meta: None,
    }
}

fn resource(uri: &str, name: &str) -> Resource {
    Annotated::new(
        RawResource {
            uri: uri.to_string(),
            name: name.to_string(),
            title: None,
            description: None,
            mime_type: Some("application/json".into()),
            size: None,
            icons: None,
            meta: None,
        },
        None,
    )
}

fn object_schema() -> JsonObject {
    let mut schema = JsonObject::new();
    schema.insert("type".into(), Value::String("object".into()));
    schema.insert("additionalProperties".into(), Value::Bool(true));
    schema
}

fn to_mcp_error(error: impl std::fmt::Display) -> McpError {
    McpError::internal_error(error.to_string(), None)
}
