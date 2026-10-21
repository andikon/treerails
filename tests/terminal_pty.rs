use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use portable_pty::CommandBuilder;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier},
    widgets::Widget,
};
use std::{
    io::{Read, Write},
    time::{Duration, Instant},
};
use treerail::cli_terminal::{Terminal, TerminalWidget, encode_key};

#[test]
#[ignore = "Child process fixture, launched only by the PTY integration tests"]
fn terminal_child_fixture() {
    if std::env::var("TREERAIL_TERMINAL_FIXTURE").as_deref() != Ok("1") {
        return;
    }
    crossterm::terminal::enable_raw_mode().unwrap();
    struct Raw;
    impl Drop for Raw {
        fn drop(&mut self) {
            let _ = crossterm::terminal::disable_raw_mode();
        }
    }
    let _raw = Raw;
    #[cfg(windows)]
    let _vt = virtual_input::enable();
    if std::env::var("TREERAIL_TERMINAL_FLOOD").as_deref() == Ok("1") {
        let bytes = vec![b'x'; 16384];
        loop {
            if std::io::stdout().write_all(&bytes).is_err() {
                return;
            }
        }
    }
    print!(
        "\x1b[?1049h\x1b[2J\x1b[2;3H\x1b[1;31mHello🦀\x1b[0m\x1b[4;1HREADY\x1b[?1h\x1b[?2004h\x1b[?1000h\x1b[?1006h\x1b[6n"
    );
    std::io::stdout().flush().unwrap();
    let mut input = Vec::new();
    let mut bytes = [0; 512];
    loop {
        let n = std::io::stdin().read(&mut bytes).unwrap();
        if n == 0 {
            break;
        }
        input.extend_from_slice(&bytes[..n]);
        if input.contains(&4) {
            break;
        }
        if input.last() == Some(&b'R') {
            let (cols, rows) = crossterm::terminal::size().unwrap();
            print!("\x1b[6;1HSIZE:{cols}x{rows}");
            std::io::stdout().flush().unwrap();
        }
    }
    let hex = input.iter().map(|b| format!("{b:02x}")).collect::<String>();
    print!("\x1b[8;1HINPUT:{hex}\r\n");
    std::io::stdout().flush().unwrap();
}

#[cfg(windows)]
mod virtual_input {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(kind: u32) -> *mut c_void;
        fn GetConsoleMode(handle: *mut c_void, mode: *mut u32) -> i32;
        fn SetConsoleMode(handle: *mut c_void, mode: u32) -> i32;
    }
    pub struct Restore {
        handle: *mut c_void,
        mode: u32,
    }
    pub fn enable() -> Restore {
        // The PTY fixture reads byte sequences directly instead of using
        // ReadConsoleInput as crossterm/libuv normally do on Windows.
        unsafe {
            let handle = GetStdHandle((-10_i32) as u32);
            let mut mode = 0;
            assert_ne!(GetConsoleMode(handle, &mut mode), 0);
            assert_ne!(SetConsoleMode(handle, mode | 0x200), 0);
            Restore { handle, mode }
        }
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            // Restores the mode read from this same live standard-input handle.
            unsafe {
                SetConsoleMode(self.handle, self.mode);
            }
        }
    }
}
fn child(flood: bool) -> Terminal {
    let mut command = CommandBuilder::new(std::env::current_exe().unwrap());
    command.args([
        "--ignored",
        "--exact",
        "terminal_child_fixture",
        "--nocapture",
    ]);
    command.env("TREERAIL_TERMINAL_FIXTURE", "1");
    if flood {
        command.env("TREERAIL_TERMINAL_FLOOD", "1");
    }
    Terminal::command(command, 24, 100)
}
fn until(terminal: &mut Terminal, condition: impl Fn(&Terminal) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        terminal.poll();
        if condition(terminal) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Timed out: {} {:?}",
            terminal.screen().contents(),
            terminal.exit
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn child_terminal_handles_rendering_input_queries_mouse_and_resize() {
    let mut terminal = child(false);
    until(&mut terminal, |t| t.screen().contents().contains("READY"));
    assert!(terminal.screen().application_cursor());
    assert!(terminal.screen().bracketed_paste());
    let area = Rect::new(10, 3, 100, 24);
    let mut buffer = Buffer::empty(Rect::new(0, 0, 120, 30));
    TerminalWidget(terminal.screen()).render(area, &mut buffer);
    let cell = &buffer[(12, 4)];
    assert_eq!(cell.symbol(), "H");
    assert_eq!(cell.fg, Color::Indexed(1));
    assert!(cell.modifier.contains(Modifier::BOLD));
    assert_eq!(buffer[(17, 4)].symbol(), "🦀");
    assert_eq!(buffer[(0, 0)].symbol(), " ");
    terminal.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    terminal.paste("ab\ncd");
    terminal.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    terminal.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 14,
            row: 6,
            modifiers: KeyModifiers::NONE,
        },
        area,
    );
    terminal.resize(20, 100);
    std::thread::sleep(Duration::from_millis(50));
    terminal.key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE));
    until(&mut terminal, |t| {
        t.screen().contents().contains("SIZE:100x20")
    });
    terminal.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
    until(&mut terminal, |t| t.screen().contents().contains("INPUT:"));
    let text = terminal.screen().contents();
    assert!(text.contains("1b4f41"), "Application arrow: {text}");
    assert!(
        text.contains("1b5b3230307e61620a63641b5b3230317e"),
        "Bracketed paste: {text}"
    );
    assert!(
        text.contains("1b5b3c303b353b344d"),
        "Pane-relative mouse: {text}"
    );
    terminal.close();
}
#[test]
fn closing_a_noisy_hidden_terminal_does_not_deadlock() {
    let mut terminal = child(true);
    until(&mut terminal, |t| t.started);
    std::thread::sleep(Duration::from_millis(150));
    let start = Instant::now();
    terminal.close();
    assert!(start.elapsed() < Duration::from_secs(5));
}
#[test]
fn key_encoding_preserves_copilot_shortcuts_and_modified_enter() {
    for (code, mods, expected) in [
        (KeyCode::Esc, KeyModifiers::NONE, b"\x1b".as_slice()),
        (KeyCode::Tab, KeyModifiers::NONE, b"\t".as_slice()),
        (
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
            b"\x03".as_slice(),
        ),
        (
            KeyCode::Char('q'),
            KeyModifiers::CONTROL,
            b"\x11".as_slice(),
        ),
        (
            KeyCode::Enter,
            KeyModifiers::SHIFT,
            b"\x1b[13;2u".as_slice(),
        ),
    ] {
        assert_eq!(encode_key(KeyEvent::new(code, mods), false), expected);
    }
}

