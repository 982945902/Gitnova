use gitnova_wiki::{
    ContentFormat, DeepTask, Evidence, OutlinePage, PageKind, PatchMode, TaskStatus, WikiOutline,
    WikiPage, WikiStore,
};

#[test]
fn upsert_page_builds_nested_html_index_tree() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let store = WikiStore::open(temp.path())?;

    store.upsert_page(
        WikiPage::new("storage/indexlib", "Indexlib Storage", PageKind::Index)
            .with_summary("Storage, indexing, tablet, KV, KKV, and merge subsystem map."),
    )?;
    store.upsert_page(
        WikiPage::new("storage/indexlib/kv", "KV Index", PageKind::Index)
            .with_summary("KV write, read, and hash table mechanisms."),
    )?;
    store.upsert_page(
        WikiPage::new(
            "storage/indexlib/kv/hash-table-strategy",
            "Hash Table Strategy",
            PageKind::Article,
        )
        .with_summary(
            "How Havenask chooses and uses dense, cuckoo, and separate-chain hash tables.",
        ),
    )?;

    let schema = store.read_schema()?;
    assert_eq!(schema.nodes.len(), 3);
    assert_eq!(
        schema
            .node("storage/indexlib/kv/hash-table-strategy")
            .unwrap()
            .parent
            .as_deref(),
        Some("storage/indexlib/kv")
    );

    let index_html =
        std::fs::read_to_string(temp.path().join("pages/storage/indexlib/index.html"))?;
    assert!(index_html.contains("Indexlib Storage"));
    assert!(index_html.contains("KV Index"));
    assert!(index_html.contains("href=\"kv/index.html\""));

    let kv_html =
        std::fs::read_to_string(temp.path().join("pages/storage/indexlib/kv/index.html"))?;
    assert!(kv_html.contains("Hash Table Strategy"));
    assert!(kv_html.contains("href=\"hash-table-strategy.html\""));

    Ok(())
}

#[test]
fn patch_page_content_preserves_evidence_and_journal() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let store = WikiStore::open(temp.path())?;

    store.upsert_page(
        WikiPage::new(
            "storage/indexlib/tablet/open-flow",
            "Tablet Open Flow",
            PageKind::Article,
        )
        .with_summary("Tablet open sequence and fence integration."),
    )?;
    store.patch_page(
        "storage/indexlib/tablet/open-flow",
        ContentFormat::Markdown,
        "## Flow\n\n1. Load tablet options.\n2. Recover fence state.",
        PatchMode::Replace,
    )?;
    store.append_evidence(
        "storage/indexlib/tablet/open-flow",
        Evidence::new("aios/storage/indexlib/table/Tablet.cpp")
            .with_span(120, 220)
            .with_note("Primary Tablet open implementation."),
    )?;

    let page = store.read_page("storage/indexlib/tablet/open-flow")?;
    assert_eq!(page.content_format, ContentFormat::Markdown);
    assert!(page.content.contains("Recover fence state"));
    assert_eq!(page.evidence.len(), 1);

    let html = std::fs::read_to_string(
        temp.path()
            .join("pages/storage/indexlib/tablet/open-flow.html"),
    )?;
    assert!(html.contains("Tablet Open Flow"));
    assert!(html.contains("Recover fence state"));
    assert!(html.contains("data-gitnova-evidence"));
    assert!(html.contains("aios/storage/indexlib/table/Tablet.cpp"));
    assert!(!html.contains("<h2>Evidence</h2>"));

    let journal = std::fs::read_to_string(temp.path().join("journal.jsonl"))?;
    assert!(journal.contains("\"event\":\"upsert_page\""));
    assert!(journal.contains("\"event\":\"patch_page\""));
    assert!(journal.contains("\"event\":\"append_evidence\""));

    Ok(())
}

