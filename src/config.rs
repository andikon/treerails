use anyhow::{Context, Result};
use directories::{BaseDirs, ProjectDirs};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub project_roots: Vec<PathBuf>,
    pub recursive: bool,
    pub default_profile: String,
    pub worktrees: WorktreeConfig,
    pub profiles: BTreeMap<String, Profile>,
    pub terminals: BTreeMap<String, TerminalConfig>,
    pub projects: Vec<ProjectConfig>,
    pub keys: BTreeMap<String, String>,
    pub theme: Theme,
    pub state: State,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
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
#[serde(default, deny_unknown_fields)]
pub struct ProjectConfig {
    pub path: PathBuf,
    pub base: Option<String>,
    pub path_template: Option<String>,
    pub default_profile: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub terminal: String,
    pub env: BTreeMap<String, String>,
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            program: String::new(),
            args: vec![],
            cwd: "{worktree}".into(),
            terminal: "auto".into(),
            env: BTreeMap::new(),
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TerminalConfig {
    pub program: String,
    pub args: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Theme {
    pub accent: String,
    pub added: String,
    pub deleted: String,
    pub modified: String,
}
impl Default for Theme {
    fn default() -> Self {
        Self {
            accent: "#7dd3fc".into(),
            added: "#86efac".into(),
            deleted: "#fda4af".into(),
            modified: "#fde68a".into(),
        }
    }
}
impl Default for Config {
    fn default() -> Self {
        Self {
            project_roots: vec![],
            recursive: false,
            default_profile: "copilot".into(),
            worktrees: WorktreeConfig::default(),
            profiles: BTreeMap::from([
                (
                    "copilot".into(),
                    Profile {
                        program: "copilot".into(),
                        ..Profile::default()
                    },
                ),
                (
                    "editor".into(),
                    Profile {
                        program: "nvim".into(),
                        args: vec![".".into()],
                        ..Profile::default()
                    },
                ),
                ("terminal".into(), Profile::default()),
            ]),
            terminals: BTreeMap::new(),
            projects: vec![],
            keys: BTreeMap::new(),
            theme: Theme::default(),
            state: State::default(),
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        let valid_text = |name: &str, value: &str| -> Result<()> {
            anyhow::ensure!(
                !value.contains('\0'),
                "{name} cannot contain a NUL character"
            );
            Ok(())
        };
        anyhow::ensure!(
            !self.worktrees.path_template.trim().is_empty(),
            "Worktree path template cannot be empty"
        );
        valid_text("Worktree path template", &self.worktrees.path_template)?;
        anyhow::ensure!(
            self.profiles.contains_key(&self.default_profile),
            "Unknown default_profile '{}'",
            self.default_profile
        );
        for project in &self.projects {
            if let Some(profile) = &project.default_profile {
                anyhow::ensure!(
                    self.profiles.contains_key(profile),
                    "Unknown profile '{profile}' for {}",
                    project.path.display()
                );
            }
            if let Some(template) = &project.path_template {
                anyhow::ensure!(
                    !template.trim().is_empty(),
                    "Project worktree path cannot be empty"
                );
                valid_text("Project worktree path", template)?;
            }
            if let Some(base) = &project.base {
                anyhow::ensure!(
                    !base.trim().is_empty(),
                    "Project starting point cannot be empty"
                );
                valid_text("Starting point", base)?;
            }
        }
        for (name, profile) in &self.profiles {
            anyhow::ensure!(!name.trim().is_empty(), "Profile name cannot be empty");
            anyhow::ensure!(
                !profile.cwd.trim().is_empty(),
                "Working directory for profile '{name}' cannot be empty"
            );
            anyhow::ensure!(
                profile.terminal == "auto" || self.terminals.contains_key(&profile.terminal),
                "Unknown terminal adapter '{}' for profile '{name}'",
                profile.terminal
            );
            for value in std::iter::once(&profile.program)
                .chain(&profile.args)
                .chain(std::iter::once(&profile.cwd))
            {
                valid_text("Profile field", value)?;
            }
            for (key, value) in &profile.env {
                anyhow::ensure!(
                    !key.is_empty() && !key.contains(['=', '\0']),
                    "Invalid environment variable name '{key}'"
                );
                valid_text("Environment value", value)?;
            }
        }
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Self> {
        let Some(raw) = read_optional(path)? else {
            return Ok(Self::default());
        };
        Self::parse(&raw, path)
    }
    pub(crate) fn parse(raw: &str, path: &Path) -> Result<Self> {
        let mut config: Self = toml::from_str(raw)
            .with_context(|| format!("Invalid configuration {}", path.display()))?;
        let directory = path.parent().unwrap_or(Path::new("."));
        for root in &mut config.project_roots {
            if root.is_relative() {
                *root = directory.join(&*root);
            }
        }
        for project in &mut config.projects {
            if project.path.is_relative() {
                project.path = directory.join(&project.path);
            }
        }
        for (name, profile) in Self::default().profiles {
            config.profiles.entry(name).or_insert(profile);
        }
        anyhow::ensure!(
            config.profiles.contains_key(&config.default_profile),
            "Unknown default_profile '{}'",
            config.default_profile
        );
        config.state.original_bases = config.state.bases.clone();
        Ok(config)
    }
    pub fn project(&self, path: &Path) -> Option<&ProjectConfig> {
        self.projects.iter().find(|p| same_path(&p.path, path))
    }
    pub fn worktree_path(&self, project: &Path, branch: &str) -> PathBuf {
        let template = self
            .project(project)
            .and_then(|p| p.path_template.as_ref())
            .unwrap_or(&self.worktrees.path_template);
        let parent = project.parent().unwrap_or(project).to_string_lossy();
        let name = project.file_name().unwrap_or_default().to_string_lossy();
        let path = PathBuf::from(
            template
                .replace("{project_parent}", &parent)
                .replace("{project_name}", &name)
                .replace("{branch_slug}", &branch.replace(['/', '\\'], "-")),
        );
        if path.is_absolute() {
            path
        } else {
            project.join(path)
        }
    }
    pub fn save_roots(path: &Path, roots: &[PathBuf]) -> Result<()> {
        edit_document(path, |doc| {
            let mut array = toml_edit::Array::new();
            for root in roots {
                array.push(root.to_string_lossy().as_ref());
            }
            let decor = doc
                .get("project_roots")
                .and_then(toml_edit::Item::as_value)
                .map(|v| v.decor().clone());
            let mut value = toml_edit::Value::Array(array);
            if let Some(decor) = decor {
                *value.decor_mut() = decor;
            }
            doc["project_roots"] = toml_edit::Item::Value(value);
            Ok(())
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub bases: BTreeMap<String, String>,
    pub last_project: Option<PathBuf>,
    pub last_worktree: Option<PathBuf>,
    #[serde(skip)]
    original_bases: BTreeMap<String, String>,
}
impl State {
    pub fn load(path: &Path) -> Result<Self> {
        Ok(Config::load(path)?.state)
    }
    pub fn save(&mut self, path: &Path) -> Result<()> {
        edit_document(path, |doc| {
            if doc.get("state").is_none() {
                doc["state"] = toml_edit::Item::Table(toml_edit::Table::new());
            }
            if doc["state"].get("bases").is_none() {
                doc["state"]["bases"] = toml_edit::Item::Table(toml_edit::Table::new());
            }
            for key in self.bases.keys().chain(self.original_bases.keys()) {
                if self.bases.get(key) != self.original_bases.get(key) {
                    if let Some(value) = self.bases.get(key) {
                        replace_value(
                            &mut doc["state"]["bases"][key],
                            toml_edit::Value::from(value.as_str()),
                        );
                    } else {
                        doc["state"]["bases"]
                            .as_table_like_mut()
                            .context("Invalid state.bases table")?
                            .remove(key);
                    }
                }
            }
            for (key, path) in [
                ("last_project", &self.last_project),
                ("last_worktree", &self.last_worktree),
            ] {
                if let Some(path) = path {
                    replace_value(
                        &mut doc["state"][key],
                        toml_edit::Value::from(path.to_string_lossy().as_ref()),
                    );
                } else {
                    doc["state"]
                        .as_table_like_mut()
                        .context("Invalid state table")?
                        .remove(key);
                }
            }
            Ok(())
        })?;
        self.original_bases = self.bases.clone();
        Ok(())
    }
}
/// The combined configuration location. macOS deliberately uses the Unix path.
pub fn default_path() -> Result<PathBuf> {
    #[cfg(windows)]
    let directory = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| BaseDirs::new().map(|dirs| dirs.data_local_dir().to_owned()))
        .context("Cannot determine local application data directory; use --config")?;
    #[cfg(not(windows))]
    let directory = BaseDirs::new()
        .context("Cannot determine home directory; use --config")?
        .home_dir()
        .join(".config");
    Ok(directory.join("treerails").join("config.toml"))
}
pub fn legacy_paths() -> Result<(PathBuf, PathBuf)> {
    let dirs = ProjectDirs::from("", "", "treerail")
        .context("Cannot determine legacy application directories; use --config")?;
    Ok((
        dirs.config_dir().join("config.toml"),
        dirs.state_dir()
            .unwrap_or(dirs.data_local_dir())
            .join("state.json"),
    ))
}

/// Open settings and memory together. Only interactive startup creates/imports files.
/// Legacy paths are supplied explicitly so custom --config files never import defaults.
pub fn open(
    path: &Path,
    create: bool,
    legacy: Option<(PathBuf, PathBuf)>,
) -> Result<(Config, State)> {
    if read_optional(path)?.is_some() {
        let config = Config::load(path)?;
        return Ok((config.clone(), config.state));
    }
    let mut config = Config::default();
    let mut source = None;
    if let Some((old_config, old_state)) = legacy {
        if let Some(raw) = read_optional(&old_config)? {
            config = Config::load(&old_config).context("Import legacy configuration")?;
            let mut doc: toml_edit::DocumentMut = raw.parse()?;
            // Relative legacy paths must keep their original meaning after moving.
            let mut roots = toml_edit::Array::new();
            for root in &config.project_roots {
                roots.push(root.to_string_lossy().as_ref());
            }
            if doc.get("project_roots").is_some() {
                replace_value(&mut doc["project_roots"], toml_edit::Value::Array(roots));
            }
            if let Some(projects) = doc
                .get_mut("projects")
                .and_then(toml_edit::Item::as_array_of_tables_mut)
            {
                for (table, project) in projects.iter_mut().zip(&config.projects) {
                    replace_value(
                        &mut table["path"],
                        toml_edit::Value::from(project.path.to_string_lossy().as_ref()),
                    );
                }
            } else if let Some(projects) = doc
                .get_mut("projects")
                .and_then(toml_edit::Item::as_array_mut)
            {
                for (value, project) in projects.iter_mut().zip(&config.projects) {
                    if let Some(table) = value.as_inline_table_mut() {
                        let mut path =
                            toml_edit::Value::from(project.path.to_string_lossy().as_ref());
                        if let Some(old) = table.get("path") {
                            *path.decor_mut() = old.decor().clone();
                        }
                        table.insert("path", path);
                    }
                }
            }
            source = Some(doc.to_string());
        }
        if let Some(raw) = read_optional(&old_state)? {
            config.state = serde_json::from_str(&raw).with_context(|| {
                format!("Import legacy remembered state {}", old_state.display())
            })?;
        }
    }
    if create {
        edit_document(path, |doc| {
            // Another instance may have completed startup while we prepared the import.
            if read_optional(path)?.is_some() {
                return Ok(());
            }
            *doc = source
                .as_deref()
                .map(str::parse)
                .transpose()?
                .unwrap_or(toml::to_string_pretty(&config)?.parse()?);
            let state_doc: toml_edit::DocumentMut =
                toml::to_string_pretty(&config.state)?.parse()?;
            doc["state"] = toml_edit::Item::Table(state_doc.as_table().clone());
            Ok(())
        })?;
        config = Config::load(path)?;
    }
    let state = config.state.clone();
    Ok((config, state))
}

fn read_optional(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(raw) => Ok(Some(raw)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("Read {}", path.display())),
    }
}

/// Serialize read/patch/write transactions across Tree Rail instances. The sidecar
/// holds only an OS lock, never settings; atomic replacement keeps readers safe.
pub fn edit_document<T>(
    path: &Path,
    edit: impl FnOnce(&mut toml_edit::DocumentMut) -> Result<T>,
) -> Result<T> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)
        .with_context(|| format!("Create configuration directory {}", parent.display()))?;
    let mut lock_name = path.as_os_str().to_os_string();
    lock_name.push(".lock");
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(PathBuf::from(lock_name))
        .with_context(|| format!("Open configuration lock for {}", path.display()))?;
    fs2::FileExt::lock_exclusive(&lock).with_context(|| format!("Lock {}", path.display()))?;
    let source = read_optional(path)?.unwrap_or_else(|| "# Tree Rail configuration\n".into());
    Config::parse(&source, path)
        .with_context(|| format!("Configuration {} was not changed", path.display()))?;
    let mut doc: toml_edit::DocumentMut = source.parse()?;
    let result = edit(&mut doc)?;
    let updated = doc.to_string();
    Config::parse(&updated, path).context("Refusing to save invalid configuration")?;
    if updated != source {
        atomic_write(path, updated.as_bytes())?;
    }
    Ok(result)
}

fn replace_value(item: &mut toml_edit::Item, mut value: toml_edit::Value) {
    if let Some(old) = item.as_value() {
        *value.decor_mut() = old.decor().clone();
    }
    *item = toml_edit::Item::Value(value);
}
pub fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(contents)?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .map_err(|e| e.error)
        .with_context(|| format!("Save {}", path.display()))?;
    Ok(())
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
