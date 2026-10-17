//! Durable ticket conversations and the production host command surface.
use crate::workflow::{self, TicketRecord};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    future::Future,
    io::{BufRead, Read},
    path::{Path, PathBuf},
    pin::Pin,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Health {
    #[default]
    Starting,
    Live,
    Stopped,
    Interrupted,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub ticket: String,
    pub task: String,
    pub checkout: PathBuf,
    pub provider_session_id: String,
    pub starting_brief: String,
    pub created_at: u64,
    pub host_session: Option<String>,
    pub health: Health,
    #[serde(default)]
    pub current: bool,
    pub error: Option<String>,
    #[serde(default)]
    pub host_instance: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    Workflow {
        project: PathBuf,
        command: workflow::Command,
    },
    Snapshot,
    Start {
        ticket: String,
        task: String,
        handoff: String,
        rows: u16,
        cols: u16,
    },
    Continue {
        session: String,
        rows: u16,
        cols: u16,
    },
    Stop {
        session: String,
    },
    SelectSession {
        session: String,
    },
    Handover {
        from_session: String,
        ticket: String,
        task: String,
        handoff: String,
        rows: u16,
        cols: u16,
    },
    SendRevision {
        session: String,
        instructions: String,
        client: String,
        token: u64,
    },
}

// Workflow events retain complete durable snapshots.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    Workflow(Box<workflow::Event>),
    State(workflow::WorkflowState),
    Started(SessionRecord),
    Stopped(SessionRecord),
    RevisionSent,
}

pub fn starting_brief(ticket: &TicketRecord, task: &str, handoff: &str) -> Result<String> {
    let objective = ticket
        .tasks
        .iter()
        .find(|candidate| candidate.id == task)
        .context("Unknown task")?;
    let checkout = ticket
        .worktree
        .as_ref()
        .context("Prepare or adopt this ticket's shared worktree first")?;
    ensure!(
        checkout.path.is_dir(),
        "Ticket checkout is missing; reconcile it before starting work"
    );
    Ok(format!(
        "GitHub ticket: {}\n{}\n\nTask: {}\nWorktree: {}\nBranch: {}\n\nIssue requirements:\n{}\n\nExplicit handoff:\n{}",
        ticket.issue.url,
        ticket.issue.title,
        objective.title,
        checkout.path.display(),
        checkout.branch,
        ticket.issue.body,
        if handoff.trim().is_empty() {
            "None supplied."
        } else {
            handoff
        }
    ))
}

pub fn selected_session<'a>(
    state: &'a workflow::WorkflowState,
    ticket: &str,
    task: &str,
) -> Option<&'a SessionRecord> {
    state
        .sessions
        .values()
        .filter(|record| record.ticket == ticket && record.task == task)
        .max_by_key(|record| (record.current, record.created_at, &record.id))
}

/// A narrow command/event seam also used by controlled provider lifecycle tests.
trait SessionRuntime {
    fn request(
        &mut self,
        command: crate::host::runtime::HostCommand,
    ) -> Pin<Box<dyn Future<Output = Result<crate::host::runtime::Response>> + '_>>;
    fn provider_home(&self) -> Result<PathBuf> {
        Ok(directories::BaseDirs::new()
            .context("Current user's provider home is unavailable")?
            .home_dir()
            .join(".copilot"))
    }
}
impl SessionRuntime for crate::host::runtime::Host {
    fn request(
        &mut self,
        command: crate::host::runtime::HostCommand,
    ) -> Pin<Box<dyn Future<Output = Result<crate::host::runtime::Response>> + '_>> {
        Box::pin(self.execute(command))
    }
}

