use crate::{
    app::{Field, edit},
    config::{Config, Profile, ProjectConfig, same_path},
    startup::normalize_sources,
};
use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub enum Kind {
    Source(Option<usize>),
    Global,
    Project(Option<PathBuf>),
    Profile(Option<String>),
}
pub struct Editor {
    pub title: String,
    pub fields: Vec<Field>,
    pub active: usize,
    pub kind: Kind,
}
impl Editor {
    pub fn source(value: String, index: Option<usize>) -> Self {
        Self {
            title: "Project source".into(),
            fields: vec![Field::new("Source directory", value, false)],
            active: 0,
            kind: Kind::Source(index),
        }
    }
    pub fn global(config: &Config) -> Self {
        Self {
            title: "Global worktree defaults".into(),
            fields: vec![
                Field::new(
                    "Worktree path template",
                    config.worktrees.path_template.clone(),
                    false,
                ),
                Field::new(
                    "Default launch profile",
                    config.default_profile.clone(),
                    false,
                ),
            ],
            active: 0,
            kind: Kind::Global,
        }
    }
    pub fn project(config: &Config, path: Option<PathBuf>) -> Self {
        let configured = path.as_deref().and_then(|path| config.project(path));
        let mut directory = Field::new(
            "Project directory",
            path.as_deref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            false,
        );
        directory.readonly = path.is_some();
        Self {
            title: "Project defaults · blank values inherit".into(),
            fields: vec![
                directory,
                Field::new(
                    "Starting point · blank uses remembered / Git default",
                    configured.and_then(|p| p.base.clone()).unwrap_or_default(),
                    false,
                ),
                Field::new(
                    "Worktree path · blank uses global template",
                    configured
                        .and_then(|p| p.path_template.clone())
                        .unwrap_or_default(),
                    false,
                ),
                Field::new(
                    "Launch profile · blank uses global profile",
                    configured
                        .and_then(|p| p.default_profile.clone())
                        .unwrap_or_default(),
                    false,
                ),
            ],
            active: usize::from(path.is_some()),
            kind: Kind::Project(path),
        }
    }
    pub fn profile(config: &Config, name: Option<String>) -> Self {
        let profile = name
            .as_ref()
            .and_then(|name| config.profiles.get(name))
            .cloned()
            .unwrap_or_default();
        let mut name_field = Field::new("Profile name", name.clone().unwrap_or_default(), false);
        name_field.readonly = name.is_some();
        Self {
            title: "Launch profile".into(),
            fields: vec![
                name_field,
                Field::new(
                    "Program · blank opens a plain terminal",
                    profile.program,
                    false,
                ),
                Field::new(
                    "Arguments · TOML array, e.g. ['.', '--flag']",
                    toml::Value::try_from(profile.args)
                        .unwrap_or(toml::Value::Array(vec![]))
                        .to_string(),
                    false,
                ),
                Field::new("Working directory template", profile.cwd, false),
                Field::new(
                    "Environment · TOML map, e.g. { MODE = 'dev' }",
                    toml::Value::try_from(profile.env)
                        .unwrap_or(toml::Value::Table(Default::default()))
                        .to_string(),
                    false,
                ),
                Field::new(
                    "Terminal adapter · auto or a configured name",
                    profile.terminal,
                    false,
                ),
            ],
            active: usize::from(name.is_some()),
            kind: Kind::Profile(name),
        }
    }
    /// Returns true when Enter accepts the final field.
    pub fn key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('u') {
            let field = &mut self.fields[self.active];
            if !field.readonly {
                field.value.drain(..field.cursor);
                field.cursor = 0;
            }
            return false;
        }
        match key.code {
            KeyCode::Tab => self.active = (self.active + 1) % self.fields.len(),
            KeyCode::BackTab => {
                self.active = (self.active + self.fields.len() - 1) % self.fields.len()
            }
            KeyCode::Enter if self.active + 1 == self.fields.len() => return true,
            KeyCode::Enter => self.active += 1,
            _ => edit(&mut self.fields[self.active], key),
        }
        false
    }
    pub fn paste(&mut self, text: &str) {
        let field = &mut self.fields[self.active];
        if !field.readonly {
            let text = text.replace(['\r', '\n'], " ");
            field.value.insert_str(field.cursor, &text);
            field.cursor += text.len();
        }
    }
    pub fn commit(&self, config: &mut Config, base: &Config, path: &Path) -> Result<()> {
        let value = |index: usize| self.fields[index].value.trim().to_owned();
        let optional = |index: usize| {
            let value = self.fields[index].value.clone();
            if value.trim().is_empty() {
                None
            } else {
                Some(value)
            }
        };
        match &self.kind {
            Kind::Source(index) => {
                let value = value(0);
                anyhow::ensure!(!value.is_empty(), "Enter a project source directory");
                let directory = path.parent().unwrap_or(Path::new("."));
                let sources = normalize_sources([PathBuf::from(value)], directory)?;
                let source = sources[0].clone();
                anyhow::ensure!(
                    source.is_dir() || base.project_roots.iter().any(|p| same_path(p, &source)),
                    "Source directory is unavailable: {}",
                    source.display()
                );
                if let Some(index) = index {
                    config.project_roots[*index] = source;
                } else {
                    config.project_roots.push(source);
                }
                config.project_roots = normalize_sources(config.project_roots.clone(), directory)?;
            }
            Kind::Global => {
                config.worktrees.path_template = self.fields[0].value.clone();
                config.default_profile = self.fields[1].value.clone();
            }
            Kind::Project(identity) => {
                anyhow::ensure!(!value(0).is_empty(), "Enter a project directory");
                let directory = path.parent().unwrap_or(Path::new("."));
                let paths = normalize_sources([PathBuf::from(value(0))], directory)?;
                let project_path = paths[0].clone();
                anyhow::ensure!(
                    project_path.is_dir() || base.project(&project_path).is_some(),
                    "Project directory is unavailable: {}",
                    project_path.display()
                );
                let existing = config
                    .projects
                    .iter()
                    .position(|p| same_path(&p.path, &project_path));
                anyhow::ensure!(
                    identity.is_some() || existing.is_none(),
                    "This project already has defaults; edit its existing entry"
                );
                let project = ProjectConfig {
                    path: project_path,
                    base: optional(1),
                    path_template: optional(2),
                    default_profile: optional(3),
                };
                if let Some(index) = existing {
                    config.projects[index] = project;
                } else {
                    config.projects.push(project);
                }
            }
            Kind::Profile(identity) => {
                let name = identity.clone().unwrap_or_else(|| value(0));
                anyhow::ensure!(!name.is_empty(), "Enter a profile name");
                anyhow::ensure!(
                    identity.is_some() || !config.profiles.contains_key(&name),
                    "Profile '{name}' already exists; edit its existing entry"
                );
                #[derive(serde::Deserialize)]
                struct Arguments {
                    value: Vec<String>,
                }
                #[derive(serde::Deserialize)]
                struct Environment {
                    value: BTreeMap<String, String>,
                }
                let args: Arguments = toml::from_str(&format!("value = {}", value(2)))
                    .context("Arguments must be a TOML array of strings")?;
                let env: Environment = toml::from_str(&format!("value = {}", value(4)))
                    .context("Environment must be a TOML map of string values")?;
                config.profiles.insert(
                    name,
                    Profile {
                        program: self.fields[1].value.clone(),
                        args: args.value,
                        cwd: self.fields[3].value.clone(),
                        env: env.value,
                        terminal: self.fields[5].value.clone(),
                    },
                );
            }
        }
        Ok(())
    }
}
