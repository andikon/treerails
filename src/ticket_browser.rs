//! Repository-scoped ticket discovery, saved views and discussion caches.
use crate::{
    config::{self, Config},
    workflow::{Issue, IssueSource},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Label {
    pub name: String,
    pub color: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IssueCard {
    pub issue: Issue,
    pub labels: Vec<Label>,
    pub assignees: Vec<String>,
    pub author: String,
    pub created_at: String,
    pub updated_at: String,
}
impl From<Issue> for IssueCard {
    fn from(issue: Issue) -> Self {
        Self {
            issue,
            labels: vec![],
            assignees: vec![],
            author: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Comment {
    pub id: u64,
    pub author: String,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
    pub url: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FilterMetadata {
    pub labels: Vec<Label>,
    pub assignees: Vec<String>,
    pub warning: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Discussion {
    pub card: IssueCard,
    pub comments: Vec<Comment>,
    pub complete: bool,
    pub fetched_at: u64,
    pub warning: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Page {
    pub issues: Vec<IssueCard>,
    pub page: u32,
    pub has_more: bool,
    pub total: Option<u64>,
    pub limited: bool,
    pub fetched_at: u64,
    pub warning: Option<String>,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum IssueState {
    #[default]
    Open,
    Closed,
    All,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum AssigneeFilter {
    #[default]
    Anyone,
    Me,
    Unassigned,
    Users(Vec<String>),
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum LabelMode {
    #[default]
    All,
    Any,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Updated,
    Newest,
    Oldest,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Filter {
    pub state: IssueState,
    pub assignees: AssigneeFilter,
    pub labels: Vec<String>,
    pub label_mode: LabelMode,
    pub exclude_labels: Vec<String>,
    pub text: String,
    pub include_comments: bool,
    pub sort: Sort,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SavedView {
    pub id: String,
    pub name: String,
    pub filter: Filter,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepositoryViews {
    #[serde(default)]
    pub revision: u64,
    pub views: Vec<SavedView>,
    pub active: String,
    #[serde(default)]
    pub active_collapsed: bool,
    #[serde(default)]
    pub next_id: u64,
}
impl Default for RepositoryViews {
    fn default() -> Self {
        Self {
            revision: 0,
            views: vec![SavedView {
                id: "open".into(),
                name: "Open tickets".into(),
                filter: Filter::default(),
            }],
            active: "open".into(),
            active_collapsed: false,
            next_id: 1,
        }
    }
}
impl RepositoryViews {
    pub fn selected(&self) -> &SavedView {
        static DEFAULT: std::sync::LazyLock<SavedView> =
            std::sync::LazyLock::new(|| RepositoryViews::default().views.remove(0));
        self.views
            .iter()
            .find(|v| v.id == self.active)
            .or_else(|| self.views.first())
            .unwrap_or(&DEFAULT)
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BrowserState {
    pub cards: BTreeMap<String, IssueCard>,
    pub repositories: BTreeMap<String, RepositoryViews>,
    pub pages: BTreeMap<String, Page>,
    pub discussions: BTreeMap<String, Discussion>,
    pub projects: BTreeMap<String, String>,
    pub identities: BTreeMap<String, String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    Cards {
        repository: String,
        request: u64,
        numbers: Vec<u64>,
    },
    Metadata {
        repository: String,
        request: u64,
    },
    Load {
        request: u64,
    },
    Page {
        repository: String,
        request: u64,
        filter: Filter,
        page: u32,
    },
    Discussion {
        repository: String,
        request: u64,
        number: u64,
        page: u32,
    },
    SaveView {
        repository: String,
        id: String,
        name: String,
        filter: Filter,
        expected: Option<SavedView>,
    },
    SelectView {
        repository: String,
        id: String,
    },
    DeleteView {
        repository: String,
        id: String,
    },
    DuplicateView {
        repository: String,
        id: String,
        name: String,
    },
    SetActiveCollapsed {
        repository: String,
        collapsed: bool,
    },
}
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    Cards {
        repository: String,
        request: u64,
        cards: Vec<IssueCard>,
        warning: Option<String>,
    },
    Metadata {
        repository: String,
        request: u64,
        metadata: FilterMetadata,
    },
    Loaded {
        repository: String,
        request: u64,
        views: RepositoryViews,
        page: Page,
    },
    Page {
        repository: String,
        request: u64,
        page: Page,
    },
    Discussion {
        repository: String,
        request: u64,
        number: u64,
        discussion: Discussion,
    },
    Views {
        repository: String,
        views: RepositoryViews,
    },
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn normalize_repository(value: &str) -> String {
    let clean = value
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_lowercase();
    if clean.matches('/').count() == 1 {
        format!("github.com/{clean}")
    } else {
        clean
    }
}
fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}
impl Filter {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.labels.iter().any(|l| self
                .exclude_labels
                .iter()
                .any(|e| e.eq_ignore_ascii_case(l))),
            "A label cannot be both included and excluded"
        );
        ensure!(
            !self.text.chars().any(char::is_control),
            "Search text must be one line"
        );
        Ok(())
    }
    pub fn query(&self, repository: &str) -> Result<String> {
        self.validate()?;
        let repository = normalize_repository(repository);
        ensure!(
            repository.split('/').count() == 3
                && repository.split('/').all(|part| !part.is_empty()
                    && part
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || ".-_".contains(c))),
            "Invalid repository identity"
        );
        let (_, repo) = repository.split_once('/').context("Invalid repository")?;
        let mut terms = vec![format!("repo:{repo}"), "is:issue".into()];
        match self.state {
            IssueState::Open => terms.push("is:open".into()),
            IssueState::Closed => terms.push("is:closed".into()),
            IssueState::All => {}
        };
        match &self.assignees {
            AssigneeFilter::Anyone => {}
            AssigneeFilter::Me => terms.push("assignee:@me".into()),
            AssigneeFilter::Unassigned => terms.push("no:assignee".into()),
            AssigneeFilter::Users(users) => {
                if !users.is_empty() {
                    terms.push(format!(
                        "({})",
                        users
                            .iter()
                            .map(|u| format!("assignee:{}", quote(u)))
                            .collect::<Vec<_>>()
                            .join(" OR ")
                    ));
                }
            }
        }
        if self.label_mode == LabelMode::Any && !self.labels.is_empty() {
            terms.push(format!(
                "({})",
                self.labels
                    .iter()
                    .map(|l| format!("label:{}", quote(l)))
                    .collect::<Vec<_>>()
                    .join(" OR ")
            ));
        } else {
            terms.extend(self.labels.iter().map(|l| format!("label:{}", quote(l))));
        }
        terms.extend(
            self.exclude_labels
                .iter()
                .map(|l| format!("-label:{}", quote(l))),
        );
        if !self.text.trim().is_empty() {
            terms.push(quote(self.text.trim()));
            terms.push(
                if self.include_comments {
                    "in:title,body,comments"
                } else {
                    "in:title,body"
                }
                .into(),
            );
        }
        Ok(terms.join(" "))
    }
}
pub fn fallback_page(issues: Vec<Issue>, filter: &Filter, page: u32) -> Result<Page> {
    filter.validate()?;
    ensure!(page == 1, "Legacy source does not support pagination");
    ensure!(
        filter.labels.is_empty()
            && filter.exclude_labels.is_empty()
            && filter.assignees == AssigneeFilter::Anyone,
        "Source does not support structured filters"
    );
    let issues = issues
        .into_iter()
        .filter(|i| match filter.state {
            IssueState::Open => i.state.eq_ignore_ascii_case("open"),
            IssueState::Closed => i.state.eq_ignore_ascii_case("closed"),
            IssueState::All => true,
        })
        .filter(|i| {
            filter.text.is_empty()
                || format!("{} {}", i.title, i.body)
                    .to_lowercase()
                    .contains(&filter.text.to_lowercase())
        })
        .map(Into::into)
        .collect::<Vec<_>>();
    Ok(Page {
        total: Some(issues.len() as u64),
        issues,
        page,
        has_more: false,
        limited: false,
        fetched_at: now(),
        warning: None,
    })
}
fn update<T>(path: &Path, change: impl FnOnce(&mut BrowserState) -> Result<T>) -> Result<T> {
    config::edit_document(path, |doc| {
        let mut state = Config::parse(&doc.to_string(), path)?.state.browser;
        let result = change(&mut state)?;
        let rendered: toml_edit::DocumentMut = toml::to_string(&state)?.parse()?;
        if doc.get("state").is_none() {
            doc["state"] = toml_edit::table();
        }
        let mut incoming = toml_edit::Item::Table(rendered.as_table().clone());
        if let Some(previous) = doc["state"].get("browser") {
            copy_comments(previous, &mut incoming);
        }
        doc["state"]["browser"] = incoming;
        Ok(result)
    })
}
// Rebuilding the browser subtree keeps array-of-table child positions coherent;
// carry user decorations forward without reusing stale TOML table positions.
fn copy_comments(previous: &toml_edit::Item, incoming: &mut toml_edit::Item) {
    if let (Some(old), Some(new)) = (previous.as_value(), incoming.as_value_mut()) {
        *new.decor_mut() = old.decor().clone();
    }
    if let (Some(old), Some(new)) = (previous.as_table(), incoming.as_table_mut()) {
        *new.decor_mut() = old.decor().clone();
        for (key, value) in new.iter_mut() {
            if let Some(existing) = old.get(&key) {
                copy_comments(existing, value);
            }
        }
    } else if let (Some(old), Some(new)) = (
        previous.as_array_of_tables(),
        incoming.as_array_of_tables_mut(),
    ) {
        for table in new.iter_mut() {
            let id = table.get("id").and_then(toml_edit::Item::as_str);
            if let Some(existing) = old
                .iter()
                .find(|t| t.get("id").and_then(toml_edit::Item::as_str) == id)
            {
                let mut item = toml_edit::Item::Table(table.clone());
                copy_comments(&toml_edit::Item::Table(existing.clone()), &mut item);
                *table = item.into_table().unwrap();
            }
        }
    }
}
fn page_key(repository: &str, identity: &str, filter: &Filter, page: u32) -> Result<String> {
    Ok(format!(
        "{repository}|{identity}|{}|{page}",
        serde_json::to_string(filter)?
    ))
}
async fn fetch_page(
    path: &Path,
    project: &Path,
    repository: &str,
    filter: &Filter,
    page: u32,
    source: &impl IssueSource,
) -> Result<Page> {
    ensure!(
        (1..=10).contains(&page),
        "GitHub search provides at most 1000 results; narrow this view"
    );
    let identity = identity(path, project, repository, source).await?;
    let key = page_key(repository, &identity, filter, page)?;
    match source.browser_page(project, repository, filter, page).await {
        Ok(value) => {
            update(path, |state| {
                for card in &value.issues {
                    state.cards.insert(
                        format!("{repository}|{identity}#{}", card.issue.number),
                        card.clone(),
                    );
                }
                state.pages.insert(key, value.clone());
                Ok(())
            })?;
            Ok(value)
        }
        Err(error) => {
            let mut cached = Config::load(path)?
                .state
                .browser
                .pages
                .get(&key)
                .cloned()
                .with_context(|| {
                    format!("GitHub unavailable; no cache for this exact view: {error:#}")
                })?;
            cached.warning = Some(format!("Cached; refresh failed: {error:#}"));
            Ok(cached)
        }
    }
}
pub async fn execute(
    path: &Path,
    project: &Path,
    command: Command,
    source: &impl IssueSource,
) -> Result<Event> {
    match command {
        Command::Cards {
            repository,
            request,
            numbers,
        } => {
            let identity = identity(path, project, &repository, source).await?;
            let cached = Config::load(path)?.state.browser;
            let mut cards = Vec::new();
            let mut warning = None;
            for number in numbers {
                let key = format!("{repository}|{identity}#{number}");
                match source.card(project, number).await {
                    Ok(card) => {
                        update(path, |state| {
                            state.cards.insert(key.clone(), card.clone());
                            Ok(())
                        })?;
                        cards.push(card);
                    }
                    Err(error) => {
                        if let Some(card) = cached.cards.get(&key) {
                            cards.push(card.clone());
                        }
                        warning = Some(format!(
                            "Active ticket metadata may be cached or unavailable: {error:#}"
                        ));
                    }
                }
            }
            Ok(Event::Cards {
                repository,
                request,
                cards,
                warning,
            })
        }
        Command::Metadata {
            repository,
            request,
        } => {
            let metadata = match source.filter_metadata(project).await {
                Ok(value) => value,
                Err(error) => FilterMetadata {
                    warning: Some(format!("Filter suggestions unavailable: {error:#}")),
                    ..Default::default()
                },
            };
            Ok(Event::Metadata {
                repository,
                request,
                metadata,
            })
        }
        Command::Load { request } => {
            let project_key = project.to_string_lossy().to_string();
            let repository = match source.repository_identity(project).await {
                Ok(repo) => normalize_repository(&repo),
                Err(error) => Config::load(path)?
                    .state
                    .browser
                    .projects
                    .get(&project_key)
                    .cloned()
                    .with_context(|| format!("Repository unavailable: {error:#}"))?,
            };
            let views = update(path, |state| {
                state.projects.insert(project_key, repository.clone());
                Ok(state
                    .repositories
                    .entry(repository.clone())
                    .or_default()
                    .clone())
            })?;
            let page = fetch_page(
                path,
                project,
                &repository,
                &views.selected().filter,
                1,
                source,
            )
            .await?;
            Ok(Event::Loaded {
                repository,
                request,
                views,
                page,
            })
        }
        Command::Page {
            repository,
            request,
            filter,
            page,
        } => {
            let page = fetch_page(path, project, &repository, &filter, page, source).await?;
            Ok(Event::Page {
                repository,
                request,
                page,
            })
        }
        Command::Discussion {
            repository,
            request,
            number,
            page,
        } => {
            let identity = identity(path, project, &repository, source).await?;
            let key = format!("{repository}|{identity}#{number}");
            let discussion = match source.discussion_page(project, number, page).await {
                Ok(mut value) => {
                    update(path, |state| {
                        if page > 1 {
                            let previous = state
                                .discussions
                                .get(&key)
                                .context("Discussion first page missing")?;
                            ensure!(
                                previous.card.issue == value.card.issue,
                                "Issue changed while loading discussion; refresh it"
                            );
                            let mut comments = previous.comments.clone();
                            for comment in value.comments.drain(..) {
                                if let Some(existing) =
                                    comments.iter_mut().find(|c| c.id == comment.id)
                                {
                                    *existing = comment;
                                } else {
                                    comments.push(comment)
                                }
                            }
                            value.comments = comments;
                        }
                        state.discussions.insert(key.clone(), value.clone());
                        state.cards.insert(key.clone(), value.card.clone());
                        Ok(())
                    })?;
                    value
                }
                Err(error) => {
                    let mut cached = Config::load(path)?
                        .state
                        .browser
                        .discussions
                        .get(&key)
                        .cloned()
                        .with_context(|| {
                            format!("Discussion unavailable and not cached: {error:#}")
                        })?;
                    cached.warning = Some(format!("Cached; refresh failed: {error:#}"));
                    cached
                }
            };
            Ok(Event::Discussion {
                repository,
                request,
                number,
                discussion,
            })
        }
        other => {
            let repository = match &other {
                Command::SaveView { repository, .. }
                | Command::SelectView { repository, .. }
                | Command::DeleteView { repository, .. }
                | Command::DuplicateView { repository, .. }
                | Command::SetActiveCollapsed { repository, .. } => repository.clone(),
                _ => unreachable!(),
            };
            let views = update(path, |state| {
                let views = state.repositories.entry(repository.clone()).or_default();
                let next_revision = views
                    .revision
                    .checked_add(1)
                    .context("Saved-view revision exhausted")?;
                match other {
                    Command::SaveView {
                        id,
                        name,
                        filter,
                        expected,
                        ..
                    } => {
                        filter.validate()?;
                        if !id.is_empty() {
                            let current = views
                                .views
                                .iter()
                                .find(|v| v.id == id)
                                .context("View was deleted; reload saved views")?;
                            if let Some(expected) = expected {
                                ensure!(
                                    current == &expected,
                                    "View changed in another client; reload before applying"
                                );
                            }
                        }
                        validate_name(views, &id, &name)?;
                        if let Some(view) = views.views.iter_mut().find(|v| v.id == id) {
                            view.name = name.trim().into();
                            view.filter = filter;
                        } else {
                            let id = unique_id(views);
                            views.views.push(SavedView {
                                id: id.clone(),
                                name: name.trim().into(),
                                filter,
                            });
                            views.active = id;
                        }
                    }
                    Command::SelectView { id, .. } => {
                        ensure!(
                            views.views.iter().any(|v| v.id == id),
                            "Saved view does not exist"
                        );
                        views.active = id;
                    }
                    Command::DeleteView { id, .. } => {
                        ensure!(
                            views.views.iter().any(|v| v.id == id),
                            "Saved view does not exist"
                        );
                        views.views.retain(|v| v.id != id);
                        if views.views.is_empty() {
                            let collapsed = views.active_collapsed;
                            let next = views.next_id;
                            *views = RepositoryViews::default();
                            views.active_collapsed = collapsed;
                            views.next_id = next;
                        } else if views.active == id {
                            views.active = views.views[0].id.clone();
                        }
                    }
                    Command::DuplicateView { id, name, .. } => {
                        validate_name(views, "", &name)?;
                        let mut view = views
                            .views
                            .iter()
                            .find(|v| v.id == id)
                            .context("Saved view does not exist")?
                            .clone();
                        view.id = unique_id(views);
                        view.name = name.trim().into();
                        views.active = view.id.clone();
                        views.views.push(view);
                    }
                    Command::SetActiveCollapsed { collapsed, .. } => {
                        views.active_collapsed = collapsed
                    }
                    _ => unreachable!(),
                };
                views.revision = next_revision;
                Ok(views.clone())
            })?;
            Ok(Event::Views { repository, views })
        }
    }
}
fn validate_name(views: &RepositoryViews, id: &str, name: &str) -> Result<()> {
    ensure!(!name.trim().is_empty(), "View name is required");
    ensure!(
        !views
            .views
            .iter()
            .any(|v| v.id != id && v.name.eq_ignore_ascii_case(name.trim())),
        "View name already exists"
    );
    Ok(())
}
fn unique_id(views: &mut RepositoryViews) -> String {
    loop {
        views.next_id = views.next_id.saturating_add(1);
        let id = format!("view-{}", views.next_id);
        if !views.views.iter().any(|v| v.id == id) {
            return id;
        }
    }
}
pub async fn complete_discussion(
    project: &Path,
    source: &impl IssueSource,
    number: u64,
) -> Result<Discussion> {
    let mut full = source.discussion_page(project, number, 1).await?;
    let mut page = 1;
    while !full.complete {
        page += 1;
        ensure!(page <= 10000, "Discussion pagination did not terminate");
        let next = source.discussion_page(project, number, page).await?;
        ensure!(
            next.warning.is_none(),
            "A fresh complete discussion is required"
        );
        ensure!(
            next.card.issue == full.card.issue && next.card.updated_at == full.card.updated_at,
            "Issue changed during capture; refresh and retry"
        );
        for comment in next.comments {
            if !full.comments.iter().any(|c| c.id == comment.id) {
                full.comments.push(comment)
            }
        }
        full.complete = next.complete;
    }
    ensure!(
        full.warning.is_none(),
        "A fresh complete discussion is required"
    );
    Ok(full)
}
fn card(value: &serde_json::Value) -> Result<IssueCard> {
    let text = |key: &str| value[key].as_str().unwrap_or_default().to_string();
    Ok(IssueCard {
        issue: Issue {
            number: value["number"].as_u64().context("Issue number missing")?,
            title: text("title"),
            body: text("body"),
            state: text("state"),
            url: text("html_url"),
        },
        labels: value["labels"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|l| Label {
                name: l["name"].as_str().unwrap_or_default().into(),
                color: l["color"].as_str().unwrap_or_default().into(),
            })
            .collect(),
        assignees: value["assignees"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|u| u["login"].as_str().map(str::to_owned))
            .collect(),
        author: value["user"]["login"]
            .as_str()
            .unwrap_or("deleted user")
            .into(),
        created_at: text("created_at"),
        updated_at: text("updated_at"),
    })
}
pub fn parse_comment(value: &serde_json::Value) -> Result<Comment> {
    Ok(Comment {
        id: value["id"].as_u64().context("Comment id missing")?,
        author: value["user"]["login"]
            .as_str()
            .unwrap_or("deleted user")
            .into(),
        body: value["body"].as_str().unwrap_or_default().into(),
        created_at: value["created_at"].as_str().unwrap_or_default().into(),
        updated_at: value["updated_at"].as_str().unwrap_or_default().into(),
        url: value["html_url"].as_str().unwrap_or_default().into(),
    })
}
pub async fn github_page(
    project: &Path,
    repository: &str,
    filter: &Filter,
    page: u32,
) -> Result<Page> {
    let mut resolved = filter.clone();
    if resolved.assignees == AssigneeFilter::Me {
        resolved.assignees =
            AssigneeFilter::Users(vec![crate::workflow::Github.current_login(project).await?]);
    }
    let query = resolved.query(repository)?;
    let page_text = page.to_string();
    let (sort, order) = match filter.sort {
        Sort::Updated => ("updated", "desc"),
        Sort::Newest => ("created", "desc"),
        Sort::Oldest => ("created", "asc"),
    };
    let host = normalize_repository(repository)
        .split('/')
        .next()
        .unwrap_or("github.com")
        .to_owned();
    let value: serde_json::Value = crate::workflow::Github::json(
        project,
        &[
            "api",
            "--hostname",
            &host,
            "--method",
            "GET",
            "search/issues",
            "-f",
            "advanced_search=true",
            "-f",
            &format!("q={query}"),
            "-f",
            &format!("sort={sort}"),
            "-f",
            &format!("order={order}"),
            "-f",
            "per_page=100",
            "-f",
            &format!("page={page_text}"),
        ],
    )
    .await?;
    let total = value["total_count"].as_u64();
    let issues = value["items"]
        .as_array()
        .context("Search results missing")?
        .iter()
        .map(card)
        .collect::<Result<Vec<_>>>()?;
    let loaded = page as u64 * 100;
    let incomplete = value["incomplete_results"].as_bool().unwrap_or(false);
    Ok(Page {
        has_more: page < 10 && total.is_some_and(|count| count > loaded),
        limited: total.is_some_and(|count| count > 1000),
        issues,
        page,
        total,
        fetched_at: now(),
        warning: incomplete.then(|| {
            "GitHub returned incomplete search results; refresh or narrow the view".into()
        }),
    })
}
pub async fn github_discussion(project: &Path, number: u64, page: u32) -> Result<Discussion> {
    ensure!(page > 0, "Page starts at 1");
    let host = github_host(project).await?;
    let value: serde_json::Value = crate::workflow::Github::json(
        project,
        &[
            "api",
            "--hostname",
            &host,
            &format!("repos/{{owner}}/{{repo}}/issues/{number}"),
        ],
    )
    .await?;
    let issue = card(&value)?;
    let values: Vec<serde_json::Value> = crate::workflow::Github::json(
        project,
        &[
            "api",
            "--hostname",
            &host,
            &format!("repos/{{owner}}/{{repo}}/issues/{number}/comments?per_page=100&page={page}"),
        ],
    )
    .await?;
    let complete = values.len() < 100;
    let comments = values
        .iter()
        .map(parse_comment)
        .collect::<Result<Vec<_>>>()?;
    Ok(Discussion {
        card: issue,
        comments,
        complete,
        fetched_at: now(),
        warning: None,
    })
}
async fn identity(
    path: &Path,
    project: &Path,
    repository: &str,
    source: &impl IssueSource,
) -> Result<String> {
    match source.current_login(project).await {
        Ok(login) => {
            update(path, |state| {
                state.identities.insert(repository.into(), login.clone());
                Ok(())
            })?;
            Ok(login)
        }
        Err(_) => Ok(Config::load(path)?
            .state
            .browser
            .identities
            .get(repository)
            .cloned()
            .unwrap_or_else(|| "unknown".into())),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn issue(n: u64) -> Issue {
        Issue {
            number: n,
            title: format!("Issue {n}"),
            body: "body".into(),
            state: "open".into(),
            url: format!("https://github.com/example/repo/issues/{n}"),
        }
    }
    struct Source;
    impl IssueSource for Source {
        async fn repository(&self, _: &Path) -> Result<String> {
            Ok("example/repo".into())
        }
        async fn list(&self, _: &Path) -> Result<Vec<Issue>> {
            Ok(vec![issue(1)])
        }
        async fn get(&self, _: &Path, n: u64) -> Result<Issue> {
            Ok(issue(n))
        }
        async fn current_login(&self, _: &Path) -> Result<String> {
            Ok("dev".into())
        }
        async fn discussion_page(&self, _: &Path, n: u64, page: u32) -> Result<Discussion> {
            Ok(Discussion {
                card: issue(n).into(),
                comments: vec![Comment {
                    id: page as u64,
                    body: format!("page {page}"),
                    ..Default::default()
                }],
                complete: page == 2,
                fetched_at: now(),
                warning: None,
            })
        }
    }
    #[test]
    fn query_preserves_label_boolean_semantics() {
        let f = Filter {
            labels: vec!["ready".into(), "back end".into()],
            exclude_labels: vec!["blocked".into()],
            assignees: AssigneeFilter::Me,
            ..Default::default()
        };
        let q = f.query("github.com/example/repo").unwrap();
        assert!(q.contains("label:\"ready\" label:\"back end\""));
        assert!(q.contains("-label:\"blocked\""));
        assert!(q.contains("assignee:@me"));
        let any = Filter {
            label_mode: LabelMode::Any,
            ..f
        };
        assert!(
            any.query("example/repo")
                .unwrap()
                .contains("(label:\"ready\" OR label:\"back end\")")
        );
    }
    #[test]
    fn contradictory_labels_rejected_and_search_text_is_literal() {
        assert!(
            Filter {
                labels: vec!["READY".into()],
                exclude_labels: vec!["ready".into()],
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        let query = Filter {
            text: "repo:elsewhere is:closed Rust foo::bar".into(),
            ..Default::default()
        }
        .query("example/repo")
        .unwrap();
        assert_eq!(
            query,
            "repo:example/repo is:issue is:open \"repo:elsewhere is:closed Rust foo::bar\" in:title,body"
        );
        let escaped = Filter {
            text: "\" OR repo:elsewhere".into(),
            ..Default::default()
        }
        .query("example/repo")
        .unwrap();
        assert!(escaped.contains("\"\\\" OR repo:elsewhere\""));
    }
    #[tokio::test]
    async fn views_survive_restarts_preserve_settings_and_reject_stale_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "# personal note\ndesktop_notifications = true\n").unwrap();
        let repo = "github.com/example/repo".to_string();
        let event = execute(&path, dir.path(), Command::Load { request: 3 }, &Source)
            .await
            .unwrap();
        assert!(matches!(event, Event::Loaded { request: 3, .. }));
        assert!(
            Config::load(&path)
                .unwrap()
                .state
                .workflow
                .tickets
                .is_empty()
        );
        execute(
            &path,
            dir.path(),
            Command::SaveView {
                repository: repo.clone(),
                id: String::new(),
                name: "Backend".into(),
                filter: Filter {
                    labels: vec!["backend".into()],
                    ..Default::default()
                },
                expected: None,
            },
            &Source,
        )
        .await
        .unwrap();
        let state = Config::load(&path).unwrap().state.browser;
        let old = state.repositories[&repo].selected().clone();
        execute(
            &path,
            dir.path(),
            Command::SaveView {
                repository: repo.clone(),
                id: old.id.clone(),
                name: "Backend renamed".into(),
                filter: old.filter.clone(),
                expected: Some(old.clone()),
            },
            &Source,
        )
        .await
        .unwrap();
        assert!(
            execute(
                &path,
                dir.path(),
                Command::SaveView {
                    repository: repo.clone(),
                    id: old.id.clone(),
                    name: "Stale write".into(),
                    filter: old.filter.clone(),
                    expected: Some(old.clone())
                },
                &Source
            )
            .await
            .is_err()
        );
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("# personal note")
        );
        assert!(Config::load(&path).unwrap().desktop_notifications);
        execute(
            &path,
            dir.path(),
            Command::DeleteView {
                repository: repo.clone(),
                id: old.id,
            },
            &Source,
        )
        .await
        .unwrap();
        assert_eq!(
            Config::load(&path).unwrap().state.browser.repositories[&repo]
                .selected()
                .name,
            "Open tickets"
        );
    }
    #[tokio::test]
    async fn discussion_accumulates_and_capture_is_complete() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        for page in 1..=2 {
            let event = execute(
                &path,
                dir.path(),
                Command::Discussion {
                    repository: "github.com/example/repo".into(),
                    request: page as u64,
                    number: 1,
                    page,
                },
                &Source,
            )
            .await
            .unwrap();
            let Event::Discussion { discussion, .. } = event else {
                panic!()
            };
            assert_eq!(discussion.comments.len(), page as usize);
            assert_eq!(discussion.complete, page == 2);
        }
        let all = complete_discussion(dir.path(), &Source, 1).await.unwrap();
        assert!(all.complete);
        assert_eq!(all.comments.len(), 2);
    }
}
pub async fn github_metadata(project: &Path) -> Result<FilterMetadata> {
    let host = github_host(project).await?;
    let mut metadata = FilterMetadata::default();
    for (endpoint, is_labels) in [("labels", true), ("assignees", false)] {
        for page in 1..=100 {
            let values: Vec<serde_json::Value> = crate::workflow::Github::json(
                project,
                &[
                    "api",
                    "--hostname",
                    &host,
                    &format!("repos/{{owner}}/{{repo}}/{endpoint}?per_page=100&page={page}"),
                ],
            )
            .await?;
            for value in &values {
                if is_labels {
                    metadata.labels.push(Label {
                        name: value["name"].as_str().unwrap_or_default().into(),
                        color: value["color"].as_str().unwrap_or_default().into(),
                    });
                } else if let Some(login) = value["login"].as_str() {
                    metadata.assignees.push(login.into());
                }
            }
            if values.len() < 100 {
                break;
            }
            if page == 100 {
                metadata.warning = Some(
                    "Suggestions capped at 10000 entries; existing filters remain available".into(),
                );
            }
        }
    }
    Ok(metadata)
}
#[cfg(test)]
mod cache_tests {
    use super::*;
    struct Source {
        online: bool,
        user: &'static str,
    }
    impl IssueSource for Source {
        async fn repository(&self, _: &Path) -> Result<String> {
            ensure!(self.online, "offline");
            Ok("example/repo".into())
        }
        async fn list(&self, _: &Path) -> Result<Vec<Issue>> {
            ensure!(self.online, "offline");
            Ok(vec![])
        }
        async fn get(&self, _: &Path, _: u64) -> Result<Issue> {
            anyhow::bail!("unused")
        }
        async fn current_login(&self, _: &Path) -> Result<String> {
            ensure!(self.online, "offline");
            Ok(self.user.into())
        }
    }
    #[tokio::test]
    async fn offline_reopen_uses_only_exact_query_and_auth_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let online = Source {
            online: true,
            user: "alice",
        };
        let offline = Source {
            online: false,
            user: "alice",
        };
        execute(&path, dir.path(), Command::Load { request: 1 }, &online)
            .await
            .unwrap();
        let Event::Loaded {
            page, repository, ..
        } = execute(&path, dir.path(), Command::Load { request: 2 }, &offline)
            .await
            .unwrap()
        else {
            panic!()
        };
        assert!(page.issues.is_empty());
        assert!(page.warning.unwrap().contains("Cached"));
        assert!(
            execute(
                &path,
                dir.path(),
                Command::Page {
                    repository: repository.clone(),
                    request: 3,
                    filter: Filter {
                        text: "uncached".into(),
                        ..Default::default()
                    },
                    page: 1
                },
                &offline
            )
            .await
            .is_err()
        );
        update(&path, |state| {
            state.identities.insert(repository.clone(), "bob".into());
            Ok(())
        })
        .unwrap();
        assert!(
            execute(&path, dir.path(), Command::Load { request: 4 }, &offline)
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn deleting_last_view_recreates_default_without_reusing_ids() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let repo = "github.com/example/repo".to_string();
        let source = Source {
            online: true,
            user: "alice",
        };
        let mut ids = Vec::new();
        for name in ["One", "Two"] {
            let Event::Views { views, .. } = execute(
                &path,
                dir.path(),
                Command::SaveView {
                    repository: repo.clone(),
                    id: String::new(),
                    name: name.into(),
                    filter: Filter::default(),
                    expected: None,
                },
                &source,
            )
            .await
            .unwrap() else {
                panic!()
            };
            ids.push(views.active.clone());
            execute(
                &path,
                dir.path(),
                Command::DeleteView {
                    repository: repo.clone(),
                    id: views.active,
                },
                &source,
            )
            .await
            .unwrap();
        }
        assert_ne!(ids[0], ids[1]);
        assert_eq!(
            Config::load(&path).unwrap().state.browser.repositories[&repo]
                .selected()
                .name,
            "Open tickets"
        );
    }
    #[test]
    fn host_identity_isolates_enterprise_and_normalizes_clones() {
        assert_eq!(
            normalize_repository("https://GitHub.com/Example/Repo/"),
            "github.com/example/repo"
        );
        assert_ne!(
            normalize_repository("https://enterprise.test/example/repo"),
            normalize_repository("example/repo")
        );
    }
}

pub async fn github_host(project: &Path) -> Result<String> {
    let identity = crate::workflow::Github.repository_identity(project).await?;
    Ok(identity
        .split('/')
        .next()
        .context("GitHub repository host unavailable")?
        .to_owned())
}
pub async fn github_card(project: &Path, number: u64) -> Result<IssueCard> {
    let host = github_host(project).await?;
    let value: serde_json::Value = crate::workflow::Github::json(
        project,
        &[
            "api",
            "--hostname",
            &host,
            &format!("repos/{{owner}}/{{repo}}/issues/{number}"),
        ],
    )
    .await?;
    card(&value)
}
#[cfg(test)]
mod metadata_tests {
    use super::*;
    #[test]
    fn github_card_preserves_label_and_assignee_metadata() {
        let value = serde_json::json!({"number":42,"title":"Fix","body":"Details","state":"open","html_url":"https://github.com/a/b/issues/42","labels":[{"name":"backend","color":"abcdef"}],"assignees":[{"login":"dev"}],"user":{"login":"author"},"created_at":"2026-10-01T00:00:00Z","updated_at":"2026-10-02T00:00:00Z"});
        let result = card(&value).unwrap();
        assert_eq!(
            result.labels,
            vec![Label {
                name: "backend".into(),
                color: "abcdef".into()
            }]
        );
        assert_eq!(result.assignees, vec!["dev"]);
        assert_eq!(result.author, "author");
    }
    #[test]
    fn specific_assignees_are_a_single_remote_or_group() {
        let query = Filter {
            assignees: AssigneeFilter::Users(vec!["alice".into(), "bob".into()]),
            ..Default::default()
        }
        .query("github.com/example/repo")
        .unwrap();
        assert_eq!(
            query,
            "repo:example/repo is:issue is:open (assignee:\"alice\" OR assignee:\"bob\")"
        );
    }
}
#[cfg(test)]
mod revision_tests {
    use super::*;
    struct Source;
    impl IssueSource for Source {
        async fn repository(&self, _: &Path) -> Result<String> {
            Ok("a/b".into())
        }
        async fn list(&self, _: &Path) -> Result<Vec<Issue>> {
            Ok(vec![])
        }
        async fn get(&self, _: &Path, _: u64) -> Result<Issue> {
            anyhow::bail!("unused")
        }
    }
    #[tokio::test]
    async fn view_revisions_order_mutations_and_survive_last_view_reset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let repository = "github.com/a/b".to_string();
        let mut snapshots = Vec::new();
        for command in [
            Command::SelectView {
                repository: repository.clone(),
                id: "open".into(),
            },
            Command::SetActiveCollapsed {
                repository: repository.clone(),
                collapsed: true,
            },
            Command::DeleteView {
                repository: repository.clone(),
                id: "open".into(),
            },
        ] {
            let Event::Views { views, .. } =
                execute(&path, dir.path(), command, &Source).await.unwrap()
            else {
                panic!()
            };
            snapshots.push(views);
        }
        assert_eq!(
            snapshots.iter().map(|v| v.revision).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert!(snapshots[0].revision < snapshots[2].revision);
        assert_eq!(snapshots[2].selected().name, "Open tickets");
        assert!(snapshots[2].active_collapsed);
        let reopened = Config::load(&path).unwrap();
        assert_eq!(reopened.state.browser.repositories[&repository].revision, 3);
        assert!(
            execute(
                &path,
                dir.path(),
                Command::SelectView {
                    repository: repository.clone(),
                    id: "missing".into()
                },
                &Source
            )
            .await
            .is_err()
        );
        assert_eq!(
            Config::load(&path).unwrap().state.browser.repositories[&repository].revision,
            3
        );
    }
}
