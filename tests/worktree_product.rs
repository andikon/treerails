use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;
use std::{fs, path::PathBuf};
use tempfile::TempDir;
use treerail::{
    agent::{Attention, Command, Conversation},
    app::{Action, App, Completion, Confirm, FormKind, Input, Modal, Operation, View},
    config::{Config, State},
    git::{Project, Snapshot, Worktree},
    startup::Session,
    ui,
};

fn setup() -> (TempDir, App) {
    let temp = TempDir::new().unwrap();
    let project = Project {
        name: "treerails".into(),
        path: temp.path().join("treerails"),
    };
    fs::create_dir(&project.path).unwrap();
    let tree = Worktree {
        path: project.path.clone(),
        branch: "feat/search".into(),
        head: "b7c42de012345".into(),
        original: true,
        locked: false,
        prunable: false,
    };
    let mut app = App::new(Session {
        config: Config::default(),
        state: State::default(),
        config_path: temp.path().join("config.toml"),
        roots: vec![temp.path().into()],
        recursive: false,
    });
    app.projects = vec![project.clone()];
    app.open_project(0);
    app.accept(
        format!("trees:{}", project.path.display()),
        Ok(Completion::Trees(project, vec![tree])),
    );
    app.snapshots.insert(
        app.tree().unwrap().path.clone(),
        Snapshot {
            branch: "feat/search".into(),
            ..Default::default()
        },
    );
    (temp, app)
}
fn conversation(app: &App, id: &str, attention: Attention) -> Conversation {
    Conversation {
        id: id.into(),
        project: app.tab().unwrap().project.path.clone(),
        worktree: app.tree().unwrap().path.clone(),
        title: format!("Conversation {id}"),
        session_uri: id.into(),
        chat_uri: Some(format!("ahp-chat:/{id}")),
        attention,
        archived: false,
        draft: String::new(),
        chat: None,
        error: None,
        pending: vec![],
        options: json!({}),
        owned: true,
        initial_prompt: None,
        creation_failed: false,
        model: None,
    }
}
fn press(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}
fn render(app: &mut App, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| ui::draw(f, app)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
        .collect()
}