#[test]
fn evidence_renders_as_public_source_spans() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let store = WikiStore::open(temp.path())?;
    let repo = temp.path().join("repo");
    let source_file = repo.join("aios/ha3/ha3/search/query_executor/QueryExecutorCreator.cpp");
    std::fs::create_dir_all(source_file.parent().unwrap())?;
    std::fs::write(
        &source_file,
        (1..=220)
            .map(|line| format!("source line {line}\n"))
            .collect::<String>(),
    )?;

    store.upsert_page(WikiPage::new(
        "ha3/search/query-executors",
        "Query Executors",
        PageKind::Article,
    ))?;
    store.patch_page(
        "ha3/search/query-executors",
        ContentFormat::Markdown,
        "## Overview\n\nQuery executors route parsed query nodes into concrete executor families.",
        PatchMode::Replace,
    )?;
    store.append_evidence(
        "ha3/search/query-executors",
        Evidence::new("aios/ha3/ha3/search/query_executor/QueryExecutorCreator.cpp")
            .with_span(124, 217)
            .with_note("Term executor factory and posting-type dispatch."),
    )?;
    assert!(store.publish_source_file(
        &repo,
        "aios/ha3/ha3/search/query_executor/QueryExecutorCreator.cpp"
    )?);

    let html = std::fs::read_to_string(temp.path().join("pages/ha3/search/query-executors.html"))?;
    assert!(html.contains("<aside class=\"source-spans\""));
    assert!(html.contains("<h2>Source Spans</h2>"));
    assert!(html.contains(
        "href=\"../../_sources/aios/ha3/ha3/search/query_executor/QueryExecutorCreator.cpp.html#L124-L217\""
    ));
    assert!(html.contains("aios/ha3/ha3/search/query_executor/QueryExecutorCreator.cpp:124-217"));
    assert!(html.contains("Term executor factory and posting-type dispatch."));
    assert!(html.contains("data-gitnova-evidence"));
    let source_html =
        std::fs::read_to_string(temp.path().join(
            "pages/_sources/aios/ha3/ha3/search/query_executor/QueryExecutorCreator.cpp.html",
        ))?;
    assert!(source_html
        .contains("<h1>aios/ha3/ha3/search/query_executor/QueryExecutorCreator.cpp</h1>"));
    assert!(source_html.contains("id=\"L124\""));
    assert!(source_html.contains("source line 124"));
    assert!(source_html.contains("highlightHashSpan"));

    Ok(())
}

#[test]
fn source_markers_render_inline_citations_instead_of_bottom_list() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let store = WikiStore::open(temp.path())?;
    let repo = temp.path().join("repo");
    let source_file = repo.join("aios/ha3/ha3/search/query_executor/QueryExecutorCreator.cpp");
    std::fs::create_dir_all(source_file.parent().unwrap())?;
    std::fs::write(
        &source_file,
        (1..=220)
            .map(|line| format!("source line {line}\n"))
            .collect::<String>(),
    )?;

    store.upsert_page(WikiPage::new(
        "ha3/search/query-executors",
        "Query Executors",
        PageKind::Article,
    ))?;
    store.append_evidence(
        "ha3/search/query-executors",
        Evidence::new("aios/ha3/ha3/search/query_executor/QueryExecutorCreator.cpp")
            .with_span(124, 217)
            .with_note("Term executor factory and posting-type dispatch."),
    )?;
    assert!(store.publish_source_file(
        &repo,
        "aios/ha3/ha3/search/query_executor/QueryExecutorCreator.cpp"
    )?);
    store.patch_page(
        "ha3/search/query-executors",
        ContentFormat::Markdown,
        "Term queries call `createTermQueryExecutor`. {{source:1}}",
        PatchMode::Replace,
    )?;

    let html = std::fs::read_to_string(temp.path().join("pages/ha3/search/query-executors.html"))?;
    assert!(html.contains("class=\"source-cite\""));
    assert!(html.contains(">Term queries call <code>createTermQueryExecutor</code>. <a"));
    assert!(html.contains(
        "href=\"../../_sources/aios/ha3/ha3/search/query_executor/QueryExecutorCreator.cpp.html#L124-L217\""
    ));
    assert!(html.contains("[1]</a>"));
    assert!(!html.contains("<aside class=\"source-spans\""));
    assert!(html.contains("data-gitnova-evidence"));

    Ok(())
}

