use super::*;
use crate::{
    host::runtime::{Client, HostCommand, Response, SessionSummary, TerminalSnapshot},
    hosted,
};
use std::collections::{BTreeSet, VecDeque};

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
    pub errors: VecDeque<String>,
    pub pending_launch: Option<hosted::Command>,
    pub discussion_approval: Option<crate::discussion_snapshot::Approval>,
    notifications_enabled: Option<bool>,
    configured_sessions: BTreeSet<String>,
    notification_retry: Instant,
    active_notification: Option<String>,
    last_request_poll: bool,
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
            errors: VecDeque::new(),
            pending_launch: None,
            discussion_approval: None,
            notifications_enabled: None,
            configured_sessions: BTreeSet::new(),
            notification_retry: Instant::now(),
            active_notification: None,
            last_request_poll: false,
        }
    }
}
pub enum Dialog {
    Start {
        ticket: String,
        task: String,
        brief: String,
        handoff: Field,
        editing: bool,
        scroll: u16,
        from_session: Option<String>,
        request_id: String,
    },
    Reconcile(String),
    RetryStart {
        scroll: u16,
    },
    Diagnostics {
        scroll: u16,
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
                command: hosted::Command::Start { .. }
                    | hosted::Command::CaptureDiscussion { .. }
                    | hosted::Command::StartDocument { .. }
                    | hosted::Command::StartRequest { .. }
                    | hosted::Command::Continue { .. }
                    | hosted::Command::Handover { .. }
                    | hosted::Command::Reconcile { .. },
                ..
            }
        );
        let client = if launch {
            let current = std::env::current_exe()?;
            let companion = current.with_file_name(if cfg!(windows) {
                "treerail-session-host.exe"
            } else {
                "treerail-session-host"
            });
            let executable = if companion.is_file() {
                companion
            } else {
                current
            };
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
        // Adjacent input packets may share a frame, preserving exact byte order.
        // Authority-changing commands remain separate FIFO barriers.
        if let HostCommand::Input {
            session,
            client,
            token,
            bytes,
        } = &command
            && let Some(HostCommand::Input {
                session: previous_session,
                client: previous_client,
                token: previous_token,
                bytes: previous_bytes,
            }) = self.hosted.queue.back_mut()
            && (session, client, token) == (&*previous_session, &*previous_client, &*previous_token)
            && previous_bytes.len() + bytes.len() <= 48 * 1024
        {
            previous_bytes.extend_from_slice(bytes);
            return;
        }
        if self.hosted.queue.len() >= 128 {
            self.hosted.input = false;
            self.record_hosted_error("Agent input queue is full. The latest input was not queued; navigation restored. Wait for pending input before retrying.".into());
        } else {
            self.hosted.queue.push_back(command);
        }
    }
    fn record_hosted_error(&mut self, message: String) {
        if self.hosted.errors.back() != Some(&message) {
            self.hosted.errors.push_back(message.clone());
            while self.hosted.errors.len() > 32 {
                self.hosted.errors.pop_front();
            }
        }
        self.say(format!("{message} · E details"), true);
    }
    pub fn hosted_request_failed(&mut self, message: String) {
        self.hosted.input = false;
        self.hosted.token = None;
        self.hosted.queue.retain(|command| {
            !matches!(
                command,
                HostCommand::Input { .. } | HostCommand::Resize { .. }
            )
        });
        if let Some(session) = self.hosted.active_notification.take() {
            self.hosted.configured_sessions.remove(&session);
            self.hosted.notification_retry = Instant::now() + Duration::from_secs(5);
        }
        if let Some(snapshot) = &mut self.hosted.terminal {
            snapshot.attention.unavailable(message.clone());
        }
        self.record_hosted_error(message);
    }
    pub fn hosted_diagnostics(&mut self) {
        self.hosted.dialog = Some(Dialog::Diagnostics { scroll: 0 });
    }
    pub fn session_command(&mut self, command: hosted::Command) {
        self.queue_host(HostCommand::Ticket {
            config_path: self.config_path.clone(),
            command,
        });
    }
    pub fn hosted_dimensions(&self) -> (u16, u16) {
        let cols = (self.viewport_width.saturating_mul(65) / 100)
            .saturating_sub(2)
            .clamp(1, 500) as u16;
        let rows = self
            .viewport_height
            .saturating_sub(if self.latest_operation.is_some() {
                16
            } else {
                14
            })
            .clamp(1, 500) as u16;
        (rows.min((20_000 / u32::from(cols)) as u16), cols)
    }
    fn focused_record(&self) -> Option<&hosted::SessionRecord> {
        let ticket = self.ticket.as_ref()?;
        let task = ticket.tasks.get(self.task_cursor)?;
        hosted::selected_session(&self.state.workflow, &ticket.id(), &task.id)
    }
    pub fn begin_agent(&mut self, new: bool) {
        if self.hosted.pending_launch.is_some() {
            self.hosted.dialog = Some(Dialog::RetryStart { scroll: 0 });
            return;
        }

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
        match hosted::discussion_context(ticket, &task.id, "") {
            Ok(brief) => {
                let capture = hosted::Command::CaptureDiscussion {
                    ticket: ticket.id(),
                    task: task.id.clone(),
                    handoff: String::new(),
                };
                self.hosted.discussion_approval = None;
                self.hosted.dialog = Some(Dialog::Start {
                    ticket: ticket.id(),
                    task: task.id.clone(),
                    brief,
                    handoff: Field::new("Optional handoff", String::new(), true),
                    editing: false,
                    scroll: 0,
                    from_session: None,
                    request_id: crate::copilot::new_session_id(),
                });
                self.session_command(capture);
            }
            Err(error) => self.say(format!("{error:#}"), true),
        }
    }
    pub fn hosted_key(&mut self, key: KeyEvent) -> bool {
        if let Some(dialog) = self.hosted.dialog.take() {
            if key.kind == KeyEventKind::Repeat
                && !matches!(&dialog, Dialog::Start { editing: true, .. })
                && !matches!(
                    key.code,
                    KeyCode::Char('j' | 'k')
                        | KeyCode::Up
                        | KeyCode::Down
                        | KeyCode::PageUp
                        | KeyCode::PageDown
                )
            {
                self.hosted.dialog = Some(dialog);
                return true;
            }
            let confirmed = key.code == KeyCode::Char('y') && key.modifiers == KeyModifiers::NONE;
            match dialog {
                Dialog::Start { .. } => {
                    self.brief_dialog_key(dialog, key);
                }
                Dialog::Reconcile(session) if confirmed => {
                    self.session_command(hosted::Command::Reconcile { session })
                }
                Dialog::RetryStart { mut scroll } => {
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('q') => return true,
                        KeyCode::Char('y') if confirmed => {
                            if let Some(command) = self.hosted.pending_launch.clone() {
                                self.session_command(command);
                            }
                            return true;
                        }
                        KeyCode::Char('f') if key.modifiers == KeyModifiers::NONE => {
                            self.hosted.pending_launch = None;
                            self.say("Local retry reminder cleared; saved conversations and running work remain unchanged",false);
                            return true;
                        }
                        KeyCode::Char('j') | KeyCode::Down => scroll = scroll.saturating_add(1),
                        KeyCode::Char('k') | KeyCode::Up => scroll = scroll.saturating_sub(1),
                        KeyCode::PageDown => scroll = scroll.saturating_add(10),
                        KeyCode::PageUp => scroll = scroll.saturating_sub(10),
                        _ => {}
                    }
                    self.hosted.dialog = Some(Dialog::RetryStart { scroll });
                }
                Dialog::Diagnostics { mut scroll } => {
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('q') => return true,
                        KeyCode::Char('j') | KeyCode::Down => scroll = scroll.saturating_add(1),
                        KeyCode::Char('k') | KeyCode::Up => scroll = scroll.saturating_sub(1),
                        KeyCode::PageDown => scroll = scroll.saturating_add(10),
                        KeyCode::PageUp => scroll = scroll.saturating_sub(10),
                        _ => {}
                    }
                    self.hosted.dialog = Some(Dialog::Diagnostics { scroll });
                }
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
                                self.detach_agent();
                                self.session_command(hosted::Command::SelectSession {
                                    session: record.id.clone(),
                                });
                            }
                            return true;
                        }
                        KeyCode::Char('c') if key.modifiers == KeyModifiers::NONE => {
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
            if key.code == KeyCode::Char('E') && key.modifiers == KeyModifiers::NONE {
                self.hosted_diagnostics();
                return true;
            }
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
        if let Some(Dialog::Start {
            handoff, editing, ..
        }) = &mut self.hosted.dialog
        {
            if *editing {
                self.hosted.discussion_approval = None;
                let text = text.replace("\r\n", "\n").replace('\r', "\n");
                if handoff.value.len() + text.len() <= 48 * 1024 {
                    handoff.value.insert_str(handoff.cursor, &text);
                    handoff.cursor += text.len();
                } else {
                    self.say("Optional handoff exceeds the 48 KiB limit", true);
                }
            }
            return true;
        }
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
        if let Some(session) = self.pending_cleanup_stop.take() {
            self.session_command(hosted::Command::Stop { session });
        }
        if self.hosted.notifications_enabled != Some(self.config.desktop_notifications) {
            self.hosted.notifications_enabled = Some(self.config.desktop_notifications);
            self.hosted.configured_sessions.clear();
            self.hosted.notification_retry = Instant::now();
        }
        if Instant::now() >= self.hosted.notification_retry {
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
                if self.hosted.queue.len() >= 128 {
                    break;
                }
                if self.hosted.configured_sessions.insert(session.clone()) {
                    self.queue_host(HostCommand::ConfigureSession {
                        session,
                        ticket,
                        desktop_notifications: self.config.desktop_notifications,
                    });
                }
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
        if !self.inflight.contains("host") {
            let may_poll = self.hosted.queue.is_empty()
                || (!self.hosted.last_request_poll
                    && matches!(self.hosted.queue.front(), Some(HostCommand::Input { .. })));
            if may_poll {
                // Summaries have their own deadline: slow screen polls cannot
                // starve attention from other tickets, even while typing.
                if self.hosted.summary_poll.elapsed() > Duration::from_secs(1)
                    && self
                        .state
                        .workflow
                        .sessions
                        .values()
                        .any(|record| record.health == hosted::Health::Live)
                {
                    self.hosted.summary_poll = Instant::now();
                    self.hosted.queue.push_front(HostCommand::Summary);
                } else if self.hosted.poll.elapsed() > Duration::from_millis(200)
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
                    self.hosted.queue.push_front(command);
                }
            }
            if let Some(command) = self.hosted.queue.pop_front() {
                self.hosted.last_request_poll = matches!(
                    command,
                    HostCommand::Summary
                        | HostCommand::Snapshot { .. }
                        | HostCommand::Resize { .. }
                );
                self.hosted.active_notification =
                    if let HostCommand::ConfigureSession { session, .. } = &command {
                        Some(session.clone())
                    } else {
                        None
                    };
                self.enqueue(Job::Host(self.config_path.clone(), command));
            }
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
        self.hosted.active_notification = None;
        match response {
            Response::Ticket(event) => match *event {
                hosted::Event::Started(record) => {
                    self.state
                        .workflow
                        .sessions
                        .insert(record.id.clone(), record.clone());
                    if self.hosted.pending_launch.as_ref().is_some_and(|command| matches!(command,hosted::Command::StartRequest {request_id,..}|hosted::Command::StartDocument {request_id,..} if request_id==&record.id)) { self.hosted.pending_launch=None; }
                    self.hosted.record = Some(record.clone());
                    self.hosted.input = false;
                    self.hosted.terminal = None;
                    self.hosted.token = None;
                    self.hosted.queue.retain(|command| {
                        !matches!(
                            command,
                            HostCommand::Input { .. } | HostCommand::Resize { .. }
                        )
                    });
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
                        .insert(record.id.clone(), record.clone());
                    if self
                        .hosted
                        .record
                        .as_ref()
                        .is_some_and(|current| current.id == record.id)
                    {
                        self.hosted.record = Some(record);
                        self.hosted.input = false;
                        self.hosted.terminal = None;
                        self.hosted.token = None;
                    }
                    self.say("Session stopped; recorded conversation retained", false);
                }
                hosted::Event::State(state) => self.state.workflow = state,
                hosted::Event::RevisionSent => self.say(
                    "Revised requirements sent to the selected live conversation",
                    false,
                ),
                hosted::Event::Workflow(_) => {}
                hosted::Event::DiscussionCaptured(approval) => {
                    if let Some(Dialog::Start {
                        ticket,
                        task,
                        handoff,
                        brief,
                        scroll,
                        ..
                    }) = &mut self.hosted.dialog
                    {
                        let matching = self
                            .state
                            .workflow
                            .tickets
                            .get(ticket)
                            .or(self.ticket.as_ref().filter(|record| record.id() == *ticket))
                            .and_then(|record| {
                                hosted::discussion_context(record, task, &handoff.value).ok()
                            })
                            .is_some_and(|context| context == approval.context);
                        if matching {
                            match approval
                                .snapshot
                                .validate(&self.config_path)
                                .and_then(|()| {
                                    std::fs::read_to_string(&approval.snapshot.path)
                                        .map_err(Into::into)
                                }) {
                                Ok(document) => {
                                    *brief = format!(
                                        "{} comments · {} bytes\nRetained document: {}\nSHA-256: {}\n\n{}",
                                        approval.snapshot.comments,
                                        approval.snapshot.bytes,
                                        approval.snapshot.path.display(),
                                        approval.snapshot.sha256,
                                        document
                                    );
                                    *scroll = 0;
                                    self.hosted.discussion_approval = Some(approval);
                                }
                                Err(error) => self.record_hosted_error(format!("{error:#}")),
                            }
                        }
                    }
                }
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
                    if snapshot.exited {
                        self.hosted.input = false;
                        self.hosted.token = None;
                    }
                    self.hosted.terminal = Some(snapshot);
                }
            }
            Response::Summary(summaries) => {
                for summary in &summaries {
                    if let Some(error) = &summary.notification_error
                        && !self.hosted.summaries.iter().any(|previous| {
                            previous.session == summary.session
                                && previous.notification_error.as_ref() == Some(error)
                        })
                    {
                        self.record_hosted_error(format!(
                            "Desktop notification for {}: {error}",
                            summary.session
                        ));
                    }
                }
                self.hosted.summaries = summaries;
            }
            Response::Error(message) => self.hosted_request_failed(message),
            Response::Done => {}
            _ => self.say("Unexpected session host response", true),
        }
    }
    fn brief_dialog_key(&mut self, dialog: Dialog, key: KeyEvent) {
        let Dialog::Start {
            ticket,
            task,
            brief,
            mut handoff,
            mut editing,
            mut scroll,
            from_session,
            request_id,
        } = dialog
        else {
            return;
        };
        if editing {
            self.hosted.discussion_approval = None;
            match key.code {
                KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab => editing = false,
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    handoff.value.clear();
                    handoff.cursor = 0;
                }
                _ => {
                    let growth = match key.code {
                        KeyCode::Char(c)
                            if !key
                                .modifiers
                                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                        {
                            c.len_utf8()
                        }
                        KeyCode::Enter => 1,
                        _ => 0,
                    };
                    if handoff.value.len().saturating_add(growth) <= 48 * 1024 {
                        edit(&mut handoff, key);
                    } else {
                        self.say("Optional handoff exceeds the 48 KiB limit", true);
                    }
                }
            }
        } else {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.hosted.discussion_approval = None;
                    self.hosted.queue.retain(|command| {
                        !matches!(
                            command,
                            HostCommand::Ticket {
                                command: hosted::Command::CaptureDiscussion { .. },
                                ..
                            }
                        )
                    });
                    return;
                }
                KeyCode::Tab | KeyCode::BackTab => editing = true,
                KeyCode::Char('j') | KeyCode::Down => scroll = scroll.saturating_add(1),
                KeyCode::Char('k') | KeyCode::Up => scroll = scroll.saturating_sub(1),
                KeyCode::PageDown => scroll = scroll.saturating_add(10),
                KeyCode::PageUp => scroll = scroll.saturating_sub(10),
                KeyCode::Home => scroll = 0,
                KeyCode::Char('y') if key.modifiers == KeyModifiers::NONE => {
                    let (rows, cols) = self.hosted_dimensions();
                    if let Some(approval) = self.hosted.discussion_approval.take() {
                        let command = hosted::Command::StartDocument {
                            request_id,
                            ticket,
                            task,
                            handoff: handoff.value,
                            rows,
                            cols,
                            approval,
                            from_session,
                        };
                        self.hosted.pending_launch = Some(command.clone());
                        self.session_command(command);
                        return;
                    }
                    self.session_command(hosted::Command::CaptureDiscussion {
                        ticket: ticket.clone(),
                        task: task.clone(),
                        handoff: handoff.value.clone(),
                    });
                    self.say("Fetching complete fresh discussion; inspect the resulting document before confirming",false);
                }
                _ => {}
            }
        }
        self.hosted.dialog = Some(Dialog::Start {
            ticket,
            task,
            brief,
            handoff,
            editing,
            scroll,
            from_session,
            request_id,
        });
    }
    pub fn begin_handover(&mut self) {
        let Some(ticket) = self.ticket.as_ref() else {
            return;
        };
        let Some(checkout) = &ticket.worktree else {
            self.say("Prepare the ticket checkout before handover", true);
            return;
        };
        let owner = workflow::deletion_owner(&self.state.workflow, &checkout.path).cloned();
        let Some(owner) = owner else {
            self.say(
                "No recorded owner needs handover; use n for a new session",
                false,
            );
            return;
        };
        self.begin_agent(true);
        if let Some(Dialog::Start { from_session, .. }) = &mut self.hosted.dialog {
            *from_session = Some(owner.id);
        }
    }
    pub fn reconcile_agent_dialog(&mut self) {
        if let Some(record) = self.focused_record() {
            self.hosted.dialog = Some(Dialog::Reconcile(record.id.clone()));
        } else {
            self.say(
                "Select a recorded conversation before examining its execution",
                true,
            );
        }
    }
    pub fn retry_agent_dialog(&mut self) {
        if self.hosted.pending_launch.is_some() {
            self.hosted.dialog = Some(Dialog::RetryStart { scroll: 0 });
        } else {
            self.say("There is no unresolved local launch request", false);
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
        let mut records = self
            .state
            .workflow
            .sessions
            .values()
            .filter(|record| record.ticket == ticket.id() && record.task == task.id)
            .cloned()
            .collect::<Vec<_>>();
        records.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        let cursor = records
            .iter()
            .position(|record| record.current)
            .unwrap_or(0);
        self.hosted.dialog = Some(Dialog::Sessions { records, cursor });
    }
}

pub fn draft_brief(brief: &str, handoff: &str) -> String {
    let prefix = brief.strip_suffix("None supplied.").unwrap_or(brief);
    format!(
        "{prefix}{}",
        if handoff.trim().is_empty() {
            "None supplied."
        } else {
            handoff
        }
    )
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