#[test]
fn both_views_share_one_footer_and_fixed_pane_geometry() {
    let (_temp, mut app) = setup();
    let before = render(&mut app, 140, 36);
    app.action(Action::Switch(View::Changes));
    let after = render(&mut app, 140, 36);
    assert_eq!(before[35], after[35]);
    assert!(before[35].contains("feat/search"));
    assert!(before[2].contains("Worktrees"));
    assert!(before[2].contains("Sessions"));
    assert!(after[2].contains("Changes"));
    for y in 2..35 {
        assert_eq!(before[y].chars().nth(29), after[y].chars().nth(29));
    }
    assert!(!before[0].contains("feat/search"));
    assert!(!before[35].contains("Ctrl"));
    assert!(!before.join("\n").contains("Tickets"));
}
#[test]
fn compact_and_narrow_windows_do_not_panic() {
    let (_t, mut app) = setup();
    for (width, height) in [(140, 36), (80, 24), (60, 16), (40, 10)] {
        render(&mut app, width, height);
        app.action(Action::NewSessionOptions);
        render(&mut app, width, height);
        app.action(Action::Cancel);
        app.action(Action::Help);
        render(&mut app, width, height);
        app.action(Action::Cancel);
    }
}
#[test]
fn advanced_session_creation_starts_small_and_defaults_to_current_checkout() {
    let (_temp, mut app) = setup();
    app.action(Action::NewSessionOptions);
    let Some(Modal::Form(form)) = &app.modal else {
        panic!("No session form")
    };
    assert_eq!(form.kind, FormKind::Session);
    assert_eq!(form.value("new_tree"), "no");
    assert_eq!(form.visible().len(), 2);
    assert!(form.value("base").is_empty());
    assert_eq!(form.value("fetch"), "yes");
    let shown = render(&mut app, 120, 35).join("\n");
    assert!(shown.contains("First prompt"));
    assert!(!shown.contains("Starting point"));
    app.paste("Add tests");
    app.action(Action::Confirm);
    assert_eq!(app.state.conversations.len(), 1);
    assert_eq!(app.state.conversations[0].attention, Attention::Running);
    assert!(
        app.agent_commands
            .iter()
            .any(|c| matches!(c,Command::Create{prompt,..} if prompt=="Add tests"))
    );
    assert!(app.modal.is_none());
}
#[test]
fn create_new_worktree_toggle_reveals_fresh_remote_defaults() {
    let (_temp, mut app) = setup();
    app.action(Action::NewSessionOptions);
    app.action(Action::Field(1));
    let Some(Modal::Form(form)) = &app.modal else {
        panic!()
    };
    assert_eq!(form.value("new_tree"), "yes");
    assert!(form.visible().len() > 2);
    assert_eq!(form.value("base"), "");
    assert_eq!(form.value("fetch"), "yes");
}
#[test]
fn busy_worktree_blocks_all_git_writes_but_allows_fetch() {
    let (_temp, mut app) = setup();
    for attention in [
        Attention::Running,
        Attention::NeedsInput,
        Attention::Unavailable,
    ] {
        app.state.conversations = vec![conversation(&app, "active", attention)];
        for operation in [
            Operation::Stage(vec![], false),
            Operation::Stage(vec![], true),
            Operation::Commit("subject".into(), "".into()),
            Operation::Pull,
            Operation::Push(None),
            Operation::Remove,
        ] {
            app.operate(operation);
            assert!(app.mutation.is_none());
        }
        app.operate(Operation::Fetch);
        assert!(app.mutation.is_some());
        app.mutation = None;
        app.jobs.clear();
        app.accept("mutation".into(), Err(anyhow::anyhow!("fixture complete")));
    }
}
#[test]
fn close_running_project_warns_and_hides_without_stopping() {
    let (_temp, mut app) = setup();
    app.state
        .conversations
        .push(conversation(&app, "active", Attention::Running));
    app.action(Action::Close(0));
    assert!(matches!(app.modal, Some(Modal::Confirm(Confirm::Close(0)))));
    app.action(Action::Confirm);
    assert!(app.tabs.is_empty());
    assert_eq!(app.state.conversations[0].attention, Attention::Running);
    assert!(app.agent_commands.is_empty());
    app.action(Action::Activity);
    app.action(Action::Confirm);
    assert_eq!(app.tabs.len(), 1);
}
#[test]
fn quitting_active_work_requires_an_explicit_confirmation() {
    let (_t, mut app) = setup();
    app.state
        .conversations
        .push(conversation(&app, "active", Attention::NeedsInput));
    app.action(Action::Quit);
    assert!(!app.quit);
    assert!(matches!(app.modal, Some(Modal::Confirm(Confirm::Quit))));
    app.action(Action::Cancel);
    assert!(!app.quit);
    app.action(Action::Quit);
    app.action(Action::Confirm);
    assert!(app.quit);
}
#[test]
fn saved_conversation_resume_never_sends_a_prompt() {
    let (_t, mut app) = setup();
    app.state
        .conversations
        .push(conversation(&app, "saved", Attention::Ready));
    app.action(Action::Session("saved".into()));
    app.action(Action::Resume);
    assert!(matches!(app.agent_commands.as_slice(),[Command::Observe{id}] if id=="saved"));
}
#[test]
fn text_editing_handles_unicode_and_does_not_trigger_navigation() {
    let (_t, mut app) = setup();
    app.action(Action::NewSessionOptions);
    app.paste("hello 🦀 café");
    for code in [
        KeyCode::Left,
        KeyCode::Backspace,
        KeyCode::Char('q'),
        KeyCode::Char('j'),
        KeyCode::Char('k'),
    ] {
        press(&mut app, code);
    }
    assert!(!app.quit);
    assert_eq!(app.state.conversations.len(), 0);
    let Some(Modal::Form(form)) = &app.modal else {
        panic!()
    };
    assert!(form.value("prompt").contains("qjk"));
    let mut input = Input::new("🦀é");
    input.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE), false);
    assert_eq!(input.value, "🦀");
    input.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE), false);
    assert!(input.value.is_empty());
}
#[test]
fn session_plus_starts_copilot_without_a_form() {
    let (_t, mut app) = setup();
    render(&mut app, 120, 36);
    let plus = app
        .hits
        .iter()
        .find(|h| matches!(h.action, Action::NewSession))
        .unwrap()
        .rect;
    app.mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: plus.x,
        row: plus.y,
        modifiers: KeyModifiers::NONE,
    });
    assert!(app.modal.is_none());
    assert_eq!(app.state.conversations.len(), 1);
    assert!(
        matches!(app.agent_commands.as_slice(), [Command::Create { prompt, worktree, .. }] if prompt.is_empty() && worktree == &app.tree().unwrap().path)
    );
    let shown = render(&mut app, 120, 36).join("\n");
    assert!(shown.contains("Starting Copilot CLI"));
    assert!(!shown.contains("First prompt"));
}
#[test]
fn key_release_does_not_start_a_second_session() {
    let (_t, mut app) = setup();
    let mut key = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE);
    key.kind = KeyEventKind::Release;
    app.key(key);
    assert!(app.modal.is_none());
    assert!(app.agent_commands.is_empty());
}

