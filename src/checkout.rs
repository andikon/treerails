use anyhow::{Context, Result};
use std::{fs::File, path::Path};

/// An OS lease on the physical checkout, shared by every Tree Rail process.
/// A linked checkout has its own Git metadata directory; aliases converge here.
pub struct Lease {
    _file: File,
}
impl Lease {
    pub async fn acquire(path: &Path) -> Result<Self> {
        let directory = crate::git::text(path, &["rev-parse", "--absolute-git-dir"]).await?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            // Reuse the former host's filename so an older running host also
            // excludes new-version writes during an upgrade.
            .open(Path::new(&directory).join("treerail-session.lock"))?;
        fs2::FileExt::try_lock_exclusive(&file)
            .context("Another Tree Rail agent or Git operation owns this checkout")?;
        Ok(Self { _file: file })
    }
}
