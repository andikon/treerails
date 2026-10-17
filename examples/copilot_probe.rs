//! Standalone compatibility probe, not Tree Rail's production host.
use anyhow::{Context, Result, bail};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyModifiers},
    execute, terminal,
};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use treerail::copilot::{Attention, Observation};
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
struct Endpoint {
    address: String,
    token: String,
    session_id: String,
}
struct Screen {
    parser: vt100::Parser,
    bytes: u64,
    attention: Attention,
    last_event: Option<Instant>,
    sequence: u64,
    recent_events: VecDeque<Value>,
}

fn request(path: &Path, command: Value) -> Result<Value> {
    let ep: Endpoint = serde_json::from_slice(&fs::read(path)?)?;
    let mut command = command;
    command["token"] = json!(ep.token);
    let mut socket = TcpStream::connect(ep.address)?;
    socket.set_read_timeout(Some(Duration::from_secs(5)))?;
    serde_json::to_writer(&mut socket, &command)?;
    socket.write_all(b"\n")?;
    let mut response = String::new();
    BufReader::new(socket)
        .take(2 * 1024 * 1024)
        .read_line(&mut response)?;
    Ok(serde_json::from_str(&response)?)
}
fn start(path: &Path, cwd: &Path, fixture: bool, resume: Option<&str>) -> Result<()> {
    if path.exists() {
        bail!("Endpoint already exists; stop/reconcile its owner instead of duplicating it");
    }
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("host")
        .arg(path)
        .arg(cwd)
        .arg(if fixture { "fixture" } else { "copilot" });
    if let Some(id) = resume {
        command.arg(id);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x00000008 | 0x00000200); // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
    }
    command.spawn()?;
    for _ in 0..100 {
        if path.exists()
            && let Ok(snapshot) = request(path, json!({"command":"snapshot"}))
        {
            println!("{snapshot}");
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    bail!("Host did not publish an endpoint; compatibility gate has not passed")
}
fn host(path: &Path, cwd: &Path, fixture: bool, resume: Option<&str>) -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let ep = Endpoint {
        address: listener.local_addr()?.to_string(),
        token: Uuid::new_v4().to_string(),
        session_id: resume
            .map(str::to_owned)
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
    };
    let pty = native_pty_system().openpty(PtySize {
        rows: 30,
        cols: 100,
        pixel_width: 0,
        pixel_height: 0,
    })?;
    let mut cmd = if fixture {
        let mut cmd = CommandBuilder::new(std::env::current_exe()?);
        cmd.arg("fixture");
        cmd
    } else {
        let mut cmd = CommandBuilder::new("copilot");
        cmd.args([
            "--no-auto-update",
            "--experimental",
            "--disable-builtin-mcps",
        ]);
        cmd.arg(if resume.is_some() {
            "--resume"
        } else {
            "--session-id"
        });
        cmd.arg(&ep.session_id);
        if resume.is_none()
            && let Ok(brief) = std::env::var("TREERAIL_PROBE_BRIEF")
        {
            cmd.arg("--interactive");
            cmd.arg(brief);
        }
        cmd.env("TREERAIL_PROBE_ADDRESS", &ep.address);
        cmd.env("TREERAIL_PROBE_TOKEN", &ep.token);
        cmd.env("TREERAIL_PROBE_SESSION", &ep.session_id);
        cmd
    };
    cmd.cwd(cwd);
    let mut child = pty.slave.spawn_command(cmd)?;
    drop(pty.slave);
    let pid = child.process_id();
    let screen = Arc::new(Mutex::new(Screen {
        parser: vt100::Parser::new(30, 100, 1000),
        bytes: 0,
        attention: Attention::default(),
        last_event: None,
        sequence: 0,
        recent_events: VecDeque::new(),
    }));
    let drained = Arc::clone(&screen);
    let mut reader = pty.master.try_clone_reader()?;
    thread::spawn(move || {
        let mut data = [0u8; 8192];
        while let Ok(n) = reader.read(&mut data) {
            if n == 0 {
                break;
            }
            let mut screen = drained.lock().unwrap();
            screen.parser.process(&data[..n]);
            screen.bytes += n as u64;
        }
    });
    let mut input = pty.master.take_writer()?;
    // The disposable endpoint is a capability secret. Do not use this TCP prototype in production.
    let temporary = path.with_extension("pending");
    fs::write(&temporary, serde_json::to_vec(&ep)?)?;
    fs::rename(temporary, path)?;
    for accepted in listener.incoming() {
        let mut socket = accepted?;
        socket.set_read_timeout(Some(Duration::from_secs(3)))?;
        socket.set_write_timeout(Some(Duration::from_secs(3)))?;
        let mut reader = BufReader::new(socket.try_clone()?);
        // One bounded request per connection except the native observation stream.
        {
            let mut line = String::new();
            if reader.by_ref().take(65537).read_line(&mut line)? == 0 {
                continue;
            }
            if line.len() > 65536 {
                continue;
            }
            let value: Value = serde_json::from_str(&line)?;
            if value["token"].as_str() != Some(&ep.token) {
                continue;
            }
            let response = match value["command"].as_str() {
                Some("input") => {
                    input.write_all(value["text"].as_str().unwrap_or("").as_bytes())?;
                    input.flush()?;
                    json!({"ok":true})
                }
                Some("resize") => {
                    let rows = value["rows"].as_u64().unwrap_or(30).clamp(2, 200) as u16;
                    let cols = value["cols"].as_u64().unwrap_or(100).clamp(2, 300) as u16;
                    pty.master.resize(PtySize {
                        rows,
                        cols,
                        pixel_width: 0,
                        pixel_height: 0,
                    })?;
                    screen
                        .lock()
                        .unwrap()
                        .parser
                        .screen_mut()
                        .set_size(rows, cols);
                    json!({"ok":true})
                }
                Some("observe") => {
                    let observed: Observation =
                        serde_json::from_value(value["observation"].clone())?;
                    let mut state = screen.lock().unwrap();
                    if observed.session_id != ep.session_id {
                        json!({"error":"Wrong conversation"})
                    } else {
                        if observed.sequence != state.sequence + 1 {
                            state.attention.observe("bridge.lost", None);
                        } else {
                            state
                                .attention
                                .observe(&observed.kind, observed.request_id.as_deref());
                        }
                        if observed.kind != "bridge.heartbeat" {
                            if state.recent_events.len() == 64 {
                                state.recent_events.pop_front();
                            }
                            state.recent_events.push_back(json!({"kind":observed.kind,"request_id":observed.request_id,"sequence":observed.sequence}));
                        }
                        state.sequence = observed.sequence;
                        state.last_event = Some(Instant::now());
                        json!({"ok":true})
                    }
                }
                Some("stop") => {
                    child.kill()?;
                    let _ = child.wait();
                    let _ = fs::remove_file(path);
                    return Ok(());
                }
                _ => {
                    let mut state = screen.lock().unwrap();
                    if state
                        .last_event
                        .is_some_and(|last| last.elapsed() > Duration::from_secs(4))
                    {
                        state.attention.observe("bridge.lost", None);
                    }
                    json!({"pid":pid,"session_id":ep.session_id,"bytes":state.bytes,"screen":state.parser.screen().contents(),"formatted":state.parser.screen().state_formatted(),"cursor":state.parser.screen().cursor_position(),"alternate_screen":state.parser.screen().alternate_screen(),"attention":state.attention,"sequence":state.sequence,"recent_events":state.recent_events,"exited":child.try_wait()?.is_some()})
                }
            };
            serde_json::to_writer(&mut socket, &response)?;
            socket.write_all(b"\n")?;
        }
    }
    Ok(())
}
fn view(path: &Path) -> Result<()> {
    terminal::enable_raw_mode()?;
    execute!(
        std::io::stdout(),
        terminal::EnterAlternateScreen,
        event::EnableBracketedPaste
    )?;
    let result = (|| -> Result<()> {
        let (cols, rows) = terminal::size()?;
        request(
            path,
            json!({"command":"resize","rows":rows.saturating_sub(1),"cols":cols}),
        )?;
        loop {
            let value = request(path, json!({"command":"snapshot"}))?;
            execute!(
                std::io::stdout(),
                cursor::MoveTo(0, 0),
                terminal::Clear(terminal::ClearType::All)
            )?;
            if let Some(bytes) = value["formatted"].as_array() {
                std::io::stdout().write_all(
                    &bytes
                        .iter()
                        .filter_map(|n| n.as_u64().map(|n| n as u8))
                        .collect::<Vec<_>>(),
                )?;
            }
            execute!(std::io::stdout(), cursor::MoveTo(0, rows.saturating_sub(1)))?;
            print!(
                "\x1b[0;30;46m PROBE INPUT | Ctrl-] detach | {} \x1b[0m",
                value["attention"]
            );
            std::io::stdout().flush()?;
            if event::poll(Duration::from_millis(100))? {
                let text = match event::read()? {
                    Event::Key(key)
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.code == KeyCode::Char(']') =>
                    {
                        break;
                    }
                    Event::Key(key) => match key.code {
                        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            String::from_utf8(vec![(c as u8) & 0x1f])?
                        }
                        KeyCode::Char(c) => c.to_string(),
                        KeyCode::Enter => "\r".into(),
                        KeyCode::Esc => "\x1b".into(),
                        KeyCode::Backspace => "\x7f".into(),
                        KeyCode::Up => "\x1b[A".into(),
                        KeyCode::Down => "\x1b[B".into(),
                        KeyCode::Left => "\x1b[D".into(),
                        KeyCode::Right => "\x1b[C".into(),
                        KeyCode::Tab => "\t".into(),
                        _ => String::new(),
                    },
                    Event::Paste(text) => format!("\x1b[200~{text}\x1b[201~"),
                    Event::Resize(cols, rows) => {
                        request(
                            path,
                            json!({"command":"resize","rows":rows.saturating_sub(1),"cols":cols}),
                        )?;
                        String::new()
                    }
                    _ => String::new(),
                };
                if !text.is_empty() {
                    request(path, json!({"command":"input","text":text}))?;
                }
            }
        }
        Ok(())
    })();
    execute!(
        std::io::stdout(),
        event::DisableBracketedPaste,
        terminal::LeaveAlternateScreen,
        cursor::Show
    )?;
    terminal::disable_raw_mode()?;
    result
}
fn view_smoke(path: &Path) -> Result<()> {
    let before = request(path, json!({"command":"snapshot"}))?;
    for crash in [false, true] {
        let pair = native_pty_system().openpty(PtySize {
            rows: 25,
            cols: 90,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let mut command = CommandBuilder::new(std::env::current_exe()?);
        command.arg("view");
        command.arg(path);
        let mut child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader()?;
        let rendered = Arc::new(Mutex::new(0usize));
        let counted = Arc::clone(&rendered);
        thread::spawn(move || {
            let mut buf = [0; 8192];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                *counted.lock().unwrap() += n;
            }
        });
        let mut writer = pair.master.take_writer()?;
        thread::sleep(Duration::from_millis(600));
        if crash {
            child.kill()?;
        } else {
            writer.write_all(&[0x1d])?;
        }
        writer.flush()?; // Ctrl-] detach
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if child.try_wait()?.is_some() {
                break;
            }
            if Instant::now() > deadline {
                child.kill()?;
                bail!("Embedded view did not detach");
            }
            thread::sleep(Duration::from_millis(25));
        }
        let after = request(path, json!({"command":"snapshot"}))?;
        if after["pid"] != before["pid"] || after["exited"] != false {
            bail!("View detach lost native child");
        }
        println!(
            "{}",
            json!({"same_pid":true,"view_crash":crash,"view_bytes":*rendered.lock().unwrap(),"attention":after["attention"],"cursor":after["cursor"]})
        );
    }
    Ok(())
}
fn fixture() -> Result<()> {
    print!("\x1b[?1049h\x1b[?2004h");
    for i in 0..5000 {
        println!("detached output {i}\r");
    }
    println!("CONTROLLED CHILD WAITING\r");
    std::io::stdout().flush()?;
    let mut buf = [0; 128];
    loop {
        let count = std::io::stdin().read(&mut buf)?;
        if count == 0 {
            break;
        }
        println!("INPUT {:?}\r", &buf[..count]);
        std::io::stdout().flush()?;
    }
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("help");
    if mode == "fixture" {
        return fixture();
    }
    let path = args.get(2).map(PathBuf::from).context("Usage: copilot_probe start|fixture-start|host|view|snapshot|input|resize|stop ENDPOINT [WORKTREE|TEXT|ROWS COLS] [copilot|fixture] [RESUME_UUID]")?;
    match mode {
        "start" | "fixture-start" => start(
            &path,
            Path::new(args.get(3).context("Worktree required")?),
            mode == "fixture-start",
            args.get(4).map(String::as_str),
        ),
        "host" => host(
            &path,
            Path::new(args.get(3).context("Worktree required")?),
            args.get(4).is_some_and(|v| v == "fixture"),
            args.get(5).map(String::as_str),
        ),
        "view" => view(&path),
        "view-smoke" => view_smoke(&path),
        "snapshot" => {
            println!("{}", request(&path, json!({"command":"snapshot"}))?);
            Ok(())
        }
        "input" => {
            request(
                &path,
                json!({"command":"input","text":args.get(3).context("Text required")?}),
            )?;
            Ok(())
        }
        "resize" => {
            request(
                &path,
                json!({"command":"resize","rows":args.get(3).context("Rows required")?.parse::<u16>()?,"cols":args.get(4).context("Cols required")?.parse::<u16>()?}),
            )?;
            Ok(())
        }
        "stop" => {
            let _ = request(&path, json!({"command":"stop"}));
            Ok(())
        }
        _ => bail!("Unknown command"),
    }
}
