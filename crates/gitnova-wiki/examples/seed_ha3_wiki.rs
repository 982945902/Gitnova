use std::env;
use std::path::PathBuf;

use gitnova_wiki::{ContentFormat, Evidence, PageKind, PatchMode, WikiPage, WikiStore};

fn main() -> anyhow::Result<()> {
    let out_dir = env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from("/Users/lishuo121/workspace/havenask/.gitnova/wiki/ha3-demo")
    });
    if out_dir.exists() {
        std::fs::remove_dir_all(&out_dir)?;
    }

    let store = WikiStore::open(&out_dir)?;

    seed_page(
        &store,
        "ha3",
        "Havenask HA3 Search Layer",
        PageKind::Index,
        "HA3 is the query-facing search layer: query parsing, query AST, executor creation, document matching, filtering, and ranking helpers.",
        "## Reading Path\nStart from Query Pipeline to understand the request flow, then drill into parser, query model, executors, filters, and ranking.",
        Some("Suggested Next Expansion\nA real agent run should replace these summaries with code-derived flow notes and keep appending evidence after each deeper pass."),
        &[
            ("aios/ha3/ha3/isearch.h", None, None, "Top-level HA3 public include."),
            ("aios/ha3/ha3/search/query_executor/QueryExecutor.h", None, None, "Core executor abstraction."),
        ],
    )?;

    seed_page(
        &store,
        "ha3/query-pipeline",
        "Query Pipeline",
        PageKind::Article,
        "A first-pass narrative of how a HA3 query moves from parsed expression to executable search behavior.",
        r##"## Flow Sketch
1. Query text is parsed into query expression objects.
2. Expressions evaluate into common query objects such as AndQuery, OrQuery, TermQuery, PhraseQuery, and RankQuery.
3. QueryExecutorCreator maps query objects and index readers into query executors.
4. SingleLayerSearcher drives matching with layer metadata, filters, and match data collection.
5. Rank comparators order matched documents for downstream result handling.

```svg
<svg viewBox="0 0 920 220" role="img" aria-label="HA3 query pipeline flow" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <marker id="arrow" markerWidth="10" markerHeight="10" refX="8" refY="3" orient="auto" markerUnits="strokeWidth">
      <path d="M0,0 L0,6 L9,3 z" fill="#0f766e"/>
    </marker>
  </defs>
  <rect x="20" y="42" width="140" height="70" rx="8" fill="#e7f3f1" stroke="#0f766e"/>
  <text x="90" y="73" text-anchor="middle" font-size="15" font-weight="700" fill="#172033">Query Text</text>
  <text x="90" y="94" text-anchor="middle" font-size="12" fill="#667085">user syntax</text>

  <rect x="205" y="42" width="145" height="70" rx="8" fill="#fff" stroke="#d7dce5"/>
  <text x="277" y="73" text-anchor="middle" font-size="15" font-weight="700" fill="#172033">QueryParser</text>
  <text x="277" y="94" text-anchor="middle" font-size="12" fill="#667085">expression tree</text>

  <rect x="395" y="42" width="150" height="70" rx="8" fill="#fff" stroke="#d7dce5"/>
  <text x="470" y="73" text-anchor="middle" font-size="15" font-weight="700" fill="#172033">Common Query</text>
  <text x="470" y="94" text-anchor="middle" font-size="12" fill="#667085">And / Or / Term</text>

  <rect x="590" y="42" width="155" height="70" rx="8" fill="#fff" stroke="#d7dce5"/>
  <text x="668" y="73" text-anchor="middle" font-size="15" font-weight="700" fill="#172033">ExecutorCreator</text>
  <text x="668" y="94" text-anchor="middle" font-size="12" fill="#667085">runtime matching</text>

  <rect x="780" y="42" width="120" height="70" rx="8" fill="#e7f3f1" stroke="#0f766e"/>
  <text x="840" y="73" text-anchor="middle" font-size="15" font-weight="700" fill="#172033">Searcher</text>
  <text x="840" y="94" text-anchor="middle" font-size="12" fill="#667085">matchdocs</text>

  <path d="M160 77 H198" stroke="#0f766e" stroke-width="2" marker-end="url(#arrow)"/>
  <path d="M350 77 H388" stroke="#0f766e" stroke-width="2" marker-end="url(#arrow)"/>
  <path d="M545 77 H583" stroke="#0f766e" stroke-width="2" marker-end="url(#arrow)"/>
  <path d="M745 77 H773" stroke="#0f766e" stroke-width="2" marker-end="url(#arrow)"/>

  <rect x="590" y="145" width="155" height="44" rx="8" fill="#f7f8fb" stroke="#d7dce5"/>
  <text x="668" y="172" text-anchor="middle" font-size="13" fill="#172033">Index Readers</text>
  <path d="M668 145 V118" stroke="#667085" stroke-width="1.8" stroke-dasharray="4 4"/>

  <rect x="780" y="145" width="120" height="44" rx="8" fill="#f7f8fb" stroke="#d7dce5"/>
  <text x="840" y="172" text-anchor="middle" font-size="13" fill="#172033">Rank</text>
  <path d="M840 112 V145" stroke="#667085" stroke-width="1.8" stroke-dasharray="4 4"/>
</svg>
```"##,
        Some("Why This Page Exists\nThis is the cross-cutting index page. It should link implementation details without forcing readers to start from the source directory tree."),
        &[
            ("aios/ha3/ha3/queryparser/QueryParser.h", None, None, "Parser entry point."),
            ("aios/ha3/ha3/common/query/Query.h", None, None, "Common query base model."),
            ("aios/ha3/ha3/search/query_executor/QueryExecutorCreator.h", None, None, "Executor factory."),
            ("aios/ha3/ha3/search/single_layer_searcher/SingleLayerSearcher.h", None, None, "Searcher driver."),
            ("aios/ha3/ha3/rank/comparator/ComboComparator.h", None, None, "Ranking comparator composition."),
        ],
    )?;

    seed_page(
        &store,
        "ha3/queryparser",
        "Query Parser",
        PageKind::Index,
        "The parser layer turns textual query syntax into expression nodes before the common query model is built.",
        "## Main Objects\nParserContext and QueryParser coordinate scanner/parser state. QueryExpr subclasses represent logical, term, phrase, number, rank, and multi-term forms.\n\n## Local Shape\nThis directory is expression-heavy: most files are paired .h/.cpp classes with a small evaluator hierarchy.",
        None,
        &[
            ("aios/ha3/ha3/queryparser/QueryParser.h", None, None, "Parser entry class."),
            ("aios/ha3/ha3/queryparser/ParserContext.h", None, None, "Parser state and context."),
            ("aios/ha3/ha3/queryparser/QueryExpr.h", None, None, "Expression base class."),
            ("aios/ha3/ha3/queryparser/DefaultQueryExprEvaluator.h", None, None, "Expression evaluator."),
        ],
    )?;

    seed_page(
        &store,
        "ha3/common-query",
        "Common Query Model",
        PageKind::Index,
        "The query model stores parsed search intent in reusable logical and term query objects.",
        "## Main Objects\nQuery is the base abstraction. AndQuery, OrQuery, AndNotQuery, MultiTermQuery, PhraseQuery, RankQuery, TableQuery, and TermQuery encode the common query tree.\n\n## Reader Note\nThis layer is a better conceptual entry than individual parser classes because it names the semantic query operators HA3 search understands.",
        None,
        &[
            ("aios/ha3/ha3/common/query/Query.h", None, None, "Query base class."),
            ("aios/ha3/ha3/common/query/TermQuery.h", None, None, "Term query model."),
            ("aios/ha3/ha3/common/query/MultiTermQuery.h", None, None, "Multi-term query model."),
            ("aios/ha3/ha3/common/query/QueryVisitor.h", None, None, "Visitor interface."),
        ],
    )?;

    seed_page(
        &store,
        "ha3/search",
        "Search Runtime",
        PageKind::Index,
        "Search runtime contains query executors, filters, auxiliary chain visitors, and the single-layer search driver.",
        "## Subsystems\nQuery executors implement matching semantics. Filters gate matched documents. SingleLayerSearcher coordinates layer traversal and match collection. Auxiliary chain visitors collect term statistics.",
        None,
        &[
            ("aios/ha3/ha3/search/query_executor/QueryExecutor.h", None, None, "Executor base class."),
            ("aios/ha3/ha3/search/filter/Filter.h", None, None, "Filter base class."),
            ("aios/ha3/ha3/search/single_layer_searcher/SingleLayerSearcher.h", None, None, "Search driver."),
            ("aios/ha3/ha3/search/auxiliary_chain/TermDFVisitor.h", None, None, "Term document-frequency visitor."),
        ],
    )?;

    seed_page(
        &store,
        "ha3/search/query-executors",
        "Query Executors",
        PageKind::Article,
        "Executor classes turn query semantics into doc-id seeking, matching, and match-data production.",
        "## Executor Families\nLogical executors handle And, Or, AndNot, weak-and, and multi-term combinations. Term executors handle term, bitmap, buffered, range, phrase, primary-key, spatial, and sub-document matching.",
        Some("Important Agent Follow-up\nThe next deep pass should trace QueryExecutorCreator into each executor family and record which index reader APIs each executor needs."),
        &[
            ("aios/ha3/ha3/search/query_executor/QueryExecutor.h", None, None, "Executor abstraction."),
            ("aios/ha3/ha3/search/query_executor/QueryExecutorCreator.h", None, None, "Executor construction."),
            ("aios/ha3/ha3/search/query_executor/TermQueryExecutor.h", None, None, "Term executor."),
            ("aios/ha3/ha3/search/query_executor/BitmapTermQueryExecutor.h", None, None, "Bitmap term executor."),
            ("aios/ha3/ha3/search/query_executor/MultiQueryExecutor.h", None, None, "Composite executor base."),
        ],
    )?;

    seed_page(
        &store,
        "ha3/search/single-layer-searcher",
        "Single Layer Searcher",
        PageKind::Article,
        "SingleLayerSearcher appears to be the runtime coordinator around one search layer: layer metadata, query executor, filters, and match data.",
        "",
        Some("Working Hypothesis\nThis page should become the main runtime-flow page after a deeper code pass. It likely connects query execution to filtering, matchdoc collection, and ranking preparation."),
        &[
            ("aios/ha3/ha3/search/single_layer_searcher/SingleLayerSearcher.h", None, None, "Searcher class."),
            ("aios/ha3/ha3/search/single_layer_searcher/SingleLayerSearcher.cpp", None, None, "Searcher implementation."),
            ("aios/ha3/ha3/search/query_executor/LayerMetas.h", None, None, "Layer metadata."),
            ("aios/ha3/ha3/search/filter/FilterWrapper.h", None, None, "Filter integration."),
        ],
    )?;

    seed_page(
        &store,
        "ha3/rank",
        "Ranking Comparators",
        PageKind::Index,
        "The rank comparator layer orders matchdocs using comparator composition and priority queue support.",
        "## Main Objects\nComparator defines ordering, ComboComparator combines comparators, ReferenceComparator compares field references, and MatchDocPriorityQueue keeps top candidates.",
        None,
        &[
            ("aios/ha3/ha3/rank/comparator/Comparator.h", None, None, "Comparator base."),
            ("aios/ha3/ha3/rank/comparator/ComboComparator.h", None, None, "Comparator composition."),
            ("aios/ha3/ha3/rank/comparator/MatchDocPriorityQueue.h", None, None, "Priority queue."),
            ("aios/ha3/ha3/rank/comparator/ReferenceComparator.h", None, None, "Reference comparator."),
        ],
    )?;

    println!("{}", out_dir.join("pages/ha3/index.html").display());
    Ok(())
}

fn seed_page(
    store: &WikiStore,
    id: &str,
    title: &str,
    kind: PageKind,
    summary: &str,
    content: &str,
    private_note: Option<&str>,
    evidence: &[(&str, Option<u32>, Option<u32>, &str)],
) -> anyhow::Result<()> {
    store.upsert_page(WikiPage::new(id, title, kind).with_summary(summary))?;
    store.patch_page(id, ContentFormat::Markdown, content, PatchMode::Replace)?;
    if let Some(private_note) = private_note {
        store.patch_private_note(id, private_note, PatchMode::Replace)?;
    }
    for (file, start, end, note) in evidence {
        let mut item = Evidence::new(*file).with_note(*note);
        if let Some(start) = start {
            item.start_line = Some(*start);
            item.end_line = *end;
        }
        store.append_evidence(id, item)?;
    }
    Ok(())
}
