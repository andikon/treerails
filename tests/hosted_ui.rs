use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use treerail::{
    app::App,
    config::{Config, State},
    host::runtime::{HostCommand, Response, TerminalSnapshot},
    hosted, ui,
};

fn app() -> App {
    let mut app = App::new(
        Config::default(),
        State::default(),
        "unused.toml".into(),
        vec![],
        false,
    );
    let record:hosted::SessionRecord=serde_json::from_value(serde_json::json!({
        "id":"record-1","ticket":"owner/repo#1","task":"task-1","checkout":".",
        "provider_session_id":"ead8e737-ac4f-48fd-83f4-c33b0ad5f5c2","starting_brief":"Original requirements",
        "created_at":1,"host_session":"host-session-1","health":"Live","current":true,"error":null
    })).unwrap();
    app.state
        .workflow
        .sessions
        .insert(record.id.clone(), record.clone());
    app.hosted.record = Some(record);
    app.hosted.terminal = Some(TerminalSnapshot {
        session: "host-session-1".into(),
        process_id: Some(1),
        rows: 15,
        cols: 70,
        formatted: b"\x1b[?2004h\x1b[32mNative permission prompt\x1b[0m".to_vec(),
        cursor: (0, 24),
        cursor_hidden: false,
        alternate_screen: false,
        bracketed_paste: true,
        sequence: 1,
        exited: false,
        input_token: Some(5),
        provider_session_id: Some("ead8e737-ac4f-48fd-83f4-c33b0ad5f5c2".into()),
        attention: treerail::copilot::Attention::default(),
        execution_group: None,
    });
    app.hosted.token = Some(5);
    app.project = Some(treerail::git::Project {
        path: ".".into(),
        name: "repo".into(),
    });
    app
}
fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}
fn rendered(app: &App) -> (String, ratatui::buffer::Buffer) {
    let mut terminal = Terminal::new(TestBackend::new(120, 35)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let text = buffer.content.iter().map(|cell| cell.symbol()).collect();
    (text, buffer)
}

#[test]
fn agent_input_delivers_copilot_controls_paste_and_configurable_navigation_escape() {
    let mut app = app();
    app.hosted.input = true;
    app.key(key(KeyCode::Esc, KeyModifiers::NONE));
    app.key(key(KeyCode::Char('c'), KeyModifiers::CONTROL));
    app.paste("requirements\nsecond line");
    assert!(!app.quit);
    let packets = app
        .hosted
        .queue
        .drain(..)
        .flat_map(|command| match command {
            HostCommand::Input { bytes, .. } => bytes,
            _ => panic!("Input expected"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        packets,
        b"\x1b\x03\x1b[200~requirements\nsecond line\x1b[201~"
    );
    app.config
        .keys
        .insert("ctrl+g".into(), "agent_navigation".into());
    app.key(key(KeyCode::Char(']'), KeyModifiers::CONTROL));
    assert!(app.hosted.input, "Custom escape replaces the default");
    app.key(key(KeyCode::Char('g'), KeyModifiers::CONTROL));
    assert!(!app.hosted.input);
    assert!(!app.quit);
}

#[test]
fn native_screen_modes_and_background_attention_are_visible_without_focus_changes() {
    let mut app = app();
    let (navigation, nav_buffer) = rendered(&app);
    assert!(navigation.contains("NAVIGATION"));
    assert!(navigation.contains("Native permission prompt"));
    app.hosted.input = true;
    let mut attention = treerail::copilot::Attention::default();
    attention.observe("elicitation.requested", Some("question-2"));
    app.accept_host(Response::Summary(vec![
        treerail::host::runtime::SessionSummary {
            session: "other".into(),
            provider_session_id: None,
            attention,
            exited: false,
            notification_error: None,
        },
    ]));
    let (input, input_buffer) = rendered(&app);
    assert!(app.hosted.input);
    assert!(input.contains("AGENT INPUT"));
    assert!(input.contains("Needs input"));
    assert_ne!(nav_buffer, input_buffer, "Mode colors and labels change");
    app.detach_agent();
    assert!(!app.hosted.input);
    assert!(app.hosted.terminal.is_none());
    assert!(matches!(
        app.hosted.queue.back(),
        Some(HostCommand::Detach { .. })
    ));
    assert!(app.state.workflow.sessions.contains_key("record-1"));
}

#[test]
fn agent_input_legend_only_advertises_its_actual_escape_binding() {
    let mut app = app();
    app.hosted.input = true;
    let (screen, _) = rendered(&app);
    assert!(screen.contains("Ctrl+] Tree Rail navigation"));
    assert!(screen.contains("Keys go to Copilot"));
    for inactive in [
        "d detach",
        "x stop",
        "E diagnostics",
        "s continue",
        "Enter input",
    ] {
        assert!(
            !screen.contains(inactive),
            "Agent mode advertised {inactive}"
        );
    }
    app.config
        .keys
        .insert("ctrl+g".into(), "agent_navigation".into());
    let (screen, _) = rendered(&app);
    assert!(screen.contains("Ctrl+g Tree Rail navigation"));
    assert!(!screen.contains("Ctrl+]"));
    let mut terminal = Terminal::new(TestBackend::new(20, 35)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let footer = terminal.backend().buffer().content[34 * 20..]
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(footer.contains("Ctrl+g navigation"));
}

fn with_ticket(app: &mut App, directory: &std::path::Path) {
    let ticket: treerail::workflow::TicketRecord = serde_json::from_value(serde_json::json!({
        "repository":"owner/repo", "project":directory, "issue":{"number":1,"title":"Long issue","body":(0..120).map(|n|format!("requirement-line-{n:03}\n")).collect::<String>(),"state":"OPEN","url":"https://github.com/owner/repo/issues/1"},
        "fetched_at":1,"tasks":[{"id":"task-1","title":"Implement","progress":"pending"}],"next_task_id":2,"selected_task":"task-1","worktree":{"path":directory,"branch":"ticket/1","original":false}
    })).unwrap();
    app.state
        .workflow
        .tickets
        .insert(ticket.id(), ticket.clone());
    app.ticket = Some(ticket);
    if let Some(record) = app.state.workflow.sessions.get_mut("record-1") {
        record.checkout = directory.into();
    }
}
#[test]
fn brief_review_scrolls_and_handoff_is_explicit_multiline_without_replaying_native_input() {
    use treerail::app::hosted_ui::{Dialog, draft_brief};
    let temp = tempfile::tempdir().unwrap();
    let mut app = app();
    with_ticket(&mut app, temp.path());
    app.begin_agent(true);
    assert!(matches!(app.hosted.dialog, Some(Dialog::Start { .. })));
    assert!(app.hosted.queue.is_empty());
    let (first, _) = rendered(&app);
    assert!(first.contains("requirement-line-000"));
    assert!(!first.contains("requirement-line-050"));
    for _ in 0..5 {
        app.key(key(KeyCode::PageDown, KeyModifiers::NONE));
    }
    let (scrolled, _) = rendered(&app);
    assert!(scrolled.contains("requirement-line-04") || scrolled.contains("requirement-line-05"));
    app.key(key(KeyCode::Tab, KeyModifiers::NONE));
    app.paste("Review handoff\nPreserve identity");
    app.key(key(KeyCode::Char('y'), KeyModifiers::NONE));
    let Some(Dialog::Start {
        brief,
        handoff,
        editing,
        ..
    }) = &app.hosted.dialog
    else {
        panic!()
    };
    assert!(*editing);
    assert_eq!(handoff.value, "Review handoff\nPreserve identityy");
    assert!(draft_brief(brief, &handoff.value).contains("Preserve identityy"));
    assert!(app.hosted.queue.is_empty());
    app.key(key(KeyCode::Tab, KeyModifiers::NONE));
    app.key(key(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(
        matches!(app.hosted.queue.back(),Some(HostCommand::Ticket{command:hosted::Command::StartRequest{handoff,..},..})if handoff=="Review handoff\nPreserve identityy")
    );
    assert!(
        app.hosted
            .queue
            .iter()
            .all(|command| !matches!(command, HostCommand::Input { .. }))
    );
}
#[test]
fn handover_is_previewed_and_session_selection_never_continues_implicitly() {
    use treerail::app::hosted_ui::Dialog;
    let temp = tempfile::tempdir().unwrap();
    let mut app = app();
    with_ticket(&mut app, temp.path());
    app.begin_handover();
    assert!(
        matches!(&app.hosted.dialog,Some(Dialog::Start{from_session:Some(owner),..})if owner=="record-1")
    );
    assert!(app.hosted.queue.is_empty());
    app.key(key(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(
        matches!(app.hosted.queue.back(),Some(HostCommand::Ticket{command:hosted::Command::Handover{from_session,..},..})if from_session=="record-1")
    );
    app.hosted.queue.clear();
    let mut second = app.state.workflow.sessions["record-1"].clone();
    second.id = "second-record".into();
    second.provider_session_id = "different-exact-id".into();
    app.hosted.dialog = Some(Dialog::Sessions {
        records: vec![second],
        cursor: 0,
    });
    app.key(key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(
        matches!(app.hosted.queue.back(),Some(HostCommand::Ticket{command:hosted::Command::SelectSession{session},..})if session=="second-record")
    );
    assert!(app.hosted.queue.iter().all(|command| !matches!(
        command,
        HostCommand::Ticket {
            command: hosted::Command::Start { .. }
                | hosted::Command::StartRequest { .. }
                | hosted::Command::Continue { .. },
            ..
        }
    )));
    assert!(app.hosted.terminal.is_none());
}
#[test]
fn fair_polling_updates_other_tickets_and_preserves_queued_input_order() {
    use std::time::{Duration, Instant};
    use treerail::app::{Completion, Job, Payload};
    let mut app = app();
    app.jobs.clear();
    // First tick sends the current preference even when the default is disabled.
    app.hosted_tick();
    assert!(matches!(
        app.jobs.last(),
        Some(Job::Host(
            _,
            HostCommand::ConfigureSession {
                desktop_notifications: false,
                ..
            }
        ))
    ));
    app.jobs.clear();
    app.accept(Completion {
        epoch: app.epoch,
        token: "host".into(),
        result: Ok(Payload::Host(Response::Done)),
    });
    app.hosted.poll = Instant::now() - Duration::from_secs(2);
    app.hosted.summary_poll = Instant::now() - Duration::from_secs(2);
    app.agent_bytes(b"one".to_vec());
    app.agent_bytes(b"two".to_vec());
    app.hosted_tick();
    assert!(matches!(
        app.jobs.last(),
        Some(Job::Host(_, HostCommand::Summary))
    ));
    app.jobs.clear();
    app.accept(Completion {
        epoch: app.epoch,
        token: "host".into(),
        result: Ok(Payload::Host(Response::Summary(vec![]))),
    });
    app.hosted.poll = Instant::now() - Duration::from_secs(2);
    app.hosted_tick();
    assert!(
        matches!(app.jobs.last(),Some(Job::Host(_,HostCommand::Input{bytes,..}))if bytes==b"onetwo")
    );
}
#[test]
fn resize_limits_and_failed_host_requests_restore_navigation_and_keep_diagnostics() {
    use treerail::app::Completion;
    let mut app = app();
    app.viewport_width = 10000;
    app.viewport_height = 10000;
    let (rows, cols) = app.hosted_dimensions();
    assert!(u32::from(rows) * u32::from(cols) <= 20000);
    assert!(rows <= 500 && cols <= 500);
    app.hosted.input = true;
    app.agent_bytes(b"queued".to_vec());
    app.accept(Completion {
        epoch: app.epoch,
        token: "host".into(),
        result: Err(anyhow::anyhow!(
            "Input authority revoked by another client; exact diagnostic"
        )),
    });
    assert!(!app.hosted.input);
    assert!(app.hosted.token.is_none());
    assert!(
        app.hosted
            .queue
            .iter()
            .all(|command| !matches!(command, HostCommand::Input { .. }))
    );
    assert!(
        app.hosted
            .errors
            .back()
            .unwrap()
            .contains("exact diagnostic")
    );
    app.key(key(KeyCode::Char('E'), KeyModifiers::NONE));
    assert!(rendered(&app).0.contains("exact diagnostic"));
}
#[test]
fn notification_changes_reach_live_sessions_and_errors_remain_visible_during_agent_focus() {
    use treerail::app::{Completion, Job, Payload};
    let mut app = app();
    app.jobs.clear();
    app.hosted_tick();
    app.jobs.clear();
    app.accept(Completion {
        epoch: app.epoch,
        token: "host".into(),
        result: Ok(Payload::Host(Response::Done)),
    });
    app.config.desktop_notifications = true;
    app.hosted_tick();
    assert!(matches!(
        app.jobs.last(),
        Some(Job::Host(
            _,
            HostCommand::ConfigureSession {
                desktop_notifications: true,
                ..
            }
        ))
    ));
    let mut other = app.state.workflow.sessions["record-1"].clone();
    other.id = "other-record".into();
    other.ticket = "owner/other#27".into();
    other.host_session = Some("other-live".into());
    app.state.workflow.sessions.insert(other.id.clone(), other);
    let mut attention = treerail::copilot::Attention::default();
    attention.observe("elicitation.requested", Some("request-seven"));
    app.hosted.input = true;
    app.accept_host(Response::Summary(vec![
        treerail::host::runtime::SessionSummary {
            session: "other-live".into(),
            provider_session_id: None,
            attention,
            exited: false,
            notification_error: Some(
                "Desktop delivery failed: permission denied by notification service".into(),
            ),
        },
    ]));
    app.latest_operation = Some(treerail::feedback::Record::new(
        1,
        "Fetch".into(),
        ".".into(),
        ".".into(),
    ));
    let (text, _) = rendered(&app);
    assert!(text.contains("owner/other#27: Needs input"));
    assert!(app.hosted.input);
    app.key(key(KeyCode::Char(']'), KeyModifiers::CONTROL));
    app.key(key(KeyCode::Char('E'), KeyModifiers::NONE));
    assert!(
        rendered(&app)
            .0
            .contains("permission denied by notification service")
    );
}

#[test]
fn full_handoff_can_be_edited_and_stopped_sessions_do_not_keep_input_authority() {
    use treerail::app::hosted_ui::Dialog;
    let temp = tempfile::tempdir().unwrap();
    let mut app = app();
    with_ticket(&mut app, temp.path());
    app.begin_agent(true);
    app.key(key(KeyCode::Tab, KeyModifiers::NONE));
    app.paste(&"a".repeat(48 * 1024));
    app.key(key(KeyCode::Backspace, KeyModifiers::NONE));
    assert!(
        matches!(&app.hosted.dialog,Some(Dialog::Start{handoff,..})if handoff.value.len()==48*1024-1)
    );
    app.key(key(KeyCode::Esc, KeyModifiers::NONE));
    app.key(key(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.hosted.dialog.is_none());
    assert!(app.hosted.queue.is_empty());
    app.hosted.input = true;
    let mut ended = app.hosted.terminal.clone().unwrap();
    ended.exited = true;
    app.accept_host(Response::Terminal(ended));
    assert!(!app.hosted.input);
    assert!(app.hosted.token.is_none());
}

#[test]
fn unresolved_launch_retries_same_identity_and_reviewed_brief_without_automatic_replay() {
    use treerail::app::hosted_ui::Dialog;
    let temp = tempfile::tempdir().unwrap();
    let mut app = app();
    with_ticket(&mut app, temp.path());
    app.begin_agent(true);
    app.key(key(KeyCode::Char('y'), KeyModifiers::NONE));
    let command = app
        .hosted
        .pending_launch
        .clone()
        .expect("captured pending request");
    let hosted::Command::StartRequest {
        request_id,
        expected_brief: Some(expected),
        ..
    } = &command
    else {
        panic!()
    };
    assert!(treerail::copilot::is_uuid(request_id));
    assert!(expected.contains("requirement-line-119"));
    let identity = request_id.clone();
    let captured = expected.clone();
    app.hosted.queue.clear();
    app.hosted_request_failed("Host reply lost after send".into());
    assert!(app.hosted.queue.is_empty());
    app.begin_agent(true);
    assert!(matches!(app.hosted.dialog, Some(Dialog::RetryStart { .. })));
    assert!(app.hosted.queue.is_empty());
    app.key(key(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(
        matches!(app.hosted.queue.back(),Some(HostCommand::Ticket{command:hosted::Command::StartRequest{request_id,expected_brief:Some(brief),..},..})if request_id==&identity && brief==&captured)
    );
    // An unrelated response cannot discard the unresolved request identity.
    let mut response = app.state.workflow.sessions["record-1"].clone();
    app.accept_host(Response::Ticket(Box::new(hosted::Event::Started(
        response.clone(),
    ))));
    assert!(app.hosted.pending_launch.is_some());
    response.id = identity;
    app.accept_host(Response::Ticket(Box::new(hosted::Event::Started(response))));
    assert!(app.hosted.pending_launch.is_none());
}
#[test]
fn reconciliation_is_an_explicit_inspection_and_never_a_continue_action() {
    use treerail::app::hosted_ui::Dialog;
    let temp = tempfile::tempdir().unwrap();
    let mut app = app();
    with_ticket(&mut app, temp.path());
    app.state
        .workflow
        .sessions
        .get_mut("record-1")
        .unwrap()
        .health = hosted::Health::Interrupted;
    app.reconcile_agent_dialog();
    assert!(matches!(app.hosted.dialog, Some(Dialog::Reconcile(_))));
    assert!(app.hosted.queue.is_empty());
    assert!(rendered(&app).0.contains("without restarting an agent"));
    app.key(key(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(
        matches!(app.hosted.queue.back(),Some(HostCommand::Ticket{command:hosted::Command::Reconcile{session},..})if session=="record-1")
    );
    assert!(app.hosted.queue.iter().all(|command| !matches!(
        command,
        HostCommand::Ticket {
            command: hosted::Command::Continue { .. }
                | hosted::Command::Start { .. }
                | hosted::Command::StartRequest { .. },
            ..
        }
    )));
}
