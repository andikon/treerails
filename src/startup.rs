use crate::config::{self, Config, State, same_path};
use anyhow::{Context, Result};
use clap::Parser;
use std::path::{Path, PathBuf};
#[derive(Parser, Debug)]
#[command(
    version,
    about = "Projects, Git worktrees, and native agent conversations"
)]
pub struct Options {
    /// Repositories or discovery folders to remember in state
    pub paths: Vec<PathBuf>,
    #[arg(long)]
    pub paths_file: Option<PathBuf>,
    #[arg(long)]
    pub recursive: bool,
    /// Configuration file; state.json lives beside it
    #[arg(long)]
    pub config: Option<PathBuf>,
    #[arg(long)]
    pub temporary: bool,
    /// Discover projects without creating or updating files
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
    pub fn prepare(self) -> Result<Session> {
        let cwd = std::env::current_dir()?;
        let path = match self.config {
            Some(p) => absolute(&p, &cwd)?,
            None => config::default_path()?,
        };
        let (config, mut state) = config::open(&path, !self.list)?;
        let mut supplied = self.paths;
        if let Some(file) = self.paths_file {
            let file = absolute(&file, &cwd)?;
            for line in std::fs::read_to_string(&file)
                .with_context(|| format!("Read {}", file.display()))?
                .trim_start_matches('\u{feff}')
                .lines()
                .map(str::trim)
                .filter(|s| !s.is_empty() && !s.starts_with('#'))
            {
                supplied.push(absolute(Path::new(line), file.parent().unwrap_or(&cwd))?);
            }
        }
        let supplied = normalize_sources(supplied, &cwd)?;
        if !self.list && !self.temporary && !supplied.is_empty() {
            state.remembered_sources = normalize_sources(
                state
                    .remembered_sources
                    .iter()
                    .cloned()
                    .chain(supplied.clone()),
                &cwd,
            )?;
            state.save(&config::state_path(&path))?;
        }
        let mut roots = normalize_sources(
            config
                .project_roots
                .iter()
                .cloned()
                .chain(state.remembered_sources.clone())
                .chain(supplied),
            &cwd,
        )?;
        if roots.is_empty() {
            roots.push(cwd);
        }
        let recursive = self.recursive || config.recursive;
        Ok(Session {
            config,
            state,
            config_path: path,
            roots,
            recursive,
        })
    }
}
pub fn normalize_sources(
    paths: impl IntoIterator<Item = PathBuf>,
    directory: &Path,
) -> Result<Vec<PathBuf>> {
    let mut result: Vec<PathBuf> = Vec::new();
    for path in paths {
        anyhow::ensure!(
            !path.as_os_str().is_empty(),
            "Project source cannot be empty"
        );
        let path = absolute(&path, directory)?;
        if !result.iter().any(|p| same_path(p, &path)) {
            result.push(path);
        }
    }
    Ok(result)
}
fn absolute(path: &Path, cwd: &Path) -> Result<PathBuf> {
    Ok(std::path::absolute(if path.is_absolute() {
        path.to_owned()
    } else {
        cwd.join(path)
    })?)
}
