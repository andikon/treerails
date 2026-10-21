//! A real child terminal, confined to the right pane. Copilot owns its UI and
//! input; this module owns terminal emulation, I/O, resizing and child lifetime.
use anyhow::{Context, Result};
use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::Widget,
};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

/// Secrets live only in the child environment, never its arguments or state.
pub struct CopilotLaunch {
    pub program: Option<PathBuf>,
    pub directory: PathBuf,
    pub home: Option<PathBuf>,
    pub endpoint: String,
    pub session: String,
    pub model: Option<String>,
}
enum Input {
    Bytes(Vec<u8>),
    Resize(PtySize),
    Close,
}
enum Output {
    Bytes(Vec<u8>),
    Started,
    Exit(String),
}
#[derive(Default)]
struct Replies {
    bytes: Vec<u8>,
}
impl vt100::Callbacks for Replies {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        first: Option<u8>,
        _second: Option<u8>,
        params: &[&[u16]],
        command: char,
    ) {
        let value = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        let reply = match (first, command, value) {
            (None, 'n', 5) => "\x1b[0n".to_owned(),
            (None, 'n', 6) => {
                let (r, c) = screen.cursor_position();
                format!("\x1b[{};{}R", r + 1, c + 1)
            }
            (None, 'c', _) => "\x1b[?1;2c".to_owned(),
            (Some(b'>'), 'c', _) => "\x1b[>0;1;0c".to_owned(),
            _ => return,
        };
        self.bytes.extend(reply.as_bytes());
    }
    fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
        if params.get(1) != Some(&b"?".as_slice()) {
            return;
        }
        let reply = match params.first().copied() {
            Some(b"10") => b"\x1b]10;rgb:dcdc/e5e5/e7e7\x1b\\".as_slice(),
            Some(b"11") => b"\x1b]11;rgb:1111/1616/1c1c\x1b\\".as_slice(),
            _ => return,
        };
        self.bytes.extend(reply);
    }
}
pub struct Terminal {
    parser: vt100::Parser<Replies>,
    input: mpsc::Sender<Input>,
    output: Option<mpsc::Receiver<Output>>,
    worker: Option<thread::JoinHandle<()>>,
    killer: Arc<Mutex<Option<Box<dyn portable_pty::ChildKiller + Send + Sync>>>>,
    pub started: bool,
    pub exit: Option<String>,
}
impl Terminal {
    pub fn copilot(launch: CopilotLaunch, rows: u16, cols: u16) -> Self {
        Self::spawn(
            move || {
                let url = url::Url::parse(&launch.endpoint).context("Invalid AHP connection")?;
                let local = match url.host() {
                    Some(url::Host::Domain("localhost")) => true,
                    Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
                    Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
                    _ => false,
                };
                anyhow::ensure!(
                    url.scheme() == "wss" || (url.scheme() == "ws" && local),
                    "Copilot CLI authentication requires TLS or a loopback host"
                );
                let program = launch
                    .program
                    .or_else(github_copilot_sdk::install_bundled_cli)
                    .context("Extract the compatible Copilot CLI")?;
                let mut command = CommandBuilder::new(program);
                command.args([
                    "--experimental",
                    "--ahp",
                    "--ahp-authenticate-with-github",
                    "--resume",
                    launch.session.trim_start_matches("ahp-session:/"),
                ]);
                if let Some(model) = launch.model {
                    command.args(["--model", model.as_str()]);
                }
                command.cwd(launch.directory);
                command.env("COPILOT_AHP_URL", launch.endpoint);
                command.env(
                    "COPILOT_AHP_CLIENT_ID",
                    format!("treerail-cli-{}", uuid::Uuid::new_v4()),
                );
                let mut flags =
                    std::env::var("COPILOT_CLI_ENABLED_FEATURE_FLAGS").unwrap_or_default();
                if !flags.split(',').any(|flag| flag.trim() == "AHP_CLIENT") {
                    if !flags.is_empty() {
                        flags.push(',');
                    }
                    flags.push_str("AHP_CLIENT");
                }
                command.env("COPILOT_CLI_ENABLED_FEATURE_FLAGS", flags);
                command.env("TERM", "xterm-256color");
                command.env("COLORTERM", "truecolor");
                if let Some(home) = launch.home {
                    command.env("COPILOT_HOME", home);
                }
                Ok(command)
            },
            rows,
            cols,
        )
    }
    /// Also used by deterministic terminal integration tests, without Copilot.
    pub fn command(command: CommandBuilder, rows: u16, cols: u16) -> Self {
        Self::spawn(move || Ok(command), rows, cols)
    }
    fn spawn(
        build: impl FnOnce() -> Result<CommandBuilder> + Send + 'static,
        rows: u16,
        cols: u16,
    ) -> Self {
        let size = size(rows, cols);
        let (input, rx) = mpsc::channel();
        // Backpressure limits memory even when a hidden terminal is noisy.
        let (tx, output) = mpsc::sync_channel(128);
        let killer = Arc::new(Mutex::new(None));
        let process = killer.clone();
        let worker = thread::spawn(move || {
            if let Err(error) = run(build, size, rx, &tx, process) {
                // OS errors only: never include a command or its environment.
                let _ = tx.send(Output::Exit(format!("Copilot terminal: {error:#}")));
            }
        });
        Self {
            parser: vt100::Parser::new_with_callbacks(
                size.rows,
                size.cols,
                5000,
                Replies::default(),
            ),
            input,
            output: Some(output),
            worker: Some(worker),
            killer,
            started: false,
            exit: None,
        }
    }
    pub fn poll(&mut self) {
        for _ in 0..128 {
            let Some(output) = &self.output else {
                break;
            };
            match output.try_recv() {
                Ok(Output::Bytes(bytes)) => {
                    self.parser.process(&bytes);
                    let replies = std::mem::take(&mut self.parser.callbacks_mut().bytes);
                    if !replies.is_empty() {
                        let _ = self.input.send(Input::Bytes(replies));
                    }
                }
                Ok(Output::Started) => self.started = true,
                Ok(Output::Exit(message)) => self.exit = Some(message),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.exit
                        .get_or_insert_with(|| "Copilot terminal closed".into());
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
    }
    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }
    pub fn running(&self) -> bool {
        self.exit.is_none()
    }
    pub fn resize(&mut self, rows: u16, cols: u16) {
        let size = size(rows, cols);
        if self.parser.screen().size() != (size.rows, size.cols) {
            self.parser.screen_mut().set_size(size.rows, size.cols);
            let _ = self.input.send(Input::Resize(size));
        }
    }
    pub fn key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        let bytes = encode_key(key, self.screen().application_cursor());
        self.write(bytes);
    }
    pub fn paste(&mut self, text: &str) {
        let bytes = if self.screen().bracketed_paste() {
            format!("\x1b[200~{text}\x1b[201~").into_bytes()
        } else {
            text.as_bytes().to_vec()
        };
        self.write(bytes);
    }
    pub fn mouse(&mut self, event: MouseEvent, area: Rect) {
        if !area.contains((event.column, event.row).into()) {
            return;
        }
        let screen = self.screen();
        if screen.mouse_protocol_mode() == vt100::MouseProtocolMode::None {
            let delta = match event.kind {
                MouseEventKind::ScrollUp => 3,
                MouseEventKind::ScrollDown => -3,
                _ => return,
            };
            let offset = screen.scrollback().saturating_add_signed(delta);
            self.parser.screen_mut().set_scrollback(offset);
            return;
        }
        let button = |b| match b {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
        };
        let (mut code, release) = match event.kind {
            MouseEventKind::Down(b) => (button(b), false),
            MouseEventKind::Up(b) => (button(b), true),
            MouseEventKind::Drag(b) => (button(b) + 32, false),
            MouseEventKind::Moved
                if screen.mouse_protocol_mode() == vt100::MouseProtocolMode::AnyMotion =>
            {
                (35, false)
            }
            MouseEventKind::ScrollUp => (64, false),
            MouseEventKind::ScrollDown => (65, false),
            _ => return,
        };
        if event.modifiers.contains(KeyModifiers::SHIFT) {
            code += 4;
        }
        if event.modifiers.contains(KeyModifiers::ALT) {
            code += 8;
        }
        if event.modifiers.contains(KeyModifiers::CONTROL) {
            code += 16;
        }
        let (x, y) = (event.column - area.x + 1, event.row - area.y + 1);
        let bytes = if screen.mouse_protocol_encoding() == vt100::MouseProtocolEncoding::Sgr {
            format!("\x1b[<{code};{x};{y}{}", if release { 'm' } else { 'M' }).into_bytes()
        } else if x < 224 && y < 224 {
            vec![
                27,
                b'[',
                b'M',
                if release { 35 } else { code + 32 },
                (x + 32) as u8,
                (y + 32) as u8,
            ]
        } else {
            return;
        };
        self.write(bytes);
    }
    fn write(&mut self, bytes: Vec<u8>) {
        if self.running() && !bytes.is_empty() {
            self.parser.screen_mut().set_scrollback(0);
            let _ = self.input.send(Input::Bytes(bytes));
        }
    }
    pub fn close(&mut self) {
        // Stop output backpressure before closing ConPTY (its close flushes).
        self.output.take();
        let _ = self.input.send(Input::Close);
        if let Some(killer) = self.killer.lock().expect("terminal process lock").as_mut() {
            let _ = killer.kill();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.exit
            .get_or_insert_with(|| "Copilot terminal closed".into());
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        self.close();
    }
}
fn size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}
fn run(
    build: impl FnOnce() -> Result<CommandBuilder>,
    size: PtySize,
    input: mpsc::Receiver<Input>,
    output: &mpsc::SyncSender<Output>,
    killer: Arc<Mutex<Option<Box<dyn portable_pty::ChildKiller + Send + Sync>>>>,
) -> Result<()> {
    let command = build()?;
    let pair = native_pty_system()
        .openpty(size)
        .context("Open child terminal")?;
    let mut reader = pair.master.try_clone_reader()?;
    let mut writer = pair.master.take_writer()?;
    let mut child = pair
        .slave
        .spawn_command(command)
        .map_err(|_| anyhow::anyhow!("Could not start the CLI executable in this checkout"))?;
    drop(pair.slave);
    *killer.lock().expect("terminal process lock") = Some(child.clone_killer());
    let tx = output.clone();
    let reading = thread::spawn(move || {
        let mut bytes = [0; 16384];
        loop {
            match reader.read(&mut bytes) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(Output::Bytes(bytes[..n].to_vec())).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let _ = output.send(Output::Started);
    let result: Result<()> = (|| {
        loop {
            if let Some(exit) = child.try_wait()? {
                let _ = output.send(Output::Exit(format!("Copilot exited ({exit})")));
                break;
            }
            match input.recv_timeout(Duration::from_millis(20)) {
                Ok(Input::Bytes(bytes)) => {
                    writer.write_all(&bytes)?;
                    writer.flush()?;
                }
                Ok(Input::Resize(size)) => pair.master.resize(size)?,
                Ok(Input::Close) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
        Ok(())
    })();
    let _ = child.kill();
    drop(writer);
    drop(pair.master);
    let _ = child.wait();
    let _ = reading.join();
    result
}
pub fn encode_key(key: KeyEvent, application: bool) -> Vec<u8> {
    let modifiers = key.modifiers;
    let parameter = 1
        + u8::from(modifiers.contains(KeyModifiers::SHIFT))
        + 2 * u8::from(modifiers.contains(KeyModifiers::ALT))
        + 4 * u8::from(modifiers.contains(KeyModifiers::CONTROL));
    let sequence = match key.code {
        KeyCode::Char(c) if modifiers.contains(KeyModifiers::CONTROL) => {
            let code = c.to_ascii_uppercase() as u32;
            if (64..=95).contains(&code) {
                String::from((code as u8 - 64) as char)
            } else if c == ' ' {
                "\0".into()
            } else {
                return Vec::new();
            }
        }
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter if parameter > 1 => format!("\x1b[13;{parameter}u"),
        KeyCode::Enter => "\r".into(),
        KeyCode::Backspace => "\x7f".into(),
        KeyCode::Tab => "\t".into(),
        KeyCode::BackTab => "\x1b[Z".into(),
        KeyCode::Esc => "\x1b".into(),
        KeyCode::Up
        | KeyCode::Down
        | KeyCode::Right
        | KeyCode::Left
        | KeyCode::Home
        | KeyCode::End => {
            let suffix = match key.code {
                KeyCode::Up => 'A',
                KeyCode::Down => 'B',
                KeyCode::Right => 'C',
                KeyCode::Left => 'D',
                KeyCode::Home => 'H',
                _ => 'F',
            };
            if parameter > 1 {
                format!("\x1b[1;{parameter}{suffix}")
            } else {
                format!("\x1b{}{suffix}", if application { 'O' } else { '[' })
            }
        }
        KeyCode::Insert | KeyCode::Delete | KeyCode::PageUp | KeyCode::PageDown => {
            let n = match key.code {
                KeyCode::Insert => 2,
                KeyCode::Delete => 3,
                KeyCode::PageUp => 5,
                _ => 6,
            };
            if parameter > 1 {
                format!("\x1b[{n};{parameter}~")
            } else {
                format!("\x1b[{n}~")
            }
        }
        KeyCode::F(n) if (1..=4).contains(&n) => {
            let s = (b'P' + n - 1) as char;
            if parameter > 1 {
                format!("\x1b[1;{parameter}{s}")
            } else {
                format!("\x1bO{s}")
            }
        }
        KeyCode::F(n) if (5..=12).contains(&n) => {
            let n = [15, 17, 18, 19, 20, 21, 23, 24][n as usize - 5];
            if parameter > 1 {
                format!("\x1b[{n};{parameter}~")
            } else {
                format!("\x1b[{n}~")
            }
        }
        _ => return Vec::new(),
    };
    if modifiers.contains(KeyModifiers::ALT) && matches!(key.code, KeyCode::Char(_)) {
        format!("\x1b{sequence}").into_bytes()
    } else {
        sequence.into_bytes()
    }
}
pub struct TerminalWidget<'a>(pub &'a vt100::Screen);
impl Widget for TerminalWidget<'_> {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        for row in 0..area.height {
            for col in 0..area.width {
                let Some(cell) = self.0.cell(row, col) else {
                    continue;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                let mut style = Style::default()
                    .fg(color(cell.fgcolor(), Color::Rgb(220, 229, 231)))
                    .bg(color(cell.bgcolor(), Color::Rgb(17, 22, 28)));
                for (enabled, flag) in [
                    (cell.bold(), Modifier::BOLD),
                    (cell.dim(), Modifier::DIM),
                    (cell.italic(), Modifier::ITALIC),
                    (cell.underline(), Modifier::UNDERLINED),
                    (cell.inverse(), Modifier::REVERSED),
                ] {
                    if enabled {
                        style = style.add_modifier(flag);
                    }
                }
                let content = cell.contents();
                buffer[(area.x + col, area.y + row)]
                    .set_symbol(if content.is_empty() { " " } else { content })
                    .set_style(style);
            }
        }
    }
}
fn color(color: vt100::Color, default: Color) -> Color {
    match color {
        vt100::Color::Default => default,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}
#[derive(Default)]
pub struct Terminals {
    sessions: BTreeMap<String, Terminal>,
}
impl Terminals {
    pub fn insert(&mut self, id: String, terminal: Terminal) {
        self.sessions.insert(id, terminal);
    }
    pub fn get(&self, id: &str) -> Option<&Terminal> {
        self.sessions.get(id)
    }
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Terminal> {
        self.sessions.get_mut(id)
    }
    pub fn open(&mut self, id: String, launch: CopilotLaunch, rows: u16, cols: u16) {
        self.sessions
            .entry(id)
            .or_insert_with(|| Terminal::copilot(launch, rows, cols));
    }
    pub fn poll(&mut self) {
        for session in self.sessions.values_mut() {
            session.poll();
        }
    }
    pub fn close(&mut self, id: &str) {
        self.sessions.remove(id);
    }
    pub fn shutdown(&mut self) {
        self.sessions.clear();
    }
}