#[test]
fn new_session_shortcut_starts_idle_copilot_and_obeys_checkout_guards() {
    let (_temp, mut app) = setup();
    press(&mut app, KeyCode::Char('n'));
    assert!(app.modal.is_none());
    assert_eq!(app.state.conversations.len(), 1);
    assert!(
        matches!(app.agent_commands.as_slice(), [Command::Create {prompt, ..}] if prompt.is_empty())
    );
    press(&mut app, KeyCode::Char('n'));
    assert_eq!(
        app.state.conversations.len(),
        1,
        "Pending creation blocks another launch"
    );
    app.state.conversations[0].attention = Attention::Ready;
    press(&mut app, KeyCode::Char('n'));
    assert_eq!(
        app.state.conversations.len(),
        2,
        "Ready conversations permit a new session"
    );
}

#[test]
fn worktree_rows_show_colored_additions_and_deletions() {
    let (_temp, mut app) = setup();
    let snapshot = app
        .snapshots
        .get_mut(&app.tree().unwrap().path.clone())
        .unwrap();
    snapshot.additions = 42;
    snapshot.deletions = 9;
    let shown = render(&mut app, 140, 36);
    assert!(shown[..10].join("\n").contains("+42 -9"));
}
#[test]
fn missing_checkout_history_is_read_only() {
    let (_t, mut app) = setup();
    let mut c = conversation(&app, "missing", Attention::Ready);
    c.worktree = PathBuf::from("Z:/does-not-exist/removed");
    app.state.conversations.push(c);
    app.action(Action::Session("missing".into()));
    app.connected = true;
    app.composer = Input::new("Must not send");
    app.action(Action::FocusTerminal);
    assert!(app.agent_commands.is_empty());
    app.action(Action::Resume);
    assert!(app.agent_commands.is_empty());
}
#[test]
fn restarting_restores_projects_but_never_restarts_agents() {
    let (_t, mut app) = setup();
    app.state
        .conversations
        .push(conversation(&app, "running", Attention::Running));
    let state = app.state.clone();
    let app = App::new(Session {
        config: Config::default(),
        state,
        config_path: PathBuf::from("unused/config.toml"),
        roots: vec![],
        recursive: false,
    });
    assert_eq!(app.state.conversations[0].attention, Attention::Interrupted);
    assert!(app.agent_commands.is_empty());
    assert!(matches!(app.modal, Some(Modal::Projects { .. })));
}
#[test]
fn approval_escape_does_not_approve_and_explicit_choice_uses_same_request() {
    let (_t, mut app) = setup();
    let mut c = conversation(&app, "waiting", Attention::NeedsInput);
    c.pending = vec![
        json!({"kind":"toolConfirmation","chat":"ahp-chat:/waiting","turnId":"turn-1","toolCall":{"toolCallId":"tool-1","status":"pending-confirmation","displayName":"Run tests"}}),
    ];
    app.state.conversations.push(c);
    app.action(Action::Session("waiting".into()));
    app.action(Action::Respond);
    press(&mut app, KeyCode::Esc);
    assert!(app.agent_commands.is_empty());
    app.action(Action::Respond);
    press(&mut app, KeyCode::Char('y'));
    assert!(
        matches!(app.agent_commands.as_slice(),[Command::Dispatch{action,..}] if action["type"]=="chat/toolCallConfirmed" && action["turnId"]=="turn-1" && action["toolCallId"]=="tool-1" && action["approved"]==true)
    );
}

