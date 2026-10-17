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
    Ping,
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
}
#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    Pong {
        version: u32,
    },
    Started {
        session: String,
        process_id: Option<u32>,
    },
    Terminal(TerminalSnapshot),
    Done,
    Error(String),
}

pub struct Client {
    endpoint: String,
}
impl Client {
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
    parser: vt100::Parser,
    sequence: u64,
}
struct Session {
    group: super::execution::ExecutionGroup,
    _lease: CheckoutLease,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    screen: Arc<Mutex<Screen>>,
    input: InputControl,
}
impl Session {
    fn snapshot(&mut self, id: &str, token: Option<ControlToken>) -> Result<Response> {
        let exited = self.group.stopped()?;
        let state = self
            .screen
            .lock()
            .map_err(|_| anyhow::anyhow!("Terminal reader failed"))?;
        let screen = state.parser.screen();
        let (rows, cols) = screen.size();
        Ok(Response::Terminal(TerminalSnapshot {
            session: id.to_owned(),
            process_id: self.child.process_id(),
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
        }))
    }
}
#[derive(Default)]
struct Host {
    sessions: BTreeMap<String, Session>,
    starts: BTreeMap<String, (String, Option<u32>)>,
    next_id: u64,
}
impl Host {
    async fn execute(&mut self, command: HostCommand) -> Result<Response> {
        match command {
            HostCommand::Ping => Ok(Response::Pong {
                version: PROTOCOL_VERSION,
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
                    return Ok(Response::Started {
                        session: id.clone(),
                        process_id: *process_id,
                    });
                }
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
                let mut builder = CommandBuilder::new(std::env::current_exe()?);
                builder.arg("--managed-child");
                builder.arg(program);
                builder.args(args);
                builder.cwd(&checkout);
                let mut child = pair.slave.spawn_command(builder)?;
                let group = match super::execution::ExecutionGroup::assign(child.as_ref()) {
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
                let screen = Arc::new(Mutex::new(Screen {
                    parser: vt100::Parser::new(rows, cols, 2000),
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
                self.next_id = self
                    .next_id
                    .checked_add(1)
                    .context("Host session identity exhausted")?;
                let id = format!("session-{}", self.next_id);
                let process_id = child.process_id();
                self.sessions.insert(
                    id.clone(),
                    Session {
                        group,
                        _lease: lease,
                        child,
                        master: pair.master,
                        writer,
                        screen,
                        input: InputControl::default(),
                    },
                );
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
                        let token = session.input.attach(&client, takeover)?;
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
                        session.writer.write_all(&bytes)?;
                        session.writer.flush()?;
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
        rows > 0 && cols > 0 && rows <= 500 && cols <= 500,
        "Terminal dimensions must be between 1 and 500"
    );
    Ok(())
}

/// Serve one bounded request per connection. Screen readers never depend on clients.
pub async fn serve(endpoint: &str) -> Result<()> {
    let mut host = Host::default();
    loop {
        let listener = ipc::Listener::bind(endpoint)?;
        let mut pipe = listener.accept()?;
        let result = read_frame::<_, Request>(&mut pipe);
        let mut stop = false;
        let response = match result {
            Ok(request) => match handshake(request.version) {
                Ok(()) => {
                    stop = matches!(request.command, HostCommand::StopHost);
                    match host.execute(request.command).await {
                        Ok(response) => response,
                        Err(error) => {
                            stop = false;
                            Response::Error(format!("{error:#}"))
                        }
                    }
                }
                Err(error) => Response::Error(error.to_string()),
            },
            Err(error) => Response::Error(error.to_string()),
        };
        let _ = write_frame(&mut pipe, &response);
        drop(pipe);
        if stop {
            return Ok(());
        }
    }
}
