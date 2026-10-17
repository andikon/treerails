pub mod editor;
mod merge;
use crate::config::{self, Config};
use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use editor::Editor;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Sources,
    Worktrees,
    Profiles,
    Notifications,
}
impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Self::Sources => "Sources",
            Self::Worktrees => "Worktrees",
            Self::Profiles => "Profiles",
            Self::Notifications => "Notifications",
        }
    }
}

pub enum Mode {
    Edit,
    Discard { quit: bool },
    Conflict(Vec<String>),
}
#[derive(Clone, Copy)]
pub enum Resolution {
    Ask,
    Mine,
    External,
}
pub enum Action {
    None,
    Close,
    Save(Resolution),
    Discard(bool),
    Help,
    Quit,
}
pub struct Draft {
    pub base: Config,
    pub config: Config,
    pub cursor: usize,
    pub input: Option<Editor>,
    pub section: Section,
    pub project_context: Option<PathBuf>,
    pub unavailable_sources: std::collections::BTreeSet<PathBuf>,
    pub mode: Mode,
    pub message: String,
    pub error: bool,
    top_prefix: bool,
}
impl Draft {
    pub fn new(config: Config) -> Self {
        Self {
            base: config.clone(),
            config,
            cursor: 0,
            input: None,
            section: Section::Sources,
            project_context: None,
            unavailable_sources: Default::default(),
            mode: Mode::Edit,
            message: "Changes apply only after Ctrl+s".into(),
            error: false,
            top_prefix: false,
        }
    }
    pub fn dirty(&self) -> bool {
        self.input.is_some()
            || comparable_settings(&self.base).ok() != comparable_settings(&self.config).ok()
    }
    pub fn key(&mut self, key: KeyEvent, path: &Path, height: usize) -> Action {
        match &self.mode {
            Mode::Discard { quit } => {
                return match key.code {
                    KeyCode::Char('y') => Action::Discard(*quit),
                    _ => {
                        self.mode = Mode::Edit;
                        Action::None
                    }
                };
            }
            Mode::Conflict(_) => {
                return match key.code {
                    KeyCode::Char('m') => Action::Save(Resolution::Mine),
                    KeyCode::Char('e') => Action::Save(Resolution::External),
                    _ => {
                        self.mode = Mode::Edit;
                        Action::None
                    }
                };
            }
            Mode::Edit => {}
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
            return Action::Save(Resolution::Ask);
        }
        if let Some(input) = &mut self.input {
            match key.code {
                KeyCode::Esc => return Action::Close,
                _ => {
                    if input.key(key)
                        && let Err(error) = self.commit_input(path)
                    {
                        self.fail(error);
                    }
                }
            }
            return Action::None;
        }
        let count = self.rows().len().max(1);
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('d') => {
                    self.cursor = self
                        .cursor
                        .saturating_add(height.saturating_sub(10) / 2)
                        .min(count - 1)
                }
                KeyCode::Char('u') => {
                    self.cursor = self.cursor.saturating_sub(height.saturating_sub(10) / 2)
                }
                KeyCode::Char('c') => return Action::Quit,
                _ => {}
            }
            return Action::None;
        }
        if key.code != KeyCode::Char('g') {
            self.top_prefix = false;
        }
        match key.code {
            KeyCode::Char('1') => {
                self.section = Section::Sources;
                self.cursor = 0;
            }
            KeyCode::Char('2') => {
                self.section = Section::Worktrees;
                self.cursor = 0;
            }
            KeyCode::Char('3') => {
                self.section = Section::Profiles;
                self.cursor = 0;
            }
            KeyCode::Char('4') => {
                self.section = Section::Notifications;
                self.cursor = 0;
            }
            KeyCode::Enter | KeyCode::Char(' ') if self.section == Section::Notifications => {
                self.config.desktop_notifications = !self.config.desktop_notifications;
            }
            KeyCode::Char('h') | KeyCode::Left | KeyCode::BackTab => self.switch_section(-1),
            KeyCode::Char('l') | KeyCode::Right | KeyCode::Tab => self.switch_section(1),
            KeyCode::Char('?') => return Action::Help,
            KeyCode::Esc | KeyCode::Char('q') => return Action::Close,
            KeyCode::Char('j') | KeyCode::Down => self.cursor = (self.cursor + 1).min(count - 1),
            KeyCode::Char('k') | KeyCode::Up => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Char('G') | KeyCode::End => self.cursor = count - 1,
            KeyCode::Char('g') => {
                if self.top_prefix {
                    self.cursor = 0;
                    self.top_prefix = false;
                } else {
                    self.top_prefix = true;
                }
            }
            KeyCode::Home => self.cursor = 0,
            KeyCode::Char('a') => {
                self.input = Some(match self.section {
                    Section::Sources => Editor::source(String::new(), None),
                    Section::Worktrees => Editor::project(&self.config, None),
                    Section::Profiles => Editor::profile(&self.config, None),
                    Section::Notifications => return Action::None,
                });
            }
            KeyCode::Char('d')
                if self.section == Section::Sources
                    && self.cursor < self.config.project_roots.len() =>
            {
                self.config.project_roots.remove(self.cursor);
                self.cursor = self.cursor.min(self.config.project_roots.len());
            }
            KeyCode::Char('r') if self.section == Section::Sources => {
                self.config.recursive = !self.config.recursive
            }
            KeyCode::Enter
                if self.section == Section::Sources
                    && self.cursor == self.config.project_roots.len() =>
            {
                self.config.recursive = !self.config.recursive
            }
            KeyCode::Enter => {
                self.input = Some(match self.section {
                    Section::Sources => Editor::source(
                        self.config.project_roots[self.cursor].display().to_string(),
                        Some(self.cursor),
                    ),
                    Section::Worktrees if self.cursor == 0 => Editor::global(&self.config),
                    Section::Worktrees => Editor::project(
                        &self.config,
                        self.worktree_projects().get(self.cursor - 1).cloned(),
                    ),
                    Section::Profiles => Editor::profile(
                        &self.config,
                        self.config.profiles.keys().nth(self.cursor).cloned(),
                    ),
                    Section::Notifications => return Action::None,
                });
            }
            KeyCode::Char('D') if self.dirty() => self.mode = Mode::Discard { quit: false },
            _ => {}
        }
        Action::None
    }
    fn switch_section(&mut self, step: i32) {
        let index = match self.section {
            Section::Sources => 0,
            Section::Worktrees => 1,
            Section::Profiles => 2,
            Section::Notifications => 3,
        };
        self.section = match (index + step).rem_euclid(4) {
            0 => Section::Sources,
            1 => Section::Worktrees,
            2 => Section::Profiles,
            _ => Section::Notifications,
        };
        self.cursor = 0;
    }
    fn worktree_projects(&self) -> Vec<PathBuf> {
        let mut projects: Vec<PathBuf> = self.project_context.iter().cloned().collect();
        for project in &self.config.projects {
            if !projects.contains(&project.path) {
                projects.push(project.path.clone());
            }
        }
        projects
    }
    pub fn rows(&self) -> Vec<String> {
        match self.section {
            Section::Notifications => vec![format!(
                " Desktop notifications: {}  [Enter toggles; input requests and session failures only]",
                if self.config.desktop_notifications {
                    "on"
                } else {
                    "off"
                }
            )],
            Section::Sources => {
                let mut rows: Vec<String> = self
                    .config
                    .project_roots
                    .iter()
                    .map(|path| {
                        format!(
                            " {}{}",
                            path.display(),
                            if self.unavailable_sources.contains(path) {
                                "  [unavailable; kept]"
                            } else {
                                ""
                            }
                        )
                    })
                    .collect();
                rows.push(format!(
                    " Recursive discovery: {}  [r toggles]",
                    if self.config.recursive { "on" } else { "off" }
                ));
                rows
            }
            Section::Worktrees => {
                let mut rows = vec![format!(
                    " Global defaults · {} · profile {}",
                    self.config.worktrees.path_template, self.config.default_profile
                )];
                rows.extend(self.worktree_projects().iter().map(|path| {
                    format!(
                        " {} · base {}",
                        path.display(),
                        self.config
                            .projects
                            .iter()
                            .find(|project| project.path == *path)
                            .and_then(|p| p.base.as_deref())
                            .unwrap_or("remembered / Git default")
                    )
                }));
                rows
            }
            Section::Profiles => self
                .config
                .profiles
                .iter()
                .map(|(name, profile)| {
                    format!(
                        " {name} · {} · terminal {}",
                        if profile.program.is_empty() {
                            "plain terminal"
                        } else {
                            &profile.program
                        },
                        profile.terminal
                    )
                })
                .collect(),
        }
    }
    pub fn update_warnings(&mut self, warnings: &[String]) {
        self.unavailable_sources = self
            .config
            .project_roots
            .iter()
            .filter(|path| {
                warnings
                    .iter()
                    .any(|warning| warning.starts_with(&format!("{}:", path.display())))
            })
            .cloned()
            .collect();
    }
    pub fn paste(&mut self, text: &str) {
        if let Some(editor) = &mut self.input {
            editor.paste(text);
        }
    }
    fn commit_input(&mut self, path: &Path) -> Result<()> {
        let Some(editor) = &self.input else {
            return Ok(());
        };
        editor.commit(&mut self.config, &self.base, path)?;
        self.input = None;
        self.error = false;
        self.message = "Unsaved changes · Ctrl+s Save & Apply".into();
        Ok(())
    }
    pub fn save(&mut self, path: &Path, resolution: Resolution) -> Result<Option<Config>> {
        if let Some(editor) = &self.input {
            let mut candidate = self.config.clone();
            editor.commit(&mut candidate, &self.base, path)?;
            candidate.validate()?;
            self.config = candidate;
            self.input = None;
        } else {
            self.config.validate()?;
        }
        let base = settings_value(&self.base)?;
        let desired = settings_value(&self.config)?;
        let mut changes = Vec::new();
        changed_fields(&base, &desired, &mut Vec::new(), &mut changes);
        let mut conflicts = Vec::new();
        config::edit_document(path, |doc| {
            let current = settings_value(&Config::parse(&doc.to_string(), path)?)?;
            let conflict_flags: Vec<bool> = changes
                .iter()
                .map(|keys| {
                    merge::deleted_entity(&base, &current, keys)
                        || (lookup(&current, keys) != lookup(&base, keys)
                            && lookup(&current, keys) != lookup(&desired, keys))
                })
                .collect();
            for (keys, conflict) in changes.iter().zip(&conflict_flags) {
                if *conflict {
                    conflicts.push(keys.join("."));
                }
            }
            if !conflicts.is_empty() && matches!(resolution, Resolution::Ask) {
                return Ok(());
            }
            for (keys, conflict) in changes.iter().zip(conflict_flags) {
                if matches!(resolution, Resolution::External) && conflict {
                    continue;
                }
                if keys.first().is_some_and(|key| key == "projects") {
                    merge::patch_project(doc, keys, &desired, path)?;
                } else {
                    if keys.first().is_some_and(|key| key == "profiles")
                        && keys.len() > 2
                        && doc
                            .get("profiles")
                            .and_then(|profiles| profiles.get(&keys[1]))
                            .is_none()
                    {
                        let entity = &keys[..2];
                        let profile = lookup(&current, entity).or_else(|| lookup(&desired, entity));
                        patch(doc, entity, profile)?;
                    }
                    patch(doc, keys, lookup(&desired, keys))?;
                }
            }
            Config::parse(&doc.to_string(), path)?.validate()?;
            Ok(())
        })?;
        if !conflicts.is_empty() && matches!(resolution, Resolution::Ask) {
            self.mode = Mode::Conflict(conflicts);
            return Ok(None);
        }
        let loaded = Config::load(path)?;
        self.base = loaded.clone();
        self.config = loaded.clone();
        self.cursor = self.cursor.min(self.rows().len().saturating_sub(1));
        self.mode = Mode::Edit;
        self.error = false;
        self.message = "Saved & applied".into();
        Ok(Some(loaded))
    }
    pub fn fail(&mut self, error: anyhow::Error) {
        self.message = format!("{error:#}");
        self.error = true;
    }
}

