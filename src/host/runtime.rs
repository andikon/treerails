use super::{
    CheckoutLease, ControlToken, InputControl, PROTOCOL_VERSION, handshake, ipc, read_frame,
    write_frame,
};
use anyhow::{Context, Result, ensure};
use portable_pty::{CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub version: u32,
    pub command: HostCommand,
}
#[derive(Debug, Serialize, Deserialize)]
pub enum HostCommand {
    Ticket {
        config_path: PathBuf,
        command: crate::hosted::Command,
    },
    Ping,
    Summary,
    FindStart {
        request_id: String,
    },
    ConfigureSession {
        session: String,
        ticket: String,
        desktop_notifications: bool,
    },
    StartCopilot {
        request_id: String,
        checkout: PathBuf,
        provider_session_id: String,
        resume: bool,
        brief: Option<String>,
        rows: u16,
        cols: u16,
    },
    Observe {
        session: String,
        token: String,
        observation: crate::copilot::Observation,
    },
    Start {
        request_id: String,
        checkout: PathBuf,
        program: String,
        args: Vec<String>,
        rows: u16,
        cols: u16,
    },
    Attach {
        session: String,
        client: String,
        takeover: bool,
    },
    Input {
        session: String,
        client: String,
        token: u64,
        bytes: Vec<u8>,
    },
    Resize {
        session: String,
        client: String,
        token: u64,
        rows: u16,
        cols: u16,
    },
    Detach {
        session: String,
        client: String,
        token: u64,
    },
    Snapshot {
        session: String,
    },
    Stop {
        session: String,
    },
    StopHost,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TerminalSnapshot {
    pub session: String,
    pub process_id: Option<u32>,
    #[serde(default)]
    pub execution_group: Option<String>,
    pub rows: u16,
    pub cols: u16,
    pub formatted: Vec<u8>,
    pub cursor: (u16, u16),
    pub cursor_hidden: bool,
    pub alternate_screen: bool,
    pub bracketed_paste: bool,
    pub sequence: u64,
    pub exited: bool,
    pub input_token: Option<u64>,
    pub provider_session_id: Option<String>,
    pub attention: crate::copilot::Attention,
}
#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    Ticket(Box<crate::hosted::Event>),
    Pong {
        version: u32,
        instance: String,
    },
    Started {
        session: String,
        process_id: Option<u32>,
    },
    Terminal(TerminalSnapshot),
    Summary(Vec<SessionSummary>),
    Done,
    NotStarted {
        request_id: String,
    },
    Ended {
        session: String,
    },
    Error(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session: String,
    pub provider_session_id: Option<String>,
    pub attention: crate::copilot::Attention,
    pub exited: bool,
    pub notification_error: Option<String>,
}

pub struct Client {
    endpoint: String,
}
impl Client {
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
    pub fn for_config(config: &Path) -> Result<Self> {
        Ok(Self::new(super::discovery::endpoint_for_config(config)?))
    }
    pub fn connect_or_launch(executable: &Path, config: &Path) -> Result<Self> {
        let client = Self::for_config(config)?;
        let _startup = super::discovery::startup_lock(&client.endpoint)?;
        match client.execute(HostCommand::Ping) {
            Ok(Response::Pong { version, .. }) => {
                handshake(version)?;
                return Ok(client);
            }
            Ok(_) => anyhow::bail!("Unexpected session host handshake"),
            Err(error) if !format!("{error:#}").contains("Connect to local session host") => {
                return Err(error);
            }
            Err(_) => {}
        }
        let mut child = Self::launch(executable, &client.endpoint)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            match client.execute(HostCommand::Ping) {
                Ok(Response::Pong { version, .. }) => {
                    handshake(version)?;
                    return Ok(client);
                }
                Ok(_) => anyhow::bail!("Unexpected session host handshake"),
                Err(error) => {
                    if let Some(status) = child.try_wait()? {
                        anyhow::bail!("Session host exited during startup ({status}): {error:#}");
                    }
                    ensure!(
                        std::time::Instant::now() < deadline,
                        "Session host did not start: {error:#}"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        }
    }
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
        }
    }
    pub fn execute(&self, command: HostCommand) -> Result<Response> {
        let mut pipe = ipc::connect(&self.endpoint)?;
        write_frame(
            &mut pipe,
            &Request {
                version: PROTOCOL_VERSION,
                command,
            },
        )?;
        match read_frame(&mut pipe)? {
            Response::Error(message) => anyhow::bail!(message),
            response => Ok(response),
        }
    }
    /// The caller retains no terminal handles; Windows console closure cannot drop its PTYs.
    pub fn launch(executable: &Path, endpoint: &str) -> Result<std::process::Child> {
        let mut command = Command::new(executable);
        if executable
            .file_stem()
            .is_none_or(|name| name != "treerail-session-host")
        {
            command.arg("--session-host");
        }
        command
            .arg(endpoint)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x00000008 | 0x00000200);
        }
        command.spawn().context("Launch detached session host")
    }
}
struct Screen {
    parser: vt100::Parser<TerminalReplies>,
    sequence: u64,
}