#[test]
fn app_routes_copilot_keys_and_keeps_terminal_alive_across_views() {
    use treerail::{
        agent::{Attention, Conversation},
        app::{Action, App, Pane, View},
        config::{Config, State},
        git::{Project, Worktree},
        startup::Session,
    };
    let temp = tempfile::TempDir::new().unwrap();
    let project = Project {
        name: "fixture".into(),
        path: temp.path().into(),
    };
    let mut app = App::new(Session {
        config: Config::default(),
        state: State::default(),
        config_path: temp.path().join("config.toml"),
        roots: vec![],
        recursive: false,
    });
    app.projects.push(project.clone());
    app.open_project(0);
    app.tabs[0].trees.push(Worktree {
        path: project.path.clone(),
        branch: "develop".into(),
        head: "1234".into(),
        original: true,
        locked: false,
        prunable: false,
    });
    let c:Conversation=serde_json::from_value(serde_json::json!({"id":"cli","project":project.path,"worktree":project.path,"title":"CLI","session_uri":"ahp-session:/cli","attention":"Ready"})).unwrap();
    app.state.conversations.push(c);
    app.action(Action::Session("cli".into()));
    app.modal = None;
    let mut terminal = child(false);
    until(&mut terminal, |t| t.screen().contents().contains("READY"));
    app.terminals.insert("cli".into(), terminal);
    app.action(Action::FocusTerminal);
    for (code, modifiers) in [
        (KeyCode::Tab, KeyModifiers::NONE),
        (KeyCode::Esc, KeyModifiers::NONE),
        (KeyCode::Char('p'), KeyModifiers::CONTROL),
        (KeyCode::Char('q'), KeyModifiers::CONTROL),
    ] {
        app.key(KeyEvent::new(code, modifiers));
    }
    assert!(app.modal.is_none());
    assert!(!app.quit);
    assert_eq!(app.pane, Pane::Detail);
    assert!(app.cli_focused);
    app.key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL));
    assert!(!app.cli_focused);
    app.action(Action::Switch(View::Changes));
    assert!(app.terminals.get("cli").unwrap().running());
    app.action(Action::Switch(View::Sessions));
    app.action(Action::FocusTerminal);
    app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
    let terminal = app.terminals.get_mut("cli").unwrap();
    until(terminal, |t| t.screen().contents().contains("INPUT:"));
    let text = terminal.screen().contents();
    assert!(
        text.contains("091b101104"),
        "Copilot gets Tab/Esc/Ctrl+P/Ctrl+Q, without Ctrl+]: {text}"
    );
    assert!(app.agent_commands.is_empty());
    assert_eq!(app.state.conversations[0].attention, Attention::Ready);
    app.terminals.shutdown();
}
