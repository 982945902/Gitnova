use std::path::PathBuf;

use gitnova_wiki::{PatchMode, WikiStore};

fn main() -> anyhow::Result<()> {
    let wiki_root = std::env::args().nth(1).map(PathBuf::from).ok_or_else(|| {
        anyhow::anyhow!("usage: publish_page_sources <wiki-root> <repo-root> <page-id>")
    })?;
    let repo_root = std::env::args().nth(2).map(PathBuf::from).ok_or_else(|| {
        anyhow::anyhow!("usage: publish_page_sources <wiki-root> <repo-root> <page-id>")
    })?;
    let page_id = std::env::args().nth(3).ok_or_else(|| {
        anyhow::anyhow!("usage: publish_page_sources <wiki-root> <repo-root> <page-id>")
    })?;

    let store = WikiStore::open(wiki_root)?;
    let page = store.read_page(&page_id)?;
    let mut published = 0usize;
    for evidence in &page.evidence {
        if store.publish_source_file(&repo_root, &evidence.file)? {
            published += 1;
        }
    }
    store.patch_page(
        &page.id,
        page.content_format,
        &page.content,
        PatchMode::Replace,
    )?;
    println!("{} sources published for {}", published, page.id);
    Ok(())
}
