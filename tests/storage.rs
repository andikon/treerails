use clap::Parser;
use std::{fs, path::PathBuf};
use tempfile::TempDir;
use treerail::{
    app::App,
    config::{self, Config, State},
    startup::Options,
};

#[test]
fn provider_scope_includes_sibling_worktrees_without_rewriting_config() {
    let temp = TempDir::new().unwrap();
    let project = temp.path().join("project");
    fs::create_dir_all(project.join(".git")).unwrap();
    let config = Config::default();
    let prepared = config.agent_for_sources(std::slice::from_ref(&project), &[]);
    assert!(config::same_path(
        prepared.host_directory.as_ref().unwrap(),
        temp.path()
    ));
    assert!(config.agent.host_directory.is_none());
    let folder_scope = config.agent_for_sources(&[temp.path().to_path_buf()], &[]);
    assert!(config::same_path(
        folder_scope.host_directory.as_ref().unwrap(),
        temp.path()
    ));
    let mut explicit = config;
    explicit.agent.host_directory = Some(project.clone());
    assert_eq!(
        explicit
            .agent_for_sources(&[temp.path().to_path_buf()], &[])
            .host_directory,
        Some(project)
    );
}

#[test]
fn runtime_memory_never_rewrites_configuration() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("config.toml");
    let source = "# my settings\nrecursive = true\nproject_roots = ['declared']\n";
    fs::write(&path, source).unwrap();
    let (_, mut state) = config::open(&path, true).unwrap();
    state
        .remembered_sources
        .push(temp.path().join("remembered"));
    state.save(&config::state_path(&path)).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    let (_, loaded) = config::open(&path, false).unwrap();
    assert_eq!(loaded.remembered_sources, state.remembered_sources);
    assert!(
        !toml::to_string(&Config::default())
            .unwrap()
            .contains("[state]")
    );
}
#[test]
fn combined_document_migration_preserves_complete_backup_and_comments() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("config.toml");
    let source = "# keep this comment\nrecursive = true\n[state]\nlast_project = 'C:/Code/project'\n[state.workflow]\nold_ticket_records = ['retain in backup']\n";
    fs::write(&path, source).unwrap();
    let (config, state) = config::open(&path, true).unwrap();
    assert!(config.recursive);
    assert_eq!(state.last_project, Some(PathBuf::from("C:/Code/project")));
    assert_eq!(
        fs::read_to_string(path.with_extension("pre-worktree-refactor.toml")).unwrap(),
        source
    );
    let raw = fs::read_to_string(&path).unwrap();
    assert!(raw.contains("# keep this comment"));
    assert!(!raw.contains("[state"));
    assert!(config::state_path(&path).exists());
    config::open(&path, true).unwrap();
    assert_eq!(
        fs::read_to_string(path.with_extension("pre-worktree-refactor.toml")).unwrap(),
        source
    );
}
#[test]
fn listing_does_not_create_or_migrate_files() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("config.toml");
    let args = vec![
        "treerail".into(),
        "--config".into(),
        path.to_string_lossy().to_string(),
        "--list".into(),
        temp.path().to_string_lossy().to_string(),
    ];
    Options::parse_from(args).prepare().unwrap();
    assert!(!path.exists());
    assert!(!config::state_path(&path).exists());
    let old = "[state]\nlast_project = 'remember me'\n";
    fs::write(&path, old).unwrap();
    config::open(&path, false).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), old);
    assert!(!path.with_extension("pre-worktree-refactor.toml").exists());
}
#[test]
fn explicit_sources_go_to_state_before_app_creation() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("config.toml");
    fs::write(&path, "# untouched\nrecursive = false\n").unwrap();
    let options = Options::parse_from([
        "treerail",
        "--config",
        path.to_str().unwrap(),
        temp.path().to_str().unwrap(),
    ]);
    let session = options.prepare().unwrap();
    assert_eq!(
        session.state.remembered_sources,
        vec![temp.path().to_path_buf()]
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "# untouched\nrecursive = false\n"
    );
    assert!(config::state_path(&path).exists());
    let app = App::new(session);
    assert!(app.agent_commands.is_empty());
}
#[test]
fn malformed_state_is_reported_without_overwriting_either_file() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("config.toml");
    fs::write(&path, "recursive = false\n").unwrap();
    let state = config::state_path(&path);
    fs::write(&state, "broken json").unwrap();
    assert!(config::open(&path, true).is_err());
    assert_eq!(fs::read_to_string(state).unwrap(), "broken json");
    assert_eq!(fs::read_to_string(path).unwrap(), "recursive = false\n");
}
#[test]
fn separate_instances_preserve_unmodified_state_fields() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("config.toml");
    let (_, mut first) = config::open(&path, true).unwrap();
    let (_, mut second) = config::open(&path, false).unwrap();
    first.remembered_sources.push(PathBuf::from("first"));
    first.save(&config::state_path(&path)).unwrap();
    second.last_project = Some(PathBuf::from("second"));
    second.save(&config::state_path(&path)).unwrap();
    let (_, state) = config::open(&path, false).unwrap();
    assert_eq!(state.remembered_sources, vec![PathBuf::from("first")]);
    assert_eq!(state.last_project, Some(PathBuf::from("second")));
}
#[test]
fn state_file_save_reports_corruption() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("state.json");
    fs::write(&path, "bad").unwrap();
    assert!(State::default().save(&path).is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), "bad");
}

