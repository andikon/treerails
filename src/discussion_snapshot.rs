//! Retained, immutable starting context. Only references cross the UI/host boundary.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub path: PathBuf,
    pub sha256: String,
    pub bytes: u64,
    pub captured_at: u64,
    pub issue_url: String,
    pub comments: usize,
    pub complete: bool,
    pub context_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approval {
    pub snapshot: Snapshot,
    /// Exact local task/checkout/handoff state reviewed alongside the document.
    pub context: String,
    pub short_prompt: String,
}

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Retained data is outside checkouts even when --config points inside one.
/// Each canonical configuration path receives its own namespace.
pub fn directory(config: &Path) -> Result<PathBuf> {
    #[cfg(test)]
    let root = config
        .parent()
        .context("Configuration has no parent directory")?
        .join("controlled-data");
    #[cfg(not(test))]
    let root = directories::ProjectDirs::from("", "", "treerails")
        .context("Cannot locate Tree Rail's retained data directory")?
        .data_local_dir()
        .to_owned();
    directory_in(&root, config)
}

pub fn directory_in(root: &Path, config: &Path) -> Result<PathBuf> {
    let parent = config
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let canonical = fs::canonicalize(config).or_else(|_| {
        fs::canonicalize(parent).map(|parent| parent.join(config.file_name().unwrap_or_default()))
    })?;
    let identity = canonical.to_string_lossy().into_owned();
    #[cfg(windows)]
    let identity = identity.to_lowercase();
    Ok(root
        .join("discussion-snapshots")
        .join(digest(identity.as_bytes())))
}