#[test]
fn nested_article_renders_breadcrumbs_and_wiki_tree_navigation() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let store = WikiStore::open(temp.path())?;

    store.upsert_page(WikiPage::new("ha3", "HA3", PageKind::Index))?;
    store.upsert_page(WikiPage::new(
        "ha3/search",
        "Search Runtime",
        PageKind::Index,
    ))?;
    store.upsert_page(WikiPage::new(
        "ha3/search/query-executors",
        "Query Executors",
        PageKind::Article,
    ))?;
    store.upsert_page(WikiPage::new("ha3/rank", "Ranking", PageKind::Index))?;

    let html = std::fs::read_to_string(temp.path().join("pages/ha3/search/query-executors.html"))?;

    assert!(html.contains("class=\"app-shell\""));
    assert!(html.contains("aria-label=\"Wiki tree\""));
    assert!(html.contains("aria-label=\"Breadcrumb\""));
    assert!(html.contains("href=\"../index.html\""));
    assert!(html.contains("HA3"));
    assert!(html.contains("href=\"index.html\""));
    assert!(html.contains("Search Runtime"));
    assert!(html.contains("aria-current=\"page\""));
    assert!(html.contains("Query Executors"));
    assert!(html.contains("href=\"../rank/index.html\""));

    Ok(())
}

#[test]
fn child_page_cards_are_single_stable_click_targets() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let store = WikiStore::open(temp.path())?;

    store.upsert_page(WikiPage::new("ha3", "HA3", PageKind::Index))?;
    store.upsert_page(
        WikiPage::new("ha3/search", "Search Runtime", PageKind::Index)
            .with_summary("Search runtime map."),
    )?;

    let html = std::fs::read_to_string(temp.path().join("pages/ha3/index.html"))?;
    assert!(html.contains("<li><a class=\"page-card\" href=\"search/index.html\">"));
    assert!(html.contains("<strong>Search Runtime</strong>"));
    assert!(html.contains("<p>Search runtime map.</p>"));
    assert!(html.contains(".page-card:hover"));
    assert!(html.contains("text-decoration: none"));

    Ok(())
}

#[test]
fn markdown_svg_fences_render_as_inline_diagrams() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let store = WikiStore::open(temp.path())?;

    store.upsert_page(WikiPage::new(
        "ha3/query-pipeline",
        "Query Pipeline",
        PageKind::Article,
    ))?;
    store.patch_page(
        "ha3/query-pipeline",
        ContentFormat::Markdown,
        "## Flow\n\n```svg\n<svg viewBox=\"0 0 120 40\"><text x=\"8\" y=\"24\">Parser</text></svg>\n```\n\nAfter the diagram.",
        PatchMode::Replace,
    )?;

    let html = std::fs::read_to_string(temp.path().join("pages/ha3/query-pipeline.html"))?;
    assert!(html.contains("<div class=\"diagram-block\">"));
    assert!(html.contains("<svg viewBox=\"0 0 120 40\">"));
    assert!(html.contains("<text x=\"8\" y=\"24\">Parser</text>"));
    assert!(html.contains("<p>After the diagram.</p>"));

    Ok(())
}

