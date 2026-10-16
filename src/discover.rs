use crate::{
    config::same_path,
    git::{self, Project},
};
use anyhow::Result;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub async fn discover(roots: Vec<PathBuf>, recursive: bool) -> Result<(Vec<Project>, Vec<String>)> {
    let (candidates, mut warnings) = tokio::task::spawn_blocking(move || {
        let mut paths = Vec::new();
        let mut warnings = Vec::new();
        let mut visited = BTreeSet::new();
        for root in roots {
            scan(
                &root,
                recursive,
                true,
                &mut visited,
                &mut paths,
                &mut warnings,
            );
        }
        (paths, warnings)
    })
    .await?;
    let mut projects: Vec<Project> = Vec::new();
    for path in candidates {
        match git::project(&path).await {
            Ok(project) => {
                if !projects.iter().any(|p| same_path(&p.path, &project.path)) {
                    projects.push(project);
                }
            }
            Err(error) => warnings.push(format!("{}: {error:#}", path.display())),
        }
    }
    projects.sort_by_key(|p| p.name.to_lowercase());
    Ok((projects, warnings))
}
fn scan(
    path: &Path,
    recursive: bool,
    root: bool,
    visited: &mut BTreeSet<PathBuf>,
    result: &mut Vec<PathBuf>,
    warnings: &mut Vec<String>,
) {
    if path.components().any(|p| {
        p.as_os_str()
            .to_string_lossy()
            .to_ascii_lowercase()
            .contains(".worktrees")
    }) {
        return;
    }
    let canonical = match path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            warnings.push(format!("{}: {e}", path.display()));
            return;
        }
    };
    if !visited.insert(canonical) {
        return;
    }
    if path.join(".git").exists() {
        result.push(path.to_owned());
        return;
    }
    if !root && !recursive {
        return;
    }
    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(e) => {
            warnings.push(format!("{}: {e}", path.display()));
            return;
        }
    };
    for entry in entries {
        match entry {
            Ok(entry) => {
                let child = entry.path();
                if entry
                    .file_type()
                    .is_ok_and(|t| t.is_dir() && !t.is_symlink())
                    && (child.join(".git").exists() || recursive)
                {
                    scan(&child, recursive, false, visited, result, warnings);
                }
            }
            Err(error) => warnings.push(error.to_string()),
        }
    }
}
