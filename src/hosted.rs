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
    #[serde(default)]
    pub pending_start: Option<PendingStart>,
    #[serde(default)]
    pub execution_group: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingStart {
    pub request_id: String,
    pub host_instance: String,
    pub resume: bool,
    pub rows: u16,
    pub cols: u16,
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
    StartRequest {
        request_id: String,
        ticket: String,
        task: String,
        handoff: String,
        rows: u16,
        cols: u16,
        #[serde(default)]
        expected_brief: Option<String>,
    },
    Reconcile {
        session: String,
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
        #[serde(default)]
        expected_brief: Option<String>,
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
    fn execution_identity(&self, host: &str, request: &str) -> Result<Option<String>> {
        Ok(Some(crate::host::execution::execution_group_identity(
            host, request,
        )?))
    }
    fn execution_stopped(&self, group: &str) -> Result<bool> {
        crate::host::execution::execution_group_stopped(group)
    }
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
        Command::Snapshot => {
            refresh_health(runtime, &workflow).await?;
            Ok(Event::State(workflow.snapshot()?))
        }
        Command::Reconcile { session } => reconcile(runtime, &workflow, &session).await,
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
            Box::pin(execute_with(
                runtime,
                config_path,
                Command::StartRequest {
                    request_id: crate::copilot::new_session_id(),
                    ticket,
                    task,
                    handoff,
                    rows,
                    cols,
                    expected_brief: None,
                },
            ))
            .await
        }
        Command::StartRequest {
            request_id,
            ticket,
            task,
            handoff,
            rows,
            cols,
            expected_brief,
        } => {
            ensure!(
                crate::copilot::is_uuid(&request_id),
                "Start request requires a stable UUID"
            );
            if let Some(record) = workflow.snapshot()?.sessions.get(&request_id).cloned() {
                ensure!(
                    record.ticket == ticket && record.task == task,
                    "Start request UUID was reused for a different target"
                );
                if let Some(expected) = expected_brief {
                    ensure!(
                        expected == record.starting_brief,
                        "Start retry differs from its original approved brief"
                    );
                }
                if record.health == Health::Stopped {
                    return Ok(Event::Stopped(record));
                }
                return reconcile(runtime, &workflow, &record.id).await;
            }
            let host_instance = instance(runtime).await?;
            let execution_group = runtime.execution_identity(&host_instance, &request_id)?;
            let record = workflow.update(|state| {
                let ticket_record = state.tickets.get(&ticket).context("Unknown ticket")?;
                let brief = starting_brief(ticket_record, &task, &handoff)?;
                if let Some(expected) = &expected_brief { ensure!(expected == &brief, "Issue, task or workspace changed after brief review; inspect it again before starting"); }
                ensure!(brief.len() <= 48 * 1024, "Starting brief exceeds the hosted limit; refine the ticket before starting");
                let checkout = ticket_record.worktree.as_ref().unwrap();
                ensure!(has_owner(state, &ticket, &checkout.path, None).is_none(), "Ticket worktree is owned by another managed conversation; stop it explicitly or use Handover");
                let id = request_id.clone();
                let record = SessionRecord {
                    id: id.clone(), ticket: ticket.clone(), task: task.clone(), checkout: checkout.path.clone(), provider_session_id: id.clone(),
                    starting_brief: brief, created_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs(),
                    host_session: None, health: Health::Starting, current: true, error: None,
                    host_instance: Some(host_instance.clone()), branch: Some(checkout.branch.clone()),
                    pending_start: Some(PendingStart {request_id: id.clone(),host_instance:host_instance.clone(),resume:false,rows,cols}),
                    execution_group: execution_group.clone(),
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
            if workflow
                .snapshot()?
                .sessions
                .get(&session)
                .is_some_and(|record| record.pending_start.is_some())
            {
                let outcome = reconcile(runtime, &workflow, &session).await?;
                if matches!(outcome, Event::Started(_)) {
                    return Ok(outcome);
                }
            }
            let state = workflow.snapshot()?;
            let mut record = state
                .sessions
                .get(&session)
                .context("Unknown recorded conversation")?
                .clone();
            verify_checkout(&state, &record).await?;
            let host_instance = instance(runtime).await?;
            if let Some(id) = record.host_session.clone() {
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
                        return finish_start(
                            &workflow,
                            runtime,
                            record,
                            Ok(Response::Started {
                                session: id.clone(),
                                process_id: snapshot.process_id,
                            }),
                        )
                        .await;
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
            let pending = PendingStart {
                request_id: crate::copilot::new_session_id(),
                host_instance: host_instance.clone(),
                resume: true,
                rows,
                cols,
            };
            let execution_group =
                runtime.execution_identity(&host_instance, &pending.request_id)?;
            record = workflow.update(|state| {
                let record = state
                    .sessions
                    .get_mut(&session)
                    .context("Unknown session")?;
                record.health = Health::Starting;
                record.pending_start = Some(pending.clone());
                record.host_instance = Some(host_instance);
                record.execution_group = execution_group.clone();
                Ok(record.clone())
            })?;
            let result = runtime
                .request(HostCommand::StartCopilot {
                    request_id: pending.request_id,
                    checkout: record.checkout.clone(),
                    provider_session_id: record.provider_session_id.clone(),
                    resume: true,
                    brief: None,
                    rows,
                    cols,
                })
                .await;
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
            expected_brief,
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
            let brief = starting_brief(target, &task, &handoff)?;
            if let Some(expected) = &expected_brief {
                ensure!(
                    expected == &brief,
                    "Issue, task or workspace changed after handover review; inspect it again"
                );
            }
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
                Command::StartRequest {
                    request_id: crate::copilot::new_session_id(),
                    ticket,
                    task,
                    handoff,
                    rows,
                    cols,
                    expected_brief: Some(brief),
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
async fn refresh_health(
    runtime: &mut impl SessionRuntime,
    workflow: &workflow::Workflow,
) -> Result<()> {
    use crate::host::runtime::{HostCommand, Response};
    let active = instance(runtime).await?;
    let snapshot = workflow.snapshot()?;
    for record in snapshot.sessions.values().filter(|record| {
        matches!(
            record.health,
            Health::Starting | Health::Live | Health::Interrupted
        )
    }) {
        if record.host_instance.as_deref() != Some(&active) {
            interrupt(
                workflow,
                &record.id,
                "The previous host instance is gone. Reconcile execution before explicitly resuming.",
            )?;
        } else if record.pending_start.is_some() {
            // Read-only result discovery closes the lost-reply/save window; never resubmits Start.
            let _ = Box::pin(reconcile(runtime, workflow, &record.id)).await;
        } else if let Some(id) = &record.host_session {
            match runtime
                .request(HostCommand::Snapshot {
                    session: id.clone(),
                })
                .await
            {
                Ok(Response::Terminal(terminal))
                    if terminal.provider_session_id.as_deref()
                        == Some(&record.provider_session_id)
                        && !terminal.exited => {}
                Ok(Response::Terminal(terminal)) if terminal.exited => {
                    interrupt(
                        workflow,
                        &record.id,
                        "Managed execution ended; Continue or Reconcile verifies its release before resume.",
                    )?;
                }
                _ => {
                    interrupt(
                        workflow,
                        &record.id,
                        "Recorded terminal identity is unavailable; reconcile it without restarting work.",
                    )?;
                }
            }
        }
    }
    Ok(())
}
fn interrupt(workflow: &workflow::Workflow, session: &str, reason: &str) -> Result<()> {
    workflow.update(|state| {
        let record = state
            .sessions
            .get_mut(session)
            .context("Unknown recorded conversation")?;
        record.health = Health::Interrupted;
        record.error = Some(reason.into());
        Ok(())
    })
}
async fn reconcile(
    runtime: &mut impl SessionRuntime,
    workflow: &workflow::Workflow,
    session: &str,
) -> Result<Event> {
    use crate::host::runtime::{HostCommand, Response};
    let state = workflow.snapshot()?;
    let record = state
        .sessions
        .get(session)
        .context("Unknown recorded conversation")?
        .clone();
    if matches!(record.health, Health::Stopped | Health::Failed) && record.pending_start.is_none() {
        return Ok(Event::State(state));
    }
    verify_checkout(&state, &record).await?;
    let active = instance(runtime).await?;
    if record.host_instance.as_deref() == Some(&active) {
        if let Some(pending) = &record.pending_start {
            ensure!(
                pending.host_instance == active,
                "Pending start belongs to another host instance"
            );
            match runtime
                .request(HostCommand::FindStart {
                    request_id: pending.request_id.clone(),
                })
                .await?
            {
                started @ Response::Started { .. } => {
                    return finish_start(workflow, runtime, record, Ok(started)).await;
                }
                Response::NotStarted { request_id } if request_id == pending.request_id => {
                    // Authoritative host has no accepted operation and no checkout lease remains.
                    let _lease = crate::host::CheckoutLease::acquire(&record.checkout).await?;
                    workflow.update(|state| {let record=state.sessions.get_mut(session).context("Unknown session")?;record.health=Health::Failed;record.host_session=None;record.pending_start=None;record.error=Some("The saved start request was not accepted; choose Start new session explicitly.".into());Ok(())})?;
                    return Ok(Event::State(workflow.snapshot()?));
                }
                Response::Ended { session: id } => {
                    if let Ok(Response::Terminal(terminal)) = runtime
                        .request(HostCommand::Snapshot {
                            session: id.clone(),
                        })
                        .await
                    {
                        ensure!(terminal.exited, "Managed execution is still active");
                        ensure!(
                            matches!(
                                runtime.request(HostCommand::Stop { session: id }).await?,
                                Response::Done
                            ),
                            "Stop could not be verified"
                        );
                    }
                    let _lease = crate::host::CheckoutLease::acquire(&record.checkout).await?;
                    return Ok(Event::Stopped(mark_stopped(workflow, session)?));
                }
                _ => {
                    anyhow::bail!("Start outcome is uncertain; managed ownership remains retained")
                }
            }
        }
        if let Some(id) = &record.host_session {
            match runtime
                .request(HostCommand::Snapshot {
                    session: id.clone(),
                })
                .await
            {
                Ok(Response::Terminal(terminal)) if !terminal.exited => {
                    ensure!(
                        terminal.provider_session_id.as_deref()
                            == Some(&record.provider_session_id),
                        "Provider identity differs; ownership remains retained"
                    );
                    return finish_start(
                        workflow,
                        runtime,
                        record.clone(),
                        Ok(Response::Started {
                            session: id.clone(),
                            process_id: terminal.process_id,
                        }),
                    )
                    .await;
                }
                Ok(Response::Terminal(_)) => {
                    ensure!(
                        matches!(
                            runtime
                                .request(HostCommand::Stop {
                                    session: id.clone()
                                })
                                .await?,
                            Response::Done
                        ),
                        "Stop could not be verified"
                    );
                    return Ok(Event::Stopped(mark_stopped(workflow, session)?));
                }
                Err(_) => {} // A missing terminal is uncertainty, never stop evidence.
                _ => anyhow::bail!("Unexpected reconciliation response"),
            }
        }
    }
    interrupt(
        workflow,
        session,
        "Managed execution is interrupted; verifying its original execution group.",
    )?;
    let group=record.execution_group.as_ref().context("Execution group identity was not recorded. Ownership cannot be safely released; inspect the previous host instead of starting another agent.")?;
    ensure!(
        runtime.execution_stopped(group)?,
        "Original managed execution is still active; checkout ownership is retained"
    );
    let _lease = crate::host::CheckoutLease::acquire(&record.checkout)
        .await
        .context("Another managed owner still holds this checkout")?;
    Ok(Event::Stopped(mark_stopped(workflow, session)?))
}

fn mark_stopped(workflow: &workflow::Workflow, session: &str) -> Result<SessionRecord> {
    workflow.update(|state| {
        let record = state.sessions.get_mut(session).context("Unknown session")?;
        record.host_session = None;
        record.health = Health::Stopped;
        record.error = None;
        record.pending_start = None;
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
            let snapshot = match runtime
                .request(crate::host::runtime::HostCommand::Snapshot {
                    session: session.clone(),
                })
                .await?
            {
                Response::Terminal(snapshot) => snapshot,
                _ => anyhow::bail!(
                    "Started terminal identity is unavailable; reconcile the saved start request"
                ),
            };
            ensure!(
                snapshot.provider_session_id.as_deref() == Some(&record.provider_session_id),
                "Started provider identity differs; reconcile managed execution"
            );
            if let Some(expected) = &record.execution_group {
                ensure!(
                    snapshot.execution_group.as_ref() == Some(expected),
                    "Execution group identity differs from the saved start intent"
                );
            }
            let record = workflow.update(|state| {
                let record = state
                    .sessions
                    .get_mut(&record.id)
                    .context("Starting conversation record disappeared")?;
                record.host_session = Some(session);
                record.pending_start = None;
                record.execution_group = snapshot.execution_group.clone();
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
                record.health = Health::Interrupted;
                record.error = Some(format!("Start outcome requires reconciliation: {error}"));
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
        fail_setup: bool,
        lose_reply: bool,
        execution_free: bool,
        starts: BTreeMap<String, String>,
    }
    impl SessionRuntime for Runtime {
        fn execution_stopped(&self, _: &str) -> Result<bool> {
            Ok(self.execution_free)
        }
        fn execution_identity(&self, host: &str, request: &str) -> Result<Option<String>> {
            Ok(Some(format!("controlled-{host}-{request}")))
        }
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
                        request_id,
                        checkout,
                        provider_session_id,
                        resume,
                        brief,
                        ..
                    } => {
                        let id = format!("terminal-{}", self.launches.len());
                        self.starts.insert(request_id.clone(), id.clone());
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
                                execution_group: self
                                    .execution_identity(&self.instance, &request_id)?,
                            },
                        );
                        if self.lose_reply {
                            self.lose_reply = false;
                            anyhow::bail!("Injected lost successful Start reply");
                        }
                        Ok(Response::Started {
                            session: id,
                            process_id: Some(42),
                        })
                    }
                    HostCommand::FindStart { request_id } => {
                        if let Some(id) = self.starts.get(&request_id) {
                            if self.terminals.contains_key(id) {
                                Ok(Response::Started {
                                    session: id.clone(),
                                    process_id: Some(42),
                                })
                            } else {
                                Ok(Response::Ended {
                                    session: id.clone(),
                                })
                            }
                        } else {
                            Ok(Response::NotStarted { request_id })
                        }
                    }
                    HostCommand::ConfigureSession { .. } => {
                        ensure!(!self.fail_setup, "Injected notification setup failure");
                        Ok(Response::Done)
                    }
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
                        serde_json::from_value(serde_json::json!({
                            "repository":"owner/repo","project":checkout,"issue":{"number":1,"title":"Deliver feature","body":"Acceptance criteria remain issue requirements","state":"OPEN","url":"https://github.com/owner/repo/issues/1"},
                            "fetched_at":1,"tasks":[{"id":"implementation","title":"Implement","progress":"in_progress"},{"id":"review","title":"Review","progress":"pending"}],
                            "next_task_id":2,"selected_task":"implementation","worktree":{"path":checkout,"branch":"feature","original":false}
                        })).unwrap(),
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
                fail_setup: false,
                lose_reply: false,
                execution_free: false,
                starts: BTreeMap::new(),
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
                expected_brief: None,
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
                expected_brief: None,
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
    #[tokio::test]
    async fn successful_start_reply_loss_discovers_exact_operation_without_another_launch() {
        let mut f = Fixture::new();
        let request_id = crate::copilot::new_session_id();
        let brief = starting_brief(
            &f.state().tickets["owner/repo#1"],
            "implementation",
            "handoff",
        )
        .unwrap();
        let request = Command::StartRequest {
            request_id: request_id.clone(),
            ticket: "owner/repo#1".into(),
            task: "implementation".into(),
            handoff: "handoff".into(),
            rows: 30,
            cols: 100,
            expected_brief: Some(brief.clone()),
        };
        f.runtime.lose_reply = true;
        assert!(f.command(request.clone()).await.is_err());
        let interrupted = f.state().sessions[&request_id].clone();
        assert_eq!(interrupted.health, Health::Interrupted);
        assert!(interrupted.pending_start.is_some());
        assert!(interrupted.execution_group.is_some());
        let Event::Started(recovered) = f.command(request.clone()).await.unwrap() else {
            panic!()
        };
        assert_eq!(recovered.id, request_id);
        assert_eq!(recovered.starting_brief, brief);
        assert!(recovered.pending_start.is_none());
        assert_eq!(f.runtime.launches.len(), 1);
        f.command(request).await.unwrap();
        assert_eq!(f.runtime.launches.len(), 1);
        assert!(
            f.command(Command::StartRequest {
                request_id,
                ticket: "owner/repo#1".into(),
                task: "review".into(),
                handoff: "handoff".into(),
                rows: 30,
                cols: 100,
                expected_brief: Some(brief)
            })
            .await
            .is_err()
        );
    }
    #[tokio::test]
    async fn approved_brief_drift_is_rejected_before_start_or_stopping_an_owner() {
        let mut f = Fixture::new();
        let brief = starting_brief(
            &f.state().tickets["owner/repo#1"],
            "implementation",
            "handoff",
        )
        .unwrap();
        workflow::Workflow::new(&f.path)
            .update(|state| {
                state.tickets.get_mut("owner/repo#1").unwrap().issue.body =
                    "Another client changed requirements".into();
                Ok(())
            })
            .unwrap();
        assert!(
            f.command(Command::StartRequest {
                request_id: crate::copilot::new_session_id(),
                ticket: "owner/repo#1".into(),
                task: "implementation".into(),
                handoff: "handoff".into(),
                rows: 30,
                cols: 100,
                expected_brief: Some(brief)
            })
            .await
            .is_err()
        );
        assert!(f.runtime.launches.is_empty());
        assert!(f.state().sessions.is_empty());
        let owner = f.start("implementation").await;
        assert!(
            f.command(Command::Handover {
                expected_brief: Some("Stale approved review brief".into()),
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
    }
    #[tokio::test]
    async fn changed_host_stays_interrupted_until_execution_barrier_and_explicit_resume() {
        let mut f = Fixture::new();
        let owner = f.start("implementation").await;
        f.runtime.instance = "host-two".into();
        f.runtime.terminals.clear();
        f.command(Command::Snapshot).await.unwrap();
        assert_eq!(f.state().sessions[&owner.id].health, Health::Interrupted);
        assert_eq!(f.runtime.launches.len(), 1);
        assert!(
            f.command(Command::Reconcile {
                session: owner.id.clone()
            })
            .await
            .is_err()
        );
        assert!(
            f.command(Command::Start {
                ticket: owner.ticket.clone(),
                task: "review".into(),
                handoff: String::new(),
                rows: 30,
                cols: 100
            })
            .await
            .is_err()
        );
        f.runtime.execution_free = true;
        assert!(matches!(
            f.command(Command::Reconcile {
                session: owner.id.clone()
            })
            .await
            .unwrap(),
            Event::Stopped(_)
        ));
        assert_eq!(f.runtime.launches.len(), 1);
        f.command(Command::Continue {
            session: owner.id.clone(),
            rows: 30,
            cols: 100,
        })
        .await
        .unwrap();
        assert_eq!(f.runtime.launches.len(), 2);
        assert_eq!(
            f.runtime.launches[1],
            (owner.provider_session_id, true, None)
        );
        assert_eq!(
            f.state().sessions[&owner.id].starting_brief,
            owner.starting_brief
        );
    }
    #[tokio::test]
    async fn missing_execution_identity_is_never_replaced_by_a_free_lease_or_stale_pid() {
        let mut f = Fixture::new();
        let owner = f.start("implementation").await;
        f.runtime.instance = "host-two".into();
        f.runtime.terminals.clear();
        f.runtime.execution_free = true;
        workflow::Workflow::new(&f.path)
            .update(|state| {
                state.sessions.get_mut(&owner.id).unwrap().execution_group = None;
                Ok(())
            })
            .unwrap();
        let error = f
            .command(Command::Reconcile {
                session: owner.id.clone(),
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("identity was not recorded"));
        assert_eq!(f.state().sessions[&owner.id].health, Health::Interrupted);
        assert_eq!(f.runtime.launches.len(), 1);
    }
    #[tokio::test]
    async fn an_authoritatively_unaccepted_start_can_be_abandoned_without_starting_it() {
        let mut f = Fixture::new();
        let owner = f.start("implementation").await;
        f.command(Command::Stop {
            session: owner.id.clone(),
        })
        .await
        .unwrap();
        let unknown = crate::copilot::new_session_id();
        workflow::Workflow::new(&f.path)
            .update(|state| {
                let record = state.sessions.get_mut(&owner.id).unwrap();
                record.health = Health::Starting;
                record.pending_start = Some(PendingStart {
                    request_id: unknown,
                    host_instance: "host-one".into(),
                    resume: true,
                    rows: 30,
                    cols: 100,
                });
                Ok(())
            })
            .unwrap();
        f.command(Command::Reconcile {
            session: owner.id.clone(),
        })
        .await
        .unwrap();
        let current = f.state().sessions[&owner.id].clone();
        assert_eq!(current.health, Health::Failed);
        assert!(current.pending_start.is_none());
        assert_eq!(f.runtime.launches.len(), 1);
    }
    #[tokio::test]
    async fn notification_setup_failure_keeps_the_running_conversation_reconnectable() {
        let mut f = Fixture::new();
        f.runtime.fail_setup = true;
        let owner = f.start("implementation").await;
        assert_eq!(owner.health, Health::Live);
        assert!(owner.error.is_some());
        f.runtime.fail_setup = false;
        let Event::Started(reconnected) = f
            .command(Command::Continue {
                session: owner.id,
                rows: 30,
                cols: 100,
            })
            .await
            .unwrap()
        else {
            panic!()
        };
        assert!(reconnected.error.is_none());
        assert_eq!(f.runtime.launches.len(), 1);
    }
}
