use super::{lookup, patch_table};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub(super) fn deleted_entity(base: &toml::Value, current: &toml::Value, keys: &[String]) -> bool {
    keys.len() > 2
        && matches!(keys[0].as_str(), "projects" | "profiles")
        && lookup(base, &keys[..2]).is_some()
        && lookup(current, &keys[..2]).is_none()
}

pub(super) fn identity(path: &Path) -> String {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_owned());
    let text = path.to_string_lossy().into_owned();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}
fn matches(raw: Option<&str>, identity_key: &str, config_path: &Path) -> bool {
    let Some(raw) = raw else {
        return false;
    };
    let path = PathBuf::from(raw);
    let path = if path.is_absolute() {
        path
    } else {
        config_path.parent().unwrap_or(Path::new(".")).join(path)
    };
    identity(&path) == identity_key
}
pub(super) fn patch_project(
    doc: &mut toml_edit::DocumentMut,
    keys: &[String],
    desired: &toml::Value,
    config_path: &Path,
) -> Result<()> {
    let identity_key = keys.get(1).context("Missing project identity")?;
    let project = desired
        .get("projects")
        .and_then(|projects| projects.get(identity_key))
        .context("Missing desired project")?;
    let value = lookup(desired, keys);
    if doc.get("projects").is_none() {
        doc["projects"] = toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
    }
    match &mut doc["projects"] {
        toml_edit::Item::ArrayOfTables(projects) => {
            let index = projects.iter().position(|table| {
                matches(
                    table.get("path").and_then(toml_edit::Item::as_str),
                    identity_key,
                    config_path,
                )
            });
            let index = match index {
                Some(index) => index,
                None => {
                    let table: toml_edit::DocumentMut = toml::to_string(project)?.parse()?;
                    projects.push(table.as_table().clone());
                    projects.len() - 1
                }
            };
            let table = projects.get_mut(index).context("Missing project table")?;
            patch_table(table, &keys[2..], value)?;
        }
        toml_edit::Item::Value(toml_edit::Value::Array(projects)) => {
            let index = projects.iter().position(|value| {
                value.as_inline_table().is_some_and(|table| {
                    matches(
                        table.get("path").and_then(toml_edit::Value::as_str),
                        identity_key,
                        config_path,
                    )
                })
            });
            let index = match index {
                Some(index) => index,
                None => {
                    let mut document: toml_edit::DocumentMut =
                        format!("value = {project}").parse()?;
                    let value = document
                        .remove("value")
                        .context("Missing project value")?
                        .into_value()
                        .map_err(|_| anyhow::anyhow!("Invalid project value"))?;
                    projects.push(value);
                    projects.len() - 1
                }
            };
            let table = projects
                .get_mut(index)
                .and_then(toml_edit::Value::as_inline_table_mut)
                .context("Missing inline project table")?;
            patch_table(table, &keys[2..], value)?;
        }
        _ => anyhow::bail!("Invalid projects array"),
    }
    Ok(())
}
