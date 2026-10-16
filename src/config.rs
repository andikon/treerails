use anyhow::{Context, Result};
use directories::ProjectDirs;
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
        }
    }
}
impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path).with_context(|| format!("Read {}", path.display()))?;
        let mut config: Self = toml::from_str(&raw)
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
        let source = if path.exists() {
            fs::read_to_string(path)?
        } else {
            "# Tree Rail configuration\n".into()
        };
        let mut doc: toml_edit::DocumentMut = source.parse()?;
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
        atomic_write(path, doc.to_string().as_bytes())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub bases: BTreeMap<String, String>,
    pub last_project: Option<PathBuf>,
    pub last_worktree: Option<PathBuf>,
}
impl State {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        serde_json::from_slice(&fs::read(path)?).with_context(|| {
            format!(
                "Invalid state {}; move it aside to reset remembered selections",
                path.display()
            )
        })
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        atomic_write(path, &serde_json::to_vec_pretty(self)?)
    }
}
pub fn paths() -> Result<(PathBuf, PathBuf)> {
    let dirs = ProjectDirs::from("", "", "treerail")
        .context("Cannot determine application directories; use --config and --state")?;
    Ok((
        dirs.config_dir().join("config.toml"),
        dirs.state_dir()
            .unwrap_or(dirs.data_local_dir())
            .join("state.json"),
    ))
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