#[test]
fn simultaneous_remembered_locations_are_merged() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("config.toml");
    let (_, mut a) = config::open(&path, true).unwrap();
    let (_, mut b) = config::open(&path, false).unwrap();
    a.remembered_sources.push("one".into());
    b.remembered_sources.push("two".into());
    a.save(&config::state_path(&path)).unwrap();
    b.save(&config::state_path(&path)).unwrap();
    let (_, state) = config::open(&path, false).unwrap();
    assert!(state.remembered_sources.contains(&PathBuf::from("one")));
    assert!(state.remembered_sources.contains(&PathBuf::from("two")));
}

#[test]
fn preferences_preserve_unrelated_edits_and_report_conflicts() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("config.toml");
    fs::write(&path, "# my settings\nrecursive = false\n").unwrap();
    let (base, _) = config::open(&path, false).unwrap();
    let mut desired = base.clone();
    desired.recursive = true;
    fs::write(
        &path,
        "# my settings\nrecursive = false\npublication_remote = 'origin'\n",
    )
    .unwrap();
    config::save_preferences(&path, &base, &desired).unwrap();
    let raw = fs::read_to_string(&path).unwrap();
    assert!(raw.contains("# my settings"));
    assert!(raw.contains("publication_remote = 'origin'"));
    let mut desired = base.clone();
    desired.publication_remote = Some("different".into());
    assert!(config::save_preferences(&path, &base, &desired).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), raw);
}

#[test]
fn conversation_archiving_and_drafts_merge_independently() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("config.toml");
    let (_, mut initial) = config::open(&path, true).unwrap();
    initial.conversations.push(serde_json::from_value(serde_json::json!({"id":"one","project":"project","worktree":"tree","title":"Conversation","session_uri":"ahp-session:/one"})).unwrap());
    initial.save(&config::state_path(&path)).unwrap();
    let (_, mut a) = config::open(&path, false).unwrap();
    let (_, mut b) = config::open(&path, false).unwrap();
    a.conversations[0].archived = true;
    b.conversations[0].draft = "Keep this draft".into();
    a.save(&config::state_path(&path)).unwrap();
    b.save(&config::state_path(&path)).unwrap();
    let (_, state) = config::open(&path, false).unwrap();
    assert!(state.conversations[0].archived);
    assert_eq!(state.conversations[0].draft, "Keep this draft");
}
