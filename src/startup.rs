//! CLI parsing and durable session startup, independent of terminal ownership.
use crate::config::{self, Config, State, same_path};
use anyhow::{Context, Result};
use clap::Parser;
use std::path::{Path, PathBuf};

#[derive(Parser, Debug)]
#[command(version, about = "A keyboard-first project and Git worktree explorer")]
pub struct Options {
    /// Repositories or parent folders to explore and remember
    pub paths: Vec<PathBuf>,
    /// File containing one project source directory per line
    #[arg(long)]
    pub paths_file: Option<PathBuf>,
    /// Discover repositories recursively (excluding .worktrees folders)
    #[arg(long)]
    pub recursive: bool,
    /// Combined settings and remembered selections file
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Use supplied directories for this session without remembering them
    #[arg(long)]
    pub temporary: bool,
    /// Print discovered projects as JSON without changing files
    #[arg(long)]
    pub list: bool,
}

pub struct Session {
    pub config: Config,
    pub state: State,
    pub config_path: PathBuf,
    pub roots: Vec<PathBuf>,
    pub recursive: bool,
}

impl Options {
    /// Call only after the binary has established interactive terminal ownership,
    /// unless --list is set. Listing takes the same read-only startup path.
    pub fn prepare(self) -> Result<Session> {
        let cwd = std::env::current_dir()?;
        let legacy = if self.config.is_none() {
            Some(config::legacy_paths()?)
        } else {
            None
        };
        let config_path = match self.config {
            Some(path) => absolute(&path, &cwd)?,
            None => config::default_path()?,
        };
        let mut supplied = self.paths;
        if let Some(path) = self.paths_file {
            let path = absolute(&path, &cwd)?;
            let raw = std::fs::read_to_string(&path)
                .with_context(|| format!("Read paths file {}", path.display()))?;
            let parent = path.parent().unwrap_or(&cwd);
            for entry in raw
                .trim_start_matches('\u{feff}')
                .lines()
                .map(str::trim)
                .filter(|entry| !entry.is_empty() && !entry.starts_with('#'))
            {
                supplied.push(absolute(Path::new(entry), parent)?);
            }
        }
        let supplied = normalize_sources(supplied, &cwd)?;
        let (mut config, state) = config::open(&config_path, !self.list, legacy)?;
        if !self.list && !self.temporary && !supplied.is_empty() {
            config::edit_document(&config_path, |doc| {
                let current = Config::parse(&doc.to_string(), &config_path)?;
                let roots = normalize_sources(
                    current.project_roots.into_iter().chain(supplied.clone()),
                    &cwd,
                )?;
                let mut array = toml_edit::Array::new();
                for path in roots {
                    array.push(path.to_string_lossy().as_ref());
                }
                let mut value = toml_edit::Value::Array(array);
                if let Some(old) = doc.get("project_roots").and_then(toml_edit::Item::as_value) {
                    *value.decor_mut() = old.decor().clone();
                }
                doc["project_roots"] = toml_edit::Item::Value(value);
                Ok(())
            })?;
            config = Config::load(&config_path)?;
        }
        let mut roots =
            normalize_sources(config.project_roots.iter().cloned().chain(supplied), &cwd)?;
        if roots.is_empty() {
            roots.push(cwd);
        }
        let recursive = self.recursive || config.recursive;
        Ok(Session {
            config,
            state,
            config_path,
            roots,
            recursive,
        })
    }
}

pub fn normalize_sources(
    paths: impl IntoIterator<Item = PathBuf>,
    directory: &Path,
) -> Result<Vec<PathBuf>> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for path in paths {
        anyhow::ensure!(
            !path.as_os_str().is_empty(),
            "Project source cannot be empty"
        );
        let path = absolute(&path, directory)?;
        if !roots.iter().any(|root| same_path(root, &path)) {
            roots.push(path);
        }
    }
    Ok(roots)
}
fn absolute(path: &Path, directory: &Path) -> Result<PathBuf> {
    std::path::absolute(if path.is_absolute() {
        path.to_owned()
    } else {
        directory.join(path)
    })
    .with_context(|| format!("Resolve source {}", path.display()))
}