#[test]
fn markdown_mermaid_fences_render_as_diagrams_without_leaking_fence_markers() -> anyhow::Result<()>
{
    let temp = tempfile::tempdir()?;
    let store = WikiStore::open(temp.path())?;

    store.upsert_page(WikiPage::new(
        "ha3/search/query-executors",
        "Query Executors",
        PageKind::Article,
    ))?;
    store.patch_page(
        "ha3/search/query-executors",
        ContentFormat::Markdown,
        "## Atlas Investigation\n\n```mermaid\nflowchart TD\n    A[QueryExecutorCreator] --> B[TermQueryExecutor]\n```\n\nAfter the diagram.",
        PatchMode::Replace,
    )?;

    let html = std::fs::read_to_string(temp.path().join("pages/ha3/search/query-executors.html"))?;
    assert!(html.contains("https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.min.js"));
    assert!(html.contains("<div class=\"diagram-block\" data-diagram-format=\"mermaid\">"));
    assert!(html.contains("<pre class=\"mermaid\">"));
    assert!(html.contains("flowchart TD"));
    assert!(html
        .contains("A[&quot;QueryExecutorCreator&quot;] --&gt; B[&quot;TermQueryExecutor&quot;]"));
    assert!(html.contains("A[&quot;QueryExecutorCreator&quot;]"));
    assert!(html.contains("<p>After the diagram.</p>"));
    assert!(!html.contains("<p>```mermaid</p>"));
    assert!(!html.contains("<p>```</p>"));

    Ok(())
}

#[test]
fn markdown_mermaid_labels_with_punctuation_are_quoted() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let store = WikiStore::open(temp.path())?;

    store.upsert_page(WikiPage::new(
        "ha3/search/query-executors",
        "Query Executors",
        PageKind::Article,
    ))?;
    store.patch_page(
        "ha3/search/query-executors",
        ContentFormat::Markdown,
        "```mermaid\nflowchart TD\n  Q[common::Query accept(visitor)] --> V[QueryExecutorCreator visit*]\n  V --> T[Term/Number/Phrase visitors]\n  T --> Out[_queryExecutor]\n```",
        PatchMode::Replace,
    )?;

    let html = std::fs::read_to_string(temp.path().join("pages/ha3/search/query-executors.html"))?;
    assert!(html.contains("Q[&quot;common::Query accept(visitor)&quot;]"));
    assert!(html.contains("V[&quot;QueryExecutorCreator visit*&quot;]"));
    assert!(html.contains("T[&quot;Term/Number/Phrase visitors&quot;]"));
    assert!(html.contains("Out[&quot;_queryExecutor&quot;]"));

    Ok(())
}

#[test]
fn markdown_renders_lists_inline_code_and_wrapped_paragraphs() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let store = WikiStore::open(temp.path())?;

    store.upsert_page(WikiPage::new(
        "ha3/search/query-executors",
        "Query Executors",
        PageKind::Article,
    ))?;
    store.patch_page(
        "ha3/search/query-executors",
        ContentFormat::Markdown,
        "This line starts a paragraph\nthat continues on the next line.\n\n- Term queries call `createTermQueryExecutor`.\n- OR queries create `OrQueryExecutor`.\n\nFactory helpers use **bitmap-aware** variants.",
        PatchMode::Replace,
    )?;

    let html = std::fs::read_to_string(temp.path().join("pages/ha3/search/query-executors.html"))?;
    assert!(html.contains("<p>This line starts a paragraph\nthat continues on the next line.</p>"));
    assert!(html.contains("<ul>"));
    assert!(html.contains("<li>Term queries call <code>createTermQueryExecutor</code>.</li>"));
    assert!(html.contains("<li>OR queries create <code>OrQueryExecutor</code>.</li>"));
    assert!(html.contains("<strong>bitmap-aware</strong>"));
    assert!(!html.contains("<p>- Term queries"));

    Ok(())
}

