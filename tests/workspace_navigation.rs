use treerail::config::State;

#[test]
fn remembered_project_views_merge_without_overwriting_other_clients_or_settings() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.toml");
    std::fs::write(
        &path,
        "# Keep my settings\nrecursive = true # discovery preference\n",
    )
    .unwrap();
    let mut first = State::load(&path).unwrap();
    let mut second = State::load(&path).unwrap();
    first
        .workspace_views
        .insert("project-a".into(), "work".into());
    first.save(&path).unwrap();
    second
        .workspace_views
        .insert("project-b".into(), "worktrees".into());
    second.save(&path).unwrap();
    let reopened = State::load(&path).unwrap();
    assert_eq!(reopened.workspace_views["project-a"], "work");
    assert_eq!(reopened.workspace_views["project-b"], "worktrees");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("# Keep my settings"));
    assert!(text.contains("# discovery preference"));
    let mut other_context = State::load(&directory.path().join("other.toml")).unwrap();
    assert!(other_context.workspace_views.is_empty());
    other_context
        .workspace_views
        .insert("project-a".into(), "tickets".into());
    other_context
        .save(&directory.path().join("other.toml"))
        .unwrap();
    assert_eq!(
        State::load(&path).unwrap().workspace_views["project-a"],
        "work"
    );
}

#[test]
fn unchanged_client_state_does_not_restore_an_old_view() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.toml");
    let mut first = State::load(&path).unwrap();
    first
        .workspace_views
        .insert("project-a".into(), "tickets".into());
    first.save(&path).unwrap();
    let mut stale = State::load(&path).unwrap();
    first
        .workspace_views
        .insert("project-a".into(), "worktrees".into());
    first.save(&path).unwrap();
    stale.last_project = Some(directory.path().into());
    stale.save(&path).unwrap();
    assert_eq!(
        State::load(&path).unwrap().workspace_views["project-a"],
        "worktrees"
    );
}
