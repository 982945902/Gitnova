use gitnova_wiki::{ContentFormat, Evidence, PageKind, PatchMode, WikiPage, WikiStore};

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
    assert!(html.contains("A[QueryExecutorCreator] --&gt; B[TermQueryExecutor]"));
    assert!(html.contains("<p>After the diagram.</p>"));
    assert!(!html.contains("<p>```mermaid</p>"));
    assert!(!html.contains("<p>```</p>"));

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
