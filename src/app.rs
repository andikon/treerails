use crate::{
    agent::{self, Attention, Conversation},
    config::{self, Config, Navigation, State, same_path},
    discover,
    git::{self, ChangedFile, Project, Snapshot, Worktree},
};
use anyhow::{Context, Result};
use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Sessions,
    Changes,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Worktrees,
    List,
    Detail,
}
#[derive(Debug, Clone)]
pub struct Tab {
    pub project: Project,
    pub trees: Vec<Worktree>,
    pub selected: usize,
    pub view: View,
    pub conversation: Option<String>,
    pub file: usize,
}
#[derive(Debug, Clone)]
pub enum Operation {
    Stage(Vec<ChangedFile>, bool),
    Commit(String, String),
    Fetch,
    Pull,
    Push(Option<String>),
    Remove,
    Create {
        branch: String,
        base: String,
        destination: PathBuf,
        source: PathBuf,
        existing: bool,
        session: Option<(String, Value)>,
    },
}
impl Operation {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Stage(_, false) => "Stage",
            Self::Stage(_, true) => "Unstage",
            Self::Commit(..) => "Commit",
            Self::Fetch => "Fetch",
            Self::Pull => "Pull",
            Self::Push(_) => "Push / publish",
            Self::Remove => "Remove worktree",
            Self::Create { .. } => "Create worktree",
        }
    }
}
#[derive(Debug)]
pub enum Job {
    Discover(Vec<PathBuf>, bool),
    Trees(Project),
    Remotes(Project),
    Branches(Project),
    Associate(PathBuf, Value),
    Status(PathBuf),
    Diff(PathBuf, ChangedFile, bool),
    Operate {
        project: PathBuf,
        target: PathBuf,
        operation: Operation,
    },
}
#[derive(Debug)]
pub enum Completion {
    Projects(Vec<Project>, Vec<String>),
    Trees(Project, Vec<Worktree>),
    Remotes(PathBuf, Vec<String>),
    Branches(PathBuf, Vec<git::Branch>),
    Associated(Value),
    Status(PathBuf, Snapshot),
    Diff(PathBuf, PathBuf, bool, String),
    Operated {
        project: PathBuf,
        target: PathBuf,
        created: Option<PathBuf>,
        session: Option<(String, Value)>,
    },
}
impl Job {
    pub fn token(&self) -> String {
        match self {
            Self::Discover(..) => "discover".into(),
            Self::Trees(p) => format!("trees:{}", p.path.display()),
            Self::Remotes(p) => format!("remotes:{}", p.path.display()),
            Self::Branches(p) => format!("branches:{}", p.path.display()),
            Self::Associate(_, v) => format!("associate:{}", v["resource"]),
            Self::Status(p) => format!("status:{}", p.display()),
            Self::Diff(p, f, s) => format!("diff:{}:{}:{s}", p.display(), f.path.display()),
            Self::Operate { .. } => "mutation".into(),
        }
    }
    pub async fn execute(self) -> Result<Completion> {
        match self {
            Self::Discover(roots, recursive) => {
                let (p, w) = discover::discover(roots, recursive).await?;
                Ok(Completion::Projects(p, w))
            }
            Self::Trees(project) => {
                let trees = git::worktrees(&project.path).await?;
                Ok(Completion::Trees(project, trees))
            }
            Self::Remotes(project) => Ok(Completion::Remotes(
                project.path.clone(),
                git::remotes(&project.path).await?,
            )),
            Self::Branches(project) => {
                git::run(&project.path, ["fetch", "--progress", "--all", "--prune"]).await?;
                Ok(Completion::Branches(
                    project.path.clone(),
                    git::branches(&project.path).await?,
                ))
            }
            Self::Associate(path, mut item) => {
                let project = git::project(&path).await?;
                item["_treerailProject"] = serde_json::to_value(project.path)?;
                Ok(Completion::Associated(item))
            }
            Self::Status(path) => Ok(Completion::Status(path.clone(), git::status(&path).await?)),
            Self::Diff(path, file, staged) => Ok(Completion::Diff(
                path.clone(),
                file.path.clone(),
                staged,
                git::diff(&path, &file, staged).await?,
            )),
            Self::Operate {
                project,
                target,
                operation,
            } => {
                let mut created = None;
                let mut session = None;
                match operation {
                    Operation::Stage(files, unstage) => {
                        git::stage(&target, &files, unstage).await?
                    }
                    Operation::Commit(subject, body) => {
                        git::commit(&target, &subject, &body).await?;
                    }
                    Operation::Fetch => {
                        git::run(&target, ["fetch", "--progress", "--all", "--prune"]).await?;
                    }
                    Operation::Pull => git::pull(&target).await?,
                    Operation::Push(remote) => {
                        git::push_or_publish(&target, remote.as_deref()).await?
                    }
                    Operation::Remove => git::remove_worktree(&project, &target).await?,
                    Operation::Create {
                        branch,
                        base,
                        destination,
                        source,
                        existing,
                        session: start,
                    } => {
                        created = Some(
                            git::create_from_start(
                                &project,
                                &source,
                                &base,
                                &branch,
                                &destination,
                                existing,
                            )
                            .await?,
                        );
                        session = start;
                    }
                }
                Ok(Completion::Operated {
                    project,
                    target,
                    created,
                    session,
                })
            }
        }
    }
}
#[derive(Debug, Clone, Default)]
pub struct Input {
    pub value: String,
    pub cursor: usize,
}
impl Input {
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let cursor = value.len();
        Self { value, cursor }
    }
    pub fn insert(&mut self, value: &str) {
        self.value.insert_str(self.cursor, value);
        self.cursor += value.len();
    }
    pub fn key(&mut self, key: KeyEvent, multiline: bool) {
        match key.code {
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.insert(&c.to_string())
            }
            KeyCode::Left => {
                self.cursor = self.value[..self.cursor]
                    .char_indices()
                    .next_back()
                    .map(|(i, _)| i)
                    .unwrap_or(0)
            }
            KeyCode::Right => {
                self.cursor += self.value[self.cursor..]
                    .chars()
                    .next()
                    .map(char::len_utf8)
                    .unwrap_or(0)
            }
            KeyCode::Home => {
                self.cursor = if key.modifiers.contains(KeyModifiers::CONTROL) {
                    0
                } else {
                    self.value[..self.cursor].rfind('\n').map_or(0, |i| i + 1)
                }
            }
            KeyCode::End => {
                self.cursor = if key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.value.len()
                } else {
                    self.value[self.cursor..]
                        .find('\n')
                        .map_or(self.value.len(), |i| self.cursor + i)
                }
            }
            KeyCode::Up | KeyCode::Down if multiline => {
                let start = self.value[..self.cursor].rfind('\n').map_or(0, |i| i + 1);
                let column = self.value[start..self.cursor].chars().count();
                let bounds = if key.code == KeyCode::Up && start > 0 {
                    let end = start - 1;
                    Some((self.value[..end].rfind('\n').map_or(0, |i| i + 1), end))
                } else if key.code == KeyCode::Down {
                    self.value[self.cursor..].find('\n').map(|i| {
                        let start = self.cursor + i + 1;
                        let end = self.value[start..]
                            .find('\n')
                            .map_or(self.value.len(), |i| start + i);
                        (start, end)
                    })
                } else {
                    None
                };
                if let Some((start, end)) = bounds {
                    self.cursor = start
                        + self.value[start..end]
                            .char_indices()
                            .nth(column)
                            .map_or(end - start, |(i, _)| i);
                }
            }
            KeyCode::Backspace if self.cursor > 0 => {
                let prev = self.value[..self.cursor]
                    .char_indices()
                    .next_back()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                self.value.drain(prev..self.cursor);
                self.cursor = prev;
            }
            KeyCode::Delete => {
                if let Some(c) = self.value[self.cursor..].chars().next() {
                    self.value.drain(self.cursor..self.cursor + c.len_utf8());
                }
            }
            KeyCode::Enter if multiline => self.insert("\n"),
            _ => {}
        }
    }
}
#[derive(Debug, Clone)]
pub enum FieldType {
    Text,
    Branch,
    Multiline,
    Toggle,
    Choice(Vec<String>),
}
#[derive(Debug, Clone)]
pub struct Field {
    pub key: String,
    pub label: String,
    pub input: Input,
    pub kind: FieldType,
}
impl Field {
    fn text(key: &str, label: &str, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            input: Input::new(value),
            kind: FieldType::Text,
        }
    }
    fn toggle(key: &str, label: &str, on: bool) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            input: Input::new(if on { "yes" } else { "no" }),
            kind: FieldType::Toggle,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormKind {
    Session,
    Worktree,
    Commit,
    AddPath,
    Attach,
    Preferences,
    Question,
    SessionOptions,
    PublishRemote,
}
#[derive(Debug, Clone)]
pub struct Form {
    pub kind: FormKind,
    pub fields: Vec<Field>,
    pub focused: usize,
    pub advanced: bool,
    pub title: String,
    pub context: Value,
}
impl Form {
    pub fn value(&self, key: &str) -> String {
        self.fields
            .iter()
            .find(|f| f.key == key)
            .map(|f| f.input.value.clone())
            .unwrap_or_default()
    }
    pub fn visible(&self) -> Vec<usize> {
        self.fields
            .iter()
            .enumerate()
            .filter(|(_, f)| match self.kind {
                FormKind::Session => match f.key.as_str() {
                    "base" | "mode" => self.value("new_tree") == "yes",
                    "branch" => {
                        self.value("new_tree") == "yes"
                            && self.value("mode") != "Use existing branch"
                    }
                    "name" | "model" => self.advanced,
                    k if k.starts_with("config:") => self.advanced,
                    _ => true,
                },
                FormKind::Worktree if f.key == "branch" => {
                    self.value("mode") != "Use existing branch"
                }
                _ => true,
            })
            .map(|(i, _)| i)
            .collect()
    }
}
#[derive(Debug, Clone)]
pub enum Confirm {
    Close(usize),
    Quit,
    Remove,
    Trust { id: String, path: PathBuf },
}
#[derive(Debug, Clone)]
pub enum Modal {
    Branches {
        form: Box<Form>,
        query: Input,
        selected: usize,
    },
    Projects {
        query: Input,
        selected: usize,
    },
    Help(usize),
    Form(Form),
    Confirm(Confirm),
    Activity(usize),
    Host,
    Notice(String),
    Sources(usize),
    Approval(Value),
    Completions {
        items: Vec<Value>,
        selected: usize,
    },
}
#[derive(Debug, Clone)]
pub enum Action {
    PickBranch,
    ChooseBranch(usize),
    Project(usize),
    Tab(usize),
    Close(usize),
    Tree(usize),
    Session(String),
    File(usize),
    Switch(View),
    NewTree,
    NewSession,
    NewSessionOptions,
    Help,
    Projects,
    Confirm,
    Cancel,
    Git(&'static str),
    Stop,
    Archive,
    History,
    Quit,
    Activity,
    Host,
    Resume,
    FocusTerminal,
    CloseTerminal,
    Options,
    Attach,
    Preferences,
    Field(usize),
    ToggleAdvanced,
    Scroll(i16),
    Expand,
    Respond,
    Approve(bool, Option<String>),
    Completion(usize),
    Details,
    Retry,
    WorktreeInfo,
    Sources,
    ForgetSource(usize),
}
#[derive(Debug, Clone)]
pub struct Hit {
    pub rect: Rect,
    pub action: Action,
}
pub struct App {
    pub terminals: crate::cli_terminal::Terminals,
    pub terminal_area: Rect,
    terminal_requested: BTreeMap<String, Option<String>>,
    pub config: Config,
    pub state: State,
    pub config_path: PathBuf,
    pub roots: Vec<PathBuf>,
    pub recursive: bool,
    pub projects: Vec<Project>,
    pub tabs: Vec<Tab>,
    pub selected_tab: usize,
    pub pane: Pane,
    pub modal: Option<Modal>,
    pub snapshots: BTreeMap<PathBuf, Snapshot>,
    pub branches: BTreeMap<PathBuf, Vec<git::Branch>>,
    pub remotes: BTreeMap<PathBuf, Vec<String>>,
    pub diff: String,
    pub diff_staged: bool,
    pub diff_scroll: u16,
    pub transcript_scroll: u16,
    pub marked: BTreeSet<PathBuf>,
    pub composer: Input,
    pub cli_focused: bool,
    pub attachments: Vec<Value>,
    pub expanded: bool,
    pub jobs: Vec<Job>,
    pub agent_commands: Vec<agent::Command>,
    pending: BTreeSet<String>,
    pub mutation: Option<(PathBuf, String)>,
    pub notice: Option<(String, bool, Instant)>,
    pub operation_output: String,
    pub connected: bool,
    pub attached: BTreeSet<String>,
    pub connection_url: Option<String>,
    pub agents: Value,
    pub provider_options: Value,
    pub completions: Value,
    pub hits: Vec<Hit>,
    pub quit: bool,
    pub confirm_yes: bool,
    failed_form: Option<Form>,
    last_refresh: Instant,
    last_agent_refresh: Instant,
    last_save: Instant,
    dirty: bool,
}
impl App {
    pub fn new(session: crate::startup::Session) -> Self {
        let mut state = session.state;
        for c in &mut state.conversations {
            if let Some(prompt) = c.initial_prompt.take()
                && c.draft.is_empty()
            {
                c.draft = prompt;
            }
            if c.attention.busy() {
                c.attention = Attention::Interrupted;
            }
        }
        let projects = state.known_projects.clone();
        let tabs = state
            .tabs
            .iter()
            .map(|path| {
                let nav = state
                    .navigation
                    .get(&path.to_string_lossy().to_string())
                    .cloned()
                    .unwrap_or_default();
                let project = projects
                    .iter()
                    .find(|p| same_path(&p.path, path))
                    .cloned()
                    .unwrap_or(Project {
                        path: path.clone(),
                        name: path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into(),
                    });
                Tab {
                    project,
                    trees: vec![],
                    selected: 0,
                    view: if nav.changes {
                        View::Changes
                    } else {
                        View::Sessions
                    },
                    conversation: nav.conversation,
                    file: 0,
                }
            })
            .collect::<Vec<_>>();
        let selected_tab = tabs
            .iter()
            .position(|t| {
                state
                    .last_project
                    .as_ref()
                    .is_some_and(|p| same_path(p, &t.project.path))
            })
            .unwrap_or(0);
        let mut app = Self {
            terminals: Default::default(),
            terminal_area: Rect::default(),
            terminal_requested: BTreeMap::new(),
            config: session.config,
            state,
            config_path: session.config_path,
            roots: session.roots,
            recursive: session.recursive,
            projects,
            tabs,
            selected_tab,
            pane: Pane::Worktrees,
            modal: Some(Modal::Projects {
                query: Input::default(),
                selected: 0,
            }),
            snapshots: BTreeMap::new(),
            branches: BTreeMap::new(),
            remotes: BTreeMap::new(),
            diff: String::new(),
            diff_staged: false,
            diff_scroll: 0,
            transcript_scroll: 0,
            marked: BTreeSet::new(),
            composer: Input::default(),
            cli_focused: false,
            attachments: vec![],
            expanded: false,
            jobs: vec![],
            agent_commands: vec![],
            pending: BTreeSet::new(),
            mutation: None,
            notice: None,
            operation_output: String::new(),
            connected: false,
            attached: BTreeSet::new(),
            connection_url: None,
            agents: json!([]),
            provider_options: json!({}),
            completions: Value::Null,
            hits: vec![],
            quit: false,
            confirm_yes: false,
            failed_form: None,
            last_refresh: Instant::now(),
            last_agent_refresh: Instant::now(),
            last_save: Instant::now(),
            dirty: false,
        };
        app.enqueue(Job::Discover(app.roots.clone(), app.recursive));
        for project in app
            .tabs
            .iter()
            .map(|t| t.project.clone())
            .collect::<Vec<_>>()
        {
            app.enqueue(Job::Trees(project));
        }
        app
    }
    pub fn tab(&self) -> Option<&Tab> {
        self.tabs.get(self.selected_tab)
    }
    pub fn tree(&self) -> Option<&Worktree> {
        self.tab().and_then(|t| t.trees.get(t.selected))
    }
    pub fn snapshot(&self) -> Option<&Snapshot> {
        self.tree().and_then(|t| self.snapshots.get(&t.path))
    }
    pub fn conversation(&self) -> Option<&Conversation> {
        self.tab()
            .and_then(|t| t.conversation.as_ref())
            .and_then(|id| self.state.conversations.iter().find(|c| &c.id == id))
    }
    pub fn sessions(&self) -> Vec<&Conversation> {
        self.tree()
            .map(|t| {
                self.state
                    .conversations
                    .iter()
                    .filter(|c| {
                        same_path(&c.worktree, &t.path) && (!c.archived || self.show_archived())
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn show_archived(&self) -> bool {
        self.state
            .navigation
            .get("@archived")
            .is_some_and(|n| n.changes)
    }
    pub fn busy(&self, path: &Path) -> bool {
        self.state
            .conversations
            .iter()
            .any(|c| same_path(&c.worktree, path) && c.attention.busy())
    }
    pub fn say(&mut self, message: impl Into<String>, error: bool) {
        self.notice = Some((message.into(), error, Instant::now()));
    }
    pub fn enqueue(&mut self, job: Job) {
        if self.pending.insert(job.token()) {
            self.jobs.push(job);
        }
    }
    pub fn visible_projects(&self, query: &str) -> Vec<usize> {
        let q = query.to_lowercase();
        self.projects
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                p.name.to_lowercase().contains(&q)
                    || p.path.to_string_lossy().to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect()
    }
    pub fn matching_branches(&self, form: &Form, query: &str) -> Vec<git::Branch> {
        let project = PathBuf::from(form.context["project"].as_str().unwrap_or_default());
        let mut matches = self
            .branches
            .get(&project)
            .into_iter()
            .flatten()
            .filter_map(|branch| {
                fuzzy_score(&branch.name, query).map(|score| (score, branch.clone()))
            })
            .collect::<Vec<_>>();
        matches.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1.remote.cmp(&b.1.remote))
                .then(a.1.name.cmp(&b.1.name))
        });
        matches.into_iter().map(|(_, b)| b).collect()
    }
    pub fn branch_loading(&self, form: &Form) -> bool {
        self.pending.contains(&format!(
            "branches:{}",
            form.context["project"].as_str().unwrap_or_default()
        ))
    }
    pub fn open_project(&mut self, index: usize) {
        let Some(project) = self.projects.get(index).cloned() else {
            return;
        };
        if let Some(index) = self
            .tabs
            .iter()
            .position(|t| same_path(&t.project.path, &project.path))
        {
            self.selected_tab = index;
        } else {
            let nav = self
                .state
                .navigation
                .get(&project.path.to_string_lossy().to_string())
                .cloned()
                .unwrap_or_default();
            self.tabs.push(Tab {
                project: project.clone(),
                trees: vec![],
                selected: 0,
                view: if nav.changes {
                    View::Changes
                } else {
                    View::Sessions
                },
                conversation: nav.conversation,
                file: 0,
            });
            self.selected_tab = self.tabs.len() - 1;
        }
        self.modal = None;
        self.cli_focused = false;
        self.dirty = true;
        self.enqueue(Job::Remotes(project.clone()));
        self.enqueue(Job::Trees(project));
        self.restore_composer();
    }
    pub fn accept(&mut self, token: String, result: Result<Completion>) {
        self.pending.remove(&token);
        if token == "mutation" {
            self.mutation = None;
        }
        let completion = match result {
            Ok(c) => c,
            Err(e) => {
                self.say(format!("{e:#}"), true);
                if token == "mutation"
                    && let Some(project) = self.tab().map(|t| t.project.clone())
                {
                    self.enqueue(Job::Trees(project));
                }
                if token == "mutation" && self.modal.is_none() {
                    self.modal = self.failed_form.clone().map(Modal::Form);
                }
                return;
            }
        };
        match completion {
            Completion::Branches(path, branches) => {
                self.branches.insert(path, branches);
            }
            Completion::Remotes(path, remotes) => {
                self.remotes.insert(path, remotes);
            }
            Completion::Associated(item) => self.agent_event(agent::Event::Catalog(vec![item])),
            Completion::Projects(projects, warnings) => {
                let mut projects = projects;
                for old in &self.projects {
                    if !old.path.is_dir() && !projects.iter().any(|p| same_path(&p.path, &old.path))
                    {
                        projects.push(old.clone());
                    }
                }
                self.projects = projects;
                self.state.known_projects = self.projects.clone();
                self.dirty = true;
                if !warnings.is_empty() {
                    self.say(warnings.join("; "), true);
                }
            }
            Completion::Trees(project, mut trees) => {
                // Missing checkouts keep their saved conversations readable.
                for c in &self.state.conversations {
                    if same_path(&c.project, &project.path)
                        && !trees.iter().any(|t| same_path(&t.path, &c.worktree))
                    {
                        trees.push(Worktree {
                            path: c.worktree.clone(),
                            branch: "(removed)".into(),
                            head: String::new(),
                            original: false,
                            locked: false,
                            prunable: true,
                        });
                    }
                }
                if let Some(tab) = self
                    .tabs
                    .iter_mut()
                    .find(|t| same_path(&t.project.path, &project.path))
                {
                    let previous =
                        tab.trees
                            .get(tab.selected)
                            .map(|t| t.path.clone())
                            .or_else(|| {
                                self.state
                                    .navigation
                                    .get(&project.path.to_string_lossy().to_string())
                                    .and_then(|n| n.worktree.clone())
                            });
                    tab.selected = previous
                        .and_then(|p| trees.iter().position(|t| same_path(&t.path, &p)))
                        .unwrap_or(0);
                    tab.trees = trees;
                }
                let paths = self
                    .tabs
                    .iter()
                    .find(|t| same_path(&t.project.path, &project.path))
                    .into_iter()
                    .flat_map(|t| &t.trees)
                    .filter(|t| !t.prunable)
                    .map(|t| t.path.clone())
                    .collect::<Vec<_>>();
                for path in paths {
                    self.enqueue(Job::Status(path));
                }
                self.selection_changed();
            }
            Completion::Status(path, snapshot) => {
                let selected = self
                    .tree()
                    .filter(|t| same_path(&t.path, &path))
                    .and_then(|_| self.selected_file().map(|f| f.path.clone()));
                if let Some(tab) = self.tabs.get_mut(self.selected_tab)
                    && tab
                        .trees
                        .get(tab.selected)
                        .is_some_and(|t| same_path(&t.path, &path))
                {
                    tab.file = selected
                        .and_then(|p| snapshot.files.iter().position(|f| f.path == p))
                        .unwrap_or_else(|| tab.file.min(snapshot.files.len().saturating_sub(1)));
                }
                self.snapshots.insert(path.clone(), snapshot);
                if self.tree().is_some_and(|t| same_path(&t.path, &path)) {
                    self.request_diff();
                }
            }
            Completion::Diff(path, file, staged, text) => {
                if self.tree().is_some_and(|t| same_path(&t.path, &path))
                    && self.selected_file().is_some_and(|f| f.path == file)
                    && self.diff_staged == staged
                {
                    self.diff = text;
                }
            }
            Completion::Operated {
                project,
                target,
                created,
                session,
            } => {
                self.say("Operation completed", false);
                self.failed_form = None;
                self.marked.clear();
                if let Some(path) = created {
                    self.state
                        .navigation
                        .entry(project.to_string_lossy().into())
                        .or_default()
                        .worktree = Some(path.clone());
                    if let Some(tab) = self
                        .tabs
                        .iter_mut()
                        .find(|t| same_path(&t.project.path, &project))
                    {
                        tab.trees.push(Worktree {
                            path: path.clone(),
                            branch: String::new(),
                            head: String::new(),
                            original: false,
                            locked: false,
                            prunable: false,
                        });
                        tab.selected = tab.trees.len() - 1;
                    }
                    if let Some((prompt, config)) = session {
                        self.start_conversation(
                            project.clone(),
                            path,
                            prompt,
                            String::new(),
                            config,
                        );
                    }
                }
                self.enqueue(Job::Trees(Project {
                    name: project
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into(),
                    path: project,
                }));
                if target.is_dir() {
                    self.enqueue(Job::Status(target));
                }
            }
        }
    }
    pub fn selected_file(&self) -> Option<&ChangedFile> {
        self.snapshot().and_then(|s| s.files.get(self.tab()?.file))
    }
    pub fn request_diff(&mut self) {
        if self.tab().is_none_or(|t| t.view != View::Changes) {
            return;
        }
        if let (Some(tree), Some(file)) = (self.tree(), self.selected_file()) {
            self.enqueue(Job::Diff(tree.path.clone(), file.clone(), self.diff_staged));
        } else {
            self.diff.clear();
        }
    }
    pub fn selection_changed(&mut self) {
        self.diff_scroll = 0;
        self.transcript_scroll = 0;
        self.marked.clear();
        self.cli_focused = false;
        self.dirty = true;
        if let Some(path) = self.tree().filter(|t| !t.prunable).map(|t| t.path.clone()) {
            self.enqueue(Job::Status(path));
        }
        let current = self.tab().and_then(|t| t.conversation.clone());
        let sessions = self.sessions();
        let chosen = sessions
            .iter()
            .find(|s| Some(&s.id) == current.as_ref())
            .or_else(|| sessions.first())
            .map(|s| s.id.clone());
        if let Some(tab) = self.tabs.get_mut(self.selected_tab) {
            tab.conversation = chosen;
        }
        self.restore_composer();
        self.request_diff();
    }
    fn restore_composer(&mut self) {
        self.composer = Input::new(
            self.conversation()
                .map(|c| c.draft.clone())
                .unwrap_or_default(),
        );
        self.attachments.clear();
    }
    fn sync_draft(&mut self) {
        if let Some(id) = self.tab().and_then(|t| t.conversation.clone())
            && let Some(c) = self.state.conversations.iter_mut().find(|c| c.id == id)
        {
            c.draft = self.composer.value.clone();
            self.dirty = true;
        }
    }
    pub fn operate(&mut self, operation: Operation) {
        let (Some(project), Some(tree)) = (
            self.tab().map(|t| t.project.path.clone()),
            self.tree().cloned(),
        ) else {
            return;
        };
        if self.mutation.is_some() {
            self.say("Wait for the current Git operation", true);
            return;
        }
        if !matches!(operation, Operation::Fetch | Operation::Create { .. })
            && self.busy(&tree.path)
        {
            self.say(
                "Agent active: wait for idle or stop its turn before Git writes",
                true,
            );
            return;
        }
        if tree.prunable && !matches!(operation, Operation::Create { .. }) {
            self.say(
                "This checkout is missing; its conversations are read-only",
                true,
            );
            return;
        }
        self.mutation = Some((tree.path.clone(), operation.label().into()));
        self.operation_output.clear();
        self.enqueue(Job::Operate {
            project,
            target: tree.path,
            operation,
        });
    }
    fn creation_form(&mut self, session: bool) {
        let Some(tree) = self.tree().cloned() else {
            return;
        };
        let Some(project) = self.tab().map(|t| t.project.path.clone()) else {
            return;
        };
        let mut fields = Vec::new();
        if session {
            fields.push(Field {
                key: "prompt".into(),
                label: "First prompt (optional)".into(),
                input: Input::default(),
                kind: FieldType::Multiline,
            });
            fields.push(Field::toggle("new_tree", "Create in a new worktree", false));
        }
        fields.extend([
            Field {
                key: "base".into(),
                label: "Starting point".into(),
                input: Input::new(if tree.branch == "(detached)" {
                    tree.head.clone()
                } else {
                    tree.branch.clone()
                }),
                kind: FieldType::Branch,
            },
            Field {
                key: "mode".into(),
                label: "Branch".into(),
                input: Input::new("Create new branch"),
                kind: FieldType::Choice(vec![
                    "Create new branch".into(),
                    "Use existing branch".into(),
                ]),
            },
            Field::text("branch", "New branch name", ""),
        ]);
        if session {
            fields.push(Field::text("name", "Conversation name · optional", ""));
            fields.push(self.model_field(None));
            fields.extend(config_fields(&self.provider_options));
        }
        self.modal = Some(Modal::Form(Form {
            kind: if session {
                FormKind::Session
            } else {
                FormKind::Worktree
            },
            fields,
            focused: 0,
            advanced: false,
            title: if session {
                "New Copilot session"
            } else {
                "New worktree"
            }
            .into(),
            context: json!({"worktree":tree.path,"project":project,"base_label":tree.branch,"base_ref":format!("refs/heads/{}", tree.branch)}),
        }));
        if let Some(project) = self.tab().map(|t| t.project.clone()) {
            self.enqueue(Job::Branches(project));
        }
        if session {
            self.agent_commands.push(agent::Command::Options {
                worktree: tree.path,
            });
        }
    }
    fn start_conversation(
        &mut self,
        project: PathBuf,
        worktree: PathBuf,
        prompt: String,
        name: String,
        config: Value,
    ) {
        if self.busy(&worktree) {
            self.say("This worktree already has an active turn", true);
            return;
        }
        let id = format!("ahp-session:/{}", uuid::Uuid::new_v4());
        let name = if name.trim().is_empty() {
            config
                .get("_treerailName")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        } else {
            name
        };
        let title = if name.trim().is_empty() {
            prompt
                .lines()
                .next()
                .unwrap_or("New conversation")
                .chars()
                .take(64)
                .collect()
        } else {
            name
        };
        self.state.conversations.push(Conversation {
            id: id.clone(),
            project: project.clone(),
            worktree,
            title,
            session_uri: id.clone(),
            chat_uri: None,
            attention: Attention::Running,
            archived: false,
            draft: String::new(),
            chat: None,
            error: None,
            pending: vec![],
            options: Value::Null,
            owned: true,
            initial_prompt: (!prompt.trim().is_empty()).then_some(prompt.clone()),
            creation_failed: false,
            model: config
                .get("_treerailModel")
                .and_then(Value::as_str)
                .map(str::to_owned),
        });
        if let Some(tab) = self
            .tabs
            .iter_mut()
            .find(|t| same_path(&t.project.path, &project))
        {
            tab.conversation = Some(id.clone());
            tab.view = View::Sessions;
        }
        let worktree = self
            .state
            .conversations
            .last()
            .expect("created")
            .worktree
            .clone();
        self.agent_commands.push(agent::Command::Create {
            id: id.clone(),
            worktree,
            prompt,
            config,
        });
        let model = self
            .state
            .conversations
            .last()
            .and_then(|c| c.model.clone());
        self.terminal_requested.insert(id, model);
        self.modal = None;
        self.dirty = true;
        self.restore_composer();
    }
    fn submit_form(&mut self, form: Form) -> Result<()> {
        match form.kind {
            FormKind::Session | FormKind::Worktree => {
                anyhow::ensure!(
                    self.mutation.is_none(),
                    "Wait for the current Git operation"
                );
                let is_session = form.kind == FormKind::Session;
                let makes_tree = !is_session || form.value("new_tree") == "yes";
                let prompt = form.value("prompt");
                let project = self.tab().context("Choose a project")?.project.path.clone();
                let tree = self.tree().context("Choose a worktree")?.path.clone();
                let mut options = self.provider_options["values"]
                    .as_object()
                    .cloned()
                    .unwrap_or_default();
                if !form.value("model").is_empty() {
                    options.insert("_treerailModel".into(), json!(form.value("model")));
                }
                if !form.value("name").is_empty() {
                    options.insert("_treerailName".into(), json!(form.value("name")));
                }
                for field in &form.fields {
                    if let Some(key) = field.key.strip_prefix("config:") {
                        options.insert(
                            key.into(),
                            field_value(
                                field,
                                &self.provider_options["schema"]["properties"][key],
                            )?,
                        );
                    }
                }
                if makes_tree {
                    self.failed_form = Some(form.clone());
                    anyhow::ensure!(
                        !self.branch_loading(&form),
                        "Fetching branches; wait for the starting points to load"
                    );
                    let base = form.value("base").trim().to_owned();
                    anyhow::ensure!(!base.is_empty(), "Select a starting point");
                    let base = if form.context["base_label"].as_str() == Some(&base) {
                        form.context["base_ref"]
                            .as_str()
                            .unwrap_or(&base)
                            .to_owned()
                    } else {
                        base
                    };
                    let existing = form.value("mode") == "Use existing branch";
                    let branch = if existing {
                        self.branches
                            .get(&project)
                            .into_iter()
                            .flatten()
                            .find(|b| b.reference() == base || b.name == base)
                            .context("Choose an existing branch from the picker")?
                            .checkout_name
                            .clone()
                    } else {
                        form.value("branch").trim().to_owned()
                    };
                    anyhow::ensure!(!branch.is_empty(), "Enter a new branch name");
                    let destination = self.config.worktree_path(&project, &branch);
                    self.operate(Operation::Create {
                        branch,
                        base,
                        destination,
                        source: tree,
                        existing,
                        session: if is_session {
                            Some((prompt, Value::Object(options)))
                        } else {
                            None
                        },
                    });
                    anyhow::ensure!(self.mutation.is_some(), "Worktree creation could not start");
                } else {
                    anyhow::ensure!(
                        !self.busy(&tree),
                        "Agent active: stop its turn or create a new worktree"
                    );
                    anyhow::ensure!(
                        self.mutation
                            .as_ref()
                            .is_none_or(|(p, _)| !same_path(p, &tree)),
                        "Wait for Git to finish"
                    );
                    anyhow::ensure!(tree.is_dir(), "Checkout is missing");
                    self.failed_form = Some(form.clone());
                    self.start_conversation(
                        project,
                        tree,
                        prompt,
                        form.value("name"),
                        Value::Object(options),
                    );
                }
            }
            FormKind::Commit => {
                anyhow::ensure!(
                    !form.value("subject").trim().is_empty(),
                    "Enter a commit subject"
                );
                anyhow::ensure!(
                    self.snapshot()
                        .is_some_and(|s| s.files.iter().any(ChangedFile::staged)),
                    "Stage files before committing"
                );
                self.operate(Operation::Commit(form.value("subject"), form.value("body")));
            }
            FormKind::AddPath => {
                let path = std::path::absolute(form.value("path"))?;
                anyhow::ensure!(path.is_dir(), "That directory does not exist");
                if !self
                    .state
                    .remembered_sources
                    .iter()
                    .any(|p| same_path(p, &path))
                {
                    self.state.remembered_sources.push(path.clone());
                }
                if !self.roots.iter().any(|p| same_path(p, &path)) {
                    self.roots.push(path);
                }
                self.dirty = true;
                self.enqueue(Job::Discover(self.roots.clone(), self.recursive));
            }
            FormKind::Attach => {
                let path = PathBuf::from(form.value("path"));
                let path = if path.is_absolute() {
                    path
                } else {
                    self.tree().context("Choose a worktree")?.path.join(path)
                };
                let path = path.canonicalize()?;
                anyhow::ensure!(path.is_file(), "Choose a file");
                let uri = url::Url::from_file_path(&path)
                    .map_err(|_| anyhow::anyhow!("Invalid file path"))?;
                self.attachments.push(json!({"type":"resource","label":path.file_name().unwrap_or_default().to_string_lossy(),"uri":uri.as_str()}));
            }
            FormKind::Question => {
                let mut answers = serde_json::Map::new();
                for field in &form.fields {
                    let kind = form.context["questions"]
                        .as_array()
                        .and_then(|qs| qs.iter().find(|q| q["id"].as_str() == Some(&field.key)))
                        .and_then(|q| q["kind"].as_str())
                        .unwrap_or("text");
                    let (kind, value) = match kind {
                        "number" => (
                            "number",
                            serde_json::from_str::<Value>(&field.input.value)
                                .context("Enter a valid number")?,
                        ),
                        "boolean" => ("boolean", json!(field.input.value == "yes")),
                        "single-select" => ("selected", json!(field.input.value)),
                        "multi-select" => (
                            "selected-many",
                            json!(
                                field
                                    .input
                                    .value
                                    .split(',')
                                    .map(str::trim)
                                    .filter(|s| !s.is_empty())
                                    .collect::<Vec<_>>()
                            ),
                        ),
                        _ => ("text", json!(field.input.value)),
                    };
                    answers.insert(
                        field.key.clone(),
                        json!({"state":"submitted","value":{"kind":kind,"value":value}}),
                    );
                }
                let channel = form.context["chat"]
                    .as_str()
                    .context("Question has no chat channel")?
                    .to_owned();
                self.agent_commands.push(agent::Command::Dispatch{channel,worktree:None,action:json!({"type":"chat/inputCompleted","requestId":form.context["requestId"],"response":"accept","answers":answers})});
            }
            FormKind::SessionOptions => {
                let mut values = serde_json::Map::new();
                for field in &form.fields {
                    if field.key == "model" {
                        continue;
                    }
                    let key = field.key.trim_start_matches("config:");
                    values.insert(
                        key.into(),
                        field_value(field, &form.context["schema"]["properties"][key])?,
                    );
                }
                let channel = self
                    .conversation()
                    .context("Choose a conversation")?
                    .session_uri
                    .clone();
                if let Some(c) = self
                    .state
                    .conversations
                    .iter_mut()
                    .find(|c| c.session_uri == channel)
                {
                    c.model = if form.value("model").is_empty() {
                        None
                    } else {
                        Some(form.value("model"))
                    };
                    self.dirty = true;
                }
                if !values.is_empty() {
                    self.agent_commands.push(agent::Command::Dispatch {
                        channel,
                        worktree: None,
                        action: json!({"type":"session/configChanged","config":values}),
                    });
                }
            }
            FormKind::PublishRemote => {
                self.operate(Operation::Push(Some(form.value("remote"))));
            }
            FormKind::Preferences => {
                let baseline: Config = serde_json::from_value(form.context.clone())
                    .unwrap_or_else(|_| self.config.clone());
                let mut desired = baseline.clone();
                desired.recursive = form.value("recursive") == "yes";
                desired.worktrees.path_template = form.value("template");
                let source_paths = form
                    .value("sources")
                    .lines()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .collect::<Vec<_>>();
                desired.project_roots = crate::startup::normalize_sources(
                    source_paths,
                    self.config_path.parent().unwrap_or(Path::new(".")),
                )?;
                let remote = form.value("remote");
                desired.publication_remote = if remote.is_empty() {
                    None
                } else {
                    Some(remote)
                };
                self.config = config::save_preferences(&self.config_path, &baseline, &desired)?;
                self.recursive = self.config.recursive;
                self.roots = crate::startup::normalize_sources(
                    self.config
                        .project_roots
                        .iter()
                        .cloned()
                        .chain(self.state.remembered_sources.clone()),
                    self.config_path.parent().unwrap_or(Path::new(".")),
                )?;
                if self.roots.is_empty() {
                    self.roots.push(std::env::current_dir()?);
                }
                self.enqueue(Job::Discover(self.roots.clone(), self.recursive));
                self.say(
                    "Preferences saved; remembered paths remain in state.json",
                    false,
                );
            }
        }
        self.modal = None;
        Ok(())
    }
    fn simple_form(&mut self, kind: FormKind, title: &str, fields: Vec<Field>) {
        let context = if kind == FormKind::Preferences {
            serde_json::to_value(&self.config).unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        self.modal = Some(Modal::Form(Form {
            kind,
            title: title.into(),
            fields,
            focused: 0,
            advanced: false,
            context,
        }));
    }
    pub fn action(&mut self, action: Action) {
        if matches!(self.modal, Some(Modal::Confirm(Confirm::Trust { .. })))
            && matches!(
                action,
                Action::Projects | Action::Preferences | Action::Close(_)
            )
        {
            self.action(Action::Cancel);
        }
        match action {
            Action::PickBranch => {
                if matches!(self.modal, Some(Modal::Form(_)))
                    && let Some(Modal::Form(form)) = self.modal.take()
                {
                    self.modal = Some(Modal::Branches {
                        form: Box::new(form),
                        query: Input::default(),
                        selected: 0,
                    });
                }
            }
            Action::ChooseBranch(index) => {
                if let Some(Modal::Branches {
                    mut form, query, ..
                }) = self.modal.clone()
                    && let Some(branch) = self.matching_branches(&form, &query.value).get(index)
                {
                    if let Some(field) = form.fields.iter_mut().find(|f| f.key == "base") {
                        field.input = Input::new(&branch.name);
                    }
                    form.context["base_label"] = json!(branch.name);
                    form.context["base_ref"] = json!(branch.reference());
                    form.focused = form
                        .fields
                        .iter()
                        .position(|f| f.key == "mode")
                        .unwrap_or(0);
                    self.modal = Some(Modal::Form(*form));
                }
            }
            Action::Project(i) => self.open_project(i),
            Action::Tab(i) => {
                if i < self.tabs.len() {
                    self.selected_tab = i;
                    self.modal = None;
                    self.selection_changed();
                }
            }
            Action::Close(i) => {
                if let Some(tab) = self.tabs.get(i) {
                    if self
                        .state
                        .conversations
                        .iter()
                        .any(|c| same_path(&c.project, &tab.project.path) && c.attention.busy())
                    {
                        self.modal = Some(Modal::Confirm(Confirm::Close(i)));
                        self.confirm_yes = false;
                    } else {
                        self.close_tab(i);
                    }
                }
            }
            Action::Tree(i) => {
                self.pane = Pane::Worktrees;
                if let Some(tab) = self.tabs.get_mut(self.selected_tab)
                    && i < tab.trees.len()
                {
                    tab.selected = i;
                    tab.file = 0;
                    self.selection_changed();
                }
            }
            Action::Session(id) => {
                self.cli_focused = false;
                self.pane = Pane::List;
                if let Some(tab) = self.tabs.get_mut(self.selected_tab) {
                    tab.conversation = Some(id);
                }
                self.restore_composer();
                self.transcript_scroll = 0;
                self.dirty = true;
            }
            Action::File(i) => {
                self.pane = Pane::List;
                if let Some(tab) = self.tabs.get_mut(self.selected_tab) {
                    tab.file = i;
                }
                self.diff_scroll = 0;
                self.request_diff();
            }
            Action::Switch(view) => {
                if let Some(tab) = self.tabs.get_mut(self.selected_tab) {
                    tab.view = view;
                }
                self.cli_focused = false;
                self.dirty = true;
                self.request_diff();
            }
            Action::NewTree => self.creation_form(false),
            Action::NewSession => {
                if let (Some(tree), Some(project)) = (
                    self.tree().cloned(),
                    self.tab().map(|t| t.project.path.clone()),
                ) {
                    if !tree.path.is_dir() || tree.prunable {
                        self.say("Checkout is missing", true);
                    } else if self
                        .mutation
                        .as_ref()
                        .is_some_and(|(p, _)| same_path(p, &tree.path))
                    {
                        self.say("Wait for Git to finish", true);
                    } else {
                        self.failed_form = None;
                        self.start_conversation(
                            project,
                            tree.path,
                            String::new(),
                            String::new(),
                            json!({}),
                        );
                        self.pane = Pane::Detail;
                    }
                }
            }
            Action::NewSessionOptions => self.creation_form(true),
            Action::Help => self.modal = Some(Modal::Help(0)),
            Action::Projects => {
                self.modal = Some(Modal::Projects {
                    query: Input::default(),
                    selected: 0,
                })
            }
            Action::Cancel => {
                if matches!(self.modal, Some(Modal::Branches { .. }))
                    && let Some(Modal::Branches { form, .. }) = self.modal.take()
                {
                    self.modal = Some(Modal::Form(*form));
                    return;
                }
                if let Some(Modal::Confirm(Confirm::Trust { id, .. })) = &self.modal {
                    self.agent_commands.push(agent::Command::TrustWorkspace {
                        id: id.clone(),
                        approved: false,
                    });
                }
                self.modal = None;
                self.cli_focused = false;
            }
            Action::Confirm => {
                if let Some(modal) = self.modal.clone() {
                    match modal {
                        Modal::Branches { selected, .. } => {
                            self.action(Action::ChooseBranch(selected))
                        }
                        Modal::Confirm(Confirm::Trust { id, .. }) => {
                            self.modal = None;
                            self.agent_commands
                                .push(agent::Command::TrustWorkspace { id, approved: true });
                        }
                        Modal::Confirm(Confirm::Close(i)) => self.close_tab(i),
                        Modal::Confirm(Confirm::Quit) => {
                            self.quit = true;
                            self.modal = None;
                        }
                        Modal::Confirm(Confirm::Remove) => {
                            self.modal = None;
                            self.operate(Operation::Remove);
                        }
                        Modal::Form(form) => {
                            if let Err(e) = self.submit_form(form) {
                                self.say(format!("{e:#}"), true);
                            }
                        }
                        Modal::Projects { query, selected } => {
                            if let Some(i) = self.visible_projects(&query.value).get(selected) {
                                self.open_project(*i);
                            }
                        }
                        Modal::Help(selected) => {
                            if let Some((_, _, action)) = self.help_actions().get(selected).cloned()
                            {
                                self.modal = None;
                                self.action(action);
                            }
                        }
                        Modal::Activity(selected) => {
                            if let Some(c) = self
                                .state
                                .conversations
                                .iter()
                                .filter(|c| c.attention.busy())
                                .nth(selected)
                                .cloned()
                                && let Some(index) = self
                                    .projects
                                    .iter()
                                    .position(|p| same_path(&p.path, &c.project))
                            {
                                self.open_project(index);
                                if let Some(tab) = self.tabs.get_mut(self.selected_tab) {
                                    tab.conversation = Some(c.id);
                                    if let Some(i) = tab
                                        .trees
                                        .iter()
                                        .position(|t| same_path(&t.path, &c.worktree))
                                    {
                                        tab.selected = i;
                                    }
                                }
                                self.restore_composer();
                            }
                        }
                        _ => {
                            self.modal = None;
                        }
                    }
                }
            }
            Action::Git(which) => match which {
                "add-path" => self.simple_form(
                    FormKind::AddPath,
                    "Remember a project path",
                    vec![Field::text("path", "Repository or discovery folder", "")],
                ),
                "stage" | "unstage" => {
                    if self.tab().is_none_or(|t| t.view != View::Changes) {
                        self.say("Select files in Changes before staging or unstaging", true);
                        return;
                    }
                    let files = self
                        .snapshot()
                        .map(|s| {
                            s.files
                                .iter()
                                .filter(|f| {
                                    if self.marked.is_empty() {
                                        self.selected_file()
                                            .is_some_and(|selected| selected.path == f.path)
                                    } else {
                                        self.marked.contains(&f.path)
                                    }
                                })
                                .cloned()
                                .collect()
                        })
                        .unwrap_or_default();
                    self.operate(Operation::Stage(files, which == "unstage"));
                }
                "commit" => self.simple_form(
                    FormKind::Commit,
                    "Commit staged changes",
                    vec![
                        Field::text("subject", "Subject", ""),
                        Field {
                            key: "body".into(),
                            label: "Body · optional".into(),
                            input: Input::default(),
                            kind: FieldType::Multiline,
                        },
                    ],
                ),
                "fetch" => self.operate(Operation::Fetch),
                "pull" => self.operate(Operation::Pull),
                "push" => {
                    let remote = self.tab().and_then(|t| self.config.remote(&t.project.path));
                    let choices = self
                        .tab()
                        .and_then(|t| self.remotes.get(&t.project.path))
                        .cloned()
                        .unwrap_or_default();
                    if remote.is_none()
                        && self.snapshot().is_some_and(|s| s.upstream.is_none())
                        && choices.len() > 1
                        && !choices.iter().any(|r| r == "origin")
                    {
                        let field = Field {
                            key: "remote".into(),
                            label: "Publish to remote".into(),
                            input: Input::new(choices[0].clone()),
                            kind: FieldType::Choice(choices),
                        };
                        self.simple_form(FormKind::PublishRemote, "Publish branch", vec![field]);
                        return;
                    }
                    self.operate(Operation::Push(remote));
                }
                "remove" => {
                    if let Some(tree) = self.tree() {
                        if tree.original
                            || tree.prunable
                            || self.busy(&tree.path)
                            || self.snapshot().is_none_or(|s| !s.files.is_empty())
                        {
                            self.say("Removal requires a clean, idle linked worktree", true);
                        } else {
                            self.modal = Some(Modal::Confirm(Confirm::Remove));
                            self.confirm_yes = false;
                        }
                    }
                }
                _ => {}
            },
            Action::Stop => {
                let active = self
                    .tree()
                    .and_then(|tree| {
                        self.state
                            .conversations
                            .iter()
                            .find(|c| same_path(&c.worktree, &tree.path) && c.attention.busy())
                    })
                    .cloned();
                if let Some(c) = active
                    && let Some(turn) = c.chat.as_ref().and_then(|chat| chat.active_turn.as_ref())
                    && let Some(channel) = c.chat_uri
                {
                    self.agent_commands.push(agent::Command::Dispatch {
                        channel,
                        worktree: None,
                        action: json!({"type":"chat/turnCancelled","turnId":turn.id,"duration":0}),
                    });
                }
            }
            Action::Resume => {
                if let Some(c) = self.conversation().cloned() {
                    if c.worktree.is_dir() {
                        if self.terminals.get(&c.id).is_some_and(|t| !t.running()) {
                            self.terminals.close(&c.id);
                        }
                        self.terminal_requested.insert(c.id.clone(), None);
                        self.agent_commands
                            .push(agent::Command::Observe { id: c.session_uri });
                        self.say(
                            "Connecting to saved conversation; no prompt was resent",
                            false,
                        );
                    } else {
                        self.say("Checkout is missing; history is read-only", true);
                    }
                }
            }
            Action::Archive => {
                if let Some(id) = self
                    .conversation()
                    .filter(|c| !c.attention.busy())
                    .map(|c| c.id.clone())
                {
                    if let Some(c) = self.state.conversations.iter_mut().find(|c| c.id == id) {
                        c.archived = !c.archived;
                        self.dirty = true;
                    }
                    self.selection_changed();
                }
            }
            Action::History => {
                let nav = self.state.navigation.entry("@archived".into()).or_default();
                nav.changes = !nav.changes;
                self.dirty = true;
                self.selection_changed();
            }
            Action::Quit => {
                if self.mutation.is_some() {
                    self.say("Wait for the Git operation to finish before quitting", true);
                    return;
                }
                if self.state.conversations.iter().any(|c| c.attention.busy()) {
                    self.modal = Some(Modal::Confirm(Confirm::Quit));
                    self.confirm_yes = false;
                } else {
                    self.quit = true;
                }
            }
            Action::Activity => self.modal = Some(Modal::Activity(0)),
            Action::Host => self.modal = Some(Modal::Host),
            Action::FocusTerminal => {
                let live = self
                    .conversation()
                    .is_some_and(|c| self.terminals.get(&c.id).is_some_and(|t| t.running()));
                if live {
                    self.cli_focused = true;
                    self.pane = Pane::Detail;
                } else {
                    self.action(Action::Resume);
                }
            }
            Action::CloseTerminal => {
                if let Some(id) = self.conversation().map(|c| c.id.clone()) {
                    self.terminals.close(&id);
                    self.terminal_requested.remove(&id);
                }
                self.cli_focused = false;
            }
            Action::ToggleAdvanced => {
                if let Some(Modal::Form(form)) = &mut self.modal {
                    form.advanced = !form.advanced;
                }
            }
            Action::Options => {
                if let Some(c) = self.conversation() {
                    let options = c.options.clone();
                    let mut fields = config_fields(&options);
                    fields.insert(0, self.model_field(c.model.as_deref()));
                    if fields.is_empty() {
                        self.modal = Some(Modal::Notice(
                            "Resume the conversation to load its provider options.".into(),
                        ));
                    } else {
                        self.modal = Some(Modal::Form(Form {
                            kind: FormKind::SessionOptions,
                            title: "Copilot session options".into(),
                            fields,
                            focused: 0,
                            advanced: true,
                            context: options,
                        }));
                    }
                }
            }
            Action::Attach => self.simple_form(
                FormKind::Attach,
                "Attach a file or image",
                vec![Field::text("path", "File path", "")],
            ),
            Action::Preferences => self.simple_form(
                FormKind::Preferences,
                "Preferences",
                vec![
                    Field {
                        key: "sources".into(),
                        label: "Declared discovery sources · one per line".into(),
                        input: Input::new(
                            self.config
                                .project_roots
                                .iter()
                                .map(|p| p.display().to_string())
                                .collect::<Vec<_>>()
                                .join("\n"),
                        ),
                        kind: FieldType::Multiline,
                    },
                    Field::toggle(
                        "recursive",
                        "Recursive project discovery",
                        self.config.recursive,
                    ),
                    Field::text(
                        "template",
                        "Worktree directory template",
                        self.config.worktrees.path_template.clone(),
                    ),
                    Field::text(
                        "remote",
                        "Publication remote · optional",
                        self.config.publication_remote.clone().unwrap_or_default(),
                    ),
                ],
            ),
            Action::Field(i) => {
                if let Some(Modal::Form(form)) = &mut self.modal {
                    form.focused = i;
                    if let Some(field) = form.fields.get_mut(i) {
                        match &field.kind {
                            FieldType::Toggle => {
                                field.input = Input::new(if field.input.value == "yes" {
                                    "no"
                                } else {
                                    "yes"
                                })
                            }
                            FieldType::Choice(choices) if !choices.is_empty() => {
                                let selected = choices
                                    .iter()
                                    .position(|c| *c == field.input.value)
                                    .unwrap_or(0);
                                field.input = Input::new(&choices[(selected + 1) % choices.len()]);
                            }
                            _ => {}
                        }
                    }
                }
            }
            Action::Scroll(delta) => {
                self.pane = Pane::Detail;
                if self.tab().is_some_and(|t| t.view == View::Changes) {
                    self.diff_scroll = self.diff_scroll.saturating_add_signed(delta);
                } else {
                    self.transcript_scroll = self.transcript_scroll.saturating_add_signed(delta);
                }
            }
            Action::Expand => {
                self.expanded = !self.expanded;
                self.pane = Pane::Detail;
            }
            Action::Respond => {
                if let Some(request) = self.conversation().and_then(|c| c.pending.first()).cloned()
                {
                    if request["kind"] == "toolConfirmation" {
                        self.modal = Some(Modal::Approval(request));
                    } else if request["kind"] == "chatInput" {
                        let mut fields = vec![];
                        for question in request["request"]["questions"]
                            .as_array()
                            .into_iter()
                            .flatten()
                        {
                            let Some(id) = question["id"].as_str() else {
                                continue;
                            };
                            let label = question["title"]
                                .as_str()
                                .or_else(|| question["message"].as_str())
                                .unwrap_or(id);
                            let mut field = Field::text(id, label, "");
                            if question["kind"] == "boolean" {
                                field = Field::toggle(id, label, false);
                            } else if question["kind"] == "single-select" {
                                let choices = question["options"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .filter_map(|o| o["id"].as_str().map(str::to_owned))
                                    .collect::<Vec<_>>();
                                field.input =
                                    Input::new(choices.first().cloned().unwrap_or_default());
                                field.kind = FieldType::Choice(choices);
                            }
                            fields.push(field);
                        }
                        if fields.is_empty() {
                            fields.push(Field::toggle("confirm", "Accept this request", true));
                        }
                        self.modal = Some(Modal::Form(Form {
                            kind: FormKind::Question,
                            title: request["request"]["message"]
                                .as_str()
                                .unwrap_or("Copilot needs input")
                                .into(),
                            fields,
                            focused: 0,
                            advanced: false,
                            context: json!({"chat":request["chat"],"requestId":request["request"]["id"],"questions":request["request"]["questions"]}),
                        }));
                    } else {
                        self.modal = Some(Modal::Notice(
                            serde_json::to_string_pretty(&request).unwrap_or_default(),
                        ));
                    }
                }
            }
            Action::Approve(approved, option) => {
                if let Some(Modal::Approval(request)) = &self.modal {
                    let channel = request["chat"].as_str().unwrap_or_default().to_owned();
                    let result = request["toolCall"]["status"] == "pending-result-confirmation";
                    let action = json!({"type":if result{"chat/toolCallResultConfirmed"}else{"chat/toolCallConfirmed"},"turnId":request["turnId"],"toolCallId":request["toolCall"]["toolCallId"],"approved":approved,"confirmed":if approved{Some("user-action")}else{None},"reason":if approved{None}else{Some("denied")},"selectedOptionId":option});
                    self.agent_commands.push(agent::Command::Dispatch {
                        channel,
                        action,
                        worktree: None,
                    });
                    self.modal = None;
                }
            }
            Action::Completion(index) => {
                if let Some(Modal::Completions { items, .. }) = &self.modal
                    && let Some(item) = items.get(index)
                {
                    let start = item["rangeStart"]
                        .as_u64()
                        .and_then(|i| utf16_byte(&self.composer.value, i as usize))
                        .unwrap_or(self.composer.cursor);
                    let end = item["rangeEnd"]
                        .as_u64()
                        .and_then(|i| utf16_byte(&self.composer.value, i as usize))
                        .unwrap_or(self.composer.cursor);
                    if start <= end && end <= self.composer.value.len() {
                        self.composer.value.replace_range(
                            start..end,
                            item["insertText"].as_str().unwrap_or_default(),
                        );
                        self.composer.cursor =
                            start + item["insertText"].as_str().unwrap_or_default().len();
                        if let Some(attachment) = item.get("attachment") {
                            self.attachments.push(attachment.clone());
                        }
                    }
                }
                self.modal = None;
                self.sync_draft();
            }
            Action::Details => {
                self.modal = Some(Modal::Notice(format!(
                    "{}\n\n{}",
                    self.notice
                        .as_ref()
                        .map(|(m, _, _)| m.as_str())
                        .unwrap_or("No operation result yet"),
                    self.operation_output
                )))
            }
            Action::Retry => {
                if let Some(form) = &self.failed_form {
                    self.modal = Some(Modal::Form(form.clone()));
                } else {
                    self.say("No failed form to retry", false);
                }
            }
            Action::WorktreeInfo => {
                if let Some(tree) = self.tree() {
                    let tracking = self
                        .snapshot()
                        .map(|s| {
                            format!(
                                "{} · ↑{} ↓{}",
                                s.upstream.as_deref().unwrap_or("No upstream"),
                                s.ahead,
                                s.behind
                            )
                        })
                        .unwrap_or_else(|| "Git status loading".into());
                    self.modal = Some(Modal::Notice(format!(
                        "Worktree: {}\nBranch: {}\nHEAD: {}\nTracking: {}\n\n{}",
                        tree.path.file_name().unwrap_or_default().to_string_lossy(),
                        tree.branch,
                        tree.head,
                        tracking,
                        tree.path.display()
                    )));
                }
            }
            Action::Sources => self.modal = Some(Modal::Sources(0)),
            Action::ForgetSource(index) => {
                if index < self.state.remembered_sources.len() {
                    let path = self.state.remembered_sources.remove(index);
                    if !self
                        .config
                        .project_roots
                        .iter()
                        .any(|p| same_path(p, &path))
                    {
                        self.roots.retain(|p| !same_path(p, &path));
                    }
                    self.dirty = true;
                    self.enqueue(Job::Discover(self.roots.clone(), self.recursive));
                }
                self.modal = Some(Modal::Sources(
                    index.min(self.state.remembered_sources.len().saturating_sub(1)),
                ));
            }
        }
    }
    fn close_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.remember_navigation();
            self.tabs.remove(index);
            self.selected_tab = self.selected_tab.min(self.tabs.len().saturating_sub(1));
            self.modal = if self.tabs.is_empty() {
                Some(Modal::Projects {
                    query: Input::default(),
                    selected: 0,
                })
            } else {
                None
            };
            self.dirty = true;
            self.restore_composer();
        }
    }
    pub fn help_actions(&self) -> Vec<(&'static str, &'static str, Action)> {
        vec![
            ("New worktree", "a", Action::NewTree),
            ("New session", "n", Action::NewSession),
            (
                "New session with options / new worktree",
                "",
                Action::NewSessionOptions,
            ),
            ("Worktree details", "", Action::WorktreeInfo),
            ("Remove worktree", "d", Action::Git("remove")),
            ("Stage selected files", "s", Action::Git("stage")),
            ("Unstage selected files", "u", Action::Git("unstage")),
            ("Commit staged files", "c", Action::Git("commit")),
            ("Fetch", "f", Action::Git("fetch")),
            ("Pull · fast-forward only", "P", Action::Git("pull")),
            ("Push / publish branch", "p", Action::Git("push")),
            ("Resume conversation", "r", Action::Resume),
            ("Stop active turn", "Ctrl+C", Action::Stop),
            ("Open / focus Copilot CLI", "i", Action::FocusTerminal),
            ("Return from Copilot CLI", "Ctrl+]", Action::Cancel),
            ("Close terminal view", "", Action::CloseTerminal),
            ("Archive / restore conversation", "e", Action::Archive),
            ("Show archived & removed", "", Action::History),
            ("Projects", "Ctrl+P", Action::Projects),
            ("Remember a path", "+", Action::Git("add-path")),
            ("Remembered locations", "", Action::Sources),
            ("Agent activity", "", Action::Activity),
            ("AHP connection details", "", Action::Host),
            ("Preferences", "Ctrl+,", Action::Preferences),
            ("Operation details", "", Action::Details),
            ("Retry last failed form", "", Action::Retry),
            ("Quit Tree Rail", "Ctrl+Q", Action::Quit),
        ]
    }
    fn model_field(&self, selected: Option<&str>) -> Field {
        let mut choices = vec![String::new()];
        for agent in self
            .agents
            .as_array()
            .into_iter()
            .flatten()
            .filter(|a| a["provider"] == "copilot")
        {
            for model in agent["models"].as_array().into_iter().flatten() {
                if let Some(id) = model["id"].as_str()
                    && !choices.iter().any(|s| s == id)
                {
                    choices.push(id.into());
                }
            }
        }
        Field {
            key: "model".into(),
            label: "Model · provider default when empty".into(),
            input: Input::new(selected.unwrap_or_default()),
            kind: if choices.len() > 1 {
                FieldType::Choice(choices)
            } else {
                FieldType::Text
            },
        }
    }
    fn cli_input_blocked(&self) -> bool {
        self.conversation().is_some_and(|selected| {
            !selected.worktree.is_dir()
                || self
                    .mutation
                    .as_ref()
                    .is_some_and(|(path, _)| same_path(path, &selected.worktree))
                || (!selected.attention.busy()
                    && self.state.conversations.iter().any(|c| {
                        c.id != selected.id
                            && same_path(&c.worktree, &selected.worktree)
                            && c.attention.busy()
                    }))
        })
    }
    fn open_terminal(&mut self, id: &str) {
        if !self.connected {
            return;
        }
        let Some(c) = self
            .state
            .conversations
            .iter()
            .find(|c| c.id == id)
            .cloned()
        else {
            return;
        };
        if !self.attached.contains(&c.session_uri)
            || c.chat_uri.is_none()
            || c.initial_prompt.is_some()
            || c.creation_failed
            || c.error.is_some()
            || !c.worktree.is_dir()
        {
            return;
        }
        let Some(endpoint) = self.connection_url.clone() else {
            return;
        };
        let (rows, cols) = if self.terminal_area.is_empty() {
            (24, 80)
        } else {
            (self.terminal_area.height, self.terminal_area.width)
        };
        self.terminals.open(
            id.into(),
            crate::cli_terminal::CopilotLaunch {
                program: self.config.agent.cli.clone(),
                directory: c.worktree,
                home: self.config.agent.home.clone(),
                endpoint,
                session: c.session_uri,
                model: self.terminal_requested.get(id).cloned().flatten(),
            },
            rows,
            cols,
        );
        self.terminal_requested.remove(id);
        if self.conversation().is_some_and(|c| c.id == id) {
            self.cli_focused = true;
            self.pane = Pane::Detail;
        }
    }
    pub fn key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if self.cli_focused
            && self.modal.is_none()
            && self.tab().is_some_and(|t| t.view == View::Sessions)
        {
            if ctrl && key.code == KeyCode::Char(']') {
                self.cli_focused = false;
                self.pane = Pane::List;
            } else if self.cli_input_blocked() && !(ctrl && key.code == KeyCode::Char('c')) {
                self.say(
                    "Another conversation is active in this worktree. Ctrl+] returns to Tree Rail",
                    true,
                );
            } else if let Some(id) = self.conversation().map(|c| c.id.clone())
                && let Some(terminal) = self.terminals.get_mut(&id)
            {
                terminal.key(key);
            }
            return;
        }
        if key.kind == KeyEventKind::Repeat
            && !matches!(
                key.code,
                KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::Left
                    | KeyCode::Right
                    | KeyCode::Backspace
                    | KeyCode::Delete
                    | KeyCode::Char('j' | 'k' | 'h' | 'l')
            )
            && !self.cli_focused
            && !matches!(self.modal, Some(Modal::Form(_) | Modal::Projects { .. }))
        {
            return;
        }
        if ctrl && key.code == KeyCode::Char('q') {
            self.action(Action::Quit);
            return;
        }
        if ctrl && key.code == KeyCode::Char('p') {
            self.action(Action::Projects);
            return;
        }
        if ctrl && key.code == KeyCode::Char(',') {
            self.action(Action::Preferences);
            return;
        }
        if key.modifiers.contains(KeyModifiers::ALT) && key.code == KeyCode::Char('w') {
            self.action(Action::Close(self.selected_tab));
            return;
        }
        if let Some(mut modal) = self.modal.take() {
            if key.code == KeyCode::Esc {
                self.modal = Some(modal);
                self.action(Action::Cancel);
                self.cli_focused = false;
                return;
            }
            let mut confirm = false;
            let mut add_path = false;
            let mut followup = None;
            match &mut modal {
                Modal::Branches {
                    form,
                    query,
                    selected,
                } => match key.code {
                    KeyCode::Down => {
                        *selected = (*selected + 1).min(
                            self.matching_branches(form, &query.value)
                                .len()
                                .saturating_sub(1),
                        )
                    }
                    KeyCode::Up => *selected = selected.saturating_sub(1),
                    KeyCode::Enter => confirm = true,
                    _ => {
                        query.key(key, false);
                        *selected = 0;
                    }
                },
                Modal::Sources(selected) => match key.code {
                    KeyCode::Down | KeyCode::Char('j') => {
                        *selected = (*selected + 1)
                            .min(self.state.remembered_sources.len().saturating_sub(1))
                    }
                    KeyCode::Up | KeyCode::Char('k') => *selected = selected.saturating_sub(1),
                    KeyCode::Delete => followup = Some(Action::ForgetSource(*selected)),
                    KeyCode::Char('+') => add_path = true,
                    _ => {}
                },
                Modal::Approval(request) => {
                    if key.kind == KeyEventKind::Press {
                        if let Some(options) = request["toolCall"]["options"].as_array() {
                            if let KeyCode::Char(c) = key.code
                                && let Some(option) = c
                                    .to_digit(10)
                                    .and_then(|i| i.checked_sub(1))
                                    .and_then(|i| options.get(i as usize))
                            {
                                followup = Some(Action::Approve(
                                    option["kind"] == "approve",
                                    option["id"].as_str().map(str::to_owned),
                                ));
                            }
                        } else {
                            match key.code {
                                KeyCode::Char('y') => followup = Some(Action::Approve(true, None)),
                                KeyCode::Char('n') => followup = Some(Action::Approve(false, None)),
                                _ => {}
                            }
                        }
                    }
                }
                Modal::Completions { items, selected } => match key.code {
                    KeyCode::Down | KeyCode::Tab => {
                        *selected = (*selected + 1).min(items.len().saturating_sub(1))
                    }
                    KeyCode::Up | KeyCode::BackTab => *selected = selected.saturating_sub(1),
                    KeyCode::Enter => followup = Some(Action::Completion(*selected)),
                    _ => {}
                },
                Modal::Projects { query, selected } => match key.code {
                    KeyCode::Down => {
                        *selected = (*selected + 1)
                            .min(self.visible_projects(&query.value).len().saturating_sub(1))
                    }
                    KeyCode::Up => *selected = selected.saturating_sub(1),
                    KeyCode::Enter => confirm = true,
                    KeyCode::Char('+') if query.value.is_empty() => add_path = true,
                    _ => {
                        query.key(key, false);
                        *selected = 0;
                    }
                },
                Modal::Help(selected) => match key.code {
                    KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                        *selected = (*selected + 1) % self.help_actions().len()
                    }
                    KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                        *selected = selected
                            .checked_sub(1)
                            .unwrap_or(self.help_actions().len() - 1)
                    }
                    KeyCode::Enter => confirm = true,
                    _ => {}
                },
                Modal::Activity(selected) => match key.code {
                    KeyCode::Down | KeyCode::Char('j') => {
                        *selected = (*selected + 1).min(
                            self.state
                                .conversations
                                .iter()
                                .filter(|c| c.attention.busy())
                                .count()
                                .saturating_sub(1),
                        )
                    }
                    KeyCode::Up | KeyCode::Char('k') => *selected = selected.saturating_sub(1),
                    KeyCode::Enter => confirm = true,
                    _ => {}
                },
                Modal::Confirm(_) => {
                    if key.kind == KeyEventKind::Press {
                        match key.code {
                            KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
                                self.confirm_yes = !self.confirm_yes
                            }
                            KeyCode::Char('y') => confirm = true,
                            KeyCode::Char('n') => followup = Some(Action::Cancel),
                            KeyCode::Enter => {
                                if self.confirm_yes {
                                    confirm = true;
                                } else {
                                    followup = Some(Action::Cancel);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Modal::Form(form) => {
                    let visible = form.visible();
                    match key.code {
                        KeyCode::Enter if ctrl => confirm = true,
                        KeyCode::Down | KeyCode::Enter
                            if form
                                .fields
                                .get(form.focused)
                                .is_some_and(|f| matches!(f.kind, FieldType::Branch)) =>
                        {
                            followup = Some(Action::PickBranch)
                        }
                        KeyCode::Char('o') if ctrl => form.advanced = !form.advanced,
                        KeyCode::Tab | KeyCode::BackTab => {
                            let pos = visible.iter().position(|i| *i == form.focused).unwrap_or(0);
                            form.focused = visible[if key.code == KeyCode::Tab {
                                (pos + 1) % visible.len()
                            } else {
                                (pos + visible.len() - 1) % visible.len()
                            }];
                        }
                        _ => {
                            if let Some(field) = form.fields.get_mut(form.focused) {
                                match &field.kind {
                                    FieldType::Toggle
                                        if matches!(
                                            key.code,
                                            KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right
                                        ) =>
                                    {
                                        field.input = Input::new(if field.input.value == "yes" {
                                            "no"
                                        } else {
                                            "yes"
                                        })
                                    }
                                    FieldType::Choice(choices)
                                        if matches!(
                                            key.code,
                                            KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')
                                        ) && !choices.is_empty() =>
                                    {
                                        let i = choices
                                            .iter()
                                            .position(|s| s == &field.input.value)
                                            .unwrap_or(0);
                                        field.input = Input::new(
                                            choices[if key.code == KeyCode::Left {
                                                (i + choices.len() - 1) % choices.len()
                                            } else {
                                                (i + 1) % choices.len()
                                            }]
                                            .clone(),
                                        );
                                    }
                                    FieldType::Text | FieldType::Branch | FieldType::Multiline => {
                                        field
                                            .input
                                            .key(key, matches!(field.kind, FieldType::Multiline))
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                _ => {
                    if key.code == KeyCode::Enter {
                        confirm = true;
                    }
                }
            }
            self.modal = Some(modal);
            if let Some(action) = followup {
                self.action(action);
                return;
            }
            if confirm {
                self.action(Action::Confirm);
            }
            if add_path {
                self.simple_form(
                    FormKind::AddPath,
                    "Remember a project path",
                    vec![Field::text("path", "Repository or discovery folder", "")],
                );
            }
            return;
        }
        if ctrl && key.code == KeyCode::Char('c') {
            self.action(Action::Stop);
            return;
        }
        if ctrl && matches!(key.code, KeyCode::Char('d' | 'u')) {
            self.action(Action::Scroll(if key.code == KeyCode::Char('d') {
                10
            } else {
                -10
            }));
            return;
        }
        if ctrl && key.code == KeyCode::Tab && !self.tabs.is_empty() {
            self.action(Action::Tab((self.selected_tab + 1) % self.tabs.len()));
            return;
        }
        match key.code {
            KeyCode::Char('?') => self.action(Action::Help),
            KeyCode::Char('v') => self.action(Action::Switch(
                if self.tab().is_some_and(|t| t.view == View::Sessions) {
                    View::Changes
                } else {
                    View::Sessions
                },
            )),
            KeyCode::Char('a') => self.action(Action::NewTree),
            KeyCode::Char('n') => self.action(Action::NewSession),
            KeyCode::Char('r') => self.action(Action::Resume),
            KeyCode::Char('e') => self.action(Action::Archive),
            KeyCode::Char('R') => {
                self.enqueue(Job::Discover(self.roots.clone(), self.recursive));
                if let Some(project) = self.tab().map(|t| t.project.clone()) {
                    self.enqueue(Job::Trees(project));
                }
                self.agent_commands.push(agent::Command::Discover);
            }
            KeyCode::Char('s') => self.action(Action::Git("stage")),
            KeyCode::Char('u') => self.action(Action::Git("unstage")),
            KeyCode::Char('c') => self.action(Action::Git("commit")),
            KeyCode::Char('f') => self.action(Action::Git("fetch")),
            KeyCode::Char('p') => self.action(Action::Git("push")),
            KeyCode::Char('P') => self.action(Action::Git("pull")),
            KeyCode::Char('d') => self.action(Action::Git("remove")),
            KeyCode::Char('q') => self.action(Action::Quit),
            KeyCode::Esc => self.notice = None,
            KeyCode::Tab | KeyCode::Char('l') | KeyCode::Right => {
                self.pane = match self.pane {
                    Pane::Worktrees => Pane::List,
                    Pane::List => Pane::Detail,
                    Pane::Detail => Pane::Worktrees,
                }
            }
            KeyCode::BackTab | KeyCode::Char('h') | KeyCode::Left => {
                self.pane = match self.pane {
                    Pane::Worktrees => Pane::Detail,
                    Pane::List => Pane::Worktrees,
                    Pane::Detail => Pane::List,
                }
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Enter => {
                if self.pane == Pane::Detail {
                    self.action(Action::Expand);
                } else if self.tab().is_some_and(|t| t.view == View::Sessions) {
                    self.action(Action::FocusTerminal);
                }
            }
            KeyCode::Char('i') => self.action(Action::FocusTerminal),
            KeyCode::Char(' ') => {
                if let Some(file) = self.selected_file() {
                    let path = file.path.clone();
                    if !self.marked.remove(&path) {
                        self.marked.insert(path);
                    }
                }
            }
            KeyCode::Char('t') => {
                self.diff_staged = !self.diff_staged;
                self.request_diff();
            }
            _ => {}
        }
    }
    fn move_selection(&mut self, delta: i16) {
        match self.pane {
            Pane::Worktrees => {
                if let Some(tab) = self.tab() {
                    let i = tab
                        .selected
                        .saturating_add_signed(delta as isize)
                        .min(tab.trees.len().saturating_sub(1));
                    self.action(Action::Tree(i));
                }
            }
            Pane::List => {
                if self.tab().is_some_and(|t| t.view == View::Changes) {
                    let len = self.snapshot().map_or(0, |s| s.files.len());
                    let i = self
                        .tab()
                        .map_or(0, |t| t.file)
                        .saturating_add_signed(delta as isize)
                        .min(len.saturating_sub(1));
                    self.action(Action::File(i));
                } else {
                    let sessions = self.sessions();
                    let i = sessions
                        .iter()
                        .position(|c| {
                            self.tab().and_then(|t| t.conversation.as_ref()) == Some(&c.id)
                        })
                        .unwrap_or(0)
                        .saturating_add_signed(delta as isize)
                        .min(sessions.len().saturating_sub(1));
                    if let Some(c) = sessions.get(i) {
                        self.action(Action::Session(c.id.clone()));
                    }
                }
            }
            Pane::Detail => self.action(Action::Scroll(delta)),
        }
    }
    pub fn paste(&mut self, text: &str) {
        if self.cli_focused && self.modal.is_none() {
            if self.cli_input_blocked() {
                self.say(
                    "Wait for the active worktree operation before sending CLI input",
                    true,
                );
                return;
            }
            if let Some(id) = self.conversation().map(|c| c.id.clone())
                && let Some(terminal) = self.terminals.get_mut(&id)
            {
                terminal.paste(text);
            }
            return;
        }
        let text = text.replace('\r', "");
        if let Some(Modal::Form(form)) = &mut self.modal {
            if let Some(field) = form.fields.get_mut(form.focused)
                && matches!(
                    field.kind,
                    FieldType::Text | FieldType::Branch | FieldType::Multiline
                )
            {
                field.input.insert(&text);
            }
        } else if let Some(Modal::Projects { query, .. }) = &mut self.modal {
            query.insert(&text);
        } else if let Some(Modal::Branches {
            query, selected, ..
        }) = &mut self.modal
        {
            query.insert(&text);
            *selected = 0;
        } else if self.cli_focused {
            self.composer.insert(&text);
            self.sync_draft();
        }
    }
    pub fn mouse(&mut self, event: MouseEvent) {
        if self.modal.is_none()
            && self.tab().is_some_and(|t| t.view == View::Sessions)
            && self
                .terminal_area
                .contains((event.column, event.row).into())
            && self.cli_input_blocked()
        {
            self.say(
                "Wait for the active worktree operation before sending CLI input",
                true,
            );
            return;
        }
        if self.modal.is_none()
            && self.tab().is_some_and(|t| t.view == View::Sessions)
            && self
                .terminal_area
                .contains((event.column, event.row).into())
            && let Some(id) = self.conversation().map(|c| c.id.clone())
            && let Some(terminal) = self.terminals.get_mut(&id)
            && terminal.running()
        {
            if matches!(event.kind, MouseEventKind::Down(_)) {
                self.cli_focused = true;
                self.pane = Pane::Detail;
            }
            terminal.mouse(event, self.terminal_area);
            return;
        }
        if self.modal.is_some()
            && matches!(
                event.kind,
                MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
            )
        {
            self.key(KeyEvent::new(
                if event.kind == MouseEventKind::ScrollDown {
                    KeyCode::Down
                } else {
                    KeyCode::Up
                },
                KeyModifiers::NONE,
            ));
            return;
        }
        if matches!(
            event.kind,
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
        ) && let Some(hit) = self
            .hits
            .iter()
            .rev()
            .find(|h| h.rect.contains((event.column, event.row).into()))
        {
            match hit.action {
                Action::Tree(_) => self.pane = Pane::Worktrees,
                Action::Session(_) | Action::File(_) => self.pane = Pane::List,
                Action::Expand | Action::FocusTerminal | Action::Scroll(_) => {
                    self.pane = Pane::Detail
                }
                _ => {}
            }
        }
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(hit) = self
                    .hits
                    .iter()
                    .rev()
                    .find(|h| h.rect.contains((event.column, event.row).into()))
                    .cloned()
                {
                    self.action(hit.action);
                }
            }
            MouseEventKind::ScrollDown => self.move_selection(1),
            MouseEventKind::ScrollUp => self.move_selection(-1),
            _ => {}
        }
    }
    pub fn agent_event(&mut self, event: agent::Event) {
        match event {
            agent::Event::FolderTrust { id, path } => {
                if let Some(c) = self.state.conversations.iter_mut().find(|c| c.id == id) {
                    c.attention = Attention::NeedsInput;
                }
                self.confirm_yes = false;
                self.modal = Some(Modal::Confirm(Confirm::Trust { id, path }));
                self.dirty = true;
            }
            agent::Event::Connected {
                connection_url,
                agents,
            } => {
                self.connected = true;
                self.connection_url = Some(connection_url);
                self.agents = agents;
                self.agent_commands.push(agent::Command::Discover);
            }
            agent::Event::Options(options) => {
                self.provider_options = options.clone();
                if let Some(Modal::Form(form)) = &mut self.modal
                    && form.kind == FormKind::Session
                {
                    let fresh = config_fields(&options);
                    for field in fresh {
                        if !form.fields.iter().any(|f| f.key == field.key) {
                            form.fields.push(field);
                        }
                    }
                }
            }
            agent::Event::Agents(agents) => {
                self.agents = agents;
                let model = self.model_field(None);
                if let Some(Modal::Form(form)) = &mut self.modal
                    && let Some(field) = form.fields.iter_mut().find(|f| f.key == "model")
                {
                    field.kind = model.kind;
                }
            }
            agent::Event::Catalog(items) => {
                for item in items {
                    let Some(id) = item["resource"].as_str() else {
                        continue;
                    };
                    let directory = item["workingDirectories"]
                        .as_array()
                        .and_then(|d| d.first())
                        .and_then(Value::as_str)
                        .and_then(|uri| url::Url::parse(uri).ok())
                        .and_then(|u| u.to_file_path().ok());
                    let Some(worktree) = directory else {
                        continue;
                    };
                    let project = self
                        .tabs
                        .iter()
                        .find(|t| t.trees.iter().any(|w| same_path(&w.path, &worktree)))
                        .map(|t| t.project.path.clone())
                        .or_else(|| {
                            self.projects
                                .iter()
                                .find(|p| same_path(&p.path, &worktree))
                                .map(|p| p.path.clone())
                        });
                    let project = project.or_else(|| {
                        item["_treerailProject"]
                            .as_str()
                            .map(PathBuf::from)
                            .filter(|path| self.projects.iter().any(|p| same_path(&p.path, path)))
                    });
                    let Some(project) = project else {
                        if item.get("_treerailProject").is_none() {
                            self.enqueue(Job::Associate(worktree, item));
                        }
                        continue;
                    };
                    if !self.state.conversations.iter().any(|c| c.session_uri == id) {
                        self.state.conversations.push(Conversation {
                            id: id.into(),
                            project,
                            worktree,
                            title: item["title"]
                                .as_str()
                                .unwrap_or("Copilot conversation")
                                .into(),
                            session_uri: id.into(),
                            chat_uri: item["defaultChat"].as_str().map(str::to_owned),
                            attention: Attention::from_status(
                                item["status"].as_u64().unwrap_or(1) as u32
                            ),
                            archived: false,
                            draft: String::new(),
                            chat: None,
                            error: None,
                            pending: vec![],
                            options: Value::Null,
                            owned: false,
                            initial_prompt: None,
                            creation_failed: false,
                            model: None,
                        });
                        self.dirty = true;
                    }
                    if item["status"]
                        .as_u64()
                        .is_some_and(|status| status & 8 != 0)
                    {
                        self.agent_commands
                            .push(agent::Command::Observe { id: id.into() });
                    }
                }
            }
            agent::Event::Session { id, state } => {
                self.attached.insert(id.clone());
                if let Some(c) = self
                    .state
                    .conversations
                    .iter_mut()
                    .find(|c| c.session_uri == id)
                {
                    c.chat_uri = state.default_chat.clone();
                    if !state.title.is_empty() {
                        c.title = state.title.clone();
                    }
                    if c.initial_prompt.is_none() || state.status & 8 != 0 {
                        c.attention = Attention::from_status(state.status);
                    }
                    c.creation_failed = state.creation_error.is_some();
                    c.error = state.creation_error.map(|e| format!("{e:?}"));
                    if c.creation_failed {
                        c.attention = Attention::Interrupted;
                        if let Some(prompt) = c.initial_prompt.take()
                            && c.draft.is_empty()
                        {
                            c.draft = prompt;
                        }
                    }
                    c.pending = state
                        .input_needed
                        .unwrap_or_default()
                        .iter()
                        .filter_map(|r| serde_json::to_value(r).ok())
                        .collect();
                    c.options = state
                        .config
                        .and_then(|c| serde_json::to_value(c).ok())
                        .unwrap_or(Value::Null);
                    self.dirty = true;
                }
            }
            agent::Event::Chat { session, state } => {
                if (state.active_turn.is_some() || !state.turns.is_empty())
                    && self
                        .failed_form
                        .as_ref()
                        .is_some_and(|f| f.kind == FormKind::Session)
                {
                    self.failed_form = None;
                }
                if let Some(c) = self.state.conversations.iter_mut().find(|c| {
                    c.session_uri == session
                        && c.chat_uri.as_ref().is_none_or(|uri| uri == &state.resource)
                }) {
                    c.chat_uri = Some(state.resource.clone());
                    if state.active_turn.is_some() || !state.turns.is_empty() {
                        c.initial_prompt = None;
                    }
                    if c.initial_prompt.is_none() {
                        c.attention = Attention::from_status(state.status);
                    }
                    c.chat = Some(*state);
                    self.dirty = true;
                }
            }
            agent::Event::Completions(value) => {
                self.completions = value.clone();
                let items = value["items"].as_array().cloned().unwrap_or_default();
                if items.is_empty() {
                    self.say("No provider completions at this position", false);
                } else {
                    self.modal = Some(Modal::Completions { items, selected: 0 });
                }
            }
            agent::Event::Error { id, message } => {
                if let Some(c) = self.state.conversations.iter_mut().find(|c| {
                    id.as_ref()
                        .is_some_and(|id| &c.session_uri == id || c.chat_uri.as_ref() == Some(id))
                }) {
                    c.attention = if c.creation_failed {
                        Attention::Interrupted
                    } else {
                        Attention::Unavailable
                    };
                    c.error = Some(message.clone());
                    if let Some(prompt) = c.initial_prompt.take()
                        && c.draft.is_empty()
                    {
                        c.draft = prompt;
                    }
                    self.dirty = true;
                }
                self.say(message, true);
                if self.composer.value.is_empty() {
                    self.restore_composer();
                }
            }
            agent::Event::Disconnected => {
                self.connected = false;
                self.attached.clear();
                for c in &mut self.state.conversations {
                    if c.attention.busy() {
                        c.attention = Attention::Interrupted;
                    }
                }
                self.dirty = true;
            }
            agent::Event::LaunchFailed { id, message } => {
                if let Some(c) = self.state.conversations.iter_mut().find(|c| c.id == id) {
                    c.attention = Attention::Interrupted;
                    c.creation_failed = true;
                    c.error = Some(message.clone());
                    if let Some(prompt) = c.initial_prompt.take()
                        && c.draft.is_empty()
                    {
                        c.draft = prompt;
                    }
                }
                self.dirty = true;
                self.say(message, true);
                if self.composer.value.is_empty() {
                    self.restore_composer();
                }
            }
        }
    }
    fn remember_navigation(&mut self) {
        self.sync_draft();
        for tab in &self.tabs {
            self.state.navigation.insert(
                tab.project.path.to_string_lossy().into(),
                Navigation {
                    worktree: tab.trees.get(tab.selected).map(|t| t.path.clone()),
                    conversation: tab.conversation.clone(),
                    changes: tab.view == View::Changes,
                },
            );
        }
        self.state.tabs = self.tabs.iter().map(|t| t.project.path.clone()).collect();
        self.state.last_project = self.tab().map(|t| t.project.path.clone());
        self.state.last_worktree = self.tree().map(|t| t.path.clone());
    }
    pub fn save(&mut self) -> Result<()> {
        self.remember_navigation();
        self.state.save(&config::state_path(&self.config_path))?;
        self.dirty = false;
        self.last_save = Instant::now();
        Ok(())
    }
    pub fn tick(&mut self) {
        self.terminals.poll();
        if self
            .conversation()
            .and_then(|c| self.terminals.get(&c.id))
            .is_some_and(|t| !t.running())
        {
            self.cli_focused = false;
        }
        for (id, _) in self.terminal_requested.clone() {
            self.open_terminal(&id);
        }
        if self.connected && self.last_agent_refresh.elapsed() > Duration::from_secs(10) {
            self.last_agent_refresh = Instant::now();
            self.agent_commands.push(agent::Command::Discover);
        }
        if self.last_refresh.elapsed() > Duration::from_secs(2) {
            self.last_refresh = Instant::now();
            let paths = self
                .tab()
                .into_iter()
                .flat_map(|t| &t.trees)
                .filter(|t| !t.prunable)
                .map(|t| t.path.clone())
                .collect::<Vec<_>>();
            for path in paths {
                self.enqueue(Job::Status(path));
            }
        }
        if self.dirty
            && self.last_save.elapsed() > Duration::from_secs(2)
            && let Err(e) = self.save()
        {
            self.say(format!("State could not be saved: {e:#}"), true);
        }
        if self
            .notice
            .as_ref()
            .is_some_and(|(_, error, t)| !error && t.elapsed() > Duration::from_secs(6))
        {
            self.notice = None;
        }
    }
}
fn fuzzy_score(name: &str, query: &str) -> Option<usize> {
    let name = name.to_lowercase().chars().collect::<Vec<_>>();
    let mut position = 0;
    let mut score = 0;
    for c in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let offset = name[position..].iter().position(|n| *n == c)?;
        score += offset;
        position += offset + 1;
    }
    Some(score)
}
fn config_fields(options: &Value) -> Vec<Field> {
    let mut fields = vec![];
    if let Some(properties) = options["schema"]["properties"].as_object() {
        for (key, property) in properties {
            if !matches!(key.as_str(), "approvalMode" | "mode") {
                continue;
            }
            let label = property["title"].as_str().unwrap_or(key);
            let value = options["values"]
                .get(key)
                .or_else(|| property.get("default"))
                .cloned()
                .unwrap_or(Value::Null);
            let mut field = Field::text(
                &format!("config:{key}"),
                label,
                value.as_str().map(str::to_owned).unwrap_or_else(|| {
                    if value.is_null() {
                        String::new()
                    } else {
                        value.to_string()
                    }
                }),
            );
            if property["type"] == "boolean" {
                field = Field::toggle(
                    &format!("config:{key}"),
                    label,
                    value.as_bool().unwrap_or(false),
                );
            } else if let Some(choices) = property["enum"].as_array() {
                field.kind = FieldType::Choice(
                    choices
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect(),
                );
            }
            if matches!(
                property["type"].as_str(),
                Some("string" | "boolean" | "number" | "integer")
            ) || property.get("enum").is_some()
            {
                fields.push(field);
            }
        }
    }
    fields
}
fn field_value(field: &Field, schema: &Value) -> Result<Value> {
    match schema["type"].as_str() {
        Some("boolean") => Ok(json!(field.input.value == "yes")),
        Some("number" | "integer") => Ok(serde_json::from_str(&field.input.value)
            .with_context(|| format!("Invalid number for {}", field.label))?),
        _ => Ok(json!(field.input.value)),
    }
}
fn utf16_byte(text: &str, offset: usize) -> Option<usize> {
    let mut count = 0;
    for (i, c) in text.char_indices() {
        if count == offset {
            return Some(i);
        }
        count += c.len_utf16();
    }
    if count == offset {
        Some(text.len())
    } else {
        None
    }
}
