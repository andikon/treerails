use std::{fs, path::PathBuf};
use tempfile::TempDir;
use treerail::{
    app::{App, Completion, Payload},
    config::{self, Config, State},
    git::{Project, Worktree},
};

#[test]
fn fresh_session_creates_one_document_and_reopens_its_selection() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("treerails/config.toml");
    let (config, state) = config::open(&path, true, None).unwrap();
    assert!(path.is_file());
    let mut app = App::new(config, state, path.clone(), vec![], false);
    let project = Project {
        path: temp.path().join("project"),
        name: "project".into(),
    };
    app.projects.push(project.clone());
    app.open_basic_project();
    let trees = vec![
        Worktree {
            path: project.path.clone(),
            branch: "main".into(),
            head: "123".into(),
            original: true,
            locked: false,
            prunable: false,
        },
        Worktree {
            path: temp.path().join("feature"),
            branch: "feature".into(),
            head: "123".into(),
            original: false,
            locked: false,
            prunable: false,
        },
    ];
    app.accept(Completion {
        epoch: app.epoch,
        token: "trees".into(),
        result: Ok(Payload::Trees(
            project.path.clone(),
            trees.clone(),
            "main".into(),
        )),
    });
    app.key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('j'),
        crossterm::event::KeyModifiers::NONE,
    ));
    assert_eq!(app.selected_tree().unwrap().branch, "feature");
    app.save().unwrap();
    let (config, state) = config::open(&path, true, None).unwrap();
    assert_eq!(state.last_project, Some(temp.path().join("project")));
    assert_eq!(config.default_profile, "copilot");
    let mut reopened = App::new(config, state, path, vec![], false);
    reopened.accept(Completion {
        epoch: reopened.epoch,
        token: "discover".into(),
        result: Ok(Payload::Projects(vec![project.clone()], vec![])),
    });
    reopened.open_basic_project();
    reopened.accept(Completion {
        epoch: reopened.epoch,
        token: "trees".into(),
        result: Ok(Payload::Trees(project.path, trees, "main".into())),
    });
    assert_eq!(reopened.selected_tree().unwrap().branch, "feature");
    assert!(!temp.path().join("state.json").exists());
}

#[test]
fn migration_imports_both_components_once_without_changing_originals() {
    let temp = TempDir::new().unwrap();
    let old_config = temp.path().join("legacy.toml");
    let old_state = temp.path().join("state.json");
    let new = temp.path().join("new/config.toml");
    let settings = "# Keep my notes\nproject_roots = ['sources']\ndefault_profile = 'editor'\n[[projects]]\npath = 'sources/project'\nbase = 'origin/develop'\n";
    let memory = r#"{"bases":{"project":"origin/develop"},"last_project":"chosen-project","last_worktree":"chosen-worktree"}"#;
    fs::write(&old_config, settings).unwrap();
    fs::write(&old_state, memory).unwrap();
    let legacy = Some((old_config.clone(), old_state.clone()));
    let (config, state) = config::open(&new, true, legacy.clone()).unwrap();
    assert_eq!(config.default_profile, "editor");
    assert_eq!(config.project_roots, [temp.path().join("sources")]);
    assert_eq!(config.projects[0].path, temp.path().join("sources/project"));
    assert_eq!(state.last_worktree, Some(PathBuf::from("chosen-worktree")));
    assert_eq!(state.bases["project"], "origin/develop");
    assert!(
        fs::read_to_string(&new)
            .unwrap()
            .contains("# Keep my notes")
    );
    assert_eq!(fs::read_to_string(&old_config).unwrap(), settings);
    assert_eq!(fs::read_to_string(&old_state).unwrap(), memory);
    fs::write(&new, "default_profile = 'terminal'\n").unwrap();
    fs::write(&old_state, "malformed").unwrap();
    let (config, state) = config::open(&new, true, legacy).unwrap();
    assert_eq!(config.default_profile, "terminal");
    assert!(state.last_worktree.is_none());
}

#[test]
fn independently_loaded_sessions_preserve_editor_changes_and_unrelated_memory() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("config.toml");
    fs::write(&path, "# User notes\ndefault_profile = 'copilot'\n[state]\nlast_project = 'old' # remembered project\n[state.bases]\nold = 'main'\n").unwrap();
    let mut first = State::load(&path).unwrap();
    let mut second = State::load(&path).unwrap();
    let edited = fs::read_to_string(&path)
        .unwrap()
        .replace("'copilot'", "'editor'");
    fs::write(&path, edited).unwrap();
    first.last_project = Some("first".into());
    first.bases.insert("project-a".into(), "develop".into());
    first.save(&path).unwrap();
    second.last_project = Some("second".into());
    second.bases.insert("project-b".into(), "release".into());
    second.save(&path).unwrap();
    let mut third = State::load(&path).unwrap();
    third
        .bases
        .insert("project-a".into(), "updated-by-third".into());
    third.save(&path).unwrap();
    first.last_project = Some("latest".into());
    first.save(&path).unwrap();
    let loaded = Config::load(&path).unwrap();
    assert_eq!(loaded.default_profile, "editor");
    assert_eq!(loaded.state.last_project, Some("latest".into()));
    assert_eq!(loaded.state.bases["project-a"], "updated-by-third");
    assert_eq!(loaded.state.bases["project-b"], "release");
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("# User notes"));
    assert!(text.contains("# remembered project"));
}

