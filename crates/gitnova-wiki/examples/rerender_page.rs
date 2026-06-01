use std::path::PathBuf;

use gitnova_wiki::{PatchMode, WikiStore};

fn main() -> anyhow::Result<()> {
    let wiki_root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("usage: rerender_page <wiki-root> <page-id>"))?;
    let page_id = std::env::args()
        .nth(2)
        .ok_or_else(|| anyhow::anyhow!("usage: rerender_page <wiki-root> <page-id>"))?;

    let store = WikiStore::open(wiki_root)?;
    let page = store.read_page(&page_id)?;
    store.patch_page(
        &page.id,
        page.content_format,
        &page.content,
        PatchMode::Replace,
    )?;
    println!("{}", page.id);
    Ok(())
}