struct TerminalReplies(std::sync::mpsc::SyncSender<Vec<u8>>);
impl vt100::Callbacks for TerminalReplies {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        i2: Option<u8>,
        params: &[&[u16]],
        final_byte: char,
    ) {
        // portable-pty enables ConPTY cursor inheritance. Answer the terminal
        // protocol in the host even when no UI is present; otherwise startup can hang.
        if i1.is_none() && i2.is_none() && final_byte == 'n' && params == [&[6u16][..]] {
            let (row, col) = screen.cursor_position();
            let _ = self
                .0
                .try_send(format!("\x1b[{};{}R", row + 1, col + 1).into_bytes());
        }
    }
}
fn terminal_parser(
    rows: u16,
    cols: u16,
    input: std::sync::mpsc::SyncSender<Vec<u8>>,
) -> vt100::Parser<TerminalReplies> {
    vt100::Parser::new_with_callbacks(rows, cols, 2000, TerminalReplies(input))
}

#[cfg(test)]
mod terminal_query_tests {
    #[test]
    fn detached_terminal_answers_fragmented_cursor_query_from_its_current_screen() {
        let (input, replies) = std::sync::mpsc::sync_channel(8);
        let mut parser = super::terminal_parser(24, 80, input);
        parser.process(b"\x1b[3;5H\x1b[");
        parser.process(b"6");
        parser.process(b"n");
        assert_eq!(
            replies
                .try_recv()
                .expect("Host must answer the cursor query without an attached UI"),
            b"\x1b[3;5R"
        );
        assert!(replies.try_recv().is_err());
    }
}
struct Provider {
    checkout: PathBuf,
    session_id: String,
    token: String,
    sequence: u64,
    last_seen: std::time::Instant,
    valid: bool,
    attention: crate::copilot::Attention,
}
struct LaunchContext {
    local_id: String,
    provider: Provider,
}
struct Session {
    provider: Option<Provider>,
    ticket: String,
    desktop_notifications: bool,
    notification_error: Option<String>,
    group: super::execution::ExecutionGroup,
    _lease: CheckoutLease,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    writer: std::sync::mpsc::SyncSender<Vec<u8>>,
    screen: Arc<Mutex<Screen>>,
    input: InputControl,
    controller_process: Option<ipc::ClientProcess>,
}
impl Session {
    fn snapshot(&mut self, id: &str, token: Option<ControlToken>) -> Result<Response> {
        let exited = self.group.stopped()?;
        if let Some(provider) = self.provider.as_mut()
            && provider.last_seen.elapsed() > std::time::Duration::from_secs(5)
        {
            provider.valid = false;
            provider
                .attention
                .unavailable("Copilot observations lost; terminal remains available");
        }
        let state = self
            .screen
            .lock()
            .map_err(|_| anyhow::anyhow!("Terminal reader failed"))?;
        let screen = state.parser.screen();
        let (rows, cols) = screen.size();
        Ok(Response::Terminal(TerminalSnapshot {
            session: id.to_owned(),
            process_id: self.child.process_id(),
            execution_group: self.group.identity(),
            rows,
            cols,
            formatted: screen.state_formatted(),
            cursor: screen.cursor_position(),
            cursor_hidden: screen.hide_cursor(),
            alternate_screen: screen.alternate_screen(),
            bracketed_paste: screen.bracketed_paste(),
            sequence: state.sequence,
            exited,
            input_token: token.map(|t| t.0),
            provider_session_id: self
                .provider
                .as_ref()
                .map(|provider| provider.session_id.clone()),
            attention: self
                .provider
                .as_ref()
                .map(|provider| provider.attention.clone())
                .unwrap_or_default(),
        }))
    }
}
#[derive(Default)]
pub(crate) struct Host {
    sessions: BTreeMap<String, Session>,
    starts: BTreeMap<String, (String, Option<u32>)>,
    fingerprints: BTreeMap<String, Vec<u8>>,
    instance: String,
    endpoint: String,
    launch_context: Option<LaunchContext>,
    caller: Option<ipc::ClientProcess>,
    #[cfg(test)]
    launcher: Option<PathBuf>,
    notifications: crate::notifications::Notifications,
    delivery_tx: Option<tokio::sync::mpsc::Sender<crate::notifications::Notice>>,
    delivery_rx: Option<tokio::sync::mpsc::Receiver<(String, Option<String>)>>,
}
impl Host {
    pub(crate) async fn execute(&mut self, command: HostCommand) -> Result<Response> {
        let fingerprint = if matches!(
            &command,
            HostCommand::Start { .. } | HostCommand::StartCopilot { .. }
        ) {
            let bytes = serde_json::to_vec(&command)?;
            ensure!(
                bytes.len() <= 65536,
                "Agent start specification exceeds 64 KiB"
            );
            Some(bytes)
        } else {
            None
        };
        match command {
            HostCommand::Ticket {
                config_path,
                command,
            } => {
                let event = Box::pin(crate::hosted::execute(self, config_path, command)).await?;
                Ok(Response::Ticket(Box::new(event)))
            }
            HostCommand::StartCopilot {
                request_id,
                checkout,
                provider_session_id,
                resume,
                brief,
                rows,
                cols,
            } => {
                if let Some((session, process_id)) = self.starts.get(&request_id) {
                    ensure!(
                        self.fingerprints.get(&request_id) == fingerprint.as_ref(),
                        "Start request identity was reused for different work"
                    );
                    return Ok(if self.sessions.contains_key(session) {
                        Response::Started {
                            session: session.clone(),
                            process_id: *process_id,
                        }
                    } else {
                        Response::Ended {
                            session: session.clone(),
                        }
                    });
                }
                let home = directories::BaseDirs::new()
                    .context("Current user home unavailable")?
                    .home_dir()
                    .join(".copilot");
                if resume {
                    ensure!(
                        crate::copilot::is_uuid(&provider_session_id),
                        "An exact Copilot conversation UUID is required"
                    );
                    ensure!(
                        home.join("session-state")
                            .join(&provider_session_id)
                            .join("events.jsonl")
                            .is_file(),
                        "Recorded Copilot conversation is missing; choose an explicit new session instead"
                    );
                }
                let args = crate::copilot::launch_arguments(
                    &provider_session_id,
                    if resume {
                        crate::copilot::LaunchMode::Resume
                    } else {
                        crate::copilot::LaunchMode::New
                    },
                    brief.as_deref(),
                )?;
                crate::copilot::install_observer(&home)?;
                let local_id = format!("session-{}", uuid::Uuid::new_v4());
                self.launch_context = Some(LaunchContext {
                    local_id,
                    provider: Provider {
                        checkout: std::fs::canonicalize(&checkout)
                            .context("Resolve Copilot checkout")?,
                        session_id: provider_session_id,
                        token: uuid::Uuid::new_v4().to_string(),
                        sequence: 0,
                        last_seen: std::time::Instant::now(),
                        valid: true,
                        attention: crate::copilot::Attention::default(),
                    },
                });
                let original_request = request_id.clone();
                let result = Box::pin(self.execute(HostCommand::Start {
                    request_id,
                    checkout,
                    program: "copilot".into(),
                    args,
                    rows,
                    cols,
                }))
                .await;
                self.launch_context = None;
                if result.is_ok() {
                    self.fingerprints
                        .insert(original_request, fingerprint.unwrap());
                }
                result
            }
            HostCommand::Observe {
                session,
                token,
                observation,
            } => {
                let live = self
                    .sessions
                    .get_mut(&session)
                    .context("Unknown hosted session")?;
                let provider = live
                    .provider
                    .as_mut()
                    .context("Session has no native provider")?;
                ensure!(
                    provider.token == token,
                    "Provider observation authentication failed"
                );
                if provider.session_id != observation.session_id {
                    provider.valid = false;
                    provider.attention.unavailable(
                        "Provider reported another conversation identity; checkout remains owned",
                    );
                    anyhow::bail!("Provider conversation identity mismatch");
                }
                if observation.kind == "session.context_changed" {
                    let unchanged = observation
                        .cwd
                        .as_deref()
                        .and_then(|path| std::fs::canonicalize(path).ok())
                        .is_some_and(|path| crate::config::same_path(&path, &provider.checkout));
                    if !unchanged {
                        provider.valid = false;
                        provider.attention.unavailable("Copilot changed workspace context; checkout remains owned until explicitly stopped");
                        anyhow::bail!("Provider workspace context changed");
                    }
                }
                // Folder trust can delay the first native extension connection.
                // A genuine broken observation stream still cannot repair itself.
                if provider.sequence == 0
                    && observation.sequence == 1
                    && observation.kind == "bridge.connected"
                {
                    provider.valid = true;
                }
                if !provider.valid || observation.sequence != provider.sequence.saturating_add(1) {
                    provider.valid = false;
                    provider.attention.unavailable(
                        "Provider observation stream has a gap; checkout remains owned",
                    );
                    anyhow::bail!("Provider event stream unavailable");
                }
                provider.sequence = observation.sequence;
                provider.last_seen = std::time::Instant::now();
                provider
                    .attention
                    .observe(&observation.kind, observation.request_id.as_deref());
                Ok(Response::Done)
            }
            HostCommand::FindStart { request_id } => {
                let Some((session, process_id)) = self.starts.get(&request_id) else {
                    return Ok(Response::NotStarted { request_id });
                };
                Ok(if self.sessions.contains_key(session) {
                    Response::Started {
                        session: session.clone(),
                        process_id: *process_id,
                    }
                } else {
                    Response::Ended {
                        session: session.clone(),
                    }
                })
            }
            HostCommand::Summary => {
                let mut summaries = Vec::new();
                for (id, session) in &mut self.sessions {
                    let exited = session.group.stopped()?;
                    summaries.push(SessionSummary {
                        session: id.clone(),
                        provider_session_id: session
                            .provider
                            .as_ref()
                            .map(|provider| provider.session_id.clone()),
                        attention: session
                            .provider
                            .as_ref()
                            .map(|provider| provider.attention.clone())
                            .unwrap_or_default(),
                        exited,
                        notification_error: session.notification_error.clone(),
                    });
                }
                Ok(Response::Summary(summaries))
            }
            HostCommand::ConfigureSession {
                session,
                ticket,
                desktop_notifications,
            } => {
                let live = self
                    .sessions
                    .get_mut(&session)
                    .context("Unknown hosted session")?;
                live.ticket = ticket;
                live.desktop_notifications = desktop_notifications;
                Ok(Response::Done)
            }
            HostCommand::Ping => Ok(Response::Pong {
                version: PROTOCOL_VERSION,
                instance: self.instance.clone(),
            }),
            HostCommand::Start {
                request_id,
                checkout,
                program,
                args,
                rows,
                cols,
            } => {
                ensure!(
                    !request_id.is_empty() && request_id.len() <= 256,
                    "Start needs a bounded request identity"
                );
                if let Some((id, process_id)) = self.starts.get(&request_id) {
                    ensure!(
                        self.fingerprints.get(&request_id) == fingerprint.as_ref(),
                        "Start request identity was reused for different work"
                    );
                    return Ok(if self.sessions.contains_key(id) {
                        Response::Started {
                            session: id.clone(),
                            process_id: *process_id,
                        }
                    } else {
                        Response::Ended {
                            session: id.clone(),
                        }
                    });
                }
                ensure!(
                    self.starts.len() < 1024,
                    "Host request history is full; explicitly stop this host after its sessions end"
                );
                let context = self.launch_context.take();
                let id = context
                    .as_ref()
                    .map(|context| context.local_id.clone())
                    .unwrap_or_else(|| format!("session-{}", uuid::Uuid::new_v4()));
                ensure!(
                    self.sessions.len() < 128,
                    "Host session limit reached; stop unused sessions"
                );
                validate_size(rows, cols)?;
                let lease = CheckoutLease::acquire(&checkout).await?;
                let pair = portable_pty::native_pty_system().openpty(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })?;
                let launcher = std::env::current_exe()?;
                #[cfg(test)]
                let launcher = self.launcher.clone().unwrap_or(launcher);
                let mut builder = CommandBuilder::new(launcher);
                builder.arg("--managed-child");
                builder.arg(program);
                builder.args(args);
                builder.cwd(&checkout);
                if let Some(context) = &context {
                    builder.env("TREERAIL_HOST_ENDPOINT", &self.endpoint);
                    builder.env("TREERAIL_HOST_SESSION", &context.local_id);
                    builder.env("TREERAIL_HOST_TOKEN", &context.provider.token);
                    builder.env("TREERAIL_COPILOT_SESSION", &context.provider.session_id);
                }
                let mut child = pair.slave.spawn_command(builder)?;
                let group = match super::execution::ExecutionGroup::assign(
                    child.as_ref(),
                    super::execution::execution_group_identity(&self.instance, &request_id)
                        .ok()
                        .as_deref(),
                ) {
                    Ok(group) => group,
                    Err(error) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(error);
                    }
                };
                drop(pair.slave);
                let mut reader = pair.master.try_clone_reader()?;
                let mut writer = pair.master.take_writer()?;
                writer.write_all(b"treerail-start\r\n")?;
                writer.flush()?;
                let (input_tx, input_rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(8);
                std::thread::spawn(move || {
                    while let Ok(bytes) = input_rx.recv() {
                        if writer
                            .write_all(&bytes)
                            .and_then(|_| writer.flush())
                            .is_err()
                        {
                            break;
                        }
                    }
                });
                let screen = Arc::new(Mutex::new(Screen {
                    parser: terminal_parser(rows, cols, input_tx.clone()),
                    sequence: 0,
                }));
                let reader_screen = screen.clone();
                std::thread::spawn(move || {
                    let mut buffer = [0; 8192];
                    loop {
                        match reader.read(&mut buffer) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                if let Ok(mut state) = reader_screen.lock() {
                                    state.parser.process(&buffer[..n]);
                                    state.sequence = state.sequence.saturating_add(1);
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                });
                let process_id = child.process_id();
                self.sessions.insert(
                    id.clone(),
                    Session {
                        provider: context.map(|context| context.provider),
                        ticket: String::new(),
                        desktop_notifications: false,
                        notification_error: None,
                        group,
                        _lease: lease,
                        child,
                        master: pair.master,
                        writer: input_tx,
                        screen,
                        input: InputControl::default(),
                        controller_process: None,
                    },
                );
                self.fingerprints
                    .insert(request_id.clone(), fingerprint.unwrap());
                self.starts.insert(request_id, (id.clone(), process_id));
                Ok(Response::Started {
                    session: id,
                    process_id,
                })
            }
            HostCommand::StopHost => {
                for session in self.sessions.values_mut() {
                    ensure!(
                        session.group.stopped()?,
                        "Stop live sessions explicitly before stopping their host"
                    );
                }
                Ok(Response::Done)
            }
            other => {
                let id = match &other {
                    HostCommand::Attach { session, .. }
                    | HostCommand::Input { session, .. }
                    | HostCommand::Resize { session, .. }
                    | HostCommand::Detach { session, .. }
                    | HostCommand::Snapshot { session }
                    | HostCommand::Stop { session } => session.clone(),
                    _ => unreachable!(),
                };
                let session = self
                    .sessions
                    .get_mut(&id)
                    .context("Unknown hosted session")?;
                match other {
                    HostCommand::Attach {
                        client, takeover, ..
                    } => {
                        #[cfg(windows)]
                        ensure!(
                            self.caller
                                .as_ref()
                                .is_some_and(|process| process.is_alive()),
                            "UI client ended before attachment"
                        );
                        let token = session.input.attach(&client, takeover)?;
                        session.controller_process = self
                            .caller
                            .as_ref()
                            .map(|process| process.try_clone())
                            .transpose()?;
                        session.snapshot(&id, Some(token))
                    }
                    HostCommand::Input {
                        client,
                        token,
                        bytes,
                        ..
                    } => {
                        session.input.authorize(&client, ControlToken(token))?;
                        ensure!(bytes.len() <= 65536, "Input too large");
                        session.writer.try_send(bytes).context(
                            "Agent input queue is full or unavailable; input was not accepted",
                        )?;
                        Ok(Response::Done)
                    }
                    HostCommand::Resize {
                        client,
                        token,
                        rows,
                        cols,
                        ..
                    } => {
                        session.input.authorize(&client, ControlToken(token))?;
                        validate_size(rows, cols)?;
                        session.master.resize(PtySize {
                            rows,
                            cols,
                            pixel_width: 0,
                            pixel_height: 0,
                        })?;
                        session
                            .screen
                            .lock()
                            .map_err(|_| anyhow::anyhow!("Terminal reader failed"))?
                            .parser
                            .screen_mut()
                            .set_size(rows, cols);
                        session.snapshot(&id, Some(ControlToken(token)))
                    }
                    HostCommand::Detach { client, token, .. } => {
                        session.input.detach(&client, ControlToken(token))?;
                        session.controller_process = None;
                        Ok(Response::Done)
                    }
                    HostCommand::Snapshot { .. } => session.snapshot(&id, None),
                    HostCommand::Stop { .. } => {
                        if !session.group.stopped()? {
                            session.group.stop()?;
                        }
                        session.child.wait()?;
                        // Verify stop before dropping the OS checkout lease.
                        ensure!(
                            session.group.stopped()?,
                            "Agent stop could not be verified; checkout remains owned"
                        );
                        self.sessions.remove(&id);
                        self.notifications.forget(&id);
                        Ok(Response::Done)
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
}
fn validate_size(rows: u16, cols: u16) -> Result<()> {
    ensure!(
        rows > 0
            && cols > 0
            && rows <= 500
            && cols <= 500
            && u32::from(rows) * u32::from(cols) <= 20000,
        "Terminal dimensions must be between 1 and 500 with at most 20,000 cells"
    );
    Ok(())
}

struct Incoming {
    process: Option<ipc::ClientProcess>,
    request: Request,
    reply: std::sync::mpsc::Sender<(Response, bool)>,
}
fn transport(endpoint: &str, sender: tokio::sync::mpsc::Sender<Result<Incoming>>) -> Result<()> {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let stopping = Arc::new(AtomicBool::new(false));
    let active = Arc::new(AtomicUsize::new(0));
    loop {
        let listener = ipc::Listener::bind(endpoint)?;
        let mut pipe = match listener.accept() {
            Ok(pipe) => pipe,
            Err(error) => {
                let disconnected = error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| matches!(error.raw_os_error(), Some(109 | 232 | 233)));
                if disconnected {
                    continue;
                }
                return Err(error);
            }
        };
        if stopping.load(Ordering::Acquire) {
            return Ok(());
        }
        if active.fetch_add(1, Ordering::AcqRel) >= 32 {
            active.fetch_sub(1, Ordering::AcqRel);
            let _ = write_frame(
                &mut pipe,
                &Response::Error("Too many concurrent host connections".into()),
            );
            continue;
        }
        let sender = sender.clone();
        let stopping = stopping.clone();
        let active = active.clone();
        let endpoint = endpoint.to_owned();
        std::thread::spawn(move || {
            struct Count(Arc<AtomicUsize>);
            impl Drop for Count {
                fn drop(&mut self) {
                    self.0.fetch_sub(1, Ordering::AcqRel);
                }
            }
            let _count = Count(active);
            let result = (|| -> Result<bool> {
                let process = ipc::client_process(&pipe)?;
                let request = read_frame::<_, Request>(&mut pipe)?;
                let (reply, receive) = std::sync::mpsc::channel();
                if sender
                    .blocking_send(Ok(Incoming {
                        process,
                        request,
                        reply,
                    }))
                    .is_err()
                {
                    return Ok(false);
                }
                let (response, stop) = receive
                    .recv_timeout(std::time::Duration::from_secs(10))
                    .context("Session host command result timed out")?;
                let _ = write_frame(&mut pipe, &response);
                Ok(stop)
            })();
            match result {
                Ok(true) => {
                    drop(pipe);
                    stopping.store(true, Ordering::Release);
                    // Wake the accept loop only after Stop host's reply has been written.
                    let _ = ipc::connect(&endpoint);
                }
                Ok(false) => {}
                Err(error) => {
                    let _ = write_frame(&mut pipe, &Response::Error(error.to_string()));
                }
            }
        });
    }
}

impl Host {
    async fn maintain(&mut self) {
        if let Some(receiver) = &mut self.delivery_rx {
            while let Ok((id, error)) = receiver.try_recv() {
                if let Some(session) = self.sessions.get_mut(&id) {
                    session.notification_error = error;
                }
            }
        }
        for (id, session) in &mut self.sessions {
            if session
                .controller_process
                .as_ref()
                .is_some_and(|process| !process.is_alive())
            {
                session.controller_process = None;
                session.input.disconnected();
            }
            let mut action: Option<(crate::notifications::Kind, String)> = None;
            let mut message = String::new();
            if let Some(provider) = session.provider.as_mut() {
                if provider.last_seen.elapsed() > std::time::Duration::from_secs(5) {
                    provider.valid = false;
                    provider
                        .attention
                        .unavailable("Copilot observations lost; terminal remains available");
                }
                if !provider.attention.healthy {
                    // Startup is unknown rather than a failure until the timeout.
                    if provider.last_seen.elapsed() > std::time::Duration::from_secs(5)
                        || provider.sequence > 0 && !provider.valid
                        || provider.attention.reason == "session.error"
                    {
                        action = Some((
                            crate::notifications::Kind::Failure,
                            provider.attention.reason.clone(),
                        ));
                        message = provider.attention.reason.clone();
                    }
                } else if provider.attention.value
                    == Some(crate::copilot::AttentionState::NeedsInput)
                {
                    let identity = provider
                        .attention
                        .pending
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(";");
                    action = Some((crate::notifications::Kind::NeedsInput, identity));
                    message = "Copilot is waiting for your input".into();
                }
            }
            if let Ok(Some(status)) = session.child.try_wait()
                && !status.success()
            {
                action = Some((crate::notifications::Kind::Failure, "process-exit".into()));
                message = "Agent process exited unsuccessfully".into();
            }
            self.notifications
                .set_enabled(session.desktop_notifications);
            let action_ref = action
                .as_ref()
                .map(|(kind, identity)| (*kind, identity.as_str()));
            if let Some(notice) =
                self.notifications
                    .observe(id, &session.ticket, action_ref, &message)
            {
                // Delivery is isolated from managed execution and failures are surfaced
                // separately; the ledger prevents replay when clients reconnect.
                if self
                    .delivery_tx
                    .as_ref()
                    .is_none_or(|sender| sender.try_send(notice).is_err())
                {
                    session.notification_error =
                        Some("Desktop notification queue unavailable".into());
                }
            }
        }
    }
}

/// IO has bounded deadlines on a separate thread; maintenance remains active with no clients.
pub async fn serve(endpoint: &str) -> Result<()> {
    let _lifetime = super::discovery::host_lock(endpoint)?;
    let mut host = Host {
        instance: uuid::Uuid::new_v4().to_string(),
        endpoint: endpoint.to_owned(),
        ..Host::default()
    };
    let (delivery_tx, mut deliveries) =
        tokio::sync::mpsc::channel::<crate::notifications::Notice>(32);
    let (results, delivery_rx) = tokio::sync::mpsc::channel(32);
    host.delivery_tx = Some(delivery_tx);
    host.delivery_rx = Some(delivery_rx);
    tokio::spawn(async move {
        while let Some(notice) = deliveries.recv().await {
            let error = crate::notifications::deliver(&notice)
                .await
                .err()
                .map(|error| format!("{error:#}"));
            if results.send((notice.session, error)).await.is_err() {
                break;
            }
        }
    });
    let (sender, mut incoming) = tokio::sync::mpsc::channel(8);
    let endpoint = endpoint.to_owned();
    let transport_thread = std::thread::spawn(move || {
        if let Err(error) = transport(&endpoint, sender.clone()) {
            let _ = sender.blocking_send(Err(error));
        }
    });
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(200));
    loop {
        tokio::select! {
            _=ticker.tick()=>host.maintain().await,
            next=incoming.recv()=>{
                let Some(next)=next else {return Ok(());};
                let Incoming{process,request,reply}=next?;
                host.caller=process;
                // Git/GitHub preparation owns no live terminal state. Run it outside
                // the session loop so a slow fetch cannot starve provider events.
                let command=match request.command {
                    HostCommand::Ticket {config_path,command:crate::hosted::Command::Workflow {project,command}}
                        if handshake(request.version).is_ok()=>{
                        tokio::spawn(async move {
                            let response=match crate::workflow::Workflow::new(config_path).execute(&project,command).await {
                                Ok(event)=>Response::Ticket(Box::new(crate::hosted::Event::Workflow(Box::new(event)))),
                                Err(error)=>Response::Error(format!("{error:#}")),
                            };
                            let _=reply.send((response,false));
                        });
                        host.caller=None;
                        continue;
                    }
                    command=>command,
                };
                let mut stop=false;
                let response=match handshake(request.version) {
                    Err(error)=>Response::Error(error.to_string()),
                    Ok(())=>{
                        stop=matches!(command,HostCommand::StopHost);
                        match host.execute(command).await {Ok(response)=>response,Err(error)=>{stop=false;Response::Error(format!("{error:#}"))}}
                    }
                };
                host.caller=None;
                // Observe every native transition before accepting the next event.
                host.maintain().await;
                let _=reply.send((response,stop));
                if stop{let _=transport_thread.join();return Ok(());}
            }
        }
    }
}

#[cfg(all(test, windows))]
mod notification_pipeline_tests {
    use super::*;
    use crate::{
        copilot::{AttentionState, Observation},
        notifications::Kind,
    };

    // A real child launcher at the process dependency boundary. The production
    // Start command still acquires the checkout and assigns its execution job.
    fn controlled_launcher(directory: &Path) -> PathBuf {
        let source = directory.join("launcher.rs");
        let executable = directory.join("launcher.exe");
        let launcher_source = r#"
fn main() {
    let mut args=std::env::args_os().skip(1);
    assert_eq!(args.next().unwrap(),"--managed-child");
    let mut line=String::new();
    std::io::stdin().read_line(&mut line).unwrap();
    assert_eq!(line.trim(),"treerail-start");
    let status=std::process::Command::new(args.next().unwrap()).args(args).status().unwrap();
    std::process::exit(status.code().unwrap_or(1));
}
"#;
        std::fs::write(&source, launcher_source).unwrap();
        let output =
            std::process::Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
                .arg("--edition=2024")
                .arg("--crate-name=treerail_test_launcher")
                .arg(&source)
                .arg("-o")
                .arg(&executable)
                .output()
                .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        executable
    }
    async fn observe(
        host: &mut Host,
        session: &str,
        provider: &str,
        sequence: u64,
        kind: &str,
        request: Option<&str>,
    ) -> Result<Response> {
        host.execute(HostCommand::Observe {
            session: session.into(),
            token: "fixture-token".into(),
            observation: Observation {
                session_id: provider.into(),
                sequence,
                kind: kind.into(),
                request_id: request.map(str::to_owned),
                cwd: None,
            },
        })
        .await
    }
    #[tokio::test]
    async fn notifications_follow_native_host_events_without_ui_replay_or_delivery_health_changes()
    {
        let temp = tempfile::tempdir().unwrap();
        crate::git::run(temp.path(), ["init"]).await.unwrap();
        let mut host = Host {
            instance: uuid::Uuid::new_v4().to_string(),
            launcher: Some(controlled_launcher(temp.path())),
            ..Host::default()
        };
        let session = match host
            .execute(HostCommand::Start {
                request_id: uuid::Uuid::new_v4().to_string(),
                checkout: temp.path().into(),
                program: "cmd.exe".into(),
                args: vec!["/D".into(), "/Q".into()],
                rows: 24,
                cols: 80,
            })
            .await
            .unwrap()
        {
            Response::Started { session, .. } => session,
            value => panic!("{value:?}"),
        };
        let provider = uuid::Uuid::new_v4().to_string();
        host.sessions.get_mut(&session).unwrap().provider = Some(Provider {
            checkout: std::fs::canonicalize(temp.path()).unwrap(),
            session_id: provider.clone(),
            token: "fixture-token".into(),
            sequence: 0,
            last_seen: std::time::Instant::now(),
            valid: true,
            attention: crate::copilot::Attention::default(),
        });
        host.execute(HostCommand::ConfigureSession {
            session: session.clone(),
            ticket: "example/repo#1".into(),
            desktop_notifications: true,
        })
        .await
        .unwrap();
        let (delivery, mut notices) = tokio::sync::mpsc::channel(16);
        host.delivery_tx = Some(delivery);
        observe(&mut host, &session, &provider, 1, "bridge.connected", None)
            .await
            .unwrap();
        host.maintain().await;
        observe(
            &mut host,
            &session,
            &provider,
            2,
            "assistant.turn_start",
            None,
        )
        .await
        .unwrap();
        host.maintain().await;
        assert!(notices.try_recv().is_err());
        observe(
            &mut host,
            &session,
            &provider,
            3,
            "permission.requested",
            Some("permission-1"),
        )
        .await
        .unwrap();
        host.maintain().await;
        assert_eq!(notices.try_recv().unwrap().kind, Kind::NeedsInput);
        host.maintain().await;
        assert!(notices.try_recv().is_err());
        host.caller = Some(ipc::current_process().unwrap());
        let token = match host
            .execute(HostCommand::Attach {
                session: session.clone(),
                client: "ui".into(),
                takeover: false,
            })
            .await
            .unwrap()
        {
            Response::Terminal(state) => state.input_token.unwrap(),
            value => panic!("{value:?}"),
        };
        host.execute(HostCommand::Detach {
            session: session.clone(),
            client: "ui".into(),
            token,
        })
        .await
        .unwrap();
        host.caller = None;
        host.maintain().await;
        assert!(
            notices.try_recv().is_err(),
            "Reattachment must not replay the pending notice"
        );
        observe(
            &mut host,
            &session,
            &provider,
            4,
            "permission.completed",
            Some("permission-1"),
        )
        .await
        .unwrap();
        host.maintain().await;
        observe(&mut host, &session, &provider, 5, "session.idle", None)
            .await
            .unwrap();
        host.maintain().await;
        assert!(
            notices.try_recv().is_err(),
            "Running and Ready remain quiet"
        );
        let (results, receiver) = tokio::sync::mpsc::channel(1);
        host.delivery_rx = Some(receiver);
        results
            .send((session.clone(), Some("Desktop delivery unavailable".into())))
            .await
            .unwrap();
        host.maintain().await;
        let live = &host.sessions[&session];
        let state = &live.provider.as_ref().unwrap().attention;
        assert!(state.healthy);
        assert_eq!(state.value, Some(AttentionState::Ready));
        assert_eq!(
            live.notification_error.as_deref(),
            Some("Desktop delivery unavailable")
        );
        assert!(
            observe(&mut host, &session, &provider, 9, "session.idle", None)
                .await
                .is_err()
        );
        host.maintain().await;
        assert_eq!(notices.try_recv().unwrap().kind, Kind::Failure);
        host.maintain().await;
        assert!(
            notices.try_recv().is_err(),
            "Unchanged health failure must not repeat"
        );
        host.execute(HostCommand::Stop { session }).await.unwrap();
    }
}
