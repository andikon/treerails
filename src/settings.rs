use crate::{
    app::{Field, edit},
    config::{self, Config, same_path},
    startup::normalize_sources,
};
use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::{Path, PathBuf};

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
pub struct SourceInput {
    pub field: Field,
    pub index: Option<usize>,
}
pub struct Draft {
    pub base: Config,
    pub config: Config,
    pub cursor: usize,
    pub input: Option<SourceInput>,
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
            mode: Mode::Edit,
            message: "Changes apply only after Ctrl+s".into(),
            error: false,
            top_prefix: false,
        }
    }
    pub fn dirty(&self) -> bool {
        self.input.is_some() || settings_value(&self.base).ok() != settings_value(&self.config).ok()
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
                KeyCode::Enter => {
                    if let Err(error) = self.commit_input(path) {
                        self.fail(error);
                    }
                }
                _ => edit(&mut input.field, key),
            }
            return Action::None;
        }
        let count = self.config.project_roots.len() + 1;
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
            KeyCode::Char('?') => return Action::Help,
            KeyCode::Esc | KeyCode::Char('q') => return Action::Close,
            KeyCode::Char('j') | KeyCode::Down | KeyCode::Tab => {
                self.cursor = (self.cursor + 1).min(count - 1)
            }
            KeyCode::Char('k') | KeyCode::Up | KeyCode::BackTab => {
                self.cursor = self.cursor.saturating_sub(1)
            }
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
                self.input = Some(SourceInput {
                    field: Field::new("New project source", String::new(), false),
                    index: None,
                })
            }
            KeyCode::Char('d') if self.cursor < self.config.project_roots.len() => {
                self.config.project_roots.remove(self.cursor);
                self.cursor = self.cursor.min(self.config.project_roots.len());
            }
            KeyCode::Char('r') => self.config.recursive = !self.config.recursive,
            KeyCode::Enter if self.cursor == self.config.project_roots.len() => {
                self.config.recursive = !self.config.recursive
            }
            KeyCode::Enter => {
                self.input = Some(SourceInput {
                    field: Field::new(
                        "Edit project source",
                        self.config.project_roots[self.cursor].display().to_string(),
                        false,
                    ),
                    index: Some(self.cursor),
                })
            }
            KeyCode::Char('D') if self.dirty() => self.mode = Mode::Discard { quit: false },
            _ => {}
        }
        Action::None
    }
    pub fn paste(&mut self, text: &str) {
        if let Some(input) = &mut self.input {
            let text = text.replace(['\r', '\n'], " ");
            input.field.value.insert_str(input.field.cursor, &text);
            input.field.cursor += text.len();
        }
    }
    fn commit_input(&mut self, path: &Path) -> Result<()> {
        let Some(input) = &self.input else {
            return Ok(());
        };
        let value = input.field.value.trim();
        anyhow::ensure!(!value.is_empty(), "Enter a project source directory");
        let directory = path.parent().unwrap_or(Path::new("."));
        let roots = normalize_sources([PathBuf::from(value)], directory)?;
        let value = roots[0].clone();
        anyhow::ensure!(
            value.is_dir() || self.base.project_roots.iter().any(|p| same_path(p, &value)),
            "Source directory is unavailable: {}",
            value.display()
        );
        if let Some(index) = input.index {
            self.config.project_roots[index] = value;
        } else {
            self.config.project_roots.push(value);
        }
        self.config.project_roots =
            normalize_sources(self.config.project_roots.clone(), directory)?;
        self.input = None;
        self.error = false;
        self.message = "Unsaved changes · Ctrl+s Save & Apply".into();
        Ok(())
    }
    pub fn save(&mut self, path: &Path, resolution: Resolution) -> Result<Option<Config>> {
        self.commit_input(path)?;
        let base = settings_value(&self.base)?;
        let desired = settings_value(&self.config)?;
        let mut changes = Vec::new();
        changed_fields(&base, &desired, &mut Vec::new(), &mut changes);
        let mut conflicts = Vec::new();
        config::edit_document(path, |doc| {
            let current = settings_value(&Config::parse(&doc.to_string(), path)?)?;
            for keys in &changes {
                if lookup(&current, keys) != lookup(&base, keys)
                    && lookup(&current, keys) != lookup(&desired, keys)
                {
                    conflicts.push(keys.join("."));
                }
            }
            if !conflicts.is_empty() && matches!(resolution, Resolution::Ask) {
                return Ok(());
            }
            for keys in &changes {
                if matches!(resolution, Resolution::External) && conflicts.contains(&keys.join("."))
                {
                    continue;
                }
                patch(doc, keys, lookup(&desired, keys))?;
            }
            Ok(())
        })?;
        if !conflicts.is_empty() && matches!(resolution, Resolution::Ask) {
            self.mode = Mode::Conflict(conflicts);
            return Ok(None);
        }
        let loaded = Config::load(path)?;
        self.base = loaded.clone();
        self.config = loaded.clone();
        self.mode = Mode::Edit;
        self.error = false;
        self.message = "Saved & applied · projects are being rescanned".into();
        Ok(Some(loaded))
    }
    pub fn fail(&mut self, error: anyhow::Error) {
        self.message = format!("{error:#}");
        self.error = true;
    }
}

fn settings_value(config: &Config) -> Result<toml::Value> {
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
    let (key, parents) = path.split_last().context("Missing settings field")?;
    let mut table: &mut dyn toml_edit::TableLike = doc.as_table_mut();
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
