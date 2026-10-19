use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::{Path, PathBuf};
use treerail::{
    app::{
        App, Completion, Job, Modal, Operation, Payload, hosted_ui::Dialog,
        workspace_ui::WorkspaceTab,
    },
    config::{Config, State, same_path},
    git,
    workflow::{Command, Event, Issue, IssueSource, Preparation, Workflow},
};

struct Source;
impl IssueSource for Source {
    async fn repository(&self, _: &Path) -> Result<String> {
        Ok("example/repo".into())
    }
    async fn list(&self, _: &Path) -> Result<Vec<Issue>> {
        Ok(vec![])
    }
    async fn get(&self, _: &Path, number: u64) -> Result<Issue> {
        Ok(Issue {
            number,
            title: "Requested change".into(),
            body: "Complete issue requirements".into(),
            state: "OPEN".into(),
            url: format!("https://github.com/example/repo/issues/{number}"),
        })
    }
}
async fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, App) {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "--initial-branch=main"],
        vec!["config", "user.name", "Local test"],
        vec!["config", "user.email", "test@example.invalid"],
        vec!["commit", "--allow-empty", "-m", "Initial"],
    ] {
        git::run(&repo, args).await.unwrap();
    }
    let checkout = temp.path().join("feature");
    git::run(
        &repo,
        [
            "worktree",
            "add",
            "-b",
            "feature/test",
            checkout.to_str().unwrap(),
        ],
    )
    .await
    .unwrap();
    let mut app = App::new(
        Config::default(),
        State::default(),
        temp.path().join("settings.toml"),
        vec![],
        false,
    );
    app.project = Some(git::project(&repo).await.unwrap());
    app.trees = git::worktrees(&repo).await.unwrap();
    app.tree = app
        .trees
        .iter()
        .position(|tree| same_path(&tree.path, &checkout))
        .unwrap();
    app.ticket_mode = false;
    app.jobs.clear();
    app.inflight.clear();
    (temp, repo, checkout, app)
}
fn key(app: &mut App, ch: char) {
    app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
}
fn accept(app: &mut App, event: Event) {
    app.accept(Completion {
        epoch: app.epoch,
        token: "workflow".into(),
        result: Ok(Payload::Workflow(event)),
    });
}

#[tokio::test]
async fn inspecting_a_checkout_is_passive_and_start_agent_creates_a_real_local_task() {
    let (_temp, repo, checkout, mut app) = fixture().await;
    app.open_worktree_workspace();
    assert!(app.workspace_open);
    assert_eq!(app.workspace_tab, WorkspaceTab::Changes);
    assert!(app.current_workspace().is_none());
    assert!(app.state.workflow.local_work.is_empty());
    assert!(!app.config_path.exists());
    assert!(
        !app.jobs
            .iter()
            .any(|job| matches!(job, Job::Host(..) | Job::Workflow(..)))
    );
    key(&mut app, 'S');
    assert!(matches!(app.modal, Some(Modal::Form(_))));
    app.paste("Investigate the failure\nPreserve the complete reproduction details");
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    let command = app
        .jobs
        .iter()
        .find_map(|job| {
            if let Job::Workflow(_, _, command @ Command::CreateLocal { .. }) = job {
                Some(command.clone())
            } else {
                None
            }
        })
        .unwrap();
    let event = Workflow::new(&app.config_path)
        .execute_with(&repo, command, &Source)
        .await
        .unwrap();
    accept(&mut app, event);
    let workspace = app.current_workspace().unwrap();
    assert!(workspace.issue.is_none());
    assert!(workspace.id.starts_with("local:"));
    assert!(same_path(
        &workspace.worktree.as_ref().unwrap().path,
        &checkout
    ));
    assert_eq!(app.workspace_tab, WorkspaceTab::Agent);
    assert!(app.state.workflow.tickets.is_empty());
    let task = &workspace.tasks[0];
    assert!(
        app.state.workflow.local_work[&workspace.id].objectives[&task.id]
            .contains("complete reproduction")
    );
    assert!(
        matches!(&app.hosted.dialog,Some(Dialog::Start{ticket,task:chosen,..}) if ticket==&workspace.id && chosen==&task.id)
    );
    assert!(app.hosted.queue.iter().any(|command| matches!(
        command,
        treerail::host::runtime::HostCommand::Ticket {
            command: treerail::hosted::Command::CaptureDiscussion { .. },
            ..
        }
    )));
    assert!(
        app.state.workflow.sessions.is_empty(),
        "Reviewing must not start an agent"
    );
}

