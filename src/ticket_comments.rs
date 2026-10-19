//! Durable comment drafts and explicit, restart-safe publication policy.
use crate::{
    config::{self, Config},
    ticket_browser::Comment,
    workflow::IssueSource,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Draft {
    pub text: String,
    pub revision: u64,
    pub pending: Option<PendingSend>,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingSend {
    pub attempt: String,
    pub login: String,
    pub text: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Review {
    pub repository: String,
    pub number: u64,
    pub login: String,
    pub text: String,
    pub revision: u64,
}
#[derive(Debug, Clone)]
pub struct Editor {
    pub repository: String,
    pub number: u64,
    pub draft: Draft,
    pub review: Option<Review>,
    pub retry_uncertain: bool,
    pub posting: bool,
    pub status: String,
    pub scroll: u16,
}
pub fn save_text(
    path: &Path,
    repository: &str,
    number: u64,
    text: String,
    revision: u64,
) -> Result<Draft> {
    update(path, repository, number, |draft| {
        ensure!(
            draft.revision == revision,
            "Comment draft changed in another client; reopen it before saving"
        );
        draft.text = text;
        draft.revision += 1;
        draft.error = None;
        Ok(draft.clone())
    })
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    Save {
        repository: String,
        number: u64,
        text: String,
        expected_revision: u64,
    },
    Review {
        repository: String,
        number: u64,
    },
    Send {
        review: Review,
        retry_uncertain: bool,
    },
    Discard {
        repository: String,
        number: u64,
        expected_revision: u64,
    },
    Resolve {
        repository: String,
        number: u64,
        attempt: String,
        posted: bool,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    Draft {
        repository: String,
        number: u64,
        draft: Draft,
    },
    Review(Review),
    Posted {
        repository: String,
        number: u64,
        comment: Comment,
    },
    Unknown {
        repository: String,
        number: u64,
        draft: Draft,
    },
    Rejected {
        repository: String,
        number: u64,
        draft: Draft,
    },
}
pub fn key(repository: &str, number: u64) -> String {
    format!(
        "{}#{number}",
        repository.trim_end_matches('/').to_ascii_lowercase()
    )
}
pub fn load(path: &Path, repository: &str, number: u64) -> Result<Draft> {
    Ok(Config::load(path)?
        .state
        .comment_drafts
        .get(&key(repository, number))
        .cloned()
        .unwrap_or_default())
}
fn update<T>(
    path: &Path,
    repository: &str,
    number: u64,
    change: impl FnOnce(&mut Draft) -> Result<T>,
) -> Result<T> {
    config::edit_document(path, |doc| {
        let mut drafts = Config::parse(&doc.to_string(), path)?.state.comment_drafts;
        let id = key(repository, number);
        let draft = drafts.entry(id.clone()).or_default();
        let result = change(draft)?;
        let rendered: toml_edit::DocumentMut =
            toml::to_string(&BTreeMap::from([(id.clone(), draft.clone())]))?.parse()?;
        if doc.get("state").is_none() {
            doc["state"] = toml_edit::table();
        }
        if doc["state"].get("comment_drafts").is_none() {
            doc["state"]["comment_drafts"] = toml_edit::table();
        }
        doc["state"]["comment_drafts"][&id] = rendered[&id].clone();
        Ok(result)
    })
}
fn send_lock(path: &Path, repository: &str, number: u64) -> Result<std::fs::File> {
    use sha2::{Digest, Sha256};
    let id = format!("{:x}", Sha256::digest(key(repository, number).as_bytes()));
    let lock_path = path.with_extension(format!("comment-{}.lock", &id[..24]));
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    fs2::FileExt::try_lock_exclusive(&file).context("A comment send is still running in another client; wait for its result before resolving or retrying")?;
    Ok(file)
}
async fn verify_target(
    project: &Path,
    repository: &str,
    number: u64,
    source: &impl IssueSource,
) -> Result<()> {
    let issue = source.get(project, number).await?;
    ensure!(issue.number == number, "GitHub returned a different issue");
    let url = issue
        .url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let actual = url
        .split("/issues/")
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    let expected = repository
        .trim_start_matches("https://")
        .trim_end_matches('/')
        .to_ascii_lowercase();
    ensure!(
        actual == expected
            || (actual.starts_with("github.com/")
                && actual.strip_prefix("github.com/") == Some(expected.as_str())),
        "Comment target belongs to a different GitHub repository"
    );
    Ok(())
}
pub async fn execute(
    path: &Path,
    project: &Path,
    command: Command,
    source: &impl IssueSource,
) -> Result<Event> {
    match command {
        Command::Save {
            repository,
            number,
            text,
            expected_revision,
        } => {
            let draft = update(path, &repository, number, |draft| {
                ensure!(
                    draft.revision == expected_revision,
                    "Comment draft changed in another client; reopen it before saving"
                );
                draft.text = text;
                draft.revision += 1;
                draft.error = None;
                Ok(draft.clone())
            })?;
            Ok(Event::Draft {
                repository,
                number,
                draft,
            })
        }
        Command::Review { repository, number } => {
            let draft = load(path, &repository, number)?;
            ensure!(!draft.text.trim().is_empty(), "Comment is empty");
            let login = source
                .current_login(project)
                .await
                .context("Review requires the current posting identity; the draft is saved")?;
            Ok(Event::Review(Review {
                repository,
                number,
                login,
                text: draft.text,
                revision: draft.revision,
            }))
        }
        Command::Discard {
            repository,
            number,
            expected_revision,
        } => {
            let draft = update(path, &repository, number, |draft| {
                ensure!(
                    draft.revision == expected_revision,
                    "Comment draft changed in another client"
                );
                ensure!(
                    draft.pending.is_none(),
                    "Resolve the uncertain send before discarding its record"
                );
                draft.text.clear();
                draft.error = None;
                draft.revision += 1;
                Ok(draft.clone())
            })?;
            Ok(Event::Draft {
                repository,
                number,
                draft,
            })
        }
        Command::Resolve {
            repository,
            number,
            attempt,
            posted,
        } => {
            let _send_lock = send_lock(path, &repository, number)?;
            let draft = update(path, &repository, number, |draft| {
                let pending = draft
                    .pending
                    .as_ref()
                    .context("There is no uncertain send to resolve")?;
                ensure!(
                    pending.attempt == attempt,
                    "Send record changed; inspect it again"
                );
                if posted && draft.text == pending.text {
                    draft.text.clear();
                }
                draft.pending = None;
                draft.error = None;
                draft.revision += 1;
                Ok(draft.clone())
            })?;
            Ok(Event::Draft {
                repository,
                number,
                draft,
            })
        }
        Command::Send {
            review,
            retry_uncertain,
        } => {
            let _send_lock = send_lock(path, &review.repository, review.number)?;
            verify_target(project, &review.repository, review.number, source).await?;
            let login = source.current_login(project).await?;
            ensure!(
                login == review.login,
                "Posting account changed; review the comment again"
            );
            let attempt = uuid::Uuid::new_v4().to_string();
            update(path, &review.repository, review.number, |draft| {
                ensure!(
                    draft.revision == review.revision && draft.text == review.text,
                    "Comment changed since review; review again"
                );
                ensure!(
                    draft.pending.is_none() || retry_uncertain,
                    "Confirmation unknown: inspect the discussion and explicitly resolve or retry"
                );
                draft.pending = Some(PendingSend {
                    attempt: attempt.clone(),
                    login: login.clone(),
                    text: review.text.clone(),
                });
                draft.error = Some("Confirmation unknown: publication may have succeeded. Inspect the discussion before retrying.".into());
                Ok(())
            })?;
            let result = source
                .post_comment(project, review.number, &review.text)
                .await;
            let mut confirmed = None;
            let mut definitive = false;
            let draft = update(path, &review.repository, review.number, |draft| {
                ensure!(
                    draft
                        .pending
                        .as_ref()
                        .is_some_and(|pending| pending.attempt == attempt),
                    "Comment send record changed during publication; inspect the discussion"
                );
                match result {
                    Ok(comment) if comment.id != 0 && !comment.url.is_empty() => {
                        if draft.text == review.text {
                            draft.text.clear();
                        }
                        draft.pending = None;
                        draft.error = None;
                        draft.revision += 1;
                        confirmed = Some(comment);
                    }
                    Ok(_) => {
                        draft.error = Some(
                            "GitHub did not return a comment identity; confirmation unknown".into(),
                        )
                    }
                    Err(error) => {
                        definitive = error.downcast_ref::<Rejected>().is_some();
                        if definitive {
                            draft.pending = None;
                        }
                        draft.error = Some(format!("{error:#}"));
                    }
                }
                Ok(draft.clone())
            })?;
            let repository = review.repository;
            let number = review.number;
            Ok(if let Some(comment) = confirmed {
                Event::Posted {
                    repository,
                    number,
                    comment,
                }
            } else if definitive {
                Event::Rejected {
                    repository,
                    number,
                    draft,
                }
            } else {
                Event::Unknown {
                    repository,
                    number,
                    draft,
                }
            })
        }
    }
}
#[derive(Debug)]
pub struct Rejected(pub String);
impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Rejected {}

pub async fn github_current_login(project: &Path) -> Result<String> {
    let host = crate::ticket_browser::github_host(project).await?;
    #[derive(Deserialize)]
    struct User {
        login: String,
    }
    Ok(
        crate::workflow::Github::json::<User>(project, &["api", "--hostname", &host, "user"])
            .await?
            .login,
    )
}
pub async fn github_post(project: &Path, number: u64, body: &str) -> Result<Comment> {
    use std::process::Stdio;
    use tokio::io::AsyncWriteExt;
    // Use stdin, not a command argument: comments may contain arbitrary/large text.
    let payload = serde_json::to_vec(&serde_json::json!({ "body": body }))?;
    let host = crate::ticket_browser::github_host(project).await?;
    let mut child = tokio::process::Command::new("gh")
        .args([
            "api",
            "--hostname",
            &host,
            "--method",
            "POST",
            &format!("repos/{{owner}}/{{repo}}/issues/{number}/comments"),
            "--input",
            "-",
        ])
        .current_dir(project)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| Rejected(format!("Could not start GitHub CLI: {e}")))?;
    let output = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(&payload).await?;
        }
        child.wait_with_output().await
    })
    .await
    .context("Comment response timed out; confirmation unknown")??;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        if [
            "HTTP 400", "HTTP 401", "HTTP 403", "HTTP 404", "HTTP 422", "HTTP 429",
        ]
        .iter()
        .any(|status| error.contains(status))
        {
            return Err(Rejected(error).into());
        }
        anyhow::bail!("Comment confirmation unknown: {error}");
    }
    #[derive(Deserialize)]
    struct User {
        login: String,
    }
    #[derive(Deserialize)]
    struct Posted {
        id: u64,
        body: String,
        user: Option<User>,
        created_at: String,
        updated_at: String,
        html_url: String,
    }
    let posted: Posted = serde_json::from_slice(&output.stdout)
        .context("Comment response unreadable; confirmation unknown")?;
    Ok(Comment {
        id: posted.id,
        author: posted
            .user
            .map(|u| u.login)
            .unwrap_or_else(|| "[deleted]".into()),
        body: posted.body,
        created_at: posted.created_at,
        updated_at: posted.updated_at,
        url: posted.html_url,
    })
}
