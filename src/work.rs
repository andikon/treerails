//! Project work is a projection of canonical tasks, never another task store.
use crate::{
    copilot::AttentionState,
    host::runtime::SessionSummary,
    hosted::{Health, SessionRecord},
    workflow::{Checkout, Issue, Progress, WorkflowState},
};
use anyhow::{Result, bail};
use std::path::Path;

#[derive(Clone, Debug)]
pub struct WorkRow {
    pub workspace: String,
    pub task: String,
    pub title: String,
    pub workspace_title: String,
    pub issue: Option<Issue>,
    pub checkout: Option<Checkout>,
    pub progress: Progress,
    pub session: Option<SessionRecord>,
    pub attention: Option<AttentionState>,
    pub active: bool,
    pub status: String,
    pub rank: u8,
}

/// Historical conversations alone do not make a task active. A live Ready
/// conversation still owns its checkout, just like Running or Needs input.
pub fn session_active(session: &SessionRecord) -> bool {
    session.pending_start.is_some() || session.health != Health::Stopped
}

pub fn rows(state: &WorkflowState, project: &Path) -> Vec<WorkRow> {
    rows_with_attention(state, project, &[])
}

pub fn rows_with_attention(
    state: &WorkflowState,
    project: &Path,
    summaries: &[SessionSummary],
) -> Vec<WorkRow> {
    let mut rows = Vec::new();
    for workspace in crate::workspace::project_workspaces(state, project) {
        for task in &workspace.tasks {
            // Prefer an owner or unresolved execution to a newer stopped
            // conversation, so history cannot hide something needing recovery.
            let session = state
                .sessions
                .values()
                .filter(|session| session.ticket == workspace.id && session.task == task.id)
                .max_by_key(|session| {
                    (
                        session_active(session),
                        session_active(session) && session.health != Health::Live,
                        session.current,
                        session.created_at,
                    )
                });
            let active =
                task.progress == Progress::InProgress || session.is_some_and(session_active);
            let mut row = WorkRow {
                workspace: workspace.id.clone(),
                task: task.id.clone(),
                title: task.title.clone(),
                workspace_title: workspace.title.clone(),
                issue: workspace.issue.clone(),
                checkout: workspace.worktree.clone(),
                progress: task.progress,
                session: session.cloned(),
                attention: None,
                active,
                status: match task.progress {
                    Progress::Pending => "Pending",
                    Progress::InProgress => "Task in progress",
                    Progress::Done => "Done",
                }
                .into(),
                rank: if active { 3 } else { 4 },
            };
            if let Some(session) = session.filter(|session| session_active(session)) {
                let summary = summaries
                    .iter()
                    .find(|summary| session.host_session.as_ref() == Some(&summary.session));
                if session.health != Health::Live || session.pending_start.is_some() {
                    row.status = "Recovery needed".into();
                    row.rank = 0;
                } else if let Some(summary) = summary {
                    if summary.exited {
                        row.status = "Execution ended · refresh recovery".into();
                        row.rank = 0;
                    } else if !summary.attention.healthy || summary.attention.value.is_none() {
                        row.status = "Observation unavailable".into();
                        row.rank = 0;
                    } else {
                        row.attention = summary.attention.value;
                        let (status, rank) = match row.attention {
                            Some(AttentionState::NeedsInput) => ("Needs input", 0),
                            Some(AttentionState::Running) => ("Running", 1),
                            Some(AttentionState::Ready) => ("Ready · owns checkout", 2),
                            None => unreachable!(),
                        };
                        row.status = status.into();
                        row.rank = rank;
                    }
                } else {
                    row.status = "Live · observation pending".into();
                    row.rank = 1;
                }
            }
            rows.push(row);
        }
    }
    rows.sort_by(|a, b| {
        a.rank
            .cmp(&b.rank)
            .then(a.workspace.cmp(&b.workspace))
            .then(a.task.cmp(&b.task))
    });
    rows
}

/// Durable recovery state supplements the physical lease. Call immediately
/// before a destructive checkout operation and retain its physical lease for
/// the complete operation; a stopped host alone cannot prove execution ended.
pub fn mutation_guard(state: &WorkflowState, checkout: &Path) -> Result<()> {
    if let Some(owner) = crate::workflow::deletion_owner(state, checkout) {
        bail!(
            "Checkout belongs to agent session {} ({:?}); stop or reconcile it before changing or removing this checkout",
            owner.id,
            owner.health
        );
    }
    Ok(())
}