#[tokio::test]
async fn ticket_and_worktree_routes_share_checkout_and_git_stage_never_starts_an_agent() {
    let (_temp, repo, checkout, mut app) = fixture().await;
    let flow = Workflow::new(&app.config_path);
    let Event::Ticket(record) = flow
        .execute_with(&repo, Command::Open { number: 7 }, &Source)
        .await
        .unwrap()
    else {
        panic!()
    };
    let event = flow
        .execute_with(
            &repo,
            Command::Prepare {
                ticket: record.id(),
                preparation: Preparation::Adopt {
                    path: checkout.clone(),
                },
            },
            &Source,
        )
        .await
        .unwrap();
    accept(&mut app, event);
    app.ticket_mode = true;
    // Deliberately start on another checkout; the workspace must select its own files.
    app.tree = app
        .trees
        .iter()
        .position(|tree| same_path(&tree.path, &repo))
        .unwrap();
    key(&mut app, '2');
    assert!(same_path(&app.selected_tree().unwrap().path, &checkout));
    std::fs::write(checkout.join("change.txt"), "A local change").unwrap();
    let selected = app.selected_tree().unwrap().path.clone();
    app.snapshots
        .insert(selected.clone(), git::status(&selected).await.unwrap());
    app.jobs.clear();
    app.inflight.clear();
    key(&mut app, 's');
    let index = app
        .jobs
        .iter()
        .position(|job| {
            matches!(
                job,
                Job::Operation {
                    op: Operation::Stage(..),
                    ..
                }
            )
        })
        .expect("s stages in Changes even when entered from Tickets");
    let job = app.jobs.remove(index);
    assert!(matches!(&job,Job::Operation{path,..} if same_path(path,&checkout)));
    job.execute().await.unwrap();
    assert!(
        git::status(&checkout)
            .await
            .unwrap()
            .files
            .iter()
            .any(|file| file.staged())
    );
    assert!(!app.jobs.iter().any(|job| matches!(job, Job::Host(..))));
    let id = app.current_workspace().unwrap().id;
    app.switch_entry("worktrees");
    app.open_worktree_workspace();
    assert_eq!(
        app.current_workspace().unwrap().id,
        id,
        "Both routes must reuse the same task workspace"
    );
}

#[tokio::test]
async fn linking_preserves_task_identity_and_discussion_does_not_replace_browser_context() {
    let (_temp, repo, checkout, mut app) = fixture().await;
    let flow = Workflow::new(&app.config_path);
    let Event::Workspace(local) = flow
        .execute_with(
            &repo,
            Command::CreateLocal {
                path: checkout,
                objective: "Local investigation".into(),
            },
            &Source,
        )
        .await
        .unwrap()
    else {
        panic!()
    };
    let local_task = local.tasks[0].id.clone();
    app.state.workflow = flow.snapshot().unwrap();
    app.open_worktree_workspace();
    let Event::Ticket(ticket) = flow
        .execute_with(&repo, Command::Open { number: 7 }, &Source)
        .await
        .unwrap()
    else {
        panic!()
    };
    let event = flow
        .execute_with(
            &repo,
            Command::LinkLocal {
                local: local.id,
                ticket: ticket.id(),
            },
            &Source,
        )
        .await
        .unwrap();
    accept(&mut app, event);
    assert_eq!(app.ticket.as_ref().unwrap().issue.number, 7);
    assert_eq!(
        app.current_workspace().unwrap().tasks[app.task_cursor].id,
        local_task
    );
    app.browser.repository = "github.com/example/repo".into();
    app.browser.selected = Some(99);
    app.browser.preview_scroll = 17;
    key(&mut app, '4');
    assert_eq!(app.browser.selected, Some(7));
    assert_eq!(app.browser.repository, "github.com/example/repo");
    key(&mut app, 'T');
    assert!(!app.workspace_open);
    assert_eq!(app.browser.selected, Some(99));
    assert_eq!(app.browser.preview_scroll, 17);
    let rows = treerail::work::rows(&app.state.workflow, &repo);
    assert_eq!(rows.iter().filter(|row| row.task == local_task).count(), 1);
}

#[tokio::test]
async fn delayed_local_creation_stays_durable_without_reopening_a_departed_workspace() {
    let (_temp, repo, checkout, mut app) = fixture().await;
    app.open_worktree_workspace();
    key(&mut app, 'S');
    app.paste("Investigate locally");
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(app.workspace_start_pending);
    app.switch_entry("work");
    let event = Workflow::new(&app.config_path)
        .execute_with(
            &repo,
            Command::CreateLocal {
                path: checkout,
                objective: "Investigate locally".into(),
            },
            &Source,
        )
        .await
        .unwrap();
    accept(&mut app, event);
    assert!(app.work_mode);
    assert!(!app.workspace_open);
    assert!(!app.workspace_start_pending);
    assert!(app.hosted.dialog.is_none());
    assert_eq!(app.state.workflow.local_work.len(), 1);
    assert!(app.state.workflow.sessions.is_empty());
}

#[tokio::test]
async fn repeated_workspace_actions_do_not_create_tasks_or_open_agent_review() {
    let (_temp, _repo, _checkout, mut app) = fixture().await;
    app.open_worktree_workspace();
    app.jobs.clear();
    for code in [
        KeyCode::Char('S'),
        KeyCode::Char('A'),
        KeyCode::Char('s'),
        KeyCode::Char('p'),
    ] {
        let mut event = KeyEvent::new(code, KeyModifiers::NONE);
        event.kind = crossterm::event::KeyEventKind::Repeat;
        app.key(event);
    }
    assert!(app.modal.is_none());
    assert!(app.hosted.dialog.is_none());
    assert!(app.jobs.is_empty());
    key(&mut app, 'w');
    assert!(!app.workspace_open);
    assert!(!app.ticket_mode);
    assert!(!app.work_mode);
}
