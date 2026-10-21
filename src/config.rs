use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub project_roots: Vec<PathBuf>,
    pub recursive: bool,
    pub publication_remote: Option<String>,
    pub worktrees: WorktreeConfig,
    pub projects: Vec<ProjectConfig>,
    pub agent: AgentConfig,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorktreeConfig {
    pub path_template: String,
}
impl Default for WorktreeConfig {
    fn default() -> Self {
        Self {
            path_template: "{project_parent}/{project_name}.worktrees/{branch_slug}".into(),
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectConfig {
    pub path: PathBuf,
    pub path_template: Option<String>,
    pub publication_remote: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    pub runtime: Option<PathBuf>,
    /// Optional real CLI frontend, independent of the compatible host runtime.
    pub cli: Option<PathBuf>,
    /// Optional isolated Copilot state directory (the runtime's COPILOT_HOME).
    pub home: Option<PathBuf>,
    pub host_directory: Option<PathBuf>,
    /// Attach to an existing AHP host instead of starting an owned runtime.
    pub endpoint: Option<String>,
    pub token_env: Option<String>,
}
impl Config {
    pub fn parse(raw: &str, path: &Path) -> Result<Self> {
        let mut config: Self = toml::from_str(raw)
            .with_context(|| format!("Invalid configuration {}", path.display()))?;
        let parent = path.parent().unwrap_or(Path::new("."));
        if let Some(cli) = &mut config.agent.cli
            && cli.is_relative()
        {
            *cli = parent.join(&*cli);
        }
        if let Some(home) = &mut config.agent.home
            && home.is_relative()
        {
            *home = parent.join(&*home);
        }
        if let Some(directory) = &mut config.agent.host_directory
            && directory.is_relative()
        {
            *directory = parent.join(&*directory);
        }
        for root in &mut config.project_roots {
            if root.is_relative() {
                *root = parent.join(&*root);
            }
        }
        for project in &mut config.projects {
            if project.path.is_relative() {
                project.path = parent.join(&project.path);
            }
        }
        anyhow::ensure!(
            !config.worktrees.path_template.trim().is_empty(),
            "Worktree path template cannot be empty"
        );
        Ok(config)
    }
    pub fn worktree_path(&self, project: &Path, branch: &str) -> PathBuf {
        let template = self
            .projects
            .iter()
            .find(|p| same_path(&p.path, project))
            .and_then(|p| p.path_template.as_ref())
            .unwrap_or(&self.worktrees.path_template);
        let result = PathBuf::from(
            template
                .replace(
                    "{project_parent}",
                    &project.parent().unwrap_or(project).to_string_lossy(),
                )
                .replace(
                    "{project_name}",
                    &project.file_name().unwrap_or_default().to_string_lossy(),
                )
                .replace("{branch_slug}", &branch.replace(['/', '\\'], "-")),
        );
        if result.is_absolute() {
            result
        } else {
            project.join(result)
        }
    }
    /// Launch the native host in the shared declared workspace so its filesystem
    /// scope includes sibling worktrees. This is derived memory, never config.
    pub fn agent_for_sources(
        &self,
        sources: &[PathBuf],
        projects: &[crate::git::Project],
    ) -> AgentConfig {
        let mut agent = self.agent.clone();
        if agent.host_directory.is_some() {
            return agent;
        }
        let mut paths = sources.to_vec();
        for project in sources
            .iter()
            .filter(|p| p.join(".git").exists())
            .chain(projects.iter().map(|p| &p.path))
        {
            paths.push(project.clone());
            if let Some(parent) = self.worktree_path(project, "treerail-scope").parent() {
                paths.push(parent.to_path_buf());
            }
        }
        if let Some(first) = paths.first() {
            agent.host_directory = first
                .ancestors()
                .find(|root| {
                    root.is_dir()
                        && paths
                            .iter()
                            .all(|path| path.ancestors().any(|p| same_path(p, root)))
                })
                .map(Path::to_path_buf);
        }
        agent
    }
    pub fn remote(&self, project: &Path) -> Option<String> {
        self.projects
            .iter()
            .find(|p| same_path(&p.path, project))
            .and_then(|p| p.publication_remote.clone())
            .or_else(|| self.publication_remote.clone())
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Navigation {
    pub worktree: Option<PathBuf>,
    pub conversation: Option<String>,
    pub changes: bool,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub remembered_sources: Vec<PathBuf>,
    pub known_projects: Vec<crate::git::Project>,
    pub tabs: Vec<PathBuf>,
    pub last_project: Option<PathBuf>,
    pub last_worktree: Option<PathBuf>,
    pub navigation: BTreeMap<String, Navigation>,
    pub conversations: Vec<crate::agent::Conversation>,
    #[serde(skip)]
    baseline: Option<serde_json::Value>,
}
impl State {
    pub fn save(&mut self, path: &Path) -> Result<()> {
        with_lock(path, || {
            let current = serde_json::to_value(&*self)?;
            let mut disk: serde_json::Value = match read_optional(path)? {
                Some(raw) => serde_json::from_str(&raw)
                    .context("Invalid state; original file was preserved")?,
                None => serde_json::to_value(State::default())?,
            };
            let _: State = serde_json::from_value(disk.clone())
                .context("Invalid state; original file was preserved")?;
            for (key, value) in current.as_object().context("Invalid state")? {
                if self.baseline.as_ref().and_then(|b| b.get(key)) == Some(value) {
                    continue;
                }
                if key == "conversations" {
                    let mut records: BTreeMap<String, serde_json::Value> = disk[key]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|v| Some((v.get("id")?.as_str()?.to_owned(), v.clone())))
                        .collect();
                    let previous: BTreeMap<String, serde_json::Value> = self
                        .baseline
                        .as_ref()
                        .and_then(|b| b.get(key))
                        .and_then(|v| v.as_array())
                        .into_iter()
                        .flatten()
                        .filter_map(|v| Some((v.get("id")?.as_str()?.to_owned(), v.clone())))
                        .collect();
                    for record in value.as_array().into_iter().flatten() {
                        if let Some(id) = record["id"].as_str()
                            && previous.get(id) != Some(record)
                        {
                            if let (Some(old), Some(saved)) =
                                (previous.get(id), records.get_mut(id))
                            {
                                if let Some(fields) = record.as_object() {
                                    for (key, value) in fields {
                                        if old.get(key) != Some(value) {
                                            saved[key] = value.clone();
                                        }
                                    }
                                }
                            } else {
                                records.insert(id.to_owned(), record.clone());
                            }
                        }
                    }
                    disk[key] = serde_json::Value::Array(records.into_values().collect());
                } else if key == "remembered_sources" {
                    let previous = self
                        .baseline
                        .as_ref()
                        .and_then(|b| b.get(key))
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();
                    let wanted = value.as_array().cloned().unwrap_or_default();
                    let mut merged = disk[key].as_array().cloned().unwrap_or_default();
                    merged.retain(|v| !previous.contains(v) || wanted.contains(v));
                    for path in wanted {
                        if !merged.contains(&path) {
                            merged.push(path);
                        }
                    }
                    disk[key] = serde_json::Value::Array(merged);
                } else if key == "navigation" {
                    if let Some(entries) = value.as_object() {
                        for (name, entry) in entries {
                            if self
                                .baseline
                                .as_ref()
                                .and_then(|b| b.get(key))
                                .and_then(|v| v.get(name))
                                != Some(entry)
                            {
                                disk[key][name] = entry.clone();
                            }
                        }
                    }
                } else {
                    disk[key] = value.clone();
                }
            }
            atomic_write(path, &serde_json::to_vec_pretty(&disk)?)?;
            self.baseline = Some(current);
            Ok(())
        })
    }
}
pub fn default_path() -> Result<PathBuf> {
    #[cfg(windows)]
    let directory = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .context("Cannot determine LOCALAPPDATA; use --config")?;
    #[cfg(not(windows))]
    let directory = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|d| d.home_dir().join(".config")))
        .context("Cannot determine config directory; use --config")?;
    Ok(directory.join("treerails").join("config.toml"))
}
pub fn state_path(config: &Path) -> PathBuf {
    config.parent().unwrap_or(Path::new(".")).join("state.json")
}
/// Load settings and memory before terminal initialization. A complete backup
/// protects old workflow records when splitting a combined document.
pub fn open(path: &Path, create: bool) -> Result<(Config, State)> {
    let source = read_optional(path)?.unwrap_or(toml::to_string_pretty(&Config::default())?);
    let config = Config::parse(&source, path)?;
    let document: toml_edit::DocumentMut = source.parse()?;
    let state_file = state_path(path);
    let mut state = if let Some(raw) = read_optional(&state_file)? {
        serde_json::from_str(&raw)
            .with_context(|| format!("Invalid state {}", state_file.display()))?
    } else if document.get("state").is_some() {
        let raw: toml::Value = toml::from_str(&source)?;
        raw.get("state")
            .cloned()
            .map(|v| v.try_into::<State>())
            .transpose()
            .context("Migrate remembered selections")?
            .unwrap_or_default()
    } else {
        State::default()
    };
    state.baseline = Some(serde_json::to_value(&state)?);
    if create {
        if document.get("state").is_some() {
            with_lock(path, || {
                let latest =
                    read_optional(path)?.context("Configuration was removed during migration")?;
                Config::parse(&latest, path)?;
                let mut document: toml_edit::DocumentMut = latest.parse()?;
                if document.get("state").is_none() {
                    return Ok(());
                }
                let mut backup = path.with_extension("pre-worktree-refactor.toml");
                if read_optional(&backup)?.is_some_and(|old| old != latest) {
                    backup = path.with_extension(format!(
                        "pre-worktree-refactor.{}.toml",
                        uuid::Uuid::new_v4()
                    ));
                }
                if !backup.exists() {
                    let mut file = fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&backup)?;
                    file.write_all(latest.as_bytes())?;
                    file.sync_all()?;
                }
                with_lock(&state_file, || {
                    if !state_file.exists() {
                        let raw: toml::Value = toml::from_str(&latest)?;
                        let imported = raw
                            .get("state")
                            .cloned()
                            .context("Missing legacy state")?
                            .try_into::<State>()?;
                        atomic_write(&state_file, &serde_json::to_vec_pretty(&imported)?)?;
                    }
                    Ok(())
                })?;
                document.remove("state");
                atomic_write(path, document.to_string().as_bytes())
            })?;
            return open(path, false);
        } else if !path.exists() {
            with_lock(path, || {
                if !path.exists() {
                    atomic_write(path, source.as_bytes())?;
                }
                Ok(())
            })?;
            return open(path, false);
        }
    }
    Ok((config, state))
}
pub fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(contents)?;
    file.as_file().sync_all()?;
    file.persist(path)
        .map_err(|e| e.error)
        .with_context(|| format!("Save {}", path.display()))?;
    Ok(())
}
fn with_lock<T>(path: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
    fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("lock"))?;
    fs2::FileExt::lock_exclusive(&lock)?;
    f()
}
pub fn save_preferences(path: &Path, baseline: &Config, desired: &Config) -> Result<Config> {
    with_lock(path, || {
        let raw = read_optional(path)?
            .context("Configuration was removed; keep the draft and restore the file")?;
        let current = Config::parse(&raw, path)?;
        let current_value = serde_json::to_value(&current)?;
        let before = serde_json::to_value(baseline)?;
        let after = serde_json::to_value(desired)?;
        let mut document: toml_edit::DocumentMut = raw.parse()?;
        let proposed: toml_edit::DocumentMut = toml::to_string_pretty(desired)?.parse()?;
        for key in [
            "project_roots",
            "recursive",
            "publication_remote",
            "worktrees",
        ] {
            if before[key] == after[key] {
                continue;
            }
            anyhow::ensure!(
                current_value[key] == before[key] || current_value[key] == after[key],
                "'{key}' was changed by another editor; draft retained. Reopen preferences to review the latest value"
            );
            if let Some(item) = proposed.get(key) {
                let mut item = item.clone();
                if let (Some(old), Some(new)) = (
                    document.get(key).and_then(|i| i.as_value()),
                    item.as_value_mut(),
                ) {
                    *new.decor_mut() = old.decor().clone();
                }
                document[key] = item;
            } else {
                document.remove(key);
            }
        }
        let updated = Config::parse(&document.to_string(), path)?;
        atomic_write(path, document.to_string().as_bytes())?;
        Ok(updated)
    })
}
fn read_optional(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(raw) => Ok(Some(raw)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("Read {}", path.display())),
    }
}
pub fn same_path(a: &Path, b: &Path) -> bool {
    let a = a.canonicalize().unwrap_or_else(|_| a.to_owned());
    let b = b.canonicalize().unwrap_or_else(|_| b.to_owned());
    if cfg!(windows) {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    } else {
        a == b
    }
}
