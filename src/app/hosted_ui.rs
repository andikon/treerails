use super::*;
use crate::{
    host::runtime::{Client, HostCommand, Response, SessionSummary, TerminalSnapshot},
    hosted,
};
use std::collections::VecDeque;

pub struct HostedUi {
    pub client: String,
    pub record: Option<hosted::SessionRecord>,
    pub terminal: Option<TerminalSnapshot>,
    pub input: bool,
    pub token: Option<u64>,
    pub summaries: Vec<SessionSummary>,
    pub dialog: Option<Dialog>,
    pub queue: VecDeque<HostCommand>,
    pub poll: Instant,
    pub summary_poll: Instant,
    notifications_enabled: bool,
}
impl Default for HostedUi {
    fn default() -> Self {
        Self {
            client: crate::copilot::new_session_id(),
            record: None,
            terminal: None,
            input: false,
            token: None,
            summaries: vec![],
            dialog: None,
            queue: VecDeque::new(),
            poll: Instant::now(),
            summary_poll: Instant::now(),
            notifications_enabled: false,
        }
    }
}
pub enum Dialog {
    Start {
        ticket: String,
        task: String,
        brief: String,
    },
    Stop(String),
    StopHost,
    Takeover,
    Sessions {
        records: Vec<hosted::SessionRecord>,
        cursor: usize,
    },
}

pub async fn request(config: PathBuf, command: HostCommand) -> Result<Response> {
    tokio::task::spawn_blocking(move || {
        let launch = matches!(
            &command,
            HostCommand::Ticket {
                command: hosted::Command::Start { .. } | hosted::Command::Continue { .. },
                ..
            }
        );
        let client = if launch {
            let executable = std::env::current_exe()?.with_file_name(if cfg!(windows) {
                "treerail-session-host.exe"
            } else {
                "treerail-session-host"
            });
            anyhow::ensure!(
                executable.is_file(),
                "Session host executable is missing; install/build both Tree Rail binaries"
            );
            Client::connect_or_launch(&executable, &config)?
        } else {
            Client::for_config(&config)?
        };
        client.execute(command)
    })
    .await
    .context("Session host client worker failed")?
}

