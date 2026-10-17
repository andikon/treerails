use anyhow::{Result, bail};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use std::{
    fs,
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use treerail::{
    app::{App, Completion, Job, Modal, Payload},
    config, ui,
    workflow::{Command, Event, Issue, IssueSource, Progress, Workflow},
};
struct Source {
    issue: Mutex<Issue>,
    unavailable: AtomicBool,
}
impl Source {
    fn new() -> Self {
        Self {
            issue: Mutex::new(Issue {
                number: 5,
                title: "Original request".into(),
                body: "## Tasks\n- [ ] Implement".into(),
                state: "OPEN".into(),
                url: "https://github.com/owner/repo/issues/5".into(),
            }),
            unavailable: AtomicBool::new(false),
        }
    }
    fn revise(&self, body: &str) {
        self.issue.lock().unwrap().body = body.into();
    }
    fn check(&self) -> Result<()> {
        if self.unavailable.load(Ordering::Relaxed) {
            bail!("authentication unavailable")
        }
        Ok(())
    }
}
impl IssueSource for Source {
    async fn repository(&self, _: &Path) -> Result<String> {
        self.check()?;
        Ok("owner/repo".into())
    }
    async fn list(&self, _: &Path) -> Result<Vec<Issue>> {
        self.check()?;
        Ok(vec![self.issue.lock().unwrap().clone()])
    }
    async fn get(&self, _: &Path, _: u64) -> Result<Issue> {
        self.check()?;
        Ok(self.issue.lock().unwrap().clone())
    }
}
fn ticket(event: Event) -> treerail::workflow::TicketRecord {
    let Event::Ticket(record) = event else {
        panic!("ticket expected")
    };
    record
}
#[tokio::test]
async fn refresh_inspect_apply_and_revision_are_explicit_and_keep_original_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let flow = Workflow::new(&path);
    let source = Source::new();
    let first = ticket(
        flow.execute_with(temp.path(), Command::Open { number: 5 }, &source)
            .await
            .unwrap(),
    );
    let id = first.id();
    let task = first.tasks[0].id.clone();
    flow.execute_with(
        temp.path(),
        Command::SetProgress {
            ticket: id.clone(),
            task: task.clone(),
            progress: Progress::InProgress,
        },
        &source,
    )
    .await
    .unwrap();
    source.revise("Changed requirement\n## Tasks\n- [ ] Implement\n- [ ] Review");
    let fresh = ticket(
        flow.execute_with(
            temp.path(),
            Command::Refresh { ticket: id.clone() },
            &source,
        )
        .await
        .unwrap(),
    );
    assert!(fresh.issue_updated());
    assert_eq!(fresh.tasks.len(), 1);
    assert_eq!(fresh.prepared_issue, Some(first.issue.clone()));
    let Event::Requirements(review) = flow
        .execute_with(
            temp.path(),
            Command::InspectUpdate { ticket: id.clone() },
            &source,
        )
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(review.previous, first.issue);
    assert_eq!(review.current, fresh.issue);
    assert_eq!(review.additions, vec!["Implement", "Review"]);
    assert!(review.instructions().contains("Changed requirement"));
    let Event::RevisionRequested(request) = flow
        .execute_with(
            temp.path(),
            Command::ReviseInstructions {
                ticket: id.clone(),
                task: task.clone(),
                expected_issue: review.current.clone(),
            },
            &source,
        )
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(request.ticket, id);
    assert_eq!(request.task, task);
    assert_eq!(request.instructions, review.instructions());
    assert_eq!(
        flow.snapshot().unwrap().tickets[&id].tasks.len(),
        1,
        "Revision intent never silently changes tasks or starts a session"
    );
    let merged = ticket(
        flow.execute_with(
            temp.path(),
            Command::ApplyTaskMerge {
                ticket: id.clone(),
                expected_issue: review.current.clone(),
            },
            &source,
        )
        .await
        .unwrap(),
    );
    assert_eq!(merged.tasks[0].id, task);
    assert_eq!(merged.tasks[0].progress, Progress::InProgress);
    assert_eq!(merged.tasks.len(), 3);
    assert!(!merged.issue_updated());
    assert_eq!(merged.prepared_issue, Some(first.issue));
    let same = ticket(
        flow.execute_with(
            temp.path(),
            Command::ApplyTaskMerge {
                ticket: id,
                expected_issue: review.current.clone(),
            },
            &source,
        )
        .await
        .unwrap(),
    );
    assert_eq!(merged.tasks, same.tasks);
}
#[tokio::test]
async fn stale_preview_and_task_selection_cannot_send_a_different_revision() {
    let temp = tempfile::tempdir().unwrap();
    let flow = Workflow::new(temp.path().join("config.toml"));
    let source = Source::new();
    let record = ticket(
        flow.execute_with(temp.path(), Command::Open { number: 5 }, &source)
            .await
            .unwrap(),
    );
    let id = record.id();
    let old = record.issue;
    source.revise("new requirements");
    flow.execute_with(
        temp.path(),
        Command::Refresh { ticket: id.clone() },
        &source,
    )
    .await
    .unwrap();
    assert!(
        flow.execute_with(
            temp.path(),
            Command::ApplyTaskMerge {
                ticket: id.clone(),
                expected_issue: old.clone()
            },
            &source
        )
        .await
        .is_err()
    );
    assert!(
        flow.execute_with(
            temp.path(),
            Command::ReviseInstructions {
                ticket: id.clone(),
                task: "task-1".into(),
                expected_issue: old
            },
            &source
        )
        .await
        .is_err()
    );
    let current = flow.snapshot().unwrap().tickets[&id].issue.clone();
    assert!(
        flow.execute_with(
            temp.path(),
            Command::ReviseInstructions {
                ticket: id,
                task: "not-selected".into(),
                expected_issue: current
            },
            &source
        )
        .await
        .is_err()
    );
}
#[tokio::test]
async fn github_failure_keeps_known_cache_identity_and_sync_time_without_inventing_tickets() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let flow = Workflow::new(&path);
    let source = Source::new();
    let record = ticket(
        flow.execute_with(temp.path(), Command::Open { number: 5 }, &source)
            .await
            .unwrap(),
    );
    let id = record.id();
    source.unavailable.store(true, Ordering::Relaxed);
    let Event::CachedIssues { issues, warning } = flow
        .execute_with(temp.path(), Command::List, &source)
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(issues, vec![record.issue.clone()]);
    assert!(warning.contains("cached"));
    let cached = ticket(
        flow.execute_with(temp.path(), Command::Open { number: 5 }, &source)
            .await
            .unwrap(),
    );
    assert_eq!(cached.issue, record.issue);
    assert_eq!(cached.fetched_at, record.fetched_at);
    assert_eq!(cached.prepared_issue, record.prepared_issue);
    assert!(cached.github_error.unwrap().contains("authentication"));
    assert!(
        flow.execute_with(temp.path(), Command::Open { number: 99 }, &source)
            .await
            .is_err()
    );
    assert_eq!(flow.snapshot().unwrap().tickets.len(), 1);
    assert!(
        flow.execute_with(temp.path(), Command::Reopen { ticket: id }, &source)
            .await
            .is_ok()
    );
    let unknown = tempfile::tempdir().unwrap();
    let offline = Workflow::new(unknown.path().join("config.toml"));
    assert!(
        offline
            .execute_with(unknown.path(), Command::List, &source)
            .await
            .is_err()
    );
    assert!(!offline.config_path.exists());
}
#[tokio::test]
async fn refresh_preserves_comments_and_unrelated_configuration_edits() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let flow = Workflow::new(&path);
    let source = Source::new();
    fs::write(
        &path,
        "# keep my notes\nrecursive = true # external preference\n",
    )
    .unwrap();
    let record = ticket(
        flow.execute_with(temp.path(), Command::Open { number: 5 }, &source)
            .await
            .unwrap(),
    );
    let text = fs::read_to_string(&path).unwrap().replace(
        "title = \"Original request\"",
        "title = \"Original request\" # requirement note",
    );
    fs::write(&path, text).unwrap();
    source.revise("Updated");
    flow.execute_with(
        temp.path(),
        Command::Refresh {
            ticket: record.id(),
        },
        &source,
    )
    .await
    .unwrap();
    let saved = fs::read_to_string(&path).unwrap();
    assert!(saved.contains("# keep my notes"));
    assert!(saved.contains("# external preference"));
    assert!(saved.contains("# requirement note"));
    assert!(config::Config::load(&path).unwrap().recursive);
}
#[tokio::test]
async fn terminal_review_previews_exact_send_and_does_not_apply_until_explicit_key() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let source = Source::new();
    let flow = Workflow::new(&path);
    let record = ticket(
        flow.execute_with(temp.path(), Command::Open { number: 5 }, &source)
            .await
            .unwrap(),
    );
    source.revise("Changed visibly\n## Tasks\n- [ ] Review");
    let updated = ticket(
        flow.execute_with(
            temp.path(),
            Command::Refresh {
                ticket: record.id(),
            },
            &source,
        )
        .await
        .unwrap(),
    );
    let review = updated.requirements_review();
    let (config, state) = config::open(&path, false, None).unwrap();
    let mut app = App::new(config, state, path, vec![], false);
    app.project = Some(treerail::git::Project {
        path: temp.path().into(),
        name: "test".into(),
    });
    app.ticket = Some(updated);
    app.jobs.clear();
    app.accept(Completion {
        epoch: app.epoch,
        token: "workflow".into(),
        result: Ok(Payload::Workflow(Event::Requirements(review.clone()))),
    });
    assert!(matches!(app.modal, Some(Modal::Requirements(_))));
    assert!(app.jobs.is_empty());
    let mut terminal = Terminal::new(TestBackend::new(140, 35)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Previously reviewed requirements"));
    assert!(text.contains("Changed visibly"));
    assert!(text.contains("Original request"));
    assert!(text.contains("retain 1 local tasks"));
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
    assert!(matches!(
        app.jobs.last(),
        Some(Job::Workflow(_, _, Command::ReviseInstructions { .. }))
    ));
    assert!(app.pending_revision.is_none());
    let event = flow
        .execute_with(
            temp.path(),
            Command::ReviseInstructions {
                ticket: record.id(),
                task: "task-1".into(),
                expected_issue: review.current.clone(),
            },
            &source,
        )
        .await
        .unwrap();
    app.accept(Completion {
        epoch: app.epoch,
        token: "workflow".into(),
        result: Ok(Payload::Workflow(event)),
    });
    assert_eq!(
        app.pending_revision.unwrap().instructions,
        review.instructions()
    );
}
