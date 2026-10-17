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
        execution_group: None,
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
    let mut packets = app.hosted.queue.drain(..).map(|command| match command {
        HostCommand::Input { bytes, .. } => bytes,
        _ => panic!("Input expected"),
    });
    assert_eq!(packets.next().unwrap(), b"\x1b");
    assert_eq!(packets.next().unwrap(), b"\x03");
    assert_eq!(
        packets.next().unwrap(),
        b"\x1b[200~requirements\nsecond line\x1b[201~"
    );
    drop(packets);
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