impl App {
    fn queue_host(&mut self, command: HostCommand) {
        if self.hosted.queue.len() >= 128 {
            self.say("Agent input queue is full; wait before typing more", true);
        } else {
            self.hosted.queue.push_back(command);
        }
    }
    pub fn session_command(&mut self, command: hosted::Command) {
        self.queue_host(HostCommand::Ticket {
            config_path: self.config_path.clone(),
            command,
        });
    }
    pub fn hosted_dimensions(&self) -> (u16, u16) {
        (
            self.viewport_height
                .saturating_sub(if self.latest_operation.is_some() {
                    15
                } else {
                    13
                })
                .clamp(1, 500) as u16,
            (self.viewport_width * 65 / 100)
                .saturating_sub(2)
                .clamp(10, 500) as u16,
        )
    }
    fn focused_record(&self) -> Option<&hosted::SessionRecord> {
        let ticket = self.ticket.as_ref()?;
        let task = ticket.tasks.get(self.task_cursor)?;
        hosted::selected_session(&self.state.workflow, &ticket.id(), &task.id)
    }
    pub fn begin_agent(&mut self, new: bool) {
        if !new && let Some(record) = self.focused_record().cloned() {
            let (rows, cols) = self.hosted_dimensions();
            self.session_command(hosted::Command::Continue {
                session: record.id,
                rows,
                cols,
            });
            return;
        }
        let Some(ticket) = self.ticket.as_ref() else {
            return;
        };
        let Some(task) = ticket.tasks.get(self.task_cursor) else {
            return;
        };
        match hosted::starting_brief(ticket, &task.id, "") {
            Ok(brief) => {
                self.hosted.dialog = Some(Dialog::Start {
                    ticket: ticket.id(),
                    task: task.id.clone(),
                    brief,
                })
            }
            Err(error) => self.say(format!("{error:#}"), true),
        }
    }
    pub fn hosted_key(&mut self, key: KeyEvent) -> bool {
        if let Some(dialog) = self.hosted.dialog.take() {
            if key.kind == KeyEventKind::Repeat {
                self.hosted.dialog = Some(dialog);
                return true;
            }
            let confirmed = key.code == KeyCode::Char('y') && key.modifiers == KeyModifiers::NONE;
            match dialog {
                Dialog::Sessions {
                    records,
                    mut cursor,
                } => {
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('q') => return true,
                        KeyCode::Char('j') | KeyCode::Down => {
                            cursor = step(cursor, 1, records.len())
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            cursor = step(cursor, -1, records.len())
                        }
                        KeyCode::Enter => {
                            if let Some(record) = records.get(cursor) {
                                let (rows, cols) = self.hosted_dimensions();
                                self.session_command(hosted::Command::Continue {
                                    session: record.id.clone(),
                                    rows,
                                    cols,
                                });
                            }
                            return true;
                        }
                        _ => {}
                    }
                    self.hosted.dialog = Some(Dialog::Sessions { records, cursor });
                }
                Dialog::Start { ticket, task, .. } if confirmed => {
                    let (rows, cols) = self.hosted_dimensions();
                    self.session_command(hosted::Command::Start {
                        ticket,
                        task,
                        handoff: String::new(),
                        rows,
                        cols,
                    });
                }
                Dialog::Stop(session) if confirmed => {
                    self.session_command(hosted::Command::Stop { session })
                }
                Dialog::StopHost if confirmed => self.queue_host(HostCommand::StopHost),
                Dialog::Takeover if confirmed => self.attach_agent(true),
                _ => {}
            }
            return true;
        }
        if !self.hosted.input {
            return false;
        }
        let name = key_name(key);
        if (name == "ctrl+]"
            && !self
                .config
                .keys
                .values()
                .any(|action| action == "agent_navigation"))
            || self
                .config
                .keys
                .get(&name)
                .is_some_and(|action| action == "agent_navigation")
        {
            self.hosted.input = false;
            return true;
        }
        let application_cursor = self
            .hosted
            .terminal
            .as_ref()
            .map(|snapshot| {
                let mut parser = vt100::Parser::new(snapshot.rows, snapshot.cols, 0);
                parser.process(&snapshot.formatted);
                parser.screen().application_cursor()
            })
            .unwrap_or(false);
        if let Some(bytes) = encode_key(key, application_cursor) {
            self.agent_bytes(bytes);
        }
        true
    }
    pub fn agent_bytes(&mut self, bytes: Vec<u8>) {
        if let (Some(terminal), Some(token)) = (&self.hosted.terminal, self.hosted.token) {
            self.queue_host(HostCommand::Input {
                session: terminal.session.clone(),
                client: self.hosted.client.clone(),
                token,
                bytes,
            });
        }
    }
    pub fn hosted_paste(&mut self, text: &str) -> bool {
        if !self.hosted.input {
            return false;
        }
        if text.len() > 48 * 1024 {
            self.say("Paste exceeds the 48 KiB limit", true);
            return true;
        }
        let bytes = if self
            .hosted
            .terminal
            .as_ref()
            .is_some_and(|snapshot| snapshot.bracketed_paste)
        {
            format!("\x1b[200~{text}\x1b[201~").into_bytes()
        } else {
            text.as_bytes().to_vec()
        };
        self.agent_bytes(bytes);
        true
    }
    pub fn attach_agent(&mut self, takeover: bool) {
        if let Some(id) = self
            .hosted
            .record
            .as_ref()
            .and_then(|record| record.host_session.clone())
        {
            self.queue_host(HostCommand::Attach {
                session: id,
                client: self.hosted.client.clone(),
                takeover,
            });
        }
    }
    pub fn detach_agent(&mut self) {
        if let (Some(terminal), Some(token)) = (&self.hosted.terminal, self.hosted.token) {
            self.queue_host(HostCommand::Detach {
                session: terminal.session.clone(),
                client: self.hosted.client.clone(),
                token,
            });
        }
        self.hosted.input = false;
        self.hosted.terminal = None;
        self.hosted.record = None;
        self.hosted.token = None;
    }
    pub fn hosted_tick(&mut self) {
        if self.hosted.notifications_enabled != self.config.desktop_notifications {
            self.hosted.notifications_enabled = self.config.desktop_notifications;
            let active = self
                .state
                .workflow
                .sessions
                .values()
                .filter(|record| record.health == hosted::Health::Live)
                .filter_map(|record| {
                    record
                        .host_session
                        .as_ref()
                        .map(|session| (session.clone(), record.ticket.clone()))
                })
                .collect::<Vec<_>>();
            for (session, ticket) in active {
                self.queue_host(HostCommand::ConfigureSession {
                    session,
                    ticket,
                    desktop_notifications: self.config.desktop_notifications,
                });
            }
        }
        if let Some(request) = self.pending_revision.take() {
            let matching = self.hosted.record.as_ref().is_some_and(|record| {
                record.ticket == request.ticket && record.task == request.task
            });
            if matching {
                self.send_revision(request.instructions);
            } else {
                self.say("Attach the selected task's live conversation before sending revised requirements",true);
            }
        }
        if !self.inflight.contains("host") && self.hosted.queue.is_empty() {
            if self.hosted.poll.elapsed() > Duration::from_millis(200)
                && let Some(snapshot) = &self.hosted.terminal
            {
                self.hosted.poll = Instant::now();
                let (rows, cols) = self.hosted_dimensions();
                let command = if (snapshot.rows, snapshot.cols) != (rows, cols)
                    && let Some(token) = self.hosted.token
                {
                    HostCommand::Resize {
                        session: snapshot.session.clone(),
                        client: self.hosted.client.clone(),
                        token,
                        rows,
                        cols,
                    }
                } else {
                    HostCommand::Snapshot {
                        session: snapshot.session.clone(),
                    }
                };
                self.queue_host(command);
            } else if self.hosted.summary_poll.elapsed() > Duration::from_secs(1)
                && self
                    .state
                    .workflow
                    .sessions
                    .values()
                    .any(|record| record.health == hosted::Health::Live)
            {
                self.hosted.summary_poll = Instant::now();
                self.queue_host(HostCommand::Summary);
            }
        }
        if !self.inflight.contains("host")
            && let Some(command) = self.hosted.queue.pop_front()
        {
            self.enqueue(Job::Host(self.config_path.clone(), command));
        }
    }
    pub fn send_revision(&mut self, instructions: String) {
        if let (Some(record), Some(token)) = (&self.hosted.record, self.hosted.token) {
            self.session_command(hosted::Command::SendRevision {
                session: record.id.clone(),
                instructions,
                client: self.hosted.client.clone(),
                token,
            });
        } else {
            self.say(
                "Attach a live conversation before sending revised requirements",
                true,
            );
        }
    }
    pub fn accept_host(&mut self, response: Response) {
        match response {
            Response::Ticket(event) => match *event {
                hosted::Event::Started(record) => {
                    self.state
                        .workflow
                        .sessions
                        .insert(record.id.clone(), record.clone());
                    self.hosted.record = Some(record.clone());
                    self.hosted.input = false;
                    self.attach_agent(false);
                    if let Some(id) = record.host_session {
                        self.queue_host(HostCommand::ConfigureSession {
                            session: id,
                            ticket: record.ticket,
                            desktop_notifications: self.config.desktop_notifications,
                        });
                    }
                    self.say("Conversation attached · Enter agent input · Ctrl+] navigation · x stop · d detach",false);
                }
                hosted::Event::Stopped(record) => {
                    self.state
                        .workflow
                        .sessions
                        .insert(record.id.clone(), record);
                    self.hosted.input = false;
                    self.hosted.terminal = None;
                    self.hosted.token = None;
                    self.say("Session stopped; recorded conversation retained", false);
                }
                hosted::Event::State(state) => self.state.workflow = state,
                hosted::Event::RevisionSent => self.say(
                    "Revised requirements sent to the selected live conversation",
                    false,
                ),
                hosted::Event::Workflow(_) => {}
            },
            Response::Terminal(snapshot) => {
                if self
                    .hosted
                    .record
                    .as_ref()
                    .and_then(|record| record.host_session.as_ref())
                    == Some(&snapshot.session)
                {
                    if let Some(token) = snapshot.input_token {
                        self.hosted.token = Some(token);
                    }
                    self.hosted.terminal = Some(snapshot);
                }
            }
            Response::Summary(summaries) => self.hosted.summaries = summaries,
            Response::Done => {}
            _ => self.say("Unexpected session host response", true),
        }
    }
    pub fn stop_agent_dialog(&mut self) {
        if let Some(record) = self.focused_record() {
            self.hosted.dialog = Some(Dialog::Stop(record.id.clone()));
        }
    }
    pub fn show_sessions(&mut self) {
        let Some(ticket) = &self.ticket else { return };
        let Some(task) = ticket.tasks.get(self.task_cursor) else {
            return;
        };
        let records = self
            .state
            .workflow
            .sessions
            .values()
            .filter(|record| record.ticket == ticket.id() && record.task == task.id)
            .cloned()
            .collect();
        self.hosted.dialog = Some(Dialog::Sessions { records, cursor: 0 });
    }
}

