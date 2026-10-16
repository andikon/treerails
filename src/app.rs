use crate::{
    config::{Config, State, same_path},
    discover,
    git::{self, Branch, ChangedFile, Project, PullRequest, Snapshot, Worktree},
    launch,
};
use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Debug, Clone)]
pub enum Operation {
    Stage(Vec<ChangedFile>, bool),
    Commit(String, String),
    Fetch,
    Pull,
    Push,
    Publish(String, String),
    Create {
        branch: String,
        base: String,
        destination: PathBuf,
        existing: bool,
        fetch: bool,
    },
    Branch(String, String),
    DeleteBranch(String),
    Remove(PathBuf),
    Review(String, PathBuf),
    OpenPr(String),
}
#[derive(Debug)]
pub enum Job {
    Discover(Vec<PathBuf>, bool),
    Trees(PathBuf),
    Status(PathBuf),
    Diff(PathBuf, ChangedFile, bool),
    Branches(PathBuf),
    Prs(PathBuf),
    ReviewDiff(PathBuf, String),
    Operation {
        project: PathBuf,
        path: PathBuf,
        op: Operation,
    },
}
#[derive(Debug)]
pub enum Payload {
    Projects(Vec<Project>, Vec<String>),
    Trees(PathBuf, Vec<Worktree>, String),
    Status(PathBuf, Snapshot),
    Diff(String, String),
    Branches(PathBuf, Vec<Branch>),
    Prs(PathBuf, Vec<PullRequest>),
    Done(PathBuf, Option<PathBuf>, String),
}
pub struct Completion {
    pub epoch: u64,
    pub token: String,
    pub result: Result<Payload>,
}
impl Job {
    pub fn token(&self) -> String {
        match self {
            Self::Status(p) => format!("status:{}", p.display()),
            Self::Diff(p, f, s) => diff_key(p, &f.path, *s),
            Self::ReviewDiff(p, _) => format!("review:{}", p.display()),
            Self::Trees(p) => format!("trees:{}", p.display()),
            Self::Branches(p) => format!("branches:{}", p.display()),
            Self::Prs(p) => format!("prs:{}", p.display()),
            Self::Discover(..) => "discover".into(),
            Self::Operation { .. } => "operation".into(),
        }
    }
    pub async fn execute(self) -> Result<Payload> {
        match self {
            Self::Discover(roots, recursive) => {
                let (p, w) = discover::discover(roots, recursive).await?;
                Ok(Payload::Projects(p, w))
            }
            Self::Trees(path) => {
                let trees = git::worktrees(&path).await?;
                let base = git::default_base(&path).await;
                Ok(Payload::Trees(path, trees, base))
            }
            Self::Status(path) => {
                let s = git::status(&path).await?;
                Ok(Payload::Status(path, s))
            }
            Self::Diff(path, file, staged) => {
                let key = diff_key(&path, &file.path, staged);
                Ok(Payload::Diff(key, git::diff(&path, &file, staged).await?))
            }
            Self::Branches(path) => {
                let b = git::branches(&path).await?;
                Ok(Payload::Branches(path, b))
            }
            Self::Prs(path) => {
                let p = git::prs(&path).await?;
                Ok(Payload::Prs(path, p))
            }
            Self::ReviewDiff(path, branch) => {
                let key = format!("review:{}", path.display());
                Ok(Payload::Diff(key, git::review_diff(&path, &branch).await?))
            }
            Self::Operation { project, path, op } => {
                let (mut selected, mut message) = (None, "Completed".to_owned());
                match op {
                    Operation::Stage(files, undo) => {
                        git::stage(&path, &files, undo).await?;
                        message = if undo {
                            "Files unstaged"
                        } else {
                            "Files staged"
                        }
                        .into();
                    }
                    Operation::Commit(s, b) => {
                        message = git::commit(&path, &s, &b).await?;
                    }
                    Operation::Fetch => {
                        git::run(&path, ["fetch", "--all", "--prune"]).await?;
                    }
                    Operation::Pull => {
                        git::run(&path, ["pull", "--ff-only"]).await?;
                    }
                    Operation::Push => {
                        git::push(&path).await?;
                    }
                    Operation::Publish(r, b) => {
                        git::publish(&path, &r, &b).await?;
                        message = format!("Published to {r}/{b}");
                    }
                    Operation::Create {
                        branch,
                        base,
                        destination,
                        existing,
                        fetch,
                    } => {
                        selected = Some(
                            git::create_worktree(
                                &project,
                                &branch,
                                &base,
                                &destination,
                                existing,
                                fetch,
                            )
                            .await?,
                        );
                        message = "Worktree ready".into();
                    }
                    Operation::Branch(branch, base) => {
                        git::run(&project, ["check-ref-format", "--branch", &branch]).await?;
                        git::run(&project, ["branch", "--no-track", "--", &branch, &base]).await?;
                        message = "Branch created".into();
                    }
                    Operation::DeleteBranch(branch) => {
                        git::run(&project, ["branch", "-d", "--", &branch]).await?;
                        message = "Branch deleted".into();
                    }
                    Operation::Remove(target) => {
                        git::remove_worktree(&project, &target).await?;
                        message = "Worktree removed; branch retained".into();
                    }
                    Operation::Review(number, destination) => {
                        selected = Some(git::review(&project, &number, &destination).await?);
                        message = "PR review worktree ready".into();
                    }
                    Operation::OpenPr(branch) => {
                        git::open_pr(&path, &branch).await?;
                    }
                }
                Ok(Payload::Done(project, selected, message))
            }
        }
    }
}
pub fn diff_key(path: &std::path::Path, file: &std::path::Path, staged: bool) -> String {
    format!("diff:{}:{}:{staged}", path.display(), file.display())
}