#[test]
fn markdown_renders_tables_and_code_blocks_with_readable_styles() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let store = WikiStore::open(temp.path())?;

    store.upsert_page(WikiPage::new(
        "ha3/search/query-executors",
        "Query Executors",
        PageKind::Article,
    ))?;
    store.patch_page(
        "ha3/search/query-executors",
        ContentFormat::Markdown,
        "| Method | Role |\n| --- | --- |\n| `visitTermQuery` | Term dispatch |\n\n```cpp\n_queryExecutor = createTermQueryExecutor(term);\n```",
        PatchMode::Replace,
    )?;

    let html = std::fs::read_to_string(temp.path().join("pages/ha3/search/query-executors.html"))?;
    assert!(html.contains("<table>"));
    assert!(html.contains("<th>Method</th>"));
    assert!(html.contains("<code>visitTermQuery</code>"));
    assert!(html.contains("<pre><code class=\"language-cpp\">"));
    assert!(html.contains("_queryExecutor = createTermQueryExecutor(term);"));
    assert!(html.contains("table {"));
    assert!(html.contains("pre {"));

    Ok(())
}

#[test]
fn updating_outline_task_status_refreshes_private_note_task_state() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let store = WikiStore::open(temp.path())?;

    store.apply_outline(WikiOutline {
        root: "ha3".to_string(),
        pages: vec![OutlinePage {
            id: "ha3/search/query-executors".to_string(),
            title: "Query Executors".to_string(),
            kind: PageKind::Article,
            summary: None,
            parent: None,
            purpose: Some("Explain executor creation.".to_string()),
            content: None,
            deep_tasks: vec![DeepTask {
                id: "trace-query-executor-creator".to_string(),
                question: "Trace QueryExecutorCreator.".to_string(),
                scope_paths: vec![],
                scope_symbols: vec![],
                expected_outputs: vec![],
                max_nodes: None,
                max_depth: None,
                timeout_secs: None,
                status: TaskStatus::Pending,
                confidence: None,
                error: None,
            }],
        }],
        version: 1,
    })?;

    let pending_note = store.read_private_note("ha3/search/query-executors")?;
    assert!(pending_note.contains("- [pending] trace-query-executor-creator"));

    store.update_task_status(
        "ha3/search/query-executors",
        "trace-query-executor-creator",
        TaskStatus::Done,
        Some(0.91),
        None,
    )?;

    let done_note = store.read_private_note("ha3/search/query-executors")?;
    assert!(done_note.contains("- [done] trace-query-executor-creator"));
    assert!(!done_note.contains("- [pending] trace-query-executor-creator"));

    Ok(())
}

#[test]
fn retryable_tasks_can_include_failed_outline_tasks() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let store = WikiStore::open(temp.path())?;

    store.apply_outline(WikiOutline {
        root: "havenask".to_string(),
        pages: vec![OutlinePage {
            id: "havenask/ha3".to_string(),
            title: "HA3".to_string(),
            kind: PageKind::Index,
            summary: None,
            parent: Some("havenask".to_string()),
            purpose: None,
            content: None,
            deep_tasks: vec![
                DeepTask {
                    id: "map-ha3".to_string(),
                    question: "Map HA3.".to_string(),
                    scope_paths: vec![],
                    scope_symbols: vec![],
                    expected_outputs: vec![],
                    max_nodes: None,
                    max_depth: None,
                    timeout_secs: None,
                    status: TaskStatus::Failed,
                    confidence: None,
                    error: Some("temporary limit".to_string()),
                },
                DeepTask {
                    id: "map-query-parser".to_string(),
                    question: "Map parser.".to_string(),
                    scope_paths: vec![],
                    scope_symbols: vec![],
                    expected_outputs: vec![],
                    max_nodes: None,
                    max_depth: None,
                    timeout_secs: None,
                    status: TaskStatus::Pending,
                    confidence: None,
                    error: None,
                },
            ],
        }],
        version: 1,
    })?;

    let pending = store.pending_tasks("havenask", 10)?;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].1.id, "map-query-parser");

    let retryable = store.retryable_tasks("havenask", 10)?;
    assert_eq!(retryable.len(), 2);
    assert_eq!(retryable[0].1.id, "map-ha3");
    assert_eq!(retryable[1].1.id, "map-query-parser");

    Ok(())
}