pub fn encode_key(key: KeyEvent, application_cursor: bool) -> Option<Vec<u8>> {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(control);
    let bytes = match key.code {
        KeyCode::Char(c) if control && c.is_ascii() => vec![(c as u8) & 0x1f],
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Esc => vec![0x1b],
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
            if modifier > 1 {
                format!("\x1b[1;{modifier}{suffix}").into_bytes()
            } else {
                format!("\x1b{}{suffix}", if application_cursor { 'O' } else { '[' }).into_bytes()
            }
        }
        KeyCode::Delete | KeyCode::Insert | KeyCode::PageUp | KeyCode::PageDown => {
            let n = match key.code {
                KeyCode::Insert => 2,
                KeyCode::Delete => 3,
                KeyCode::PageUp => 5,
                _ => 6,
            };
            if modifier > 1 {
                format!("\x1b[{n};{modifier}~").into_bytes()
            } else {
                format!("\x1b[{n}~").into_bytes()
            }
        }
        KeyCode::F(n) if (1..=12).contains(&n) => {
            let code = [11, 12, 13, 14, 15, 17, 18, 19, 20, 21, 23, 24][n as usize - 1];
            if n <= 4 && modifier == 1 {
                format!("\x1bO{}", (b'P' + n - 1) as char).into_bytes()
            } else if modifier > 1 {
                format!("\x1b[{code};{modifier}~").into_bytes()
            } else {
                format!("\x1b[{code}~").into_bytes()
            }
        }
        _ => return None,
    };
    if alt
        && matches!(
            key.code,
            KeyCode::Char(_) | KeyCode::Enter | KeyCode::Backspace
        )
    {
        Some([vec![0x1b], bytes].concat())
    } else {
        Some(bytes)
    }
}
