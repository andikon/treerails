//! A shared working context, independent of the browser used to reach it.
use crate::{
    config, git,
    workflow::{self, Checkout, Issue, Task, WorkflowState},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalWorkRecord {
    pub id: String,
    pub project: PathBuf,
    pub title: String,
    pub tasks: Vec<Task>,
    pub selected_task: Option<String>,
    pub worktree: Checkout,
    pub created_at: u64,
    #[serde(default)]
    pub objectives: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub project: PathBuf,
    pub title: String,
    pub issue: Option<Issue>,
    pub tasks: Vec<Task>,
    pub selected_task: Option<String>,
    pub worktree: Option<Checkout>,
    pub checkout_removed_at: Option<u64>,
}

pub fn resolve(state: &WorkflowState, id: &str) -> Result<Workspace> {
    if let Some(ticket) = state.tickets.get(id) {
        return Ok(Workspace {
            id: id.into(),
            project: ticket.project.clone(),
            title: ticket.issue.title.clone(),
            issue: Some(ticket.issue.clone()),
            tasks: ticket.tasks.clone(),
            selected_task: ticket.selected_task.clone(),
            worktree: ticket.worktree.clone(),
            checkout_removed_at: ticket.checkout_removed_at,
        });
    }
    let record = state.local_work.get(id).context("Unknown workspace")?;
    Ok(Workspace {
        id: id.into(),
        project: record.project.clone(),
        title: record.title.clone(),
        issue: None,
        tasks: record.tasks.clone(),
        selected_task: record.selected_task.clone(),
        worktree: Some(record.worktree.clone()),
        checkout_removed_at: None,
    })
}

pub fn project_workspaces(state: &WorkflowState, project: &Path) -> Vec<Workspace> {
    state
        .tickets
        .keys()
        .chain(state.local_work.keys())
        .filter_map(|id| resolve(state, id).ok())
        .filter(|record| config::same_path(&record.project, project))
        .collect()
}

pub fn for_checkout(state: &WorkflowState, path: &Path) -> Option<Workspace> {
    state
        .tickets
        .keys()
        .chain(state.local_work.keys())
        .filter_map(|id| resolve(state, id).ok())
        .find(|record| {
            record.checkout_removed_at.is_none()
                && record
                    .worktree
                    .as_ref()
                    .is_some_and(|checkout| config::same_path(&checkout.path, path))
        })
}

pub fn select(state: &mut WorkflowState, id: &str, task: &str) -> Result<()> {
    if let Some(ticket) = state.tickets.get_mut(id) {
        ticket.selected_task = Some(task.into());
    } else {
        state
            .local_work
            .get_mut(id)
            .context("Recorded work is missing")?
            .selected_task = Some(task.into());
    }
    Ok(())
}

pub fn context(state: &WorkflowState, id: &str, task: &str, handoff: &str) -> Result<String> {
    let record = resolve(state, id)?;
    if let Some(ticket) = state.tickets.get(id) {
        let mut context = crate::hosted::discussion_context(ticket, task, handoff)?;
        if let Some(objective) = state.task_objectives.get(&format!("{id}/{task}")) {
            context.push_str(&format!("\nOriginal local objective:\n{objective}"));
        }
        return Ok(context);
    }
    let objective = record
        .tasks
        .iter()
        .find(|candidate| candidate.id == task)
        .context("Unknown task")?;
    let checkout = record.worktree.context("Workspace has no checkout")?;
    let text = state
        .local_work
        .get(id)
        .and_then(|local| local.objectives.get(task))
        .unwrap_or(&objective.title);
    Ok(format!(
        "Local workspace: {id}\nTask: {} ({task})\nObjective:\n{text}\nWorktree: {}\nBranch: {}\nExplicit handoff:\n{handoff}",
        objective.title,
        checkout.path.display(),
        checkout.branch
    ))
}

pub async fn create_local(
    workflow: &workflow::Workflow,
    project: &Path,
    path: &Path,
    objective: String,
) -> Result<Workspace> {
    let objective = objective.trim().to_owned();
    ensure!(!objective.is_empty(), "Supply a task objective");
    ensure!(
        objective.len() <= 48 * 1024,
        "Task objective exceeds 48 KiB"
    );
    let _lock = workflow::preparation_lock(project).await?;
    let project = project.canonicalize()?;
    let path = path.canonicalize()?;
    let tree = git::worktrees(&project)
        .await?
        .into_iter()
        .find(|tree| config::same_path(&tree.path, &path))
        .context("Only a registered project checkout can host a task")?;
    ensure!(!tree.prunable, "Checkout needs recovery");
    let before = workflow.snapshot()?;
    let existing = for_checkout(&before, &path);
    ensure!(
        existing.as_ref().is_none_or(|work| work.issue.is_none()),
        "This checkout already belongs to a ticket; add a task in that workspace"
    );
    ensure!(
        existing
            .as_ref()
            .and_then(|work| work.worktree.as_ref())
            .is_none_or(|checkout| checkout.branch == tree.branch),
        "Checkout branch changed; reconcile the existing workspace before adding tasks"
    );
    let id = existing
        .map(|work| work.id)
        .unwrap_or_else(|| format!("local:{}", uuid::Uuid::new_v4()));
    workflow::ensure_available(&before, &id, &path)?;
    let claim = workflow::claim_path(&path).await?;
    if claim.exists() {
        ensure!(
            std::fs::read_to_string(&claim)? == id,
            "Checkout belongs to another workspace or configuration"
        );
    }
    let new_claim = !claim.exists();
    config::atomic_write(&claim, id.as_bytes())?;
    let result = workflow.update(|state| {
        workflow::ensure_available(state, &id, &path)?;
        let local = state
            .local_work
            .entry(id.clone())
            .or_insert_with(|| LocalWorkRecord {
                id: id.clone(),
                project,
                title: objective.lines().next().unwrap_or_default().into(),
                tasks: vec![],
                selected_task: None,
                worktree: Checkout {
                    path,
                    branch: tree.branch,
                    original: tree.original,
                },
                created_at: crate::ticket_browser::now(),
                objectives: BTreeMap::new(),
            });
        let task = format!("local-task:{}", uuid::Uuid::new_v4());
        local.tasks.push(Task {
            id: task.clone(),
            title: objective.lines().next().unwrap_or_default().into(),
            progress: workflow::Progress::Pending,
        });
        local.objectives.insert(task.clone(), objective);
        local.selected_task = Some(task);
        resolve(state, &id)
    });
    if result.is_err() && new_claim {
        let _ = std::fs::remove_file(claim);
    }
    result
}

pub async fn link_local(
    workflow: &workflow::Workflow,
    project: &Path,
    local_id: &str,
    ticket_id: &str,
) -> Result<Workspace> {
    let _lock = workflow::preparation_lock(project).await?;
    let before = workflow.snapshot()?;
    let local = before
        .local_work
        .get(local_id)
        .context("Unknown local workspace")?
        .clone();
    let ticket = before
        .tickets
        .get(ticket_id)
        .context("Open the destination GitHub ticket first")?;
    ensure!(
        config::same_path(project, &local.project) && config::same_path(project, &ticket.project),
        "Workspaces belong to different projects"
    );
    ensure!(
        ticket
            .worktree
            .as_ref()
            .is_none_or(|checkout| checkout == &local.worktree)
            && ticket.checkout_removed_at.is_none(),
        "Ticket already has another checkout or retained cleanup history"
    );
    ensure!(
        workflow::deletion_owner(&before, &local.worktree.path).is_none(),
        "Stop or reconcile managed agents before linking work"
    );
    ensure!(
        !before
            .sessions
            .values()
            .any(|session| session.ticket == ticket_id
                && session.health != crate::hosted::Health::Stopped),
        "Destination ticket has a live or unresolved agent"
    );
    let registered = git::worktrees(project)
        .await?
        .into_iter()
        .find(|tree| config::same_path(&tree.path, &local.worktree.path))
        .context("Local checkout is no longer registered")?;
    ensure!(
        !registered.prunable && registered.branch == local.worktree.branch,
        "Checkout branch or registration changed; reconcile before linking"
    );
    let lease = crate::host::CheckoutLease::acquire(&local.worktree.path).await?;
    let _ = &lease;
    let claim = workflow::claim_path(&local.worktree.path).await?;
    ensure!(
        std::fs::read_to_string(&claim)? == local_id,
        "Physical checkout association changed"
    );
    config::atomic_write(&claim, ticket_id.as_bytes())?;
    let result = workflow.update(|state| {
        ensure!(
            state.local_work.get(local_id) == Some(&local),
            "Local work changed; review linking again"
        );
        ensure!(
            workflow::deletion_owner(state, &local.worktree.path).is_none(),
            "Agent ownership changed; retry after stopping it"
        );
        ensure!(
            !state
                .sessions
                .values()
                .any(|session| session.ticket == ticket_id
                    && session.health != crate::hosted::Health::Stopped),
            "Destination agent ownership changed"
        );
        let target = state
            .tickets
            .get_mut(ticket_id)
            .context("Ticket disappeared")?;
        ensure!(
            target
                .worktree
                .as_ref()
                .is_none_or(|checkout| checkout == &local.worktree),
            "Ticket checkout changed"
        );
        target.worktree = Some(local.worktree.clone());
        for task in &local.tasks {
            ensure!(
                !target.tasks.iter().any(|existing| existing.id == task.id),
                "Task identity conflict"
            );
            target.tasks.push(task.clone());
        }
        target.selected_task = local.selected_task.clone();
        for record in state
            .sessions
            .values_mut()
            .filter(|record| record.ticket == local_id)
        {
            record.ticket = ticket_id.into();
        }
        for (task, objective) in &local.objectives {
            state
                .task_objectives
                .insert(format!("{ticket_id}/{task}"), objective.clone());
        }
        state.local_work.remove(local_id);
        resolve(state, ticket_id)
    });
    if result.is_err() {
        config::atomic_write(&claim, local_id.as_bytes())?;
    }
    result
}
