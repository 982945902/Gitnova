import type { CodeEdge, CodeNode } from "./types";

export interface GraphSummary {
  nodes?: number;
  edges?: number;
  files?: number;
  languages?: Record<string, number>;
  hubs?: Array<{
    id?: string;
    name?: string;
    qualified_name: string;
    in_degree: number;
    out_degree: number;
  }>;
  recent_churn?: unknown[];
  error?: string;
}

export interface RankedNode {
  node: CodeNode;
  score: number;
  explanation?: {
    strong_signals?: string[];
    weak_signals?: string[];
    penalties?: string[];
  };
}

export interface RankResponse {
  schema_version: number;
  repo_root: string;
  query: string;
  results: RankedNode[];
  error?: string;
}

export interface GraphContext {
  schema_version?: number;
  target?: CodeNode;
  nodes?: CodeNode[];
  edges?: CodeEdge[];
  incoming?: unknown[];
  outgoing?: unknown[];
  summary?: string;
  error?: string;
}

export interface AnswerResponse {
  answer?: string;
  llm_used?: boolean;
  model?: string;
  fallback_reason?: string;
  evidence?: Array<{
    node_id: string;
    qualified_name: string;
    path: string;
    span?: { start_line: number };
  }>;
  context?: GraphContext;
  error?: string;
}

const query = (params: Record<string, string | number>) =>
  Object.entries(params)
    .map(([key, value]) => `${encodeURIComponent(key)}=${encodeURIComponent(String(value))}`)
    .join("&");

export const paths = {
  summary: () => "/api/summary",
  nodes: () => "/api/nodes",
  edges: () => "/api/edges",
  rank: (value: string, limit = 10) =>
    `/api/rank?${query({ query: value, limit })}`,
  answer: (value: string, depth = 1, limit = 40) =>
    `/api/answer?${query({ query: value, depth, limit })}`,
  graphContext: (nodeId: string, depth = 1, limit = 40) =>
    `/api/graph-context?${query({ node_id: nodeId, depth, limit })}`,
};

async function fetchJson<T>(path: string): Promise<T> {
  const response = await fetch(path);
  if (!response.ok) {
    throw new Error(`${response.status} ${response.statusText}`);
  }
  return response.json() as Promise<T>;
}

export const api = {
  summary: () => fetchJson<GraphSummary>(paths.summary()),
  nodes: () => fetchJson<CodeNode[]>(paths.nodes()),
  edges: () => fetchJson<CodeEdge[]>(paths.edges()),
  rank: (value: string, limit = 10) =>
    fetchJson<RankResponse>(paths.rank(value, limit)),
  answer: (value: string, depth = 1, limit = 40) =>
    fetchJson<AnswerResponse>(paths.answer(value, depth, limit)),
  graphContext: (nodeId: string, depth = 1, limit = 40) =>
    fetchJson<GraphContext>(paths.graphContext(nodeId, depth, limit)),
};
