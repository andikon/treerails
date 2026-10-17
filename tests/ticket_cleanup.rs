use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use std::{
    fs,
    path::{Path, PathBuf},
};
use treerail::{
    app::{App, Completion, Modal, Payload},
    config, git,
    host::CheckoutLease,
    hosted::{Health, SessionRecord},
    ui,
    workflow::{self, Command, Event, Issue, IssueSource, Preparation, Progress, Workflow},
};
struct Source {
    closed: bool,
}
impl IssueSource for Source {
    async fn repository(&self, _: &Path) -> Result<String> {
        Ok("owner/repo".into())
    }
    async fn list(&self, project: &Path) -> Result<Vec<Issue>> {
        Ok(vec![self.get(project, 1).await?])
    }
    async fn get(&self, _: &Path, number: u64) -> Result<Issue> {
        Ok(Issue {
            number,
            title: "Deliver feature".into(),
            body: "Implement correctly".into(),
            state: if self.closed { "CLOSED" } else { "OPEN" }.into(),
            url: format!("https://github.com/owner/repo/issues/{number}"),
        })
    }
}
async fn repo(temp: &Path) -> PathBuf {
    let project = temp.join("project");
    fs::create_dir(&project).unwrap();
    for args in [
        vec!["init", "--initial-branch=main"],
        vec!["config", "user.name", "Cleanup Test"],
        vec!["config", "user.email", "test@example.invalid"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["commit", "--allow-empty", "-m", "initial"],
    ] {
        git::run(&project, args).await.unwrap();
    }
    project
}
fn ticket(event: Event) -> workflow::TicketRecord {
    let Event::Ticket(record) = event else {
        panic!("ticket expected")
    };
    record
}
async fn prepared(temp: &Path) -> (PathBuf, Workflow, workflow::TicketRecord) {
    let project = repo(temp).await;
    let flow = Workflow::new(temp.join("config.toml"));
    let record = ticket(
        flow.execute_with(
            &project,
            Command::Open { number: 1 },
            &Source { closed: false },
        )
        .await
        .unwrap(),
    );
    let prepared = ticket(
        flow.execute_with(
            &project,
            Command::Prepare {
                ticket: record.id(),
                preparation: Preparation::Create {
                    branch: "ticket/1".into(),
                    base: "main".into(),
                    destination: temp.join("ticket"),
                    fetch: false,
                },
            },
            &Source { closed: false },
        )
        .await
        .unwrap(),
    );
    (project, flow, prepared)
}
fn session(ticket: &workflow::TicketRecord, health: Health) -> SessionRecord {
    SessionRecord {
        id: "conversation-record".into(),
        ticket: ticket.id(),
        task: "task-1".into(),
        checkout: ticket.worktree.as_ref().unwrap().path.clone(),
        provider_session_id: "exact-provider-id".into(),
        starting_brief: "Original immutable brief".into(),
        created_at: 10,
        host_session: Some("runtime-id".into()),
        health,
        current: true,
        error: None,
    }
}
#[tokio::test]
async fn closure_cleanup_retains_branch_task_progress_briefs_and_conversations() {
    let temp = tempfile::tempdir().unwrap();
    let (project, flow, record) = prepared(temp.path()).await;
    let id = record.id();
    let checkout = record.worktree.clone().unwrap();
    let stopped = session(&record, Health::Stopped);
    flow.update(|state| {
        state.sessions.insert(stopped.id.clone(), stopped.clone());
        Ok(())
    })
    .unwrap();
    flow.execute_with(
        &project,
        Command::SetProgress {
            ticket: id.clone(),
            task: "task-1".into(),
            progress: Progress::InProgress,
        },
        &Source { closed: false },
    )
    .await
    .unwrap();
    let closed = ticket(
        flow.execute_with(
            &project,
            Command::Refresh { ticket: id.clone() },
            &Source { closed: true },
        )
        .await
        .unwrap(),
    );
    assert!(closed.is_closed());
    assert_eq!(closed.tasks[0].progress, Progress::InProgress);
    git::run(
        &checkout.path,
        ["commit", "--allow-empty", "-m", "local unpublished feature"],
    )
    .await
    .unwrap();
    let preview = flow.cleanup_preview(&project, &id).await.unwrap();
    assert!(preview.can_remove());
    assert_eq!(preview.unpublished_count, 2);
    assert!(
        preview
            .unpublished_commits
            .iter()
            .any(|line| line.contains("local unpublished feature"))
    );
    let removed = ticket(
        flow.execute_with(
            &project,
            Command::Cleanup {
                ticket: id.clone(),
                expected_checkout: checkout.clone(),
            },
            &Source { closed: true },
        )
        .await
        .unwrap(),
    );
    assert!(removed.checkout_removed_at.is_some());
    assert_eq!(removed.worktree, Some(checkout.clone()));
    assert!(!checkout.path.exists());
    assert_eq!(removed.tasks, closed.tasks);
    assert_eq!(removed.prepared_issue, closed.prepared_issue);
    assert_eq!(flow.snapshot().unwrap().sessions[&stopped.id], stopped);
    assert!(
        git::branches(&project)
            .await
            .unwrap()
            .iter()
            .any(|branch| branch.name == "ticket/1")
    );
    let absent = flow.cleanup_preview(&project, &id).await.unwrap();
    assert!(absent.missing && absent.already_removed);
    assert!(!absent.can_remove());
    assert!(
        flow.execute_with(
            &project,
            Command::Prepare {
                ticket: id,
                preparation: Preparation::Create {
                    branch: "ticket/1".into(),
                    base: "main".into(),
                    destination: checkout.path,
                    fetch: false
                }
            },
            &Source { closed: true }
        )
        .await
        .is_err()
    );
}
#[tokio::test]
async fn running_waiting_and_failed_stop_refuse_ticket_and_basic_deletion() {
    let temp = tempfile::tempdir().unwrap();
    let (project, flow, record) = prepared(temp.path()).await;
    let checkout = record.worktree.clone().unwrap();
    let lease = CheckoutLease::acquire(&checkout.path).await.unwrap();
    // Needs input retains the same live lease as Running; neither permits deletion.
    assert!(
        git::remove_worktree(&project, &checkout.path)
            .await
            .is_err()
    );
    assert!(
        flow.execute_with(
            &project,
            Command::Cleanup {
                ticket: record.id(),
                expected_checkout: checkout.clone()
            },
            &Source { closed: false }
        )
        .await
        .is_err()
    );
    assert!(checkout.path.exists());
    drop(lease);
    for health in [
        Health::Live,
        Health::Starting,
        Health::Interrupted,
        Health::Failed,
    ] {
        let owner = session(&record, health);
        flow.update(|state| {
            state.sessions.insert(owner.id.clone(), owner);
            Ok(())
        })
        .unwrap();
        let preview = flow.cleanup_preview(&project, &record.id()).await.unwrap();
        assert!(preview.owner.is_some());
        assert!(!preview.can_remove());
        assert!(
            flow.execute_with(
                &project,
                Command::Cleanup {
                    ticket: record.id(),
                    expected_checkout: checkout.clone()
                },
                &Source { closed: false }
            )
            .await
            .is_err()
        );
    }
    let (config, state) = config::open(&flow.config_path, false, None).unwrap();
    let mut app = App::new(config, state, flow.config_path.clone(), vec![], false);
    app.project = Some(git::project(&project).await.unwrap());
    app.trees = git::worktrees(&project).await.unwrap();
    app.tree = app.trees.iter().position(|tree| !tree.original).unwrap();
    app.jobs.clear();
    app.operation(treerail::app::Operation::Remove(checkout.path.clone()));
    assert!(app.jobs.is_empty());
    assert!(app.notice.contains("verified stopped"));
    assert!(checkout.path.exists());
    flow.update(|state| {
        state
            .sessions
            .get_mut("conversation-record")
            .unwrap()
            .health = Health::Stopped;
        Ok(())
    })
    .unwrap();
    assert!(
        flow.cleanup_preview(&project, &record.id())
            .await
            .unwrap()
            .can_remove()
    );
}
#[tokio::test]
async fn dirty_locked_original_missing_and_changed_preview_remain_protected() {
    let temp = tempfile::tempdir().unwrap();
    let (project, flow, record) = prepared(temp.path()).await;
    let checkout = record.worktree.clone().unwrap();
    let id = record.id();
    fs::write(checkout.path.join("precious.txt"), "keep").unwrap();
    assert!(flow.cleanup_preview(&project, &id).await.unwrap().dirty);
    assert!(
        flow.execute_with(
            &project,
            Command::Cleanup {
                ticket: id.clone(),
                expected_checkout: checkout.clone()
            },
            &Source { closed: false }
        )
        .await
        .is_err()
    );
    fs::remove_file(checkout.path.join("precious.txt")).unwrap();
    git::run(
        &project,
        ["worktree", "lock", checkout.path.to_str().unwrap()],
    )
    .await
    .unwrap();
    assert!(flow.cleanup_preview(&project, &id).await.unwrap().locked);
    assert!(
        git::remove_worktree(&project, &checkout.path)
            .await
            .is_err()
    );
    git::run(
        &project,
        ["worktree", "unlock", checkout.path.to_str().unwrap()],
    )
    .await
    .unwrap();
    let mut changed = checkout.clone();
    changed.branch = "another".into();
    assert!(
        flow.execute_with(
            &project,
            Command::Cleanup {
                ticket: id.clone(),
                expected_checkout: changed
            },
            &Source { closed: false }
        )
        .await
        .is_err()
    );
    let original = ticket(
        flow.execute_with(
            &project,
            Command::Open { number: 2 },
            &Source { closed: false },
        )
        .await
        .unwrap(),
    );
    let original = ticket(
        flow.execute_with(
            &project,
            Command::Prepare {
                ticket: original.id(),
                preparation: Preparation::Adopt {
                    path: project.clone(),
                },
            },
            &Source { closed: false },
        )
        .await
        .unwrap(),
    );
    assert!(
        flow.cleanup_preview(&project, &original.id())
            .await
            .unwrap()
            .original
    );
    assert!(git::remove_worktree(&project, &project).await.is_err());
    git::run(
        &project,
        ["worktree", "remove", checkout.path.to_str().unwrap()],
    )
    .await
    .unwrap();
    assert!(flow.cleanup_preview(&project, &id).await.unwrap().missing);
    assert_eq!(flow.snapshot().unwrap().tickets[&id].tasks, record.tasks);
}
#[tokio::test]
async fn terminal_closure_and_cleanup_preview_offer_explicit_stop_then_optional_removal() {
    let temp = tempfile::tempdir().unwrap();
    let (project, flow, record) = prepared(temp.path()).await;
    let owner = session(&record, Health::Live);
    flow.update(|state| {
        state.sessions.insert(owner.id.clone(), owner);
        Ok(())
    })
    .unwrap();
    let closed = ticket(
        flow.execute_with(
            &project,
            Command::Refresh {
                ticket: record.id(),
            },
            &Source { closed: true },
        )
        .await
        .unwrap(),
    );
    let (config, state) = config::open(&flow.config_path, false, None).unwrap();
    let mut app = App::new(config, state, flow.config_path.clone(), vec![], false);
    app.project = Some(git::project(&project).await.unwrap());
    app.ticket = Some(closed);
    app.jobs.clear();
    let mut terminal = Terminal::new(TestBackend::new(140, 35)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Ticket closed"));
    assert!(text.contains("Pending"));
    let preview = flow.cleanup_preview(&project, &record.id()).await.unwrap();
    app.accept(Completion {
        epoch: app.epoch,
        token: "workflow".into(),
        result: Ok(Payload::Workflow(Event::CleanupPreview(preview))),
    });
    assert!(matches!(app.modal, Some(Modal::Cleanup(_))));
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Unpublished local commits"));
    assert!(text.contains("Needs input"));
    assert!(text.contains("x explicitly stop"));
    app.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(app.jobs.is_empty());
    app.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    assert_eq!(
        app.pending_cleanup_stop.as_deref(),
        Some("conversation-record")
    );
    assert!(record.worktree.unwrap().path.exists());
}
#[tokio::test]
async fn actual_host_owner_must_be_verified_stopped_before_cleanup() {
    use std::time::{Duration, Instant};
    use treerail::host::runtime::{Client, HostCommand, Response};
    struct Guard(std::process::Child);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let (project, flow, record) = prepared(temp.path()).await;
    let checkout = record.worktree.clone().unwrap();
    let endpoint = format!(
        "cleanup-{}-{}",
        std::process::id(),
        temp.path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .replace('.', "-")
    );
    let _guard = Guard(
        Client::launch(
            Path::new(env!("CARGO_BIN_EXE_treerail-session-host")),
            &endpoint,
        )
        .unwrap(),
    );
    let client = Client::new(&endpoint);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match client.execute(HostCommand::Ping) {
            Ok(Response::Pong { .. }) => break,
            Err(_) if Instant::now() < deadline => std::thread::yield_now(),
            other => panic!("Host unavailable: {other:?}"),
        }
    }
    #[cfg(windows)]
    let (program, args) = ("cmd.exe", vec!["/D".into(), "/Q".into()]);
    #[cfg(unix)]
    let (program, args) = ("sh", vec![]);
    let Response::Started {
        session: host_id, ..
    } = client
        .execute(HostCommand::Start {
            request_id: "cleanup-live".into(),
            checkout: checkout.path.clone(),
            program: program.into(),
            args,
            rows: 24,
            cols: 80,
        })
        .unwrap()
    else {
        panic!("session start expected")
    };
    let mut owner = session(&record, Health::Live);
    owner.host_session = Some(host_id.clone());
    flow.update(|state| {
        state.sessions.insert(owner.id.clone(), owner.clone());
        Ok(())
    })
    .unwrap();
    assert!(
        git::remove_worktree(&project, &checkout.path)
            .await
            .is_err()
    );
    assert!(
        flow.execute_with(
            &project,
            Command::Cleanup {
                ticket: record.id(),
                expected_checkout: checkout.clone()
            },
            &Source { closed: false }
        )
        .await
        .is_err()
    );
    assert!(matches!(
        client.execute(HostCommand::Stop { session: host_id }),
        Ok(Response::Done)
    ));
    flow.update(|state| {
        state.sessions.get_mut(&owner.id).unwrap().health = Health::Stopped;
        Ok(())
    })
    .unwrap();
    flow.execute_with(
        &project,
        Command::Cleanup {
            ticket: record.id(),
            expected_checkout: checkout.clone(),
        },
        &Source { closed: false },
    )
    .await
    .unwrap();
    assert!(!checkout.path.exists());
    assert_eq!(
        flow.snapshot().unwrap().sessions[&owner.id].starting_brief,
        owner.starting_brief
    );
    client.execute(HostCommand::StopHost).unwrap();
}