#[test]
fn import_failures_leave_legacy_and_destination_data_untouched() {
    let temp = TempDir::new().unwrap();
    let old = temp.path().join("old.toml");
    let memory = temp.path().join("old.json");
    let destination = temp.path().join("new/config.toml");
    for (settings, state, expected) in [
        ("not valid toml", "{}", "legacy configuration"),
        (
            "project_roots = []",
            "not valid json",
            "legacy remembered state",
        ),
    ] {
        fs::write(&old, settings).unwrap();
        fs::write(&memory, state).unwrap();
        let error =
            config::open(&destination, true, Some((old.clone(), memory.clone()))).unwrap_err();
        assert!(format!("{error:#}").contains(expected), "{error:#}");
        assert!(!destination.exists());
        assert_eq!(fs::read_to_string(&old).unwrap(), settings);
        assert_eq!(fs::read_to_string(&memory).unwrap(), state);
    }
    // An explicit destination is authoritative even when legacy data is malformed.
    fs::write(&destination, "project_roots = [").unwrap_err();
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    fs::write(&destination, "project_roots = [").unwrap();
    let before = fs::read(&destination).unwrap();
    assert!(config::open(&destination, true, Some((old, memory))).is_err());
    assert_eq!(fs::read(&destination).unwrap(), before);
}

#[test]
fn read_only_open_reads_legacy_without_creating_a_destination_or_lock() {
    let temp = TempDir::new().unwrap();
    let old = temp.path().join("old.toml");
    let memory = temp.path().join("old.json");
    let destination = temp.path().join("new/config.toml");
    fs::write(&old, "default_profile = 'editor'\n").unwrap();
    fs::write(&memory, r#"{"last_project":"last"}"#).unwrap();
    let (config, state) = config::open(&destination, false, Some((old, memory))).unwrap();
    assert_eq!(config.default_profile, "editor");
    assert_eq!(state.last_project, Some("last".into()));
    assert!(!destination.parent().unwrap().exists());
    let empty = temp.path().join("other/config.toml");
    config::open(&empty, false, None).unwrap();
    assert!(!empty.parent().unwrap().exists());
}

#[test]
fn state_only_and_configuration_only_legacy_setups_can_upgrade() {
    let temp = TempDir::new().unwrap();
    let old = temp.path().join("missing.toml");
    let memory = temp.path().join("old.json");
    fs::write(&memory, r#"{"last_worktree":"saved-worktree"}"#).unwrap();
    let (_, state) = config::open(
        &temp.path().join("state-only.toml"),
        true,
        Some((old.clone(), memory.clone())),
    )
    .unwrap();
    assert_eq!(state.last_worktree, Some("saved-worktree".into()));
    fs::remove_file(&memory).unwrap();
    fs::write(&old, "recursive = true\n").unwrap();
    let (config, state) = config::open(
        &temp.path().join("config-only.toml"),
        true,
        Some((old, memory)),
    )
    .unwrap();
    assert!(config.recursive);
    assert!(state.last_worktree.is_none());
}

#[test]
fn invalid_intervening_settings_are_reported_without_overwriting_them() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("config.toml");
    let (_, mut state) = config::open(&path, true, None).unwrap();
    let invalid = "# Editing this now\ndefault_profile = 'missing-profile'\n";
    fs::write(&path, invalid).unwrap();
    state.last_project = Some("project".into());
    let error = state.save(&path).unwrap_err();
    assert!(format!("{error:#}").contains("Unknown default_profile"));
    assert_eq!(fs::read_to_string(&path).unwrap(), invalid);
}

#[test]
fn default_location_follows_the_agreed_native_platform_convention() {
    #[cfg(windows)]
    let expected =
        PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap()).join("treerails/config.toml");
    #[cfg(not(windows))]
    let expected = directories::BaseDirs::new()
        .unwrap()
        .home_dir()
        .join(".config/treerails/config.toml");
    assert_eq!(config::default_path().unwrap(), expected);
}

#[test]
fn cli_listing_does_not_create_an_explicit_combined_document() {
    let temp = TempDir::new().unwrap();
    let destination = temp.path().join("nested/config.toml");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_treerail"))
        .args(["--list", "--config"])
        .arg(&destination)
        .arg(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!destination.parent().unwrap().exists());
}

#[test]
fn legacy_inline_project_tables_keep_their_original_directory() {
    let temp = TempDir::new().unwrap();
    let old = temp.path().join("legacy.toml");
    fs::write(
        &old,
        "projects = [{path = 'app', base = 'origin/develop'}] # my project\n",
    )
    .unwrap();
    let destination = temp.path().join("new/config.toml");
    let (config, _) = config::open(
        &destination,
        true,
        Some((old, temp.path().join("missing.json"))),
    )
    .unwrap();
    assert_eq!(config.projects[0].path, temp.path().join("app"));
    assert!(
        fs::read_to_string(destination)
            .unwrap()
            .contains("# my project")
    );
}

#[test]
fn simultaneous_sessions_preserve_each_projects_saved_starting_point() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("config.toml");
    config::open(&path, true, None).unwrap();
    let barrier = std::sync::Barrier::new(6);
    std::thread::scope(|scope| {
        for index in 0..6 {
            let path = &path;
            let barrier = &barrier;
            scope.spawn(move || {
                let mut state = State::load(path).unwrap();
                state
                    .bases
                    .insert(format!("project-{index}"), "origin/develop".into());
                barrier.wait();
                state.save(path).unwrap();
            });
        }
    });
    let state = State::load(&path).unwrap();
    assert_eq!(state.bases.len(), 6);
    assert!(state.bases.values().all(|base| base == "origin/develop"));
}
