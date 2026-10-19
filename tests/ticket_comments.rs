use anyhow::{Result, bail};
use std::{
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use treerail::{
    config::Config,
    ticket_browser::Comment,
    ticket_comments::{self, Command, Event, Review},
    workflow::{self, Issue, IssueSource, Workflow},
};

struct Source {
    mode: AtomicUsize,
    login: Mutex<String>,
    posts: Mutex<Vec<String>>,
}
impl Source {
    fn new(mode: usize) -> Self {
        Self {
            mode: AtomicUsize::new(mode),
            login: Mutex::new("dev".into()),
            posts: Mutex::new(vec![]),
        }
    }
}
impl IssueSource for Source {
    async fn repository(&self, _: &Path) -> Result<String> {
        Ok("owner/repo".into())
    }
    async fn list(&self, _: &Path) -> Result<Vec<Issue>> {
        Ok(vec![])
    }
    async fn get(&self, _: &Path, number: u64) -> Result<Issue> {
        Ok(Issue {
            number,
            title: "Issue".into(),
            body: "Body".into(),
            state: "OPEN".into(),
            url: format!("https://github.com/owner/repo/issues/{number}"),
        })
    }
    async fn current_login(&self, _: &Path) -> Result<String> {
        Ok(self.login.lock().unwrap().clone())
    }
    async fn post_comment(&self, _: &Path, _: u64, body: &str) -> Result<Comment> {
        self.posts.lock().unwrap().push(body.into());
        if self.mode.load(Ordering::SeqCst) == 3 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        match self.mode.load(Ordering::SeqCst) {
            1 => bail!("response lost after remote creation"),
            2 => Err(ticket_comments::Rejected("HTTP 403 permission denied".into()).into()),
            _ => Ok(Comment {
                id: 99,
                author: "dev".into(),
                body: body.into(),
                url: "https://github.com/owner/repo/issues/7#issuecomment-99".into(),
                created_at: "now".into(),
                updated_at: "now".into(),
            }),
        }
    }
}
async fn execute(path: &Path, source: &Source, command: Command) -> Result<Event> {
    let event = Workflow::new(path)
        .execute_with(
            path.parent().unwrap(),
            workflow::Command::Comment(command),
            source,
        )
        .await?;
    let workflow::Event::Comment(event) = event else {
        panic!("Expected comment event")
    };
    Ok(event)
}
async fn review(path: &Path, source: &Source, text: &str) -> Review {
    execute(
        path,
        source,
        Command::Save {
            repository: "github.com/owner/repo".into(),
            number: 7,
            text: text.into(),
            expected_revision: 0,
        },
    )
    .await
    .unwrap();
    let Event::Review(review) = execute(
        path,
        source,
        Command::Review {
            repository: "github.com/owner/repo".into(),
            number: 7,
        },
    )
    .await
    .unwrap() else {
        panic!()
    };
    review
}
#[tokio::test]
async fn draft_review_and_post_are_separate_and_persist_across_restart() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let source = Source::new(0);
    let approved = review(&path, &source, "A comment with E, y and paste\nNext line").await;
    assert!(source.posts.lock().unwrap().is_empty());
    let reopened = Config::load(&path).unwrap();
    assert_eq!(
        reopened.state.comment_drafts.values().next().unwrap().text,
        approved.text
    );
    let Event::Posted { comment, .. } = execute(
        &path,
        &source,
        Command::Send {
            review: approved.clone(),
            retry_uncertain: false,
        },
    )
    .await
    .unwrap() else {
        panic!()
    };
    assert_eq!(comment.id, 99);
    assert_eq!(*source.posts.lock().unwrap(), vec![approved.text]);
    let draft = ticket_comments::load(&path, "github.com/owner/repo", 7).unwrap();
    assert!(draft.text.is_empty());
    assert!(draft.pending.is_none());
}
#[tokio::test]
async fn unknown_send_survives_restart_and_never_replays_without_explicit_retry() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let source = Source::new(1);
    let approved = review(&path, &source, "Lost response").await;
    assert!(matches!(
        execute(
            &path,
            &source,
            Command::Send {
                review: approved.clone(),
                retry_uncertain: false
            }
        )
        .await
        .unwrap(),
        Event::Unknown { .. }
    ));
    let restored = ticket_comments::load(&path, "github.com/owner/repo", 7).unwrap();
    assert!(restored.pending.is_some());
    assert_eq!(restored.text, "Lost response");
    assert!(
        execute(
            &path,
            &source,
            Command::Send {
                review: approved.clone(),
                retry_uncertain: false
            }
        )
        .await
        .is_err()
    );
    assert_eq!(source.posts.lock().unwrap().len(), 1);
    let attempt = restored.pending.unwrap().attempt;
    execute(
        &path,
        &source,
        Command::Resolve {
            repository: approved.repository.clone(),
            number: 7,
            attempt,
            posted: true,
        },
    )
    .await
    .unwrap();
    assert!(
        ticket_comments::load(&path, &approved.repository, 7)
            .unwrap()
            .text
            .is_empty()
    );
    assert_eq!(source.posts.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn definitive_rejection_keeps_draft_and_account_change_requires_new_review() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let source = Source::new(2);
    let approved = review(&path, &source, "Keep this draft").await;
    *source.login.lock().unwrap() = "other".into();
    assert!(
        execute(
            &path,
            &source,
            Command::Send {
                review: approved.clone(),
                retry_uncertain: false
            }
        )
        .await
        .is_err()
    );
    assert!(source.posts.lock().unwrap().is_empty());
    *source.login.lock().unwrap() = "dev".into();
    assert!(matches!(
        execute(
            &path,
            &source,
            Command::Send {
                review: approved.clone(),
                retry_uncertain: false
            }
        )
        .await
        .unwrap(),
        Event::Rejected { .. }
    ));
    let draft = ticket_comments::load(&path, &approved.repository, 7).unwrap();
    assert_eq!(draft.text, approved.text);
    assert!(draft.pending.is_none());
    assert!(draft.error.unwrap().contains("403"));
}
#[tokio::test]
async fn concurrent_edits_are_preserved_and_stale_review_cannot_publish() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let source = Source::new(0);
    let approved = review(&path, &source, "Original").await;
    execute(
        &path,
        &source,
        Command::Save {
            repository: approved.repository.clone(),
            number: 7,
            text: "New edit".into(),
            expected_revision: approved.revision,
        },
    )
    .await
    .unwrap();
    assert!(
        execute(
            &path,
            &source,
            Command::Send {
                review: approved.clone(),
                retry_uncertain: false
            }
        )
        .await
        .is_err()
    );
    assert!(
        execute(
            &path,
            &source,
            Command::Save {
                repository: approved.repository.clone(),
                number: 7,
                text: "Overwrite".into(),
                expected_revision: approved.revision
            }
        )
        .await
        .is_err()
    );
    assert_eq!(
        ticket_comments::load(&path, &approved.repository, 7)
            .unwrap()
            .text,
        "New edit"
    );
    assert!(source.posts.lock().unwrap().is_empty());
    execute(
        &path,
        &source,
        Command::Save {
            repository: approved.repository,
            number: 8,
            text: "Other issue".into(),
            expected_revision: 0,
        },
    )
    .await
    .unwrap();
    assert_eq!(Config::load(&path).unwrap().state.comment_drafts.len(), 2);
}
#[tokio::test]
async fn explicit_retry_and_resolution_cannot_race_an_in_flight_send() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let source = Source::new(3);
    let approved = review(&path, &source, "Publish once").await;
    let sending = execute(
        &path,
        &source,
        Command::Send {
            review: approved.clone(),
            retry_uncertain: false,
        },
    );
    let competing = async {
        for _ in 0..100 {
            if !source.posts.lock().unwrap().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(source.posts.lock().unwrap().len(), 1);
        let duplicate = execute(
            &path,
            &source,
            Command::Send {
                review: approved.clone(),
                retry_uncertain: true,
            },
        )
        .await;
        assert!(duplicate.unwrap_err().to_string().contains("still running"));
        let pending = ticket_comments::load(&path, &approved.repository, 7)
            .unwrap()
            .pending
            .unwrap();
        assert!(
            execute(
                &path,
                &source,
                Command::Resolve {
                    repository: approved.repository.clone(),
                    number: 7,
                    attempt: pending.attempt,
                    posted: false
                }
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("still running")
        );
    };
    let (result, ()) = tokio::join!(sending, competing);
    assert!(matches!(result.unwrap(), Event::Posted { .. }));
    assert_eq!(source.posts.lock().unwrap().len(), 1);
}