/// Executed inside the authoritative host, never as a second policy in a UI.
pub(crate) async fn execute(
    host: &mut crate::host::runtime::Host,
    config_path: PathBuf,
    command: Command,
) -> Result<Event> {
    execute_with(host, config_path, command).await
}
async fn instance(runtime: &mut impl SessionRuntime) -> Result<String> {
    match runtime
        .request(crate::host::runtime::HostCommand::Ping)
        .await?
    {
        crate::host::runtime::Response::Pong { instance, .. } if !instance.is_empty() => {
            Ok(instance)
        }
        _ => anyhow::bail!("Host identity is unavailable; managed work must be reconciled"),
    }
}
fn choose(state: &mut workflow::WorkflowState, session: &str) -> Result<SessionRecord> {
    let chosen = state
        .sessions
        .get(session)
        .context("Unknown recorded conversation")?
        .clone();
    for record in state
        .sessions
        .values_mut()
        .filter(|record| record.ticket == chosen.ticket && record.task == chosen.task)
    {
        record.current = record.id == session;
    }
    state
        .tickets
        .get_mut(&chosen.ticket)
        .context("Recorded ticket is missing")?
        .selected_task = Some(chosen.task.clone());
    Ok(state.sessions.get(session).unwrap().clone())
}
async fn verify_checkout(state: &workflow::WorkflowState, record: &SessionRecord) -> Result<()> {
    let ticket = state
        .tickets
        .get(&record.ticket)
        .context("Recorded ticket is missing")?;
    ensure!(
        ticket.tasks.iter().any(|task| task.id == record.task),
        "Recorded task is missing; reconcile it first"
    );
    let checkout = ticket
        .worktree
        .as_ref()
        .context("Ticket worktree is no longer associated")?;
    ensure!(
        record
            .checkout
            .canonicalize()
            .context("Recorded checkout is missing")?
            == checkout
                .path
                .canonicalize()
                .context("Ticket checkout is missing")?,
        "Ticket's worktree differs from this conversation; reconcile it before continuing"
    );
    if let Some(branch) = &record.branch {
        ensure!(
            branch == &checkout.branch,
            "Ticket branch differs from this conversation's recorded branch"
        );
        let actual = crate::git::run(&record.checkout, ["branch", "--show-current"]).await?;
        ensure!(
            String::from_utf8(actual)?.trim() == branch,
            "Git checkout branch changed outside this conversation; reconcile it before continuing"
        );
    }
    Ok(())
}
fn has_owner(
    state: &workflow::WorkflowState,
    ticket: &str,
    checkout: &Path,
    except: Option<&str>,
) -> Option<String> {
    state
        .sessions
        .values()
        .find(|record| {
            Some(record.id.as_str()) != except
                && matches!(
                    record.health,
                    Health::Live | Health::Starting | Health::Interrupted
                )
                && (record.ticket == ticket
                    || record
                        .checkout
                        .canonicalize()
                        .ok()
                        .zip(checkout.canonicalize().ok())
                        .is_some_and(|(a, b)| a == b))
        })
        .map(|record| record.id.clone())
}
async fn execute_with(
    runtime: &mut impl SessionRuntime,
    config_path: PathBuf,
    command: Command,
) -> Result<Event> {
    use crate::host::runtime::{HostCommand, Response};
    let workflow = workflow::Workflow::new(&config_path);
    match command {
        Command::Workflow { project, command } => Ok(Event::Workflow(Box::new(
            workflow.execute(&project, command).await?,
        ))),
        Command::Snapshot => Ok(Event::State(workflow.snapshot()?)),
        Command::SelectSession { session } => {
            workflow.update(|state| choose(state, &session))?;
            Ok(Event::State(workflow.snapshot()?))
        }
        Command::Start {
            ticket,
            task,
            handoff,
            rows,
            cols,
        } => {
            let host_instance = instance(runtime).await?;
            let record = workflow.update(|state| {
                let ticket_record = state.tickets.get(&ticket).context("Unknown ticket")?;
                let brief = starting_brief(ticket_record, &task, &handoff)?;
                ensure!(brief.len() <= 48 * 1024, "Starting brief exceeds the hosted limit; refine the ticket before starting");
                let checkout = ticket_record.worktree.as_ref().unwrap();
                ensure!(has_owner(state, &ticket, &checkout.path, None).is_none(), "Ticket worktree is owned by another managed conversation; stop it explicitly or use Handover");
                let id = crate::copilot::new_session_id();
                let record = SessionRecord {
                    id: id.clone(), ticket: ticket.clone(), task: task.clone(), checkout: checkout.path.clone(), provider_session_id: id,
                    starting_brief: brief, created_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs(),
                    host_session: None, health: Health::Starting, current: true, error: None,
                    host_instance: Some(host_instance), branch: Some(checkout.branch.clone()),
                };
                state.sessions.insert(record.id.clone(), record.clone()); choose(state, &record.id)
            })?;
            let result = runtime
                .request(HostCommand::StartCopilot {
                    request_id: record.id.clone(),
                    checkout: record.checkout.clone(),
                    provider_session_id: record.provider_session_id.clone(),
                    resume: false,
                    brief: Some(record.starting_brief.clone()),
                    rows,
                    cols,
                })
                .await;
            finish_start(&workflow, runtime, record, result).await
        }
        Command::Continue {
            session,
            rows,
            cols,
        } => {
            let state = workflow.snapshot()?;
            let mut record = state
                .sessions
                .get(&session)
                .context("Unknown recorded conversation")?
                .clone();
            verify_checkout(&state, &record).await?;
            let host_instance = instance(runtime).await?;
            if let Some(id) = &record.host_session {
                ensure!(
                    record.host_instance.as_ref() == Some(&host_instance),
                    "Recorded terminal belongs to another host instance; reconcile interrupted work before resuming"
                );
                match runtime
                    .request(HostCommand::Snapshot {
                        session: id.clone(),
                    })
                    .await
                {
                    Ok(Response::Terminal(snapshot)) if !snapshot.exited => {
                        ensure!(
                            snapshot.provider_session_id.as_deref()
                                == Some(&record.provider_session_id),
                            "Provider conversation identity differs; checkout remains owned until an explicit verified stop"
                        );
                        record = workflow.update(|state| choose(state, &session))?;
                        return Ok(Event::Started(record));
                    }
                    Ok(Response::Terminal(_)) => {
                        // Reap and verify the whole managed group before releasing its lease.
                        ensure!(
                            matches!(
                                runtime
                                    .request(HostCommand::Stop {
                                        session: id.clone()
                                    })
                                    .await?,
                                Response::Done
                            ),
                            "Stop was not verified"
                        );
                        record = mark_stopped(&workflow, &session)?;
                    }
                    Err(error) => {
                        workflow.update(|state| {
                            let record = state
                                .sessions
                                .get_mut(&session)
                                .context("Unknown session")?;
                            record.health = Health::Interrupted;
                            record.error = Some(format!("{error:#}"));
                            Ok(())
                        })?;
                        anyhow::bail!(
                            "Recorded terminal is unavailable. Reconcile interrupted work before explicitly resuming this conversation."
                        );
                    }
                    _ => anyhow::bail!("Unexpected terminal response"),
                }
            }
            ensure!(
                record.health == Health::Stopped,
                "Reconcile and verify interrupted work before resuming"
            );
            ensure!(
                has_owner(&state, &record.ticket, &record.checkout, Some(&record.id)).is_none(),
                "Another conversation owns this shared checkout; hand it over first"
            );
            verify_provider_record(&runtime.provider_home()?, &record)?;
            let result = runtime
                .request(HostCommand::StartCopilot {
                    request_id: crate::copilot::new_session_id(),
                    checkout: record.checkout.clone(),
                    provider_session_id: record.provider_session_id.clone(),
                    resume: true,
                    brief: None,
                    rows,
                    cols,
                })
                .await;
            record.host_instance = Some(host_instance);
            workflow.update(|state| {
                state
                    .sessions
                    .get_mut(&session)
                    .context("Unknown session")?
                    .host_instance = record.host_instance.clone();
                Ok(())
            })?;
            finish_start(&workflow, runtime, record, result).await
        }
        Command::Stop { session } => {
            let record = workflow
                .snapshot()?
                .sessions
                .get(&session)
                .context("Unknown recorded conversation")?
                .clone();
            let id = record
                .host_session
                .as_ref()
                .context("No live terminal to stop; reconcile interrupted work")?;
            ensure!(
                record.host_instance.as_ref() == Some(&instance(runtime).await?),
                "Recorded terminal belongs to another host; reconcile it first"
            );
            // Failure/cancellation leaves the record and checkout ownership unchanged.
            ensure!(
                matches!(
                    runtime
                        .request(HostCommand::Stop {
                            session: id.clone()
                        })
                        .await?,
                    Response::Done
                ),
                "Stop was not verified; checkout remains owned"
            );
            Ok(Event::Stopped(mark_stopped(&workflow, &session)?))
        }
        Command::Handover {
            from_session,
            ticket,
            task,
            handoff,
            rows,
            cols,
        } => {
            let state = workflow.snapshot()?;
            let previous = state
                .sessions
                .get(&from_session)
                .context("Unknown conversation to hand over")?;
            ensure!(
                previous.ticket == ticket,
                "Handover must remain inside this ticket"
            );
            let target = state.tickets.get(&ticket).context("Unknown ticket")?;
            starting_brief(target, &task, &handoff)?; // Validate before stopping existing work.
            Box::pin(execute_with(
                runtime,
                config_path.clone(),
                Command::Stop {
                    session: from_session,
                },
            ))
            .await?;
            Box::pin(execute_with(
                runtime,
                config_path,
                Command::Start {
                    ticket,
                    task,
                    handoff,
                    rows,
                    cols,
                },
            ))
            .await
        }
        Command::SendRevision {
            session,
            instructions,
            client,
            token,
        } => {
            ensure!(
                !instructions.trim().is_empty() && instructions.len() <= 48 * 1024,
                "Revised requirements are empty or too large"
            );
            let record = workflow
                .snapshot()?
                .sessions
                .get(&session)
                .context("Unknown session")?
                .clone();
            let id = record
                .host_session
                .context("Select a live conversation to send revised requirements")?;
            ensure!(
                record.host_instance.as_ref() == Some(&instance(runtime).await?),
                "Recorded terminal belongs to another host; reconcile it first"
            );
            let Response::Terminal(snapshot) = runtime
                .request(HostCommand::Snapshot {
                    session: id.clone(),
                })
                .await?
            else {
                anyhow::bail!("Terminal unavailable");
            };
            ensure!(
                !snapshot.exited
                    && snapshot.provider_session_id.as_deref() == Some(&record.provider_session_id)
                    && snapshot.attention.healthy
                    && snapshot.attention.value == Some(crate::copilot::AttentionState::Ready),
                "The recorded agent must be Ready before revised requirements can be sent"
            );
            ensure!(
                snapshot.bracketed_paste,
                "Wait for Copilot's prompt with bracketed paste enabled before sending multiline requirements"
            );
            let mut bytes = b"\x1b[200~".to_vec();
            bytes.extend_from_slice(instructions.as_bytes());
            bytes.extend_from_slice(b"\x1b[201~\r");
            ensure!(
                matches!(
                    runtime
                        .request(HostCommand::Input {
                            session: id,
                            client,
                            token,
                            bytes
                        })
                        .await?,
                    Response::Done
                ),
                "Requirements were not acknowledged"
            );
            Ok(Event::RevisionSent)
        }
    }
}
fn mark_stopped(workflow: &workflow::Workflow, session: &str) -> Result<SessionRecord> {
    workflow.update(|state| {
        let record = state.sessions.get_mut(session).context("Unknown session")?;
        record.host_session = None;
        record.health = Health::Stopped;
        record.error = None;
        Ok(record.clone())
    })
}
async fn finish_start(
    workflow: &workflow::Workflow,
    runtime: &mut impl SessionRuntime,
    record: SessionRecord,
    result: Result<crate::host::runtime::Response>,
) -> Result<Event> {
    use crate::host::runtime::Response;
    match result {
        Ok(Response::Started { session, .. }) => {
            let record = workflow.update(|state| {
                let record = state
                    .sessions
                    .get_mut(&record.id)
                    .context("Starting conversation record disappeared")?;
                record.host_session = Some(session);
                record.health = Health::Live;
                record.error = None;
                let id = record.id.clone();
                choose(state, &id)
            })?;
            let setup = runtime
                .request(crate::host::runtime::HostCommand::ConfigureSession {
                    session: record.host_session.clone().unwrap(),
                    ticket: record.ticket.clone(),
                    desktop_notifications: crate::config::Config::load(&workflow.config_path)?
                        .desktop_notifications,
                })
                .await;
            let record = if !matches!(setup, Ok(crate::host::runtime::Response::Done)) {
                workflow.update(|state| {
                    let record = state
                        .sessions
                        .get_mut(&record.id)
                        .context("Unknown conversation")?;
                    record.error = Some(
                        "Agent is running; notification setup failed. Reconnect to retry setup."
                            .into(),
                    );
                    Ok(record.clone())
                })?
            } else {
                record
            };
            Ok(Event::Started(record))
        }
        other => {
            let error = match other {
                Err(error) => format!("{error:#}"),
                Ok(_) => "Unexpected host start response".into(),
            };
            workflow.update(|state| {
                let record = state
                    .sessions
                    .get_mut(&record.id)
                    .context("Unknown starting conversation")?;
                record.health = Health::Failed;
                record.error = Some(error.clone());
                Ok(())
            })?;
            anyhow::bail!(error)
        }
    }
}

