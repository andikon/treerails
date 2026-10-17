//! Ticket preparation policy shared by terminal clients and the session host.
use crate::{
    config::{self, Config},
    git,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WorkflowState {
    pub tickets: BTreeMap<String, TicketRecord>,
    pub sessions: BTreeMap<String, crate::hosted::SessionRecord>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub state: String,
    pub url: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TicketRecord {
    pub repository: String,
    pub project: PathBuf,
    pub issue: Issue,
    pub fetched_at: u64,
    #[serde(default)]
    pub prepared_issue: Option<Issue>,
    #[serde(default)]
    pub reviewed_issue: Option<Issue>,
    #[serde(default)]
    pub github_error: Option<String>,
    #[serde(default)]
    pub last_attempt_at: Option<u64>,
    #[serde(default)]
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub next_task_id: u64,
    pub selected_task: Option<String>,
    pub worktree: Option<Checkout>,
    #[serde(default)]
    pub checkout_removed_at: Option<u64>,
}
impl TicketRecord {
    pub fn is_closed(&self) -> bool {
        self.issue.state.eq_ignore_ascii_case("closed")
    }
    pub fn id(&self) -> String {
        format!("{}#{}", self.repository, self.issue.number)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub progress: Progress,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Progress {
    #[default]
    Pending,
    InProgress,
    Done,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Checkout {
    pub path: PathBuf,
    pub branch: String,
    pub original: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Preparation {
    Create {
        branch: String,
        base: String,
        destination: PathBuf,
        fetch: bool,
    },
    Adopt {
        path: PathBuf,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    List,
    Open {
        number: u64,
    },
    Reopen {
        ticket: String,
    },
    Refresh {
        ticket: String,
    },
    InspectUpdate {
        ticket: String,
    },
    ApplyTaskMerge {
        ticket: String,
        expected_issue: Issue,
    },
    ReviseInstructions {
        ticket: String,
        task: String,
        expected_issue: Issue,
    },
    ImportTasks {
        ticket: String,
        titles: Vec<String>,
    },
    SelectTask {
        ticket: String,
        task: String,
    },
    SetProgress {
        ticket: String,
        task: String,
        progress: Progress,
    },
    PreviewCleanup {
        ticket: String,
    },
    Cleanup {
        ticket: String,
        expected_checkout: Checkout,
    },
    Prepare {
        ticket: String,
        preparation: Preparation,
    },
}
// Snapshots intentionally travel by value across the workflow boundary.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    Issues(Vec<Issue>),
    Ticket(TicketRecord),
    CachedIssues { issues: Vec<Issue>, warning: String },
    Requirements(RequirementsReview),
    RevisionRequested(RevisionRequest),
    CleanupPreview(CleanupPreview),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequirementsReview {
    pub ticket: String,
    pub previous: Issue,
    pub current: Issue,
    pub retained_tasks: Vec<Task>,
    pub additions: Vec<String>,
    pub fetched_at: u64,
}
impl RequirementsReview {
    pub fn instructions(&self) -> String {
        format!(
            "Revised requirements for {}\n{}\n\n{}",
            self.current.url, self.current.title, self.current.body
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevisionRequest {
    pub ticket: String,
    pub task: String,
    pub expected_issue: Issue,
    pub instructions: String,
}
impl TicketRecord {
    pub fn issue_updated(&self) -> bool {
        self.reviewed_issue
            .as_ref()
            .or(self.prepared_issue.as_ref())
            .is_some_and(|old| old.title != self.issue.title || old.body != self.issue.body)
    }
    pub fn requirements_review(&self) -> RequirementsReview {
        RequirementsReview {
            ticket: self.id(),
            previous: self
                .reviewed_issue
                .as_ref()
                .or(self.prepared_issue.as_ref())
                .unwrap_or(&self.issue)
                .clone(),
            current: self.issue.clone(),
            retained_tasks: self.tasks.clone(),
            additions: import_candidates(&self.issue.body)
                .into_iter()
                .filter(|title| !self.tasks.iter().any(|task| &task.title == title))
                .collect(),
            fetched_at: self.fetched_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupPreview {
    pub ticket: String,
    pub checkout: Checkout,
    pub owner: Option<crate::hosted::SessionRecord>,
    pub missing: bool,
    pub original: bool,
    pub locked: bool,
    pub dirty: bool,
    pub unpublished_count: u64,
    pub unpublished_commits: Vec<String>,
    pub ownership_warning: Option<String>,
    pub already_removed: bool,
}
impl CleanupPreview {
    pub fn can_remove(&self) -> bool {
        !self.missing
            && !self.original
            && !self.locked
            && !self.dirty
            && !self.already_removed
            && self.owner.is_none()
            && self.ownership_warning.is_none()
    }
}
/// A failed or interrupted stop is deliberately conservative until reconciled.
/// The OS lease additionally protects against owners in other config contexts.
pub fn deletion_owner<'a>(
    state: &'a WorkflowState,
    checkout: &Path,
) -> Option<&'a crate::hosted::SessionRecord> {
    state
        .sessions
        .values()
        .filter(|session| {
            config::same_path(&session.checkout, checkout)
                && session.health != crate::hosted::Health::Stopped
        })
        .max_by_key(|session| session.created_at)
}

/// Inject GitHub transport without substituting workflow policy in tests.
#[allow(async_fn_in_trait)]
pub trait IssueSource {
    async fn repository(&self, project: &Path) -> Result<String>;
    async fn list(&self, project: &Path) -> Result<Vec<Issue>>;
    async fn get(&self, project: &Path, number: u64) -> Result<Issue>;
}
pub struct Github;
impl Github {
    async fn json<T: serde::de::DeserializeOwned>(project: &Path, args: &[&str]) -> Result<T> {
        let mut command = tokio::process::Command::new("gh");
        command
            .args(args)
            .current_dir(project)
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);
        let output = tokio::time::timeout(std::time::Duration::from_secs(20), command.output())
            .await
            .context("GitHub request timed out after 20 seconds")?
            .context("Run GitHub CLI")?;
        ensure!(
            output.status.success(),
            "GitHub: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        serde_json::from_slice(&output.stdout).context("Read GitHub response")
    }
}
impl IssueSource for Github {
    async fn repository(&self, project: &Path) -> Result<String> {
        #[derive(Deserialize)]
        struct Repo {
            #[serde(rename = "nameWithOwner")]
            name: String,
        }
        Ok(
            Self::json::<Repo>(project, &["repo", "view", "--json", "nameWithOwner"])
                .await?
                .name,
        )
    }
    async fn list(&self, project: &Path) -> Result<Vec<Issue>> {
        Self::json(
            project,
            &[
                "issue",
                "list",
                "--state",
                "all",
                "--limit",
                "100",
                "--json",
                "number,title,body,state,url",
            ],
        )
        .await
    }
    async fn get(&self, project: &Path, number: u64) -> Result<Issue> {
        Self::json(
            project,
            &[
                "issue",
                "view",
                &number.to_string(),
                "--json",
                "number,title,body,state,url",
            ],
        )
        .await
    }
}
#[derive(Debug, Clone)]
pub struct Workflow {
    pub config_path: PathBuf,
}
impl Workflow {
    pub fn new(config_path: impl Into<PathBuf>) -> Self {
        Self {
            config_path: config_path.into(),
        }
    }
    pub async fn execute(&self, project: &Path, command: Command) -> Result<Event> {
        self.execute_with(project, command, &Github).await
    }
    pub fn snapshot(&self) -> Result<WorkflowState> {
        Ok(Config::load(&self.config_path)?.state.workflow)
    }
    pub async fn execute_with(
        &self,
        project: &Path,
        command: Command,
        source: &impl IssueSource,
    ) -> Result<Event> {
        match command {
            Command::List => match source.list(project).await {
                Ok(issues) => Ok(Event::Issues(issues)),
                Err(error) => {
                    let issues: Vec<_> = self
                        .snapshot()?
                        .tickets
                        .values()
                        .filter(|record| config::same_path(project, &record.project))
                        .map(|record| record.issue.clone())
                        .collect();
                    if issues.is_empty() {
                        return Err(error).context(
                            "GitHub unavailable; no known tickets are cached for this project",
                        );
                    }
                    Ok(Event::CachedIssues {
                        issues,
                        warning: format!(
                            "GitHub unavailable: {error:#}. Showing known cached tickets; issue state may be stale."
                        ),
                    })
                }
            },
            Command::Open { number } => {
                if let Some(record) = self.snapshot()?.tickets.values().find(|record| {
                    record.issue.number == number && config::same_path(project, &record.project)
                }) {
                    return self.refresh(project, &record.id(), source).await;
                }
                let repository = source.repository(project).await?;
                let issue = source.get(project, number).await?;
                ensure!(
                    issue.number == number && number > 0,
                    "GitHub returned another issue"
                );
                let project = std::fs::canonicalize(project)?;
                self.update(|state| {
                    let key = format!("{repository}#{number}");
                    let record = state.tickets.entry(key).or_insert_with(|| TicketRecord {
                        repository,
                        project,
                        issue: issue.clone(),
                        fetched_at: now(),
                        prepared_issue: Some(issue.clone()),
                        reviewed_issue: Some(issue.clone()),
                        github_error: None,
                        last_attempt_at: Some(now()),
                        tasks: vec![],
                        next_task_id: 1,
                        selected_task: None,
                        worktree: None,
                        checkout_removed_at: None,
                    });
                    record.issue = issue;
                    record.fetched_at = now();
                    if record.tasks.is_empty() {
                        add_task(record, record.issue.title.clone());
                    }
                    Ok(record.clone())
                })
                .map(Event::Ticket)
            }
            Command::Reopen { ticket } => Ok(Event::Ticket(self.record(project, &ticket)?)),
            Command::Refresh { ticket } => self.refresh(project, &ticket, source).await,
            Command::InspectUpdate { ticket } => Ok(Event::Requirements(
                self.record(project, &ticket)?.requirements_review(),
            )),
            Command::ApplyTaskMerge {
                ticket,
                expected_issue,
            } => self.change(project, &ticket, |record| {
                ensure!(
                    record.issue == expected_issue,
                    "Issue changed after preview; inspect the update again"
                );
                for title in record.requirements_review().additions {
                    add_task(record, title);
                }
                record.reviewed_issue = Some(record.issue.clone());
                Ok(())
            }),
            Command::ReviseInstructions {
                ticket,
                task,
                expected_issue,
            } => {
                let record = self.record(project, &ticket)?;
                ensure!(
                    record.issue == expected_issue,
                    "Issue changed after preview; inspect the update again"
                );
                ensure!(
                    record.selected_task.as_ref() == Some(&task)
                        && record.tasks.iter().any(|candidate| candidate.id == task),
                    "Select the task again before sending revised requirements"
                );
                Ok(Event::RevisionRequested(RevisionRequest {
                    ticket,
                    task,
                    expected_issue,
                    instructions: record.requirements_review().instructions(),
                }))
            }
            Command::ImportTasks { ticket, titles } => {
                ensure!(!titles.is_empty(), "Select at least one task to import");
                self.change(project, &ticket, |record| {
                    for title in titles {
                        ensure!(!title.trim().is_empty(), "Task title cannot be empty");
                        if !record.tasks.iter().any(|task| task.title == title.trim()) {
                            add_task(record, title.trim().into());
                        }
                    }
                    Ok(())
                })
            }
            Command::SelectTask { ticket, task } => self.change(project, &ticket, |record| {
                ensure!(record.tasks.iter().any(|t| t.id == task), "Unknown task");
                record.selected_task = Some(task);
                Ok(())
            }),
            Command::SetProgress {
                ticket,
                task,
                progress,
            } => self.change(project, &ticket, |record| {
                record
                    .tasks
                    .iter_mut()
                    .find(|t| t.id == task)
                    .context("Unknown task")?
                    .progress = progress;
                Ok(())
            }),
            Command::PreviewCleanup { ticket } => Ok(Event::CleanupPreview(
                self.cleanup_preview(project, &ticket).await?,
            )),
            Command::Cleanup {
                ticket,
                expected_checkout,
            } => {
                let _preparation_lock = preparation_lock(project).await?;
                let state = self.snapshot()?;
                let record = self.record(project, &ticket)?;
                ensure!(
                    record.worktree.as_ref() == Some(&expected_checkout),
                    "Ticket checkout changed after preview; inspect cleanup again"
                );
                ensure!(
                    record.checkout_removed_at.is_none(),
                    "This ticket checkout was already removed"
                );
                ensure!(
                    deletion_owner(&state, &expected_checkout.path).is_none(),
                    "A recorded managed session has not been verified stopped; stop or reconcile it before cleanup"
                );
                let lease = crate::host::CheckoutLease::acquire(&expected_checkout.path).await?;
                git::remove_worktree_leased(project, &expected_checkout.path, &lease).await?;
                self.change(project, &ticket, |record| {
                    ensure!(record.worktree.as_ref() == Some(&expected_checkout), "Checkout removed but ticket association changed concurrently; inspect its history");
                    record.checkout_removed_at = Some(now());
                    Ok(())
                })
            }
            Command::Prepare {
                ticket,
                preparation,
            } => {
                let _preparation_lock = preparation_lock(project).await?;
                let record = self.record(project, &ticket)?;
                ensure!(
                    record.checkout_removed_at.is_none(),
                    "Ticket checkout was removed; its retained history cannot be automatically recreated"
                );
                if let Some(checkout) = record.worktree {
                    ensure!(
                        git::worktrees(project)
                            .await?
                            .iter()
                            .any(|wt| config::same_path(&wt.path, &checkout.path)),
                        "Associated worktree is missing; recover it explicitly"
                    );
                    verify_claim(&checkout.path, &ticket).await?;
                    return Ok(Event::Ticket(self.record(project, &ticket)?));
                }
                let path = match preparation {
                    Preparation::Adopt { path } => path,
                    Preparation::Create {
                        branch,
                        base,
                        destination,
                        fetch,
                    } => {
                        ensure!(
                            !branch.trim().is_empty() && !base.trim().is_empty(),
                            "Branch and explicit starting point are required"
                        );
                        let trees = git::worktrees(project).await?;
                        if let Some(wt) = trees.iter().find(|wt| wt.branch == branch) {
                            self.ensure_available(&ticket, &wt.path)?;
                            verify_claim(&wt.path, &ticket).await?;
                        }
                        let existing = git::branches(project)
                            .await?
                            .iter()
                            .any(|b| b.name == branch && !b.remote);
                        git::create_worktree(project, &branch, &base, &destination, existing, fetch)
                            .await?
                    }
                };
                let canonical =
                    std::fs::canonicalize(&path).context("Resolve worktree directory")?;
                let wt = git::worktrees(project)
                    .await?
                    .into_iter()
                    .find(|wt| config::same_path(&wt.path, &canonical))
                    .context("Only a registered worktree of this project can be adopted")?;
                let claim_path = verify_claim(&canonical, &ticket).await?;
                let new_claim = !claim_path.exists();
                config::atomic_write(&claim_path, ticket.as_bytes())?;
                let result = self
                    .update(|state| {
                        ensure_available(state, &ticket, &canonical)?;
                        let record = state.tickets.get_mut(&ticket).context("Unknown ticket")?;
                        ensure!(
                            record.worktree.is_none(),
                            "Ticket workspace changed concurrently; reopen it"
                        );
                        record.worktree = Some(Checkout {
                            path: canonical,
                            branch: wt.branch,
                            original: wt.original,
                        });
                        Ok(record.clone())
                    })
                    .map(Event::Ticket);
                if result.is_err() && new_claim {
                    let _ = std::fs::remove_file(claim_path);
                }
                result
            }
        }
    }
    async fn refresh(
        &self,
        project: &Path,
        ticket: &str,
        source: &impl IssueSource,
    ) -> Result<Event> {
        let before = self.record(project, ticket)?;
        // Local attachment never depends on GitHub availability; failures update
        // only cache freshness/availability and leave provider state untouched.
        let result = source.get(project, before.issue.number).await;
        self.change(project, ticket, |record| {
            if record.prepared_issue.is_none() {
                record.prepared_issue = Some(record.issue.clone());
            }
            if record.reviewed_issue.is_none() {
                record.reviewed_issue = Some(record.issue.clone());
            }
            record.last_attempt_at = Some(now());
            match result {
                Ok(issue) => {
                    ensure!(
                        issue.number == record.issue.number && issue.url == record.issue.url,
                        "GitHub issue identity changed unexpectedly"
                    );
                    record.issue = issue;
                    record.fetched_at = now();
                    record.github_error = None;
                }
                Err(error) => {
                    record.github_error = Some(format!(
                        "GitHub unavailable: {error:#}. Cached issue state may be stale."
                    ))
                }
            }
            Ok(())
        })
    }
    pub async fn cleanup_preview(&self, project: &Path, ticket: &str) -> Result<CleanupPreview> {
        let record = self.record(project, ticket)?;
        let checkout = record
            .worktree
            .context("This ticket has no associated checkout")?;
        let state = self.snapshot()?;
        let owner = deletion_owner(&state, &checkout.path).cloned();
        let trees = git::worktrees(project).await?;
        let registered = trees
            .iter()
            .find(|tree| config::same_path(&tree.path, &checkout.path));
        let missing = !checkout.path.is_dir()
            || registered.is_none()
            || registered.is_some_and(|tree| tree.prunable);
        let original = checkout.original || registered.is_some_and(|tree| tree.original);
        let locked = registered.is_some_and(|tree| tree.locked);
        let mut dirty = false;
        let mut unpublished_count = 0;
        let mut unpublished_commits = vec![];
        let mut ownership_warning = None;
        if !missing {
            dirty = !git::status(&checkout.path).await?.files.is_empty();
            unpublished_count = git::text(
                &checkout.path,
                &["rev-list", "--count", "HEAD", "--not", "--remotes"],
            )
            .await?
            .trim()
            .parse()
            .context("Read unpublished commit count")?;
            unpublished_commits = git::text(
                &checkout.path,
                &[
                    "log",
                    "-n",
                    "100",
                    "--format=%h %s",
                    "HEAD",
                    "--not",
                    "--remotes",
                ],
            )
            .await?
            .lines()
            .map(str::to_owned)
            .collect();
            if let Err(error) = crate::host::CheckoutLease::acquire(&checkout.path).await {
                ownership_warning = Some(format!("{error:#}"));
            }
        }
        Ok(CleanupPreview {
            ticket: ticket.into(),
            checkout,
            owner,
            missing,
            original,
            locked,
            dirty,
            unpublished_count,
            unpublished_commits,
            ownership_warning,
            already_removed: record.checkout_removed_at.is_some(),
        })
    }
    fn record(&self, project: &Path, ticket: &str) -> Result<TicketRecord> {
        let state = self.snapshot()?;
        let record = state
            .tickets
            .get(ticket)
            .context("Unknown ticket; open its GitHub issue first")?;
        ensure!(
            config::same_path(project, &record.project),
            "Ticket belongs to another project"
        );
        Ok(record.clone())
    }
    fn ensure_available(&self, ticket: &str, path: &Path) -> Result<()> {
        ensure_available(&self.snapshot()?, ticket, path)
    }
    fn change(
        &self,
        project: &Path,
        ticket: &str,
        change: impl FnOnce(&mut TicketRecord) -> Result<()>,
    ) -> Result<Event> {
        self.update(|state| {
            let record = state.tickets.get_mut(ticket).context("Unknown ticket")?;
            ensure!(
                config::same_path(project, &record.project),
                "Ticket belongs to another project"
            );
            change(record)?;
            Ok(record.clone())
        })
        .map(Event::Ticket)
    }
    /// Mutations load the latest state under the shared configuration lock. They never
    /// write a stale UI snapshot; unrelated fields and comments remain in place.
    pub fn update<T>(&self, change: impl FnOnce(&mut WorkflowState) -> Result<T>) -> Result<T> {
        config::edit_document(&self.config_path, |doc| {
            let mut state = Config::parse(&doc.to_string(), &self.config_path)?
                .state
                .workflow;
            let before: toml_edit::DocumentMut = toml::to_string(&state)?.parse()?;
            let result = change(&mut state)?;
            let rendered: toml_edit::DocumentMut = toml::to_string(&state)?.parse()?;
            if doc.get("state").is_none() {
                doc["state"] = toml_edit::table();
            }
            if doc["state"].get("workflow").is_none() {
                doc["state"]["workflow"] = toml_edit::table();
            }
            patch(
                &mut doc["state"]["workflow"],
                &toml_edit::Item::Table(before.as_table().clone()),
                &toml_edit::Item::Table(rendered.as_table().clone()),
            );
            Ok(result)
        })
    }
}
fn patch(target: &mut toml_edit::Item, before: &toml_edit::Item, incoming: &toml_edit::Item) {
    if let (Some(dst), Some(src)) = (
        target.as_array_of_tables_mut(),
        incoming.as_array_of_tables(),
    ) {
        let old = before.as_array_of_tables();
        for table in src.iter() {
            let id = table.get("id").and_then(toml_edit::Item::as_str);
            if let Some(existing) = dst
                .iter_mut()
                .find(|t| t.get("id").and_then(toml_edit::Item::as_str) == id)
            {
                let previous = old
                    .and_then(|tables| {
                        tables
                            .iter()
                            .find(|t| t.get("id").and_then(toml_edit::Item::as_str) == id)
                    })
                    .cloned()
                    .unwrap_or_default();
                let mut item = toml_edit::Item::Table(existing.clone());
                patch(
                    &mut item,
                    &toml_edit::Item::Table(previous),
                    &toml_edit::Item::Table(table.clone()),
                );
                *existing = item.as_table().unwrap().clone();
            } else {
                let mut appended = table.clone();
                let last = dst
                    .iter()
                    .filter_map(toml_edit::Table::position)
                    .max()
                    .unwrap_or(0);
                appended.set_position(last + 1);
                dst.push(appended);
            }
        }
    } else if let (Some(dst), Some(src)) = (target.as_table_like_mut(), incoming.as_table_like()) {
        if let Some(previous) = before.as_table_like() {
            for (key, _) in previous.iter() {
                if !src.contains_key(key) {
                    dst.remove(key);
                }
            }
        }
        for (key, value) in src.iter() {
            if let Some(existing) = dst.get_mut(key) {
                patch(
                    existing,
                    before.get(key).unwrap_or(&toml_edit::Item::None),
                    value,
                );
            } else {
                dst.insert(key, value.clone());
            }
        }
    } else if before.to_string() != incoming.to_string() {
        let decor = target.as_value().map(|v| v.decor().clone());
        *target = incoming.clone();
        if let (Some(decor), Some(value)) = (decor, target.as_value_mut()) {
            *value.decor_mut() = decor;
        }
    }
}
fn ensure_available(state: &WorkflowState, ticket: &str, path: &Path) -> Result<()> {
    for (id, record) in &state.tickets {
        if id != ticket && !record.issue.state.eq_ignore_ascii_case("closed") {
            ensure!(
                !record
                    .worktree
                    .as_ref()
                    .is_some_and(|wt| config::same_path(path, &wt.path)),
                "Worktree already belongs to active ticket {id}"
            );
        }
    }
    Ok(())
}
fn add_task(record: &mut TicketRecord, title: String) {
    let id = format!("task-{}", record.next_task_id.max(1));
    record.next_task_id = record.next_task_id.max(1) + 1;
    if record.selected_task.is_none() {
        record.selected_task = Some(id.clone());
    }
    record.tasks.push(Task {
        id,
        title,
        progress: Progress::Pending,
    });
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
/// Only unchecked/checked Markdown items below a heading named Tasks are import
/// candidates. Acceptance criteria and prose never become tasks implicitly.
pub fn import_candidates(body: &str) -> Vec<String> {
    let mut in_tasks = false;
    let mut level = 0;
    let mut result = vec![];
    let mut fence = None;
    for line in body.lines() {
        let line = line.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            let marker = &line[..3];
            if fence == Some(marker) {
                fence = None;
            } else if fence.is_none() {
                fence = Some(marker);
            }
            continue;
        }
        if fence.is_some() {
            continue;
        }
        if line.starts_with('#') {
            let current = line.chars().take_while(|c| *c == '#').count();
            let title = line[current..].trim().trim_end_matches('#').trim();
            if title.eq_ignore_ascii_case("tasks") {
                in_tasks = true;
                level = current;
            } else if current <= level {
                in_tasks = false;
            }
        } else if in_tasks {
            for marker in ["- [ ] ", "- [x] ", "- [X] ", "* [ ] ", "* [x] ", "* [X] "] {
                if let Some(title) = line.strip_prefix(marker).filter(|s| !s.trim().is_empty())
                    && !result.iter().any(|s| s == title.trim())
                {
                    result.push(title.trim().to_owned());
                }
            }
        }
    }
    result
}

/// Claim file lives in the physical checkout's Git administration directory, so
/// alternate paths and different configuration documents cannot double-adopt it.
pub async fn claim_path(worktree: &Path) -> Result<PathBuf> {
    Ok(PathBuf::from(
        git::text(worktree, &["rev-parse", "--absolute-git-dir"])
            .await?
            .trim(),
    )
    .join("treerail-ticket"))
}
async fn verify_claim(worktree: &Path, ticket: &str) -> Result<PathBuf> {
    let path = claim_path(worktree).await?;
    match std::fs::read_to_string(&path) {
        Ok(owner) => ensure!(
            owner == ticket,
            "Worktree already belongs to ticket {owner}; release its association explicitly before adopting"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("Read physical worktree association"),
    }
    Ok(path)
}
async fn preparation_lock(project: &Path) -> Result<std::fs::File> {
    let directory = PathBuf::from(
        git::text(
            project,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
        .await?
        .trim(),
    );
    tokio::task::spawn_blocking(move || {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join("treerail-preparation.lock"))?;
        fs2::FileExt::lock_exclusive(&file)?;
        Ok(file)
    })
    .await?
}
