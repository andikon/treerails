use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use treerail::{app::App, config, ui};

#[test]
fn opt_in_is_visible_and_saves_without_losing_concurrent_edits() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    std::fs::write(&path, "# preferences\nrecursive = false # discovery\n").unwrap();
    let (config, state) = config::open(&path, false, None).unwrap();
    assert!(!config.desktop_notifications);
    let mut app = App::new(config, state, path.clone(), vec![], false);
    for key in [KeyCode::Char(','), KeyCode::Char('4')] {
        app.key(KeyEvent::new(key, KeyModifiers::NONE));
    }
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Desktop notifications: off"), "{text}");
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    std::fs::write(&path, "# preferences\nrecursive = true # discovery\n").unwrap();
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    let reopened = config::Config::load(&path).unwrap();
    assert!(reopened.desktop_notifications);
    assert!(reopened.recursive);
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(raw.contains("# preferences"));
    assert!(raw.contains("# discovery"));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(!config::Config::load(&path).unwrap().desktop_notifications);
}