fn settings_value(config: &Config) -> Result<toml::Value> {
    let mut value = comparable_settings(config)?;
    let mut projects = toml::map::Map::new();
    for project in &config.projects {
        projects
            .entry(merge::identity(&project.path))
            .or_insert(toml::Value::try_from(project)?);
    }
    value["projects"] = toml::Value::Table(projects);
    Ok(value)
}
fn comparable_settings(config: &Config) -> Result<toml::Value> {
    let mut value = toml::Value::try_from(config)?;
    value
        .as_table_mut()
        .context("Configuration must be a table")?
        .remove("state");
    Ok(value)
}
fn changed_fields(
    base: &toml::Value,
    desired: &toml::Value,
    path: &mut Vec<String>,
    changes: &mut Vec<Vec<String>>,
) {
    if let (Some(base), Some(desired)) = (base.as_table(), desired.as_table()) {
        let keys: std::collections::BTreeSet<_> = base.keys().chain(desired.keys()).collect();
        for key in keys {
            path.push(key.clone());
            match (base.get(key), desired.get(key)) {
                (Some(a), Some(b)) => changed_fields(a, b, path, changes),
                (None, Some(b)) if b.is_table() => {
                    changed_fields(&toml::Value::Table(Default::default()), b, path, changes)
                }
                _ => changes.push(path.clone()),
            }
            path.pop();
        }
    } else if base != desired {
        changes.push(path.clone());
    }
}
fn lookup<'a>(mut value: &'a toml::Value, path: &[String]) -> Option<&'a toml::Value> {
    for key in path {
        value = value.get(key)?;
    }
    Some(value)
}
fn patch(
    doc: &mut toml_edit::DocumentMut,
    path: &[String],
    value: Option<&toml::Value>,
) -> Result<()> {
    patch_table(doc.as_table_mut(), path, value)
}
fn patch_table(
    mut table: &mut dyn toml_edit::TableLike,
    path: &[String],
    value: Option<&toml::Value>,
) -> Result<()> {
    let (key, parents) = path.split_last().context("Missing settings field")?;
    for parent in parents {
        if table.get(parent).is_none() {
            table.insert(parent, toml_edit::Item::Table(toml_edit::Table::new()));
        }
        table = table
            .get_mut(parent)
            .and_then(toml_edit::Item::as_table_like_mut)
            .context("Invalid settings table")?;
    }
    if let Some(value) = value {
        let mut item: toml_edit::DocumentMut = format!("value = {}", value).parse()?;
        let mut replacement = item.remove("value").context("Missing settings value")?;
        if let (Some(old), Some(new)) = (
            table.get(key).and_then(toml_edit::Item::as_value),
            replacement.as_value_mut(),
        ) {
            *new.decor_mut() = old.decor().clone();
        }
        table.insert(key, replacement);
    } else {
        table.remove(key);
    }
    Ok(())
}
