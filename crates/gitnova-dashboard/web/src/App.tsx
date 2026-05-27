import { FormEvent, useCallback, useEffect, useMemo, useState } from "react";
import { RefreshCw, RotateCcw, Search, Sparkles } from "lucide-react";
import { api, type AnswerResponse, type GraphContext, type GraphSummary, type RankResponse } from "./api";
import { GraphScene } from "./graph/GraphScene";
import { nodeDegree } from "./graph/layout";
import { GRAPH_MODES, type GraphMode } from "./graph/modes";
import type { CodeEdge, CodeNode, NodeKind } from "./types";

const DEFAULT_QUERY = "Where is auth session validated?";
const VISIBLE_LIMIT = 72;

const edgeKey = (edge: CodeEdge) => `${edge.from}->${edge.to}:${edge.kind}`;

export function App() {
  const [query, setQuery] = useState(DEFAULT_QUERY);
  const [nodeFilter, setNodeFilter] = useState("");
  const [kindFilter, setKindFilter] = useState("");
  const [summary, setSummary] = useState<GraphSummary | null>(null);
  const [nodes, setNodes] = useState<CodeNode[]>([]);
  const [edges, setEdges] = useState<CodeEdge[]>([]);
  const [rank, setRank] = useState<RankResponse | null>(null);
  const [answer, setAnswer] = useState<AnswerResponse | null>(null);
  const [detail, setDetail] = useState<unknown>(null);
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [highlightedNodeIds, setHighlightedNodeIds] = useState<Set<string>>(new Set());
  const [highlightedEdgeKeys, setHighlightedEdgeKeys] = useState<Set<string>>(new Set());
  const [graphMode, setGraphMode] = useState<GraphMode>("analyze");
  const [graphResetSignal, setGraphResetSignal] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const visibleNodes = useMemo(() => {
    const filter = nodeFilter.trim().toLowerCase();
    return nodes
      .filter((node) => !["repository", "import"].includes(node.kind))
      .filter((node) => !kindFilter || node.kind === kindFilter)
      .filter((node) => {
        if (!filter) return true;
        return `${node.name} ${node.qualified_name} ${node.path}`.toLowerCase().includes(filter);
      })
      .sort((left, right) => nodeDegree(right) - nodeDegree(left))
      .slice(0, VISIBLE_LIMIT);
  }, [kindFilter, nodeFilter, nodes]);

  const visibleEdges = useMemo(() => {
    const visibleIds = new Set(visibleNodes.map((node) => node.id));
    return edges.filter((edge) => visibleIds.has(edge.from) && visibleIds.has(edge.to));
  }, [edges, visibleNodes]);

  const applyContext = useCallback((context: GraphContext, nodeId?: string) => {
    setHighlightedNodeIds(new Set((context.nodes ?? []).map((node) => node.id)));
    setHighlightedEdgeKeys(new Set((context.edges ?? []).map(edgeKey)));
    setSelectedNodeId(nodeId ?? context.target?.id ?? null);
    setDetail({
      summary: context.summary,
      target: context.target,
      incoming: context.incoming ?? [],
      outgoing: context.outgoing ?? [],
      edges: context.edges ?? [],
    });
  }, []);

  const focusNode = useCallback(
    async (node: CodeNode) => {
      setError(null);
      try {
        const context = await api.graphContext(node.id, 1, 40);
        applyContext(context, node.id);
      } catch (err) {
        setError(err instanceof Error ? err.message : "Unable to load graph context");
      }
    },
    [applyContext],
  );

  const runSearch = useCallback(
    async (value: string) => {
      const trimmed = value.trim() || DEFAULT_QUERY;
      setError(null);
      setLoading(true);
      try {
        const [rankResponse, answerResponse] = await Promise.all([
          api.rank(trimmed, 10),
          api.answer(trimmed, 1, 40),
        ]);
        setRank(rankResponse);
        setAnswer(answerResponse);
        if (answerResponse.context?.target) {
          applyContext(answerResponse.context, answerResponse.context.target.id);
        } else if (rankResponse.results[0]?.node) {
          await focusNode(rankResponse.results[0].node);
        }
      } catch (err) {
        setError(err instanceof Error ? err.message : "Search failed");
      } finally {
        setLoading(false);
      }
    },
    [applyContext, focusNode],
  );

  useEffect(() => {
    let cancelled = false;
    const loadGraph = async () => {
      setLoading(true);
      setError(null);
      try {
        const [summaryResponse, nodeResponse, edgeResponse] = await Promise.all([
          api.summary(),
          api.nodes(),
          api.edges(),
        ]);
        if (cancelled) return;
        setSummary(summaryResponse);
        setNodes(Array.isArray(nodeResponse) ? nodeResponse : []);
        setEdges(Array.isArray(edgeResponse) ? edgeResponse : []);
        await runSearch(DEFAULT_QUERY);
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : "Unable to load graph");
          setLoading(false);
        }
      }
    };
    void loadGraph();
    return () => {
      cancelled = true;
    };
  }, [runSearch]);

  const onSubmit = (event: FormEvent) => {
    event.preventDefault();
    void runSearch(query);
  };

  return (
    <main className="app-shell">
      <aside className="rail rail-left">
        <header className="brand">
          <span className="brand-mark">G</span>
          <div>
            <h1>Gitnova</h1>
            <p>Local code intelligence</p>
          </div>
        </header>

        <form className="search-form" onSubmit={onSubmit}>
          <label htmlFor="query">Query</label>
          <div className="input-row">
            <input
              id="query"
              name="query"
              type="search"
              value={query}
              onChange={(event) => setQuery(event.currentTarget.value)}
            />
            <button type="submit" aria-label="Search">
              <Search size={17} />
            </button>
          </div>
        </form>

        <section className="panel">
          <div className="panel-heading">
            <h2>Graph Filters</h2>
            <button type="button" aria-label="Reload graph" onClick={() => window.location.reload()}>
              <RefreshCw size={16} />
            </button>
          </div>
          <label htmlFor="node-filter">Filter nodes</label>
          <input
            id="node-filter"
            type="search"
            placeholder="auth, service, validate"
            value={nodeFilter}
            onChange={(event) => setNodeFilter(event.currentTarget.value)}
          />
          <label htmlFor="kind-filter">Node kind</label>
          <select
            id="kind-filter"
            value={kindFilter}
            onChange={(event) => setKindFilter(event.currentTarget.value as NodeKind | "")}
          >
            <option value="">All kinds</option>
            <option value="file">Files</option>
            <option value="module">Modules</option>
            <option value="function">Functions</option>
            <option value="method">Methods</option>
            <option value="class">Classes</option>
            <option value="struct">Structs</option>
            <option value="interface">Interfaces</option>
            <option value="trait">Traits</option>
          </select>
        </section>

        <SummaryPanel summary={summary} visibleNodes={visibleNodes.length} visibleEdges={visibleEdges.length} />
      </aside>

      <section className="graph-column" aria-label="Graph viewport">
        <div className="graph-toolbar">
          <div>
            <h2>Graph</h2>
            <p>{visibleNodes.length} nodes / {visibleEdges.length} edges</p>
          </div>
          <div className="graph-actions">
            <div className="segmented-control" aria-label="Graph display mode">
              {GRAPH_MODES.map((mode) => (
                <button
                  key={mode}
                  type="button"
                  className={mode === graphMode ? "is-active" : ""}
                  onClick={() => setGraphMode(mode)}
                >
                  {mode === "showcase" ? <Sparkles size={15} /> : null}
                  {mode === "analyze" ? "Analyze" : "Showcase"}
                </button>
              ))}
            </div>
            <button
              type="button"
              className="icon-button"
              aria-label="Reset camera"
              onClick={() => setGraphResetSignal((value) => value + 1)}
            >
              <RotateCcw size={16} />
            </button>
          </div>
        </div>
        <div className="graph-stage">
          <GraphScene
            nodes={visibleNodes}
            edges={visibleEdges}
            highlightedNodeIds={highlightedNodeIds}
            highlightedEdgeKeys={highlightedEdgeKeys}
            selectedNodeId={selectedNodeId}
            mode={graphMode}
            resetSignal={graphResetSignal}
            onSelectNode={focusNode}
          />
        </div>
      </section>

      <aside className="rail rail-right">
        {error ? <div className="error-box">{error}</div> : null}
        <section className="panel">
          <h2>Answer</h2>
          <p className="answer-text">
            {loading && !answer ? "Loading graph evidence..." : answer?.answer ?? "Run a query to inspect ranked evidence."}
          </p>
          {answer ? (
            <p className="answer-meta">
              {answer.llm_used ? `LLM: ${answer.model ?? "configured model"}` : `Fallback: ${answer.fallback_reason ?? "deterministic evidence answer"}`}
            </p>
          ) : null}
        </section>

        <section className="panel">
          <h2>Evidence</h2>
          <ul className="plain-list">
            {(answer?.evidence ?? []).slice(0, 8).map((item) => (
              <li key={item.node_id}>
                <button type="button" className="text-button" onClick={() => focusNodeById(item.node_id, nodes, focusNode)}>
                  {item.qualified_name}
                </button>
                <span>{item.path}{item.span ? `:${item.span.start_line}` : ""}</span>
              </li>
            ))}
          </ul>
        </section>

        <section className="panel">
          <h2>Ranked Results</h2>
          <ol className="result-list">
            {(rank?.results ?? []).map((result) => (
              <li key={result.node.id}>
                <button type="button" className="text-button" onClick={() => focusNode(result.node)}>
                  {result.node.qualified_name}
                </button>
                <span>{result.score.toFixed(3)}</span>
              </li>
            ))}
          </ol>
        </section>

        <section className="panel">
          <h2>Node Detail</h2>
          <pre>{JSON.stringify(detail ?? { selected: null }, null, 2)}</pre>
        </section>
      </aside>
    </main>
  );
}

function SummaryPanel({
  summary,
  visibleNodes,
  visibleEdges,
}: {
  summary: GraphSummary | null;
  visibleNodes: number;
  visibleEdges: number;
}) {
  return (
    <section className="panel">
      <h2>Summary</h2>
      <dl className="stats-grid">
        <div>
          <dt>Total nodes</dt>
          <dd>{summary?.nodes ?? "-"}</dd>
        </div>
        <div>
          <dt>Total edges</dt>
          <dd>{summary?.edges ?? "-"}</dd>
        </div>
        <div>
          <dt>Visible</dt>
          <dd>{visibleNodes}/{visibleEdges}</dd>
        </div>
      </dl>
      <h2>Hubs</h2>
      <ul className="plain-list">
        {(summary?.hubs ?? []).slice(0, 6).map((hub, index) => (
          <li key={hub.id ?? `${hub.qualified_name}-${index}`}>
            <span>{hub.qualified_name}</span>
            <span>{hub.in_degree + hub.out_degree}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}

function focusNodeById(
  nodeId: string,
  nodes: CodeNode[],
  focusNode: (node: CodeNode) => Promise<void>,
) {
  const node = nodes.find((candidate) => candidate.id === nodeId);
  if (node) {
    void focusNode(node);
  }
}