pub fn retain(
    config: &Path,
    discussion: &crate::ticket_browser::Discussion,
    context: String,
) -> Result<Approval> {
    ensure!(
        discussion.complete && discussion.warning.is_none(),
        "Fresh complete discussion is required; retry capture when GitHub is available"
    );
    let card = &discussion.card;
    let mut markdown = format!(
        "# {}\n\nSource: {}\nAuthor: @{}\nCreated: {}\nUpdated: {}\nCaptured: {}\n\n## Task and checkout context\n\n{}\n\n## Issue description\n\n{}\n\n## Discussion ({} comments, oldest first)\n",
        card.issue.title,
        card.issue.url,
        card.author,
        card.created_at,
        card.updated_at,
        discussion.fetched_at,
        context,
        card.issue.body,
        discussion.comments.len()
    );
    for comment in &discussion.comments {
        markdown.push_str(&format!(
            "\n### @{} — {}{}\n\nSource: {}\n\n{}\n",
            comment.author,
            comment.created_at,
            if comment.updated_at != comment.created_at {
                format!(" (edited {})", comment.updated_at)
            } else {
                String::new()
            },
            comment.url,
            comment.body
        ));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let directory = directory(config)?;
    fs::create_dir_all(&directory)?;
    let path = directory.join(format!("{id}.md"));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    file.write_all(markdown.as_bytes())?;
    file.sync_all()?;
    let snapshot = Snapshot {
        id,
        path: fs::canonicalize(&path)?,
        sha256: digest(markdown.as_bytes()),
        bytes: markdown.len() as u64,
        captured_at: discussion.fetched_at,
        issue_url: card.issue.url.clone(),
        comments: discussion.comments.len(),
        complete: true,
        context_sha256: digest(context.as_bytes()),
    };
    let short_prompt = format!(
        "Start the approved task using the complete frozen discussion document {} (SHA-256 {}).",
        snapshot.id, snapshot.sha256
    );
    Ok(Approval {
        snapshot,
        context,
        short_prompt,
    })
}

impl Snapshot {
    pub fn validate(&self, config: &Path) -> Result<()> {
        ensure!(
            self.complete && crate::copilot::is_uuid(&self.id),
            "Discussion snapshot is incomplete or has invalid identity; capture and review again"
        );
        let expected = directory(config)?.join(format!("{}.md", self.id));
        ensure!(fs::symlink_metadata(&expected).context("Approved discussion document is missing; restore the exact artifact or capture and review again")?.file_type().is_file(), "Discussion document must be a regular retained file");
        ensure!(
            fs::canonicalize(&expected).ok().as_ref() == Some(&self.path),
            "Discussion document is outside this configuration context; capture and review again"
        );
        let bytes = fs::read(&expected)?;
        ensure!(
            bytes.len() as u64 == self.bytes && digest(&bytes) == self.sha256,
            "Approved discussion document changed; restore the exact artifact or capture and review again"
        );
        Ok(())
    }
}

impl Approval {
    pub fn validate(&self, config: &Path) -> Result<()> {
        self.snapshot.validate(config)?;
        ensure!(
            digest(self.context.as_bytes()) == self.snapshot.context_sha256,
            "Approved task context differs from the captured document; capture and review again"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn discussion() -> crate::ticket_browser::Discussion {
        crate::ticket_browser::Discussion {
            card: crate::ticket_browser::IssueCard {
                issue: crate::workflow::Issue {
                    number: 7,
                    title: "Full requirements".into(),
                    body: "Entire issue body".into(),
                    state: "OPEN".into(),
                    url: "https://github.com/owner/repo/issues/7".into(),
                },
                labels: vec![],
                assignees: vec![],
                author: "requester".into(),
                created_at: "2026-10-01".into(),
                updated_at: "2026-10-02".into(),
            },
            comments: vec![crate::ticket_browser::Comment {
                id: 9,
                author: "reviewer".into(),
                body: format!(
                    "{}\nFINAL-ACCEPTANCE-CRITERION",
                    "Complete comment content. ".repeat(5000)
                ),
                created_at: "2026-10-02".into(),
                updated_at: "2026-10-03".into(),
                url: "https://github.com/owner/repo/issues/7#issuecomment-9".into(),
            }],
            complete: true,
            fetched_at: 17,
            warning: None,
        }
    }
    #[test]
    fn full_document_survives_checkout_cleanup_and_is_configuration_isolated() {
        let temporary = tempfile::tempdir().unwrap();
        let config = temporary.path().join("config.toml");
        let checkout = temporary.path().join("checkout");
        fs::create_dir(&checkout).unwrap();
        let approval = retain(
            &config,
            &discussion(),
            format!("Task: implement\nCheckout: {}", checkout.display()),
        )
        .unwrap();
        assert!(approval.snapshot.bytes > 48 * 1024);
        let text = fs::read_to_string(&approval.snapshot.path).unwrap();
        assert!(text.contains("FINAL-ACCEPTANCE-CRITERION"));
        assert!(
            text.contains("@reviewer")
                && text.contains("issuecomment-9")
                && text.contains("edited 2026-10-03")
        );
        fs::remove_dir(&checkout).unwrap();
        approval.snapshot.validate(&config).unwrap();
        assert!(
            approval
                .snapshot
                .validate(&temporary.path().join("another.toml"))
                .is_err()
        );
        assert!(
            serde_json::to_string(&approval).unwrap().len() < 2048,
            "Only artifact references/context cross IPC and live in config"
        );
    }
    #[test]
    fn incomplete_capture_missing_or_changed_documents_never_silently_fallback() {
        let temporary = tempfile::tempdir().unwrap();
        let config = temporary.path().join("config.toml");
        let mut partial = discussion();
        partial.complete = false;
        assert!(retain(&config, &partial, "Task".into()).is_err());
        partial.complete = true;
        partial.warning = Some("Page failed".into());
        assert!(retain(&config, &partial, "Task".into()).is_err());
        let approval = retain(&config, &discussion(), "Task".into()).unwrap();
        fs::write(&approval.snapshot.path, "different discussion").unwrap();
        assert!(
            approval
                .snapshot
                .validate(&config)
                .unwrap_err()
                .to_string()
                .contains("changed")
        );
        fs::remove_file(&approval.snapshot.path).unwrap();
        assert!(approval.snapshot.validate(&config).is_err());
    }
}
