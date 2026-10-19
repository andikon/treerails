//! Entry-point changes must never redirect Git operations or agent input.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use std::path::Path;
use treerail::{
    app::{App, Job, Operation, workspace_ui::WorkspaceTab},
    config::{Config, State},
    git::{Project, Worktree},
    host::runtime::{HostCommand, Response, TerminalSnapshot},
    hosted::{self, SessionRecord},
    ui,
    workflow::{Checkout, TicketRecord},
};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ticket(project: &Path, number: u64, checkout: Option<Checkout>) -> TicketRecord {
    serde_json::from_value(serde_json::json!({
        "repository":"owner/repo", "project":project,
        "issue":{"number":number,"title":format!("Ticket {number}"),"body":"Requirements","state":"OPEN","url":format!("https://github.com/owner/repo/issues/{number}")},
        "fetched_at":1,"tasks":[{"id":"task-a","title":"Task A","progress":"pending"},{"id":"task-b","title":"Task B","progress":"pending"}],
        "selected_task":"task-a","worktree":checkout
    })).unwrap()
}

fn fixture(project: &Path) -> App {
    let mut app = App::new(
        Config::default(),
        State::default(),
        project.join("config.toml"),
        vec![],
        false,
    );
    app.project = Some(Project {
        path: project.into(),
        name: "repo".into(),
    });
    app.trees = vec![tree(project)];
    app.workspace_open = true;
    app.focus = 1;
    app
}

fn tree(path: &Path) -> Worktree {
    Worktree {
        path: path.into(),
        branch: "main".into(),
        head: "head".into(),
        original: true,
        locked: false,
        prunable: false,
    }
}

fn select(app: &mut App, ticket: TicketRecord) {
    let id = ticket.id();
    app.state
        .workflow
        .tickets
        .insert(id.clone(), ticket.clone());
    app.ticket = Some(ticket);
    app.workspace = Some(treerail::workspace::resolve(&app.state.workflow, &id).unwrap());
}

fn session(project: &Path, number: u64) -> SessionRecord {
    serde_json::from_value(serde_json::json!({
        "id":"session-a","ticket":format!("owner/repo#{number}"),"task":"task-a","checkout":project,
        "provider_session_id":"ead8e737-ac4f-48fd-83f4-c33b0ad5f5c2","starting_brief":"Approved objective",
        "created_at":1,"host_session":"terminal-a","health":"Live","current":true,"error":null
    })).unwrap()
}

fn attach_fake_terminal(app: &mut App, record: SessionRecord) {
    app.state
        .workflow
        .sessions
        .insert(record.id.clone(), record.clone());
    app.hosted.record = Some(record);
    app.hosted.token = Some(5);
    app.hosted.terminal = Some(TerminalSnapshot {
        session: "terminal-a".into(),
        process_id: Some(1),
        rows: 15,
        cols: 70,
        formatted: b"PREVIOUS-TASK-SECRET-TERMINAL".to_vec(),
        cursor: (0, 0),
        cursor_hidden: false,
        alternate_screen: false,
        bracketed_paste: false,
        sequence: 1,
        exited: false,
        input_token: Some(5),
        provider_session_id: None,
        attention: Default::default(),
        execution_group: None,
    });
}

fn rendered(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(130, 35)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn unprepared_ticket_cannot_act_on_the_previously_selected_checkout() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = fixture(temp.path());
    select(&mut app, ticket(temp.path(), 1, None));
    app.key(key(KeyCode::Char('2')));
    assert!(!app.workspace_changes_ready());
    app.key(key(KeyCode::Char('s')));
    app.operation(Operation::Commit(
        "Must not commit the previous checkout".into(),
        String::new(),
    ));
    assert!(
        !app.jobs
            .iter()
            .any(|job| matches!(job, Job::Operation { .. }))
    );
    assert!(!app.busy);
    assert!(app.modal.is_none());
}

#[test]
fn another_tasks_terminal_is_hidden_and_cannot_receive_entered_input() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = fixture(temp.path());
    select(
        &mut app,
        ticket(
            temp.path(),
            1,
            Some(Checkout {
                path: temp.path().into(),
                branch: "main".into(),
                original: true,
            }),
        ),
    );
    attach_fake_terminal(&mut app, session(temp.path(), 1));
    app.workspace_tab = WorkspaceTab::Agent;
    app.task_cursor = 1;
    assert!(!rendered(&app).contains("PREVIOUS-TASK-SECRET-TERMINAL"));
    app.key(key(KeyCode::Enter));
    assert!(!app.hosted.input);
    app.paste("Do not send this to Task A");
    assert!(
        !app.hosted
            .queue
            .iter()
            .any(|command| matches!(command, HostCommand::Input { .. }))
    );
}

#[test]
fn task_navigation_releases_the_previous_tasks_input_attachment() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = fixture(temp.path());
    select(
        &mut app,
        ticket(
            temp.path(),
            1,
            Some(Checkout {
                path: temp.path().into(),
                branch: "main".into(),
                original: true,
            }),
        ),
    );
    attach_fake_terminal(&mut app, session(temp.path(), 1));
    app.workspace_tab = WorkspaceTab::Agent;
    app.key(key(KeyCode::Char('j')));
    assert_eq!(app.task_cursor, 1);
    app.key(key(KeyCode::Enter));
    assert!(!app.hosted.input);
    assert!(app.hosted.terminal.is_none());
    assert!(
        app.hosted
            .queue
            .iter()
            .any(|command| matches!(command, HostCommand::Detach { .. }))
    );
}

#[test]
fn delayed_start_reply_cannot_attach_after_switching_workspaces() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = fixture(temp.path());
    select(&mut app, ticket(temp.path(), 2, None));
    app.workspace_tab = WorkspaceTab::Agent;
    let started = session(temp.path(), 1);
    app.accept_host(Response::Ticket(Box::new(hosted::Event::Started(
        started.clone(),
    ))));
    assert_eq!(app.current_workspace().unwrap().id, "owner/repo#2");
    assert_eq!(app.state.workflow.sessions[&started.id], started);
    assert!(app.hosted.record.is_none());
    assert!(app.hosted.token.is_none());
    assert!(
        !app.hosted
            .queue
            .iter()
            .any(|command| matches!(command, HostCommand::Attach { .. }))
    );
}

#[test]
fn checkout_not_yet_loaded_cannot_use_the_old_checkout_for_git_actions() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target-checkout");
    std::fs::create_dir(&target).unwrap();
    let mut app = fixture(temp.path());
    select(
        &mut app,
        ticket(
            temp.path(),
            1,
            Some(Checkout {
                path: target.clone(),
                branch: "main".into(),
                original: false,
            }),
        ),
    );
    app.key(key(KeyCode::Char('2')));
    assert!(!app.workspace_changes_ready());
    app.operation(Operation::Push);
    assert!(
        !app.jobs
            .iter()
            .any(|job| matches!(job, Job::Operation { .. }))
    );
    app.trees.push(tree(&target));
    app.sync_workspace_checkout();
    assert!(app.workspace_changes_ready());
    assert_eq!(app.selected_tree().unwrap().path, target);
}