#[derive(Debug, Clone)]
pub enum FormKind {
    Create { existing: bool },
    Branch,
    Commit,
    Publish,
    Review,
}
#[derive(Debug, Clone)]
pub struct Field {
    pub label: String,
    pub value: String,
    pub multiline: bool,
    pub cursor: usize,
    pub readonly: bool,
}
impl Field {
    fn new(label: &str, value: String, multiline: bool) -> Self {
        let cursor = value.len();
        Self {
            label: label.into(),
            value,
            multiline,
            cursor,
            readonly: false,
        }
    }
}
#[derive(Debug, Clone)]
pub struct Form {
    pub title: String,
    pub kind: FormKind,
    pub fields: Vec<Field>,
    pub active: usize,
    pub path_custom: bool,
}
#[derive(Debug, Clone)]
pub enum PickerKind {
    Branches,
    Base(Form),
    Profiles,
    Prs,
}
#[derive(Debug, Clone)]
pub struct Picker {
    pub title: String,
    pub kind: PickerKind,
    pub entries: Vec<(String, String)>,
    pub query: String,
    pub selected: usize,
    pub editing: bool,
    pub top_prefix: bool,
}
impl Picker {
    pub fn visible(&self) -> Vec<&(String, String)> {
        self.entries
            .iter()
            .filter(|e| e.0.to_lowercase().contains(&self.query.to_lowercase()))
            .collect()
    }
}
#[derive(Debug, Clone)]
pub enum Modal {
    Form(Form),
    Picker(Picker),
    Confirm(String, Operation),
    Help,
}

