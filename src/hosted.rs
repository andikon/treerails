//! Durable ticket conversations and the production host command surface.
use crate::workflow::{self, TicketRecord};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    Workflow(workflow::Event),
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

/// Executed inside the authoritative host, never as a second policy in a UI.
pub(crate) async fn execute(
    host: &mut crate::host::runtime::Host,
    config_path: PathBuf,
    command: Command,
) -> Result<Event> {
    use crate::host::runtime::{HostCommand, Response};
    let workflow = workflow::Workflow::new(&config_path);
    match command {
        Command::Workflow { project, command } => {
            Ok(Event::Workflow(workflow.execute(&project, command).await?))
        }
        Command::Snapshot => Ok(Event::State(workflow.snapshot()?)),
        Command::Start {
            ticket,
            task,
            handoff,
            rows,
            cols,
        } => {
            let record = workflow.update(|state| {
                let ticket_record = state.tickets.get(&ticket).context("Unknown ticket")?;
                let brief = starting_brief(ticket_record, &task, &handoff)?;
                let id = crate::copilot::new_session_id();
                let record = SessionRecord {
                    id: id.clone(),
                    ticket: ticket.clone(),
                    task: task.clone(),
                    checkout: ticket_record.worktree.as_ref().unwrap().path.clone(),
                    provider_session_id: id,
                    starting_brief: brief,
                    created_at: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)?
                        .as_secs(),
                    host_session: None,
                    health: Health::Starting,
                    current: true,
                    error: None,
                };
                for previous in state
                    .sessions
                    .values_mut()
                    .filter(|s| s.ticket == ticket && s.task == task)
                {
                    previous.current = false;
                }
                state.sessions.insert(record.id.clone(), record.clone());
                Ok(record)
            })?;
            let result = Box::pin(host.execute(HostCommand::StartCopilot {
                request_id: record.id.clone(),
                checkout: record.checkout.clone(),
                provider_session_id: record.provider_session_id.clone(),
                resume: false,
                brief: Some(record.starting_brief.clone()),
                rows,
                cols,
            }))
            .await;
            finish_start(&workflow, record, result).await
        }
        Command::Continue {
            session,
            rows,
            cols,
        } => {
            let record = workflow
                .snapshot()?
                .sessions
                .get(&session)
                .context("Unknown recorded conversation")?
                .clone();
            if let Some(id) = &record.host_session {
                match Box::pin(host.execute(HostCommand::Snapshot {
                    session: id.clone(),
                }))
                .await
                {
                    Ok(Response::Terminal(snapshot)) if !snapshot.exited => {
                        return Ok(Event::Started(record));
                    }
                    Ok(Response::Terminal(_)) => {
                        Box::pin(host.execute(HostCommand::Stop {
                            session: id.clone(),
                        }))
                        .await?;
                    }
                    Err(error) => {
                        // Unknown host identity is not evidence that managed execution ended.
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
            ensure!(record.checkout.is_dir(), "Recorded checkout is missing");
            let result = Box::pin(host.execute(HostCommand::StartCopilot {
                request_id: crate::copilot::new_session_id(),
                checkout: record.checkout.clone(),
                provider_session_id: record.provider_session_id.clone(),
                resume: true,
                brief: None,
                rows,
                cols,
            }))
            .await;
            finish_start(&workflow, record, result).await
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
            // Any stop error returns before durable health or checkout ownership changes.
            Box::pin(host.execute(HostCommand::Stop {
                session: id.clone(),
            }))
            .await?;
            let record = workflow.update(|state| {
                let record = state
                    .sessions
                    .get_mut(&session)
                    .context("Unknown session")?;
                record.host_session = None;
                record.health = Health::Stopped;
                record.error = None;
                Ok(record.clone())
            })?;
            Ok(Event::Stopped(record))
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
            let Response::Terminal(snapshot) = Box::pin(host.execute(HostCommand::Snapshot {
                session: id.clone(),
            }))
            .await?
            else {
                anyhow::bail!("Terminal unavailable");
            };
            ensure!(
                !snapshot.exited
                    && snapshot.attention.healthy
                    && snapshot.attention.value == Some(crate::copilot::AttentionState::Ready),
                "The agent must be Ready before revised requirements can be sent"
            );
            ensure!(
                snapshot.bracketed_paste,
                "Wait for Copilot's prompt with bracketed paste enabled before sending multiline requirements"
            );
            let mut bytes = b"\x1b[200~".to_vec();
            bytes.extend_from_slice(instructions.as_bytes());
            bytes.extend_from_slice(b"\x1b[201~\r");
            Box::pin(host.execute(HostCommand::Input {
                session: id,
                client,
                token,
                bytes,
            }))
            .await?;
            Ok(Event::RevisionSent)
        }
    }
}

async fn finish_start(
    workflow: &workflow::Workflow,
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
                Ok(record.clone())
            })?;
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