#[test]
fn confirmation_defaults_to_cancel_and_supports_keyboard_choice() {
    let (_t, mut app) = setup();
    app.state
        .conversations
        .push(conversation(&app, "running", Attention::Running));
    app.action(Action::Quit);
    press(&mut app, KeyCode::Enter);
    assert!(!app.quit);
    assert!(app.modal.is_none());
    app.action(Action::Quit);
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Enter);
    assert!(app.quit);
}

#[test]
fn failed_worktree_creation_retains_prompt_and_explicit_retry_choices() {
    let (_t, mut app) = setup();
    app.action(Action::NewSessionOptions);
    app.paste("Keep my opening prompt");
    app.action(Action::Field(1));
    if let Some(Modal::Form(form)) = &mut app.modal {
        form.fields
            .iter_mut()
            .find(|f| f.key == "branch")
            .unwrap()
            .input = Input::new("feat/retry");
    }
    app.action(Action::Confirm);
    assert!(app.mutation.is_some());
    app.accept("mutation".into(), Err(anyhow::anyhow!("Fetch failed")));
    let Some(Modal::Form(form)) = &app.modal else {
        panic!("Draft was lost")
    };
    assert_eq!(form.value("prompt"), "Keep my opening prompt");
    assert_eq!(form.value("branch"), "feat/retry");
    assert_eq!(form.value("base"), "");
    assert!(app.state.conversations.is_empty());
}

#[test]
fn failed_native_launch_keeps_an_unsent_draft_and_releases_ui_ownership() {
    let (_t, mut app) = setup();
    app.action(Action::NewSessionOptions);
    app.paste("Preserve this prompt");
    app.action(Action::Confirm);
    let id = app.state.conversations[0].id.clone();
    app.agent_event(treerail::agent::Event::LaunchFailed {
        id,
        message: "Provider unavailable".into(),
    });
    let c = &app.state.conversations[0];
    assert_eq!(c.attention, Attention::Interrupted);
    assert_eq!(c.draft, "Preserve this prompt");
    assert!(!app.busy(&c.worktree));
    assert!(c.initial_prompt.is_none());
}

#[test]
fn multiline_input_moves_by_unicode_character_and_line() {
    let mut input = Input::new("hello\n🦀é\nlast");
    input.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), true);
    assert_eq!(&input.value[..input.cursor], "hello\n🦀é");
    input.key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE), true);
    assert_eq!(&input.value[..input.cursor], "hello\n");
    input.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), true);
    assert_eq!(&input.value[..input.cursor], "hello\n🦀é\n");
}

#[test]
fn forgetting_a_location_does_not_delete_it() {
    let (temp, mut app) = setup();
    app.state.remembered_sources.push(temp.path().to_path_buf());
    app.roots.push(temp.path().to_path_buf());
    app.action(Action::ForgetSource(0));
    assert!(app.state.remembered_sources.is_empty());
    assert!(temp.path().is_dir());
}

#[test]
fn native_folder_trust_requires_explicit_confirmation() {
    let (_temp, mut app) = setup();
    let mut c = conversation(&app, "trust", Attention::Running);
    c.initial_prompt = Some("Keep this unsent".into());
    let path = c.worktree.clone();
    app.state.conversations.push(c);
    app.agent_commands.clear();
    app.agent_event(treerail::agent::Event::FolderTrust {
        id: "trust".into(),
        path: path.clone(),
    });
    assert!(!app.confirm_yes);
    assert!(
        render(&mut app, 140, 36)
            .join("\n")
            .contains("Trust folder for Copilot?")
    );
    press(&mut app, KeyCode::Esc);
    assert!(matches!(
        app.agent_commands.last(),
        Some(Command::TrustWorkspace {
            approved: false,
            ..
        })
    ));
    assert_eq!(
        app.state.conversations[0].initial_prompt.as_deref(),
        Some("Keep this unsent")
    );
    app.agent_event(treerail::agent::Event::FolderTrust {
        id: "trust".into(),
        path,
    });
    press(&mut app, KeyCode::Char('y'));
    assert!(matches!(
        app.agent_commands.last(),
        Some(Command::TrustWorkspace { approved: true, .. })
    ));
}