/// Check only the exact provider record requested by the retained Tree Rail conversation.
/// No latest-session lookup, transcript import, or missing-ID fallback is permitted.
fn verify_provider_record(home: &Path, record: &SessionRecord) -> Result<()> {
    ensure!(
        crate::copilot::is_uuid(&record.provider_session_id),
        "Recorded provider conversation UUID is invalid"
    );
    let directory = home.join("session-state").join(&record.provider_session_id);
    let file = std::fs::File::open(directory.join("events.jsonl")).context("Saved Copilot conversation is missing; choose Start new session explicitly to create a replacement")?;
    let mut first = String::new();
    std::io::BufReader::new(file)
        .take(65537)
        .read_line(&mut first)?;
    ensure!(
        first.len() <= 65536,
        "Provider identity record exceeds supported size"
    );
    let event: serde_json::Value =
        serde_json::from_str(&first).context("Provider conversation identity record is invalid")?;
    ensure!(
        event["type"] == "session.start"
            && event["data"]["sessionId"].as_str() == Some(&record.provider_session_id),
        "Provider conversation identity differs from the recorded UUID"
    );
    // Copilot 1.0.94's current workspace metadata is intentionally parsed narrowly.
    // Unknown scalar forms fail closed rather than guessing a checkout.
    let file = std::fs::File::open(directory.join("workspace.yaml"))
        .context("Saved Copilot working context is unavailable")?;
    ensure!(
        file.metadata()?.len() <= 65536,
        "Provider workspace metadata exceeds supported size"
    );
    let mut metadata = String::new();
    file.take(65537).read_to_string(&mut metadata)?;
    let scalar = |key: &str| -> Result<String> {
        let raw = metadata
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .context("Provider workspace identity/context is missing")?
            .trim();
        if raw.starts_with('"') {
            return serde_json::from_str::<String>(raw)
                .context("Unsupported provider workspace string");
        }
        if raw.starts_with('\'') && raw.ends_with('\'') && raw.len() >= 2 {
            return Ok(raw[1..raw.len() - 1].replace("''", "'"));
        }
        ensure!(
            !raw.is_empty() && !raw.starts_with(['|', '>', '[', '{']) && !raw.contains(" #"),
            "Unsupported provider workspace metadata scalar"
        );
        Ok(raw.into())
    };
    ensure!(
        scalar("id: ")? == record.provider_session_id,
        "Provider working context belongs to a different conversation"
    );
    let cwd = PathBuf::from(scalar("cwd: ")?);
    ensure!(
        cwd.canonicalize()
            .context("Provider's saved checkout is unavailable")?
            == record.checkout.canonicalize()?,
        "Provider's saved checkout differs from this ticket; reconcile it before resuming"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::runtime::{HostCommand, Response, TerminalSnapshot};
    use std::collections::BTreeMap;
    struct Runtime {
        home: PathBuf,
        instance: String,
        terminals: BTreeMap<String, TerminalSnapshot>,
        launches: Vec<(String, bool, Option<String>)>,
        fail_stop: bool,
    }
    impl SessionRuntime for Runtime {
        fn provider_home(&self) -> Result<PathBuf> {
            Ok(self.home.clone())
        }
        fn request(
            &mut self,
            command: HostCommand,
        ) -> Pin<Box<dyn Future<Output = Result<Response>> + '_>> {
            let result = (|| -> Result<Response> {
                match command {
                    HostCommand::Ping => Ok(Response::Pong {
                        version: 1,
                        instance: self.instance.clone(),
                    }),
                    HostCommand::StartCopilot {
                        checkout,
                        provider_session_id,
                        resume,
                        brief,
                        ..
                    } => {
                        let id = format!("terminal-{}", self.launches.len());
                        self.launches
                            .push((provider_session_id.clone(), resume, brief));
                        let directory = self.home.join("session-state").join(&provider_session_id);
                        std::fs::create_dir_all(&directory)?;
                        if !resume {
                            std::fs::write(
                                directory.join("events.jsonl"),
                                serde_json::to_string(
                                    &serde_json::json!({"type":"session.start","data":{"sessionId":provider_session_id}}),
                                )?,
                            )?;
                            std::fs::write(
                                directory.join("workspace.yaml"),
                                format!("id: {provider_session_id}\ncwd: {}\n", checkout.display()),
                            )?;
                        }
                        self.terminals.insert(
                            id.clone(),
                            TerminalSnapshot {
                                session: id.clone(),
                                process_id: Some(42),
                                execution_group: None,
                                rows: 30,
                                cols: 100,
                                formatted: vec![],
                                cursor: (0, 0),
                                cursor_hidden: false,
                                alternate_screen: false,
                                bracketed_paste: true,
                                sequence: 1,
                                exited: false,
                                input_token: None,
                                provider_session_id: Some(provider_session_id),
                                attention: crate::copilot::Attention::default(),
                            },
                        );
                        Ok(Response::Started {
                            session: id,
                            process_id: Some(42),
                        })
                    }
                    HostCommand::ConfigureSession { .. } => Ok(Response::Done),
                    HostCommand::Snapshot { session } => self
                        .terminals
                        .get(&session)
                        .cloned()
                        .map(Response::Terminal)
                        .context("Unknown terminal"),
                    HostCommand::Stop { session } => {
                        ensure!(!self.fail_stop, "Injected unverified stop");
                        self.terminals
                            .remove(&session)
                            .context("Unknown terminal")?;
                        Ok(Response::Done)
                    }
                    _ => anyhow::bail!("Unexpected lifecycle command"),
                }
            })();
            Box::pin(async move { result })
        }
    }
    struct Fixture {
        _temp: tempfile::TempDir,
        path: PathBuf,
        runtime: Runtime,
    }
    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let checkout = temp.path().join("checkout");
            std::fs::create_dir(&checkout).unwrap();
            assert!(
                std::process::Command::new("git")
                    .current_dir(&checkout)
                    .args(["init", "--initial-branch=feature"])
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
            let path = temp.path().join("treerail.toml");
            let workflow = workflow::Workflow::new(&path);
            workflow
                .update(|state| {
                    state.tickets.insert(
                        "owner/repo#1".into(),
                        TicketRecord {
                            repository: "owner/repo".into(),
                            project: checkout.clone(),
                            issue: workflow::Issue {
                                number: 1,
                                title: "Deliver feature".into(),
                                body: "Acceptance criteria remain issue requirements".into(),
                                state: "OPEN".into(),
                                url: "https://github.com/owner/repo/issues/1".into(),
                            },
                            fetched_at: 1,
                            prepared_issue: None,
                            reviewed_issue: None,
                            github_error: None,
                            last_attempt_at: None,
                            checkout_removed_at: None,
                            tasks: vec![
                                workflow::Task {
                                    id: "implementation".into(),
                                    title: "Implement".into(),
                                    progress: workflow::Progress::InProgress,
                                },
                                workflow::Task {
                                    id: "review".into(),
                                    title: "Review".into(),
                                    progress: workflow::Progress::Pending,
                                },
                            ],
                            next_task_id: 2,
                            selected_task: Some("implementation".into()),
                            worktree: Some(workflow::Checkout {
                                path: checkout,
                                branch: "feature".into(),
                                original: false,
                            }),
                        },
                    );
                    Ok(())
                })
                .unwrap();
            let runtime = Runtime {
                home: temp.path().join("copilot"),
                instance: "host-one".into(),
                terminals: BTreeMap::new(),
                launches: vec![],
                fail_stop: false,
            };
            Self {
                _temp: temp,
                path,
                runtime,
            }
        }
        async fn command(&mut self, command: Command) -> Result<Event> {
            execute_with(&mut self.runtime, self.path.clone(), command).await
        }
        fn state(&self) -> workflow::WorkflowState {
            workflow::Workflow::new(&self.path).snapshot().unwrap()
        }
        async fn start(&mut self, task: &str) -> SessionRecord {
            match self
                .command(Command::Start {
                    ticket: "owner/repo#1".into(),
                    task: task.into(),
                    handoff: "Explicit handoff".into(),
                    rows: 30,
                    cols: 100,
                })
                .await
                .unwrap()
            {
                Event::Started(record) => record,
                _ => panic!("Expected Started"),
            }
        }
    }
    #[tokio::test]
    async fn implementation_handover_review_and_exact_resume_preserve_conversations() {
        let mut f = Fixture::new();
        let implementation = f.start("implementation").await;
        let live = f
            .command(Command::Continue {
                session: implementation.id.clone(),
                rows: 30,
                cols: 100,
            })
            .await
            .unwrap();
        assert!(matches!(live, Event::Started(_)));
        assert_eq!(f.runtime.launches.len(), 1);
        let Event::Started(review) = f
            .command(Command::Handover {
                from_session: implementation.id.clone(),
                ticket: implementation.ticket.clone(),
                task: "review".into(),
                handoff: "Review these changes".into(),
                rows: 30,
                cols: 100,
            })
            .await
            .unwrap()
        else {
            panic!()
        };
        assert_ne!(
            review.provider_session_id,
            implementation.provider_session_id
        );
        assert_eq!(review.checkout, implementation.checkout);
        f.command(Command::Stop {
            session: review.id.clone(),
        })
        .await
        .unwrap();
        f.command(Command::Continue {
            session: implementation.id.clone(),
            rows: 30,
            cols: 100,
        })
        .await
        .unwrap();
        assert_eq!(
            f.runtime.launches[2],
            (implementation.provider_session_id.clone(), true, None)
        );
        let state = f.state();
        assert_eq!(state.sessions.len(), 2);
        assert_eq!(
            state.sessions[&implementation.id].starting_brief,
            implementation.starting_brief
        );
        assert_eq!(state.sessions[&review.id].health, Health::Stopped);
        assert_eq!(
            state.tickets[&implementation.ticket].tasks[0].progress,
            workflow::Progress::InProgress
        );
    }
    #[tokio::test]
    async fn failed_stop_cannot_hand_over_or_replace_waiting_owner() {
        let mut f = Fixture::new();
        let owner = f.start("implementation").await;
        f.runtime.fail_stop = true;
        assert!(
            f.command(Command::Handover {
                from_session: owner.id.clone(),
                ticket: owner.ticket.clone(),
                task: "review".into(),
                handoff: String::new(),
                rows: 30,
                cols: 100
            })
            .await
            .is_err()
        );
        assert_eq!(f.state().sessions[&owner.id], owner);
        assert_eq!(f.runtime.terminals.len(), 1);
        assert_eq!(f.runtime.launches.len(), 1);
        assert!(
            f.command(Command::Start {
                ticket: owner.ticket,
                task: "review".into(),
                handoff: String::new(),
                rows: 30,
                cols: 100
            })
            .await
            .is_err()
        );
        assert_eq!(f.state().sessions.len(), 1);
    }
    #[tokio::test]
    async fn ended_terminal_becomes_stopped_before_resuming_and_never_replays_brief() {
        let mut f = Fixture::new();
        let owner = f.start("implementation").await;
        f.runtime
            .terminals
            .get_mut(owner.host_session.as_ref().unwrap())
            .unwrap()
            .exited = true;
        let Event::Started(resumed) = f
            .command(Command::Continue {
                session: owner.id.clone(),
                rows: 30,
                cols: 100,
            })
            .await
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(resumed.health, Health::Live);
        assert_eq!(resumed.provider_session_id, owner.provider_session_id);
        assert!(f.runtime.launches[1].1);
        assert_eq!(f.runtime.launches[1].2, None);
    }
    #[tokio::test]
    async fn missing_provider_history_requires_explicit_new_session_and_keeps_old_brief() {
        let mut f = Fixture::new();
        let owner = f.start("implementation").await;
        f.command(Command::Stop {
            session: owner.id.clone(),
        })
        .await
        .unwrap();
        std::fs::remove_file(
            f.runtime
                .home
                .join("session-state")
                .join(&owner.provider_session_id)
                .join("events.jsonl"),
        )
        .unwrap();
        let error = f
            .command(Command::Continue {
                session: owner.id.clone(),
                rows: 30,
                cols: 100,
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("missing"));
        assert_eq!(f.runtime.launches.len(), 1);
        let replacement = f.start("implementation").await;
        assert_ne!(owner.provider_session_id, replacement.provider_session_id);
        assert_eq!(
            f.state().sessions[&owner.id].starting_brief,
            owner.starting_brief
        );
        f.command(Command::SelectSession {
            session: owner.id.clone(),
        })
        .await
        .unwrap();
        assert_eq!(
            selected_session(&f.state(), &owner.ticket, &owner.task)
                .unwrap()
                .id,
            owner.id
        );
    }
    #[tokio::test]
    async fn mismatched_provider_identity_and_checkout_do_not_move_or_release_ownership() {
        let mut f = Fixture::new();
        let owner = f.start("implementation").await;
        f.runtime
            .terminals
            .get_mut(owner.host_session.as_ref().unwrap())
            .unwrap()
            .provider_session_id = Some(crate::copilot::new_session_id());
        assert!(
            f.command(Command::Continue {
                session: owner.id.clone(),
                rows: 30,
                cols: 100
            })
            .await
            .is_err()
        );
        assert_eq!(f.state().sessions[&owner.id], owner);
        f.command(Command::Stop {
            session: owner.id.clone(),
        })
        .await
        .unwrap();
        let metadata = f
            .runtime
            .home
            .join("session-state")
            .join(&owner.provider_session_id)
            .join("workspace.yaml");
        std::fs::write(
            metadata,
            format!(
                "id: {}\ncwd: {}\n",
                owner.provider_session_id,
                f._temp.path().display()
            ),
        )
        .unwrap();
        assert!(
            f.command(Command::Continue {
                session: owner.id.clone(),
                rows: 30,
                cols: 100
            })
            .await
            .is_err()
        );
        assert_eq!(f.runtime.launches.len(), 1);
        assert_eq!(f.state().sessions[&owner.id].checkout, owner.checkout);
    }
}
