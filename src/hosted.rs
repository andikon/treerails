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
    },
}

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