#[test]
fn public_content_is_rendered_without_heading_heuristics() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let store = WikiStore::open(temp.path())?;

    store.upsert_page(WikiPage::new(
        "ha3/query-pipeline",
        "Query Pipeline",
        PageKind::Article,
    ))?;
    store.patch_page(
        "ha3/query-pipeline",
        ContentFormat::Markdown,
        "Visible paragraph.\n\n## Why This Page Exists\nThis is public if it is written to the public block.\n\nMore visible text.",
        PatchMode::Replace,
    )?;

    let html = std::fs::read_to_string(temp.path().join("pages/ha3/query-pipeline.html"))?;
    assert!(html.contains("<p>Visible paragraph.</p>"));
    assert!(html.contains("<p>More visible text.</p>"));
    assert!(html.contains("<h2>Why This Page Exists</h2>"));
    assert!(html.contains("This is public if it is written to the public block."));

    Ok(())
}

#[test]
fn public_content_does_not_guess_internal_workflow_sections() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let store = WikiStore::open(temp.path())?;

    store.upsert_page(WikiPage::new("ha3", "HA3", PageKind::Article))?;
    store.patch_page(
        "ha3",
        ContentFormat::Markdown,
        "## Main Objects\nVisible architecture content.\n\n## Suggested Next Expansion\nA real agent run should replace these summaries.\n\n## Important Agent Follow-up\nThe next deep pass should trace QueryExecutorCreator.\n\n## Working Hypothesis\nThis page should become the main runtime-flow page after a deeper code pass.\n\n## Reader Note\nThis is visible reader guidance.",
        PatchMode::Replace,
    )?;

    let html = std::fs::read_to_string(temp.path().join("pages/ha3.html"))?;
    assert!(html.contains("<h2>Main Objects</h2>"));
    assert!(html.contains("Visible architecture content."));
    assert!(html.contains("<h2>Suggested Next Expansion</h2>"));
    assert!(html.contains("A real agent run should replace these summaries."));
    assert!(html.contains("<h2>Important Agent Follow-up</h2>"));
    assert!(html.contains("<h2>Working Hypothesis</h2>"));
    assert!(html.contains("<p>The next deep pass should trace QueryExecutorCreator.</p>"));
    assert!(html.contains("<h2>Reader Note</h2>"));
    assert!(html.contains("This is visible reader guidance."));
    assert!(!html.contains("data-gitnova-codex-note"));

    Ok(())
}

#[test]
fn private_notes_are_stored_separately_and_never_rendered() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let store = WikiStore::open(temp.path())?;

    store.upsert_page(WikiPage::new(
        "ha3/search/query-executors",
        "Query Executors",
        PageKind::Article,
    ))?;
    store.patch_page(
        "ha3/search/query-executors",
        ContentFormat::Markdown,
        "## Executor Families\nVisible executor content.",
        PatchMode::Replace,
    )?;
    store.patch_private_note(
        "ha3/search/query-executors",
        "Important Agent Follow-up\nThe next deep pass should trace QueryExecutorCreator.",
        PatchMode::Replace,
    )?;

    let html = std::fs::read_to_string(temp.path().join("pages/ha3/search/query-executors.html"))?;
    assert!(html.contains("Visible executor content."));
    assert!(!html.contains("Important Agent Follow-up"));
    assert!(!html.contains("The next deep pass should trace QueryExecutorCreator."));
    assert!(!html.contains("data-gitnova-codex-note"));

    let private_note = store.read_private_note("ha3/search/query-executors")?;
    assert!(private_note.contains("Important Agent Follow-up"));
    assert!(private_note.contains("QueryExecutorCreator"));

    let private_file = std::fs::read_to_string(
        temp.path()
            .join("blocks_private/ha3/search/query-executors.md"),
    )?;
    assert_eq!(private_file, private_note);

    let journal = std::fs::read_to_string(temp.path().join("journal.jsonl"))?;
    assert!(journal.contains("\"event\":\"patch_private_note\""));

    Ok(())
}