pub struct App {
    pub config: Config,
    pub state: State,
    pub config_path: PathBuf,
    pub state_path: PathBuf,
    pub roots: Vec<PathBuf>,
    pub recursive: bool,
    pub projects: Vec<Project>,
    pub project_cursor: usize,
    pub project: Option<Project>,
    pub trees: Vec<Worktree>,
    pub tree: usize,
    pub snapshots: BTreeMap<PathBuf, Snapshot>,
    pub file: usize,
    pub marked: BTreeSet<PathBuf>,
    pub focus: usize,
    pub filter: String,
    pub searching: bool,
    pub search_before: String,
    pub modal: Option<Modal>,
    pub diff: String,
    pub diff_token: String,
    pub diff_scroll: usize,
    pub staged: bool,
    pub pr_diff: bool,
    pub branches: Vec<Branch>,
    pub prs: Vec<PullRequest>,
    pub default_base: String,
    pub busy: bool,
    pub notice: String,
    pub error: bool,
    pub quit: bool,
    pub epoch: u64,
    pub jobs: Vec<Job>,
    pub inflight: BTreeSet<String>,
    pub prefix: Option<(char, Instant)>,
    pub last_refresh: Instant,
    pub last_summary: Instant,
    pub desired_tree: Option<PathBuf>,
    pub waiting_prs: bool,
    pub recover_form: Option<Form>,
    pub viewport_height: usize,
}
impl App {
    pub fn new(
        config: Config,
        state: State,
        config_path: PathBuf,
        state_path: PathBuf,
        roots: Vec<PathBuf>,
        recursive: bool,
    ) -> Self {
        Self {
            config,
            state,
            config_path,
            state_path,
            roots: roots.clone(),
            recursive,
            projects: vec![],
            project_cursor: 0,
            project: None,
            trees: vec![],
            tree: 0,
            snapshots: BTreeMap::new(),
            file: 0,
            marked: BTreeSet::new(),
            focus: 0,
            filter: String::new(),
            searching: false,
            search_before: String::new(),
            modal: None,
            diff: String::new(),
            diff_token: String::new(),
            diff_scroll: 0,
            staged: false,
            pr_diff: false,
            branches: vec![],
            prs: vec![],
            default_base: "HEAD".into(),
            busy: false,
            notice: "Discovering projects…".into(),
            error: false,
            quit: false,
            epoch: 0,
            jobs: vec![Job::Discover(roots, recursive)],
            inflight: BTreeSet::new(),
            prefix: None,
            last_refresh: Instant::now() - Duration::from_secs(10),
            last_summary: Instant::now() - Duration::from_secs(10),
            desired_tree: None,
            waiting_prs: false,
            recover_form: None,
            viewport_height: 24,
        }
    }
    pub fn say(&mut self, message: impl Into<String>, error: bool) {
        self.notice = message.into();
        self.error = error;
    }
    pub fn visible_projects(&self) -> Vec<&Project> {
        self.projects
            .iter()
            .filter(|p| p.name.to_lowercase().contains(&self.filter.to_lowercase()))
            .collect()
    }
    pub fn visible_trees(&self) -> Vec<&Worktree> {
        self.trees
            .iter()
            .filter(|t| {
                self.focus != 0
                    || t.branch
                        .to_lowercase()
                        .contains(&self.filter.to_lowercase())
            })
            .collect()
    }
    pub fn selected_tree(&self) -> Option<&Worktree> {
        self.visible_trees().get(self.tree).copied()
    }
    pub fn snapshot(&self) -> Option<&Snapshot> {
        self.selected_tree()
            .and_then(|w| self.snapshots.get(&w.path))
    }
    pub fn visible_files(&self) -> Vec<&ChangedFile> {
        self.snapshot()
            .map(|s| {
                s.files
                    .iter()
                    .filter(|f| {
                        self.focus != 1
                            || f.path
                                .to_string_lossy()
                                .to_lowercase()
                                .contains(&self.filter.to_lowercase())
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn selected_file(&self) -> Option<&ChangedFile> {
        self.visible_files().get(self.file).copied()
    }
    pub fn enqueue(&mut self, job: Job) {
        let token = job.token();
        if self.inflight.insert(token) {
            self.jobs.push(job);
        }
    }
    pub fn operation(&mut self, op: Operation) {
        if self.busy {
            self.say("A Git operation is already running", true);
            return;
        }
        let Some(project) = self.project.clone() else {
            return;
        };
        let path = self
            .selected_tree()
            .map(|t| t.path.clone())
            .unwrap_or(project.path.clone());
        self.busy = true;
        self.epoch += 1;
        self.say("Running Git operation…", false);
        self.jobs.push(Job::Operation {
            project: project.path,
            path,
            op,
        });
    }
    pub fn accept(&mut self, done: Completion) {
        self.inflight.remove(&done.token);
        if done.token == "operation" {
            self.busy = false;
            self.epoch += 1;
        } else if done.epoch != self.epoch {
            return;
        }
        match done.result {
            Err(error) => {
                self.say(format!("{error:#}"), true);
                if done.token == "operation"
                    && let Some(form) = self.recover_form.take()
                {
                    self.modal = Some(Modal::Form(form));
                }
            }
            Ok(Payload::Projects(projects, warnings)) => {
                self.projects = projects;
                self.project_cursor = self
                    .state
                    .last_project
                    .as_ref()
                    .and_then(|p| self.projects.iter().position(|x| same_path(&x.path, p)))
                    .unwrap_or(0);
                self.say(
                    if warnings.is_empty() {
                        format!(
                            "{} projects · Enter to explore · ? for help",
                            self.projects.len()
                        )
                    } else {
                        format!("{} projects · {}", self.projects.len(), warnings.join("; "))
                    },
                    !warnings.is_empty(),
                );
            }
            Ok(Payload::Trees(path, trees, base)) => {
                if self
                    .project
                    .as_ref()
                    .is_some_and(|p| same_path(&p.path, &path))
                {
                    let previous = self.selected_tree().map(|t| t.path.clone());
                    let old = self
                        .desired_tree
                        .take()
                        .or_else(|| previous.clone())
                        .or_else(|| self.state.last_worktree.clone());
                    self.trees = trees;
                    self.tree = old
                        .and_then(|p| {
                            self.visible_trees()
                                .iter()
                                .position(|t| same_path(&t.path, &p))
                        })
                        .unwrap_or(self.tree.min(self.visible_trees().len().saturating_sub(1)));
                    self.default_base = base;
                    if previous != self.selected_tree().map(|t| t.path.clone()) {
                        self.selection_changed();
                    }
                }
            }
            Ok(Payload::Status(path, snapshot)) => {
                let selected = self.selected_file().map(|f| f.path.clone());
                let active = self
                    .selected_tree()
                    .is_some_and(|w| same_path(&w.path, &path));
                self.snapshots.insert(path, snapshot);
                if active {
                    self.file = selected
                        .and_then(|p| self.visible_files().iter().position(|f| f.path == p))
                        .unwrap_or(self.file.min(self.visible_files().len().saturating_sub(1)));
                    self.request_diff(true);
                }
            }
            Ok(Payload::Diff(token, diff)) => {
                if token == self.diff_token {
                    self.diff = diff;
                }
            }
            Ok(Payload::Branches(path, branches)) => {
                if self
                    .project
                    .as_ref()
                    .is_some_and(|p| same_path(&p.path, &path))
                {
                    self.branches = branches;
                }
            }
            Ok(Payload::Prs(path, prs)) => {
                if self
                    .project
                    .as_ref()
                    .is_some_and(|p| same_path(&p.path, &path))
                {
                    self.prs = prs;
                    if self.waiting_prs {
                        self.waiting_prs = false;
                        if self.modal.is_none() {
                            self.open_pr_picker();
                        }
                    }
                }
            }
            Ok(Payload::Done(project, path, message)) => {
                self.recover_form = None;
                self.say(message, false);
                if self
                    .project
                    .as_ref()
                    .is_some_and(|p| same_path(&p.path, &project))
                {
                    if path.is_some() {
                        self.filter.clear();
                        self.focus = 0;
                    }
                    self.desired_tree = path;
                    self.enqueue(Job::Trees(project.clone()));
                    self.enqueue(Job::Branches(project));
                    self.last_refresh = Instant::now() - Duration::from_secs(10);
                    self.last_summary = Instant::now() - Duration::from_secs(10);
                }
            }
        }
    }
    pub fn open_project(&mut self) {
        if let Some(project) = self
            .visible_projects()
            .get(self.project_cursor)
            .cloned()
            .cloned()
        {
            self.epoch += 1;
            self.recover_form = None;
            self.project = Some(project.clone());
            self.state.last_project = Some(project.path.clone());
            self.trees.clear();
            self.tree = 0;
            self.file = 0;
            self.focus = 0;
            self.filter.clear();
            self.branches.clear();
            self.prs.clear();
            self.diff.clear();
            self.enqueue(Job::Trees(project.path.clone()));
            self.enqueue(Job::Branches(project.path));
            self.say("Loading worktrees…", false);
        }
    }
    pub fn selection_changed(&mut self) {
        self.file = 0;
        self.marked.clear();
        self.pr_diff = false;
        self.diff_scroll = 0;
        self.diff.clear();
        self.diff_token.clear();
        if let Some(w) = self.selected_tree() {
            let path = w.path.clone();
            self.state.last_worktree = Some(path.clone());
            self.enqueue(Job::Status(path));
        }
    }
    pub fn request_diff(&mut self, force: bool) {
        let Some(wt) = self.selected_tree().cloned() else {
            return;
        };
        if self.pr_diff {
            let token = format!("review:{}", wt.path.display());
            if force || self.diff_token != token {
                self.diff_token = token;
                self.enqueue(Job::ReviewDiff(wt.path, wt.branch));
            }
            return;
        }
        let Some(file) = self.selected_file().cloned() else {
            self.diff.clear();
            self.diff_token.clear();
            return;
        };
        let key = diff_key(&wt.path, &file.path, self.staged);
        if force || key != self.diff_token {
            self.diff_token = key;
            self.enqueue(Job::Diff(wt.path, file, self.staged));
        }
    }
    pub fn tick(&mut self) {
        if self
            .prefix
            .is_some_and(|(_, t)| t.elapsed() > Duration::from_secs(2))
        {
            self.prefix = None;
        }
        if self.project.is_some()
            && !self.busy
            && self.last_refresh.elapsed() > Duration::from_millis(1500)
        {
            self.last_refresh = Instant::now();
            if let Some(wt) = self.selected_tree() {
                self.enqueue(Job::Status(wt.path.clone()));
            }
        }
        if self.project.is_some()
            && !self.busy
            && self.last_summary.elapsed() > Duration::from_secs(5)
        {
            self.last_summary = Instant::now();
            if let Some(p) = self.project.clone() {
                self.enqueue(Job::Trees(p.path));
            }
            for wt in self
                .trees
                .iter()
                .skip(self.tree.saturating_sub(8))
                .take(20)
                .cloned()
                .collect::<Vec<_>>()
            {
                if wt.path.exists() {
                    self.enqueue(Job::Status(wt.path));
                }
            }
        }
    }
    fn base(&self) -> String {
        let Some(p) = &self.project else {
            return "HEAD".into();
        };
        self.config
            .project(&p.path)
            .and_then(|p| p.base.clone())
            .or_else(|| {
                self.state
                    .bases
                    .get(&p.path.to_string_lossy().into_owned())
                    .cloned()
            })
            .unwrap_or(self.default_base.clone())
    }
    pub fn create_form(&mut self, branch: String, existing: bool, branch_only: bool) {
        let Some(p) = &self.project else { return };
        let base = if existing {
            branch.clone()
        } else {
            self.base()
        };
        let destination = self
            .config
            .worktree_path(&p.path, &branch)
            .display()
            .to_string();
        let mut fields = if branch_only {
            vec![
                Field::new("Branch name", branch, false),
                Field::new("Starting point · Ctrl+b picker", base, false),
            ]
        } else {
            vec![
                Field::new("Branch name", branch, false),
                Field::new("Starting point · Ctrl+b picker", base, false),
                Field::new("Worktree directory", destination, false),
                Field::new("Fetch first? yes/no", "no".into(), false),
            ]
        };
        if existing {
            fields[0].readonly = true;
            fields[1].readonly = true;
            fields[1].label = "Existing branch · fixed starting point".into();
        }
        self.modal = Some(Modal::Form(Form {
            title: if branch_only {
                "Create branch"
            } else if existing {
                "Use existing branch"
            } else {
                "Create worktree"
            }
            .into(),
            kind: if branch_only {
                FormKind::Branch
            } else {
                FormKind::Create { existing }
            },
            fields,
            active: if existing { 2 } else { 0 },
            path_custom: false,
        }));
    }
    fn picker(&mut self, title: &str, kind: PickerKind, entries: Vec<(String, String)>) {
        self.modal = Some(Modal::Picker(Picker {
            title: title.into(),
            kind,
            entries,
            query: String::new(),
            selected: 0,
            editing: false,
            top_prefix: false,
        }));
    }
    fn open_pr_picker(&mut self) {
        let entries = self
            .prs
            .iter()
            .map(|p| {
                (
                    format!(
                        "#{}  {}  [{} → {}]",
                        p.number, p.title, p.head_ref_name, p.base_ref_name
                    ),
                    p.number.to_string(),
                )
            })
            .collect();
        self.picker(
            "Pull requests · Enter review · a enter number/URL",
            PickerKind::Prs,
            entries,
        );
    }
    fn review_form(&mut self, number: String) {
        self.modal = Some(Modal::Form(Form {
            title: "Review pull request".into(),
            kind: FormKind::Review,
            fields: vec![Field::new("PR number or URL", number, false)],
            active: 0,
            path_custom: false,
        }));
    }
    pub fn key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        let repeating = key.kind == KeyEventKind::Repeat;
        if self.modal.is_some() {
            if !repeating
                || matches!(
                    key.code,
                    KeyCode::Char(_)
                        | KeyCode::Backspace
                        | KeyCode::Left
                        | KeyCode::Right
                        | KeyCode::Up
                        | KeyCode::Down
                )
            {
                self.modal_key(key);
            }
            return;
        }
        if self.searching {
            match key.code {
                KeyCode::Esc => {
                    self.filter = self.search_before.clone();
                    self.searching = false;
                    self.reset_cursor();
                }
                KeyCode::Enter => {
                    self.searching = false;
                    if self.focus == 2 {
                        self.find_diff(1);
                    }
                }
                KeyCode::Backspace => {
                    self.filter.pop();
                    self.reset_cursor();
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.filter.push(c);
                    self.reset_cursor();
                }
                _ => {}
            }
            self.request_diff(false);
            return;
        }
        let action = self.config.keys.get(&key_name(key)).cloned();
        if let Some(action) = action {
            if !repeating || matches!(action.as_str(), "down" | "up") {
                self.action(&action);
            }
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('w') if !repeating => self.prefix = Some(('w', Instant::now())),
                KeyCode::Char('d') => self.move_by(self.half_page()),
                KeyCode::Char('u') => self.move_by(-self.half_page()),
                KeyCode::Char('c') if !self.busy => self.quit = true,
                _ => {}
            }
            return;
        }
        if let Some((prefix, _)) = self.prefix.take() {
            if prefix == 'g' && key.code == KeyCode::Char('g') {
                self.first_last(false);
                return;
            }
            if prefix == 'w' {
                match key.code {
                    KeyCode::Char('h') | KeyCode::Char('k') => self.change_focus(-1),
                    KeyCode::Char('l') | KeyCode::Char('j') => self.change_focus(1),
                    _ => {}
                }
                return;
            }
        }
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.move_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_by(-1),
            KeyCode::PageDown => self.move_by(20),
            KeyCode::PageUp => self.move_by(-20),
            KeyCode::Home => self.first_last(false),
            KeyCode::End | KeyCode::Char('G') => self.first_last(true),
            KeyCode::Tab => self.change_focus(1),
            KeyCode::BackTab => self.change_focus(-1),
            KeyCode::Char('g') if !repeating => self.prefix = Some(('g', Instant::now())),
            KeyCode::Char('/') if !repeating => {
                self.search_before = self.filter.clone();
                self.searching = true;
            }
            KeyCode::Char('n') => {
                if self.focus == 2 && !self.filter.is_empty() {
                    self.find_diff(1)
                } else {
                    self.move_by(1)
                }
            }
            KeyCode::Char('N') => {
                if self.focus == 2 && !self.filter.is_empty() {
                    self.find_diff(-1)
                } else {
                    self.move_by(-1)
                }
            }
            KeyCode::Char('h') | KeyCode::Left => self.change_focus(-1),
            KeyCode::Char('l') | KeyCode::Right => self.change_focus(1),
            KeyCode::Char(c) if !repeating => self.action(&c.to_string()),
            KeyCode::Enter if !repeating => {
                if self.project.is_none() {
                    self.open_project();
                } else {
                    self.change_focus(1);
                }
            }
            KeyCode::Esc => {
                if !self.filter.is_empty() {
                    self.clear_filter();
                } else {
                    self.back();
                }
            }
            _ => {}
        }
    }
    fn reset_cursor(&mut self) {
        if self.project.is_none() {
            self.project_cursor = 0;
        } else if self.focus == 0 {
            self.tree = 0;
            self.selection_changed();
        } else if self.focus == 1 {
            self.file = 0;
        } else {
            self.diff_scroll = 0;
        }
    }
    fn move_by(&mut self, delta: isize) {
        if self.project.is_none() {
            self.project_cursor = step(self.project_cursor, delta, self.visible_projects().len());
        } else if self.focus == 0 {
            let next = step(self.tree, delta, self.visible_trees().len());
            if next != self.tree {
                self.tree = next;
                self.selection_changed();
            }
        } else if self.focus == 1 {
            self.file = step(self.file, delta, self.visible_files().len());
            self.diff_scroll = 0;
            self.request_diff(false);
        } else {
            self.diff_scroll = step(self.diff_scroll, delta, self.diff.lines().count());
        }
    }
    fn first_last(&mut self, last: bool) {
        self.move_by(if last { isize::MAX / 2 } else { isize::MIN / 2 });
    }
    fn change_focus(&mut self, delta: isize) {
        if self.project.is_some() {
            self.clear_filter();
            self.focus = (self.focus as isize + delta).rem_euclid(3) as usize;
            self.request_diff(false);
        }
    }
    fn clear_filter(&mut self) {
        if self.project.is_none() {
            self.filter.clear();
            self.project_cursor = 0;
            return;
        }
        let tree = self.selected_tree().map(|w| w.path.clone());
        let file = self.selected_file().map(|f| f.path.clone());
        self.filter.clear();
        self.tree = tree
            .and_then(|p| self.trees.iter().position(|w| w.path == p))
            .unwrap_or(0);
        self.file = file
            .and_then(|p| self.visible_files().iter().position(|f| f.path == p))
            .unwrap_or(0);
        self.request_diff(false);
    }
    fn half_page(&self) -> isize {
        let row_height = if self.project.is_none() {
            2
        } else if self.focus == 0 {
            3
        } else {
            1
        };
        (self.viewport_height.saturating_sub(8) / row_height / 2).max(1) as isize
    }
    fn find_diff(&mut self, direction: isize) {
        let indices: Vec<usize> = self
            .diff
            .lines()
            .enumerate()
            .filter(|(_, s)| s.to_lowercase().contains(&self.filter.to_lowercase()))
            .map(|(i, _)| i)
            .collect();
        if direction > 0 {
            if let Some(i) = indices
                .iter()
                .find(|i| **i > self.diff_scroll)
                .or_else(|| indices.first())
            {
                self.diff_scroll = *i;
            }
        } else if let Some(i) = indices
            .iter()
            .rev()
            .find(|i| **i < self.diff_scroll)
            .or_else(|| indices.last())
        {
            self.diff_scroll = *i;
        }
    }
    fn back(&mut self) {
        if self.project.is_some() {
            self.epoch += 1;
            self.project = None;
            self.recover_form = None;
            self.filter.clear();
            self.modal = None;
        } else if self.busy {
            self.say("Wait for the running Git operation before quitting", true);
        } else {
            self.quit = true;
        }
    }
    pub fn action(&mut self, action: &str) {
        match action {
            "down" => {
                self.move_by(1);
                return;
            }
            "up" => {
                self.move_by(-1);
                return;
            }
            "first" => {
                self.first_last(false);
                return;
            }
            "last" => {
                self.first_last(true);
                return;
            }
            "focus_left" => {
                self.change_focus(-1);
                return;
            }
            "focus_right" => {
                self.change_focus(1);
                return;
            }
            "search" => {
                self.search_before = self.filter.clone();
                self.searching = true;
                return;
            }
            _ => {}
        }
        if action == "?" || action == "help" {
            self.modal = Some(Modal::Help);
            return;
        }
        if action == "q" || action == "back" {
            self.back();
            return;
        }
        if action == "S" || action == "save_roots" {
            match Config::save_roots(&self.config_path, &self.roots) {
                Ok(()) => self.say(
                    format!("Sources saved to {}", self.config_path.display()),
                    false,
                ),
                Err(e) => self.say(format!("{e:#}"), true),
            }
            return;
        }
        if action == "R" || action == "refresh" {
            if let Some(p) = self.project.clone() {
                self.enqueue(Job::Trees(p.path.clone()));
                self.enqueue(Job::Branches(p.path));
                self.last_refresh = Instant::now() - Duration::from_secs(10);
            } else {
                self.enqueue(Job::Discover(self.roots.clone(), self.recursive));
            }
            return;
        }
        if self.project.is_none() {
            return;
        }
        let Some(project) = self.project.clone() else {
            return;
        };
        match action {
            "a" | "create_worktree" => self.create_form(String::new(), false, false),
            "b" | "branches" => {
                let entries = self
                    .branches
                    .iter()
                    .map(|b| {
                        (
                            format!(
                                "{}{}",
                                if b.remote { "remote  " } else { "local   " },
                                b.name
                            ),
                            b.name.clone(),
                        )
                    })
                    .collect();
                self.picker(
                    "Branches · Enter worktree · a new branch · d delete local",
                    PickerKind::Branches,
                    entries,
                );
            }
            "o" | "launch" => {
                let name = self
                    .config
                    .project(&project.path)
                    .and_then(|p| p.default_profile.clone())
                    .unwrap_or(self.config.default_profile.clone());
                self.launch_profile(&name);
            }
            "O" | "profiles" => {
                let entries = self
                    .config
                    .profiles
                    .keys()
                    .map(|n| (n.clone(), n.clone()))
                    .collect();
                self.picker("Launch profile", PickerKind::Profiles, entries);
            }
            "s" | "stage" | "u" | "unstage" if self.focus == 1 => {
                let files = self
                    .snapshot()
                    .map(|s| {
                        s.files
                            .iter()
                            .filter(|f| self.marked.contains(&f.path))
                            .cloned()
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let files = if files.is_empty() {
                    self.selected_file().cloned().into_iter().collect()
                } else {
                    files
                };
                if !files.is_empty() {
                    self.operation(Operation::Stage(
                        files,
                        action == "u" || action == "unstage",
                    ));
                    self.marked.clear();
                }
            }
            " " if self.focus == 1 => {
                if let Some(f) = self.selected_file() {
                    let path = f.path.clone();
                    if !self.marked.remove(&path) {
                        self.marked.insert(path);
                    }
                }
            }
            "c" | "commit" => {
                self.modal = Some(Modal::Form(Form {
                    title: "Commit staged changes · Ctrl+s commit".into(),
                    kind: FormKind::Commit,
                    fields: vec![
                        Field::new("Subject", String::new(), false),
                        Field::new("Body", String::new(), true),
                    ],
                    active: 0,
                    path_custom: false,
                }))
            }
            "f" | "fetch" => self.operation(Operation::Fetch),
            "P" | "pull" => self.operation(Operation::Pull),
            "p" | "push" => {
                if self.snapshot().is_some_and(|s| s.upstream.is_some()) {
                    self.operation(Operation::Push);
                } else {
                    let branch = self
                        .selected_tree()
                        .map(|t| t.branch.clone())
                        .unwrap_or_default();
                    self.modal = Some(Modal::Form(Form {
                        title: "Publish branch · set upstream".into(),
                        kind: FormKind::Publish,
                        fields: vec![
                            Field::new("Remote", "origin".into(), false),
                            Field::new("Remote branch", branch, false),
                        ],
                        active: 0,
                        path_custom: false,
                    }));
                }
            }
            "d" | "remove_worktree" if self.focus == 0 => {
                if let Some(w) = self.selected_tree() {
                    if w.original {
                        self.say("The original checkout cannot be removed", true);
                    } else {
                        self.modal = Some(Modal::Confirm(
                            format!(
                                "Remove {}?\n{}\nIts branch will be retained. Dirty or locked worktrees are refused.",
                                w.branch,
                                w.path.display()
                            ),
                            Operation::Remove(w.path.clone()),
                        ));
                    }
                }
            }
            "r" | "prs" => {
                self.waiting_prs = true;
                self.say("Loading pull requests…", false);
                self.enqueue(Job::Prs(project.path));
            }
            "v" | "review_diff" => {
                self.pr_diff = !self.pr_diff;
                self.diff_scroll = 0;
                self.request_diff(true);
            }
            "w" | "open_pr" => {
                if let Some(w) = self.selected_tree() {
                    self.operation(Operation::OpenPr(w.branch.clone()));
                }
            }
            "t" | "toggle_staged" => {
                self.staged = !self.staged;
                self.pr_diff = false;
                self.diff_scroll = 0;
                self.request_diff(true);
            }
            _ => {}
        }
    }
    fn launch_profile(&mut self, name: &str) {
        if let Some(w) = self.selected_tree() {
            match launch::launch(&self.config, name, &w.path, &w.branch) {
                Ok(()) => self.say(format!("Launched {name} in a new terminal"), false),
                Err(e) => self.say(format!("{e:#}"), true),
            }
        }
    }
    fn modal_key(&mut self, key: KeyEvent) {
        let Some(modal) = self.modal.take() else {
            return;
        };
        if key.code == KeyCode::Esc {
            if let Modal::Picker(mut picker) = modal
                && picker.editing
            {
                picker.editing = false;
                self.modal = Some(Modal::Picker(picker));
            }
            return;
        }
        match modal {
            Modal::Help => {
                if !matches!(
                    key.code,
                    KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Enter
                ) {
                    self.modal = Some(Modal::Help);
                }
            }
            Modal::Confirm(prompt, op) => {
                if matches!(key.code, KeyCode::Char('y') | KeyCode::Enter)
                    && key.kind == KeyEventKind::Press
                {
                    self.operation(op);
                } else if !matches!(key.code, KeyCode::Char('n') | KeyCode::Char('q')) {
                    self.modal = Some(Modal::Confirm(prompt, op));
                }
            }
            Modal::Picker(mut picker) => {
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('n') {
                    picker.selected = step(picker.selected, 1, picker.visible().len());
                } else if key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.code == KeyCode::Char('p')
                {
                    picker.selected = step(picker.selected, -1, picker.visible().len());
                } else {
                    match key.code {
                        KeyCode::Char('/') if !picker.editing => {
                            picker.editing = true;
                        }
                        KeyCode::Char('j') if !picker.editing => {
                            picker.selected = step(picker.selected, 1, picker.visible().len())
                        }
                        KeyCode::Char('k') if !picker.editing => {
                            picker.selected = step(picker.selected, -1, picker.visible().len())
                        }
                        KeyCode::Char('g') if !picker.editing => {
                            if picker.top_prefix {
                                picker.selected = 0;
                                picker.top_prefix = false;
                            } else {
                                picker.top_prefix = true;
                            }
                        }
                        KeyCode::Char('G') | KeyCode::End if !picker.editing => {
                            picker.selected = picker.visible().len().saturating_sub(1)
                        }
                        KeyCode::Home => picker.selected = 0,
                        KeyCode::Char('q') if !picker.editing => return,
                        KeyCode::Down => {
                            picker.selected = step(picker.selected, 1, picker.visible().len())
                        }
                        KeyCode::Up => {
                            picker.selected = step(picker.selected, -1, picker.visible().len())
                        }
                        KeyCode::Enter => {
                            let value = picker.visible().get(picker.selected).map(|e| e.1.clone());
                            if let Some(v) = value {
                                self.choose(picker.kind, v);
                                return;
                            }
                        }
                        KeyCode::Char('a')
                            if !picker.editing && matches!(picker.kind, PickerKind::Branches) =>
                        {
                            self.create_form(String::new(), false, true);
                            return;
                        }
                        KeyCode::Char('a')
                            if !picker.editing && matches!(picker.kind, PickerKind::Prs) =>
                        {
                            self.review_form(String::new());
                            return;
                        }
                        KeyCode::Char('d')
                            if !picker.editing && matches!(picker.kind, PickerKind::Branches) =>
                        {
                            if let Some(value) =
                                picker.visible().get(picker.selected).map(|e| e.1.clone())
                                && self.branches.iter().any(|b| b.name == value && !b.remote)
                            {
                                self.modal = Some(Modal::Confirm(
                                    format!(
                                        "Delete local branch {value}?\nGit will refuse an unmerged or checked-out branch."
                                    ),
                                    Operation::DeleteBranch(value),
                                ));
                                return;
                            }
                        }
                        KeyCode::Backspace => {
                            picker.query.pop();
                            picker.selected = 0;
                        }
                        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                            picker.editing = true;
                            picker.query.push(c);
                            picker.selected = 0;
                        }
                        _ => {}
                    }
                }
                self.modal = Some(Modal::Picker(picker));
            }
            Modal::Form(mut form) => {
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
                    self.submit(form);
                    return;
                }
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.code == KeyCode::Char('b')
                    && matches!(
                        form.kind,
                        FormKind::Create { existing: false } | FormKind::Branch
                    )
                {
                    let entries = self
                        .branches
                        .iter()
                        .map(|b| (b.name.clone(), b.name.clone()))
                        .collect();
                    self.picker("Starting point", PickerKind::Base(form), entries);
                    return;
                }
                match key.code {
                    KeyCode::Tab => form.active = (form.active + 1) % form.fields.len(),
                    KeyCode::BackTab => {
                        form.active = (form.active + form.fields.len() - 1) % form.fields.len()
                    }
                    KeyCode::Enter if !form.fields[form.active].multiline => {
                        if form.active + 1 == form.fields.len() {
                            self.submit(form);
                            return;
                        }
                        form.active += 1;
                    }
                    _ => {
                        let active = form.active;
                        edit(&mut form.fields[active], key);
                        if active == 2 {
                            form.path_custom = true;
                        }
                        if active == 0
                            && !form.path_custom
                            && matches!(form.kind, FormKind::Create { .. })
                            && let Some(p) = &self.project
                        {
                            let destination =
                                self.config.worktree_path(&p.path, &form.fields[0].value);
                            form.fields[2].value = destination.display().to_string();
                            form.fields[2].cursor = form.fields[2].value.len();
                        }
                    }
                }
                self.modal = Some(Modal::Form(form));
            }
        }
    }
    pub fn paste(&mut self, text: &str) {
        if let Some(Modal::Form(form)) = &mut self.modal {
            let field = &mut form.fields[form.active];
            if field.readonly {
                return;
            }
            let value = if field.multiline {
                text.replace("\r\n", "\n")
            } else {
                text.replace(['\r', '\n'], " ")
            };
            field.value.insert_str(field.cursor, &value);
            field.cursor += value.len();
            if form.active == 0
                && !form.path_custom
                && matches!(form.kind, FormKind::Create { .. })
                && let Some(p) = &self.project
            {
                form.fields[2].value = self
                    .config
                    .worktree_path(&p.path, &form.fields[0].value)
                    .display()
                    .to_string();
                form.fields[2].cursor = form.fields[2].value.len();
            }
        } else if self.searching {
            self.filter.push_str(&text.replace(['\r', '\n'], " "));
            self.reset_cursor();
        } else if let Some(Modal::Picker(p)) = &mut self.modal {
            p.editing = true;
            p.query.push_str(text.trim());
            p.selected = 0;
        }
    }
    fn choose(&mut self, kind: PickerKind, value: String) {
        match kind {
            PickerKind::Profiles => self.launch_profile(&value),
            PickerKind::Prs => self.review_form(value),
            PickerKind::Base(mut form) => {
                form.fields[1].value = value;
                form.fields[1].cursor = form.fields[1].value.len();
                self.modal = Some(Modal::Form(form));
            }
            PickerKind::Branches => {
                let remote = self.branches.iter().any(|b| b.name == value && b.remote);
                if remote {
                    let branch = value
                        .split_once('/')
                        .map(|(_, n)| n)
                        .unwrap_or(&value)
                        .to_owned();
                    self.create_form(branch, false, false);
                    if let Some(Modal::Form(f)) = &mut self.modal {
                        f.fields[1].value = value;
                        f.fields[1].cursor = f.fields[1].value.len();
                    }
                } else if let Some(index) = self.trees.iter().position(|w| w.branch == value) {
                    self.tree = index;
                    self.focus = 0;
                    self.filter.clear();
                    self.selection_changed();
                } else {
                    self.create_form(value, true, false);
                }
            }
        }
    }
    fn submit(&mut self, form: Form) {
        if self.busy {
            self.say("Wait for the running Git operation before submitting", true);
            self.modal = Some(Modal::Form(form));
            return;
        }
        let recovery = form.clone();
        let values: Vec<String> = form
            .fields
            .iter()
            .map(|f| f.value.trim().to_owned())
            .collect();
        let operation = match form.kind {
            FormKind::Create { existing } => {
                if values[0].is_empty() || values[1].is_empty() || values[2].is_empty() {
                    self.say("Branch, starting point, and directory are required", true);
                    self.modal = Some(Modal::Form(form));
                    return;
                }
                if let Some(p) = &self.project {
                    self.state
                        .bases
                        .insert(p.path.to_string_lossy().into_owned(), values[1].clone());
                }
                Operation::Create {
                    branch: values[0].clone(),
                    base: values[1].clone(),
                    destination: PathBuf::from(&values[2]),
                    existing,
                    fetch: matches!(
                        values[3].to_ascii_lowercase().as_str(),
                        "yes" | "y" | "true"
                    ),
                }
            }
            FormKind::Branch => Operation::Branch(values[0].clone(), values[1].clone()),
            FormKind::Commit => Operation::Commit(values[0].clone(), form.fields[1].value.clone()),
            FormKind::Publish => Operation::Publish(values[0].clone(), values[1].clone()),
            FormKind::Review => {
                let value = values[0].trim_end_matches('/');
                let number = value.rsplit('/').next().unwrap_or(value);
                if number.parse::<u64>().is_err() {
                    self.say("Enter a PR number or a PR URL ending in its number", true);
                    self.modal = Some(Modal::Form(form));
                    return;
                }
                let Some(p) = &self.project else { return };
                let path = self
                    .config
                    .worktree_path(&p.path, &format!("review/pr-{number}"));
                Operation::Review(value.into(), path)
            }
        };
        self.recover_form = Some(recovery);
        self.operation(operation);
    }
    pub fn save(&self) -> Result<()> {
        self.state
            .save(&self.state_path)
            .context("Save remembered selections")
    }
}
fn step(value: usize, delta: isize, len: usize) -> usize {
    value
        .saturating_add_signed(delta)
        .min(len.saturating_sub(1))
}
pub fn key_name(key: KeyEvent) -> String {
    let prefix = if key.modifiers.contains(KeyModifiers::CONTROL) {
        "ctrl+"
    } else if key.modifiers.contains(KeyModifiers::ALT) {
        "alt+"
    } else {
        ""
    };
    format!(
        "{prefix}{}",
        match key.code {
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Enter => "enter".into(),
            KeyCode::Tab => "tab".into(),
            KeyCode::Esc => "esc".into(),
            _ => format!("{:?}", key.code).to_lowercase(),
        }
    )
}
fn edit(field: &mut Field, key: KeyEvent) {
    if field.readonly {
        return;
    }
    let previous = |s: &str, p: usize| s[..p].char_indices().last().map(|(i, _)| i).unwrap_or(0);
    let next = |s: &str, p: usize| s[p..].chars().next().map(|c| p + c.len_utf8()).unwrap_or(p);
    match key.code {
        KeyCode::Left => field.cursor = previous(&field.value, field.cursor),
        KeyCode::Right => field.cursor = next(&field.value, field.cursor),
        KeyCode::Home => {
            field.cursor = field.value[..field.cursor]
                .rfind('\n')
                .map(|i| i + 1)
                .unwrap_or(0)
        }
        KeyCode::End => {
            field.cursor = field.value[field.cursor..]
                .find('\n')
                .map(|i| field.cursor + i)
                .unwrap_or(field.value.len())
        }
        KeyCode::Backspace => {
            let p = previous(&field.value, field.cursor);
            field.value.replace_range(p..field.cursor, "");
            field.cursor = p;
        }
        KeyCode::Delete => {
            let n = next(&field.value, field.cursor);
            field.value.replace_range(field.cursor..n, "");
        }
        KeyCode::Up | KeyCode::Down if field.multiline => {
            let start = field.value[..field.cursor]
                .rfind('\n')
                .map(|i| i + 1)
                .unwrap_or(0);
            let col = field.value[start..field.cursor].chars().count();
            let target = if key.code == KeyCode::Up {
                if start == 0 {
                    return;
                }
                let end = start - 1;
                let begin = field.value[..end].rfind('\n').map(|i| i + 1).unwrap_or(0);
                (begin, end)
            } else {
                let Some(offset) = field.value[field.cursor..].find('\n') else {
                    return;
                };
                let begin = field.cursor + offset + 1;
                let end = field.value[begin..]
                    .find('\n')
                    .map(|i| begin + i)
                    .unwrap_or(field.value.len());
                (begin, end)
            };
            field.cursor = field.value[target.0..target.1]
                .char_indices()
                .nth(col)
                .map(|(i, _)| target.0 + i)
                .unwrap_or(target.1);
        }
        KeyCode::Enter if field.multiline => {
            field.value.insert(field.cursor, '\n');
            field.cursor += 1;
        }
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            field.value.insert(field.cursor, c);
            field.cursor += c.len_utf8();
        }
        _ => {}
    }
}
