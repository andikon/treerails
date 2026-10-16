use std::{
    fs,
    path::{Path, PathBuf},
};
use tempfile::TempDir;
use treerail::{
    config::{Config, Profile, State, TerminalConfig},
    discover, git, launch,
};

async fn repo(parent: &Path, name: &str) -> PathBuf {
    let path = parent.join(name);
    fs::create_dir_all(&path).unwrap();
    git::run(&path, ["init", "--initial-branch=main"])
        .await
        .unwrap();
    for (key, value) in [
        ("user.name", "Tree Rail Test"),
        ("user.email", "test@example.invalid"),
        ("commit.gpgsign", "false"),
        ("core.autocrlf", "false"),
    ] {
        git::run(&path, ["config", key, value]).await.unwrap();
    }
    let hooks = path.join(".git").join("empty-hooks");
    fs::create_dir_all(&hooks).unwrap();
    git::run(&path, ["config", "core.hooksPath", hooks.to_str().unwrap()])
        .await
        .unwrap();
    fs::write(path.join("readme.txt"), "initial\n").unwrap();
    git::run(&path, ["add", "--", "readme.txt"]).await.unwrap();
    git::commit(&path, "Initial commit", "").await.unwrap();
    path
}
#[tokio::test]
async fn discover_deduplicates_and_excludes_worktree_folders() {
    let temp = TempDir::new().unwrap();
    let original = repo(temp.path(), "project with spaces").await;
    let tree = temp
        .path()
        .join("project with spaces.worktrees")
        .join("feature");
    git::create_worktree(&original, "feature", "main", &tree, false, false)
        .await
        .unwrap();
    let nested = repo(&temp.path().join("nested"), "another").await;
    let (projects, warnings) =
        discover::discover(vec![temp.path().into(), original.clone()], false)
            .await
            .unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].name, "project with spaces");
    let (projects, _) = discover::discover(vec![temp.path().into()], true)
        .await
        .unwrap();
    assert_eq!(projects.len(), 2);
    assert!(
        projects
            .iter()
            .any(|p| treerail::config::same_path(&p.path, &nested))
    );
    let linked = git::project(&tree).await.unwrap();
    assert!(treerail::config::same_path(&linked.path, &original));
    let trees = git::worktrees(&original).await.unwrap();
    assert!(trees[0].original);
    assert!(!trees[1].original);
}
#[tokio::test]
async fn non_head_base_status_commit_and_safe_cleanup() {
    let temp = TempDir::new().unwrap();
    let path = repo(temp.path(), "app").await;
    git::run(&path, ["checkout", "-b", "develop"])
        .await
        .unwrap();
    fs::write(path.join("develop.txt"), "from develop").unwrap();
    git::run(&path, ["add", "--", "develop.txt"]).await.unwrap();
    git::commit(&path, "Develop", "").await.unwrap();
    git::run(&path, ["checkout", "main"]).await.unwrap();
    let bare = temp.path().join("remote.git");
    fs::create_dir(&bare).unwrap();
    git::run(&bare, ["init", "--bare"]).await.unwrap();
    git::run(&path, ["remote", "add", "origin", bare.to_str().unwrap()])
        .await
        .unwrap();
    git::run(&path, ["push", "origin", "develop"])
        .await
        .unwrap();
    let tree = temp.path().join("app.worktrees").join("feature-test-123");
    git::create_worktree(
        &path,
        "feature/test-123",
        "origin/develop",
        &tree,
        false,
        false,
    )
    .await
    .unwrap();
    assert!(tree.join("develop.txt").exists());
    assert!(git::status(&tree).await.unwrap().upstream.is_none());
    // Choosing this branch again selects its existing worktree, regardless of a new suggested path.
    let reused = git::create_worktree(
        &path,
        "feature/test-123",
        "main",
        &temp.path().join("unused"),
        false,
        false,
    )
    .await
    .unwrap();
    assert_eq!(reused, tree);
    fs::write(tree.join("a file with spaces.txt"), "hello\n").unwrap();
    let snapshot = git::status(&tree).await.unwrap();
    assert_eq!(snapshot.files.len(), 1);
    assert!(snapshot.files[0].untracked);
    assert!(
        git::remove_worktree(&path, &tree)
            .await
            .unwrap_err()
            .to_string()
            .contains("uncommitted")
    );
    git::stage(&tree, &snapshot.files, false).await.unwrap();
    let snapshot = git::status(&tree).await.unwrap();
    assert!(snapshot.files[0].staged());
    assert!(
        git::diff(&tree, &snapshot.files[0], true)
            .await
            .unwrap()
            .contains("+hello")
    );
    git::stage(&tree, &snapshot.files, true).await.unwrap();
    assert!(git::status(&tree).await.unwrap().files[0].untracked);
    git::stage(&tree, &snapshot.files, false).await.unwrap();
    git::commit(&tree, "Add file", "A multiline\ncommit body.")
        .await
        .unwrap();
    assert!(git::status(&tree).await.unwrap().files.is_empty());
    assert!(
        git::text(&tree, &["log", "-1", "--format=%B"])
            .await
            .unwrap()
            .contains("A multiline\ncommit body.")
    );
    git::publish(&tree, "origin", "feature/test-123")
        .await
        .unwrap();
    git::run(&tree, ["config", "push.default", "nothing"])
        .await
        .unwrap();
    git::push(&tree).await.unwrap();
    assert_eq!(
        git::status(&tree).await.unwrap().upstream.as_deref(),
        Some("origin/feature/test-123")
    );
    assert!(
        git::text(&bare, &["rev-parse", "refs/heads/feature/test-123"])
            .await
            .is_ok()
    );
    git::remove_worktree(&path, &tree).await.unwrap();
    assert!(!tree.exists());
    assert!(
        git::text(&path, &["rev-parse", "refs/heads/feature/test-123"])
            .await
            .is_ok()
    );
    assert!(
        git::remove_worktree(&path, &path)
            .await
            .unwrap_err()
            .to_string()
            .contains("original")
    );
}
#[tokio::test]
async fn status_handles_renames_and_unborn_unstage() {
    let temp = TempDir::new().unwrap();
    let path = repo(temp.path(), "rename").await;
    git::run(&path, ["mv", "--", "readme.txt", "renamed file.txt"])
        .await
        .unwrap();
    let snapshot = git::status(&path).await.unwrap();
    assert_eq!(snapshot.files[0].index, 'R');
    assert_eq!(
        snapshot.files[0].original.as_deref(),
        Some(Path::new("readme.txt"))
    );
    git::stage(&path, &snapshot.files, true).await.unwrap();
    assert!(path.join("renamed file.txt").exists());
    let newborn = temp.path().join("unborn");
    fs::create_dir(&newborn).unwrap();
    git::run(&newborn, ["init"]).await.unwrap();
    fs::write(newborn.join("first.txt"), "first").unwrap();
    git::run(&newborn, ["add", "--", "first.txt"])
        .await
        .unwrap();
    let snapshot = git::status(&newborn).await.unwrap();
    git::stage(&newborn, &snapshot.files, true).await.unwrap();
    assert!(newborn.join("first.txt").exists());
    assert!(git::status(&newborn).await.unwrap().files[0].untracked);
}
#[tokio::test]
async fn fast_forward_pull_refuses_divergence() {
    let temp = TempDir::new().unwrap();
    let path = repo(temp.path(), "local").await;
    let bare = temp.path().join("remote.git");
    fs::create_dir(&bare).unwrap();
    git::run(&bare, ["init", "--bare"]).await.unwrap();
    git::run(&path, ["remote", "add", "origin", bare.to_str().unwrap()])
        .await
        .unwrap();
    git::publish(&path, "origin", "main").await.unwrap();
    let second = temp.path().join("second");
    git::run(
        temp.path(),
        [
            "clone",
            "--branch",
            "main",
            bare.to_str().unwrap(),
            second.to_str().unwrap(),
        ],
    )
    .await
    .unwrap();
    for (key, value) in [
        ("user.name", "Other"),
        ("user.email", "other@example.invalid"),
        ("commit.gpgsign", "false"),
    ] {
        git::run(&second, ["config", key, value]).await.unwrap();
    }
    fs::write(second.join("remote.txt"), "remote").unwrap();
    git::run(&second, ["add", "."]).await.unwrap();
    git::commit(&second, "Remote work", "").await.unwrap();
    git::run(&second, ["push"]).await.unwrap();
    fs::write(path.join("local.txt"), "local").unwrap();
    git::run(&path, ["add", "."]).await.unwrap();
    git::commit(&path, "Local work", "").await.unwrap();
    assert!(git::run(&path, ["pull", "--ff-only"]).await.is_err());
    assert_eq!(
        git::text(&path, &["log", "-1", "--format=%s"])
            .await
            .unwrap(),
        "Local work"
    );
}
#[test]
fn configuration_preserves_comments_and_uses_separate_state() {
    let temp = TempDir::new().unwrap();
    let config = temp.path().join("config.toml");
    fs::write(&config,"# My settings\nproject_roots = [] # sources\n\n[profiles.editor]\nprogram = 'nvim'\nargs = ['.']\n").unwrap();
    Config::save_roots(&config, &[PathBuf::from(r"C:\Code\my project")]).unwrap();
    let raw = fs::read_to_string(&config).unwrap();
    assert!(raw.contains("# My settings"));
    assert!(raw.contains("# sources"));
    let config = Config::load(&config).unwrap();
    assert_eq!(
        config.project_roots[0],
        PathBuf::from(r"C:\Code\my project")
    );
    assert!(config.profiles.contains_key("copilot"));
    let state_path = temp.path().join("state.json");
    let mut state = State::default();
    state
        .bases
        .insert("project".into(), "origin/develop".into());
    state.save(&state_path).unwrap();
    assert_eq!(
        State::load(&state_path).unwrap().bases["project"],
        "origin/develop"
    );
    assert!(
        config
            .worktree_path(Path::new("/code/app"), "feature/login")
            .ends_with("app.worktrees/feature-login")
    );
}
#[test]
fn terminal_arguments_keep_paths_and_arguments_separate() {
    let temp = TempDir::new().unwrap();
    let cwd = temp.path().join("work with spaces");
    fs::create_dir(&cwd).unwrap();
    let mut config = Config::default();
    config.terminals.insert(
        "custom".into(),
        TerminalConfig {
            program: "my-terminal".into(),
            args: vec![
                "--cwd".into(),
                "{worktree}".into(),
                "--".into(),
                "{command}".into(),
            ],
        },
    );
    let profile = Profile {
        program: "nvim".into(),
        args: vec!["a file.txt".into()],
        terminal: "custom".into(),
        ..Profile::default()
    };
    let plan = launch::plan(&config, &profile, &cwd, "feature/login").unwrap();
    assert_eq!(plan.args[1], cwd.as_os_str());
    assert_eq!(plan.args[3], "nvim");
    assert_eq!(plan.args[4], "a file.txt");
    assert_eq!(plan.cwd, cwd);
}
#[test]
fn cli_help_and_noninteractive_discovery() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_treerail"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("--paths-file"));
    let temp = TempDir::new().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_treerail"))
        .args(["--list", "--config"])
        .arg(temp.path().join("none.toml"))
        .arg(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "[]");
}

#[tokio::test]
async fn pr_comparison_uses_merge_base_and_excludes_uncommitted_changes() {
    let temp = TempDir::new().unwrap();
    let path = repo(temp.path(), "review").await;
    git::run(&path, ["checkout", "-b", "review/pr-123"])
        .await
        .unwrap();
    fs::write(path.join("feature.txt"), "PR feature\n").unwrap();
    git::run(&path, ["add", "--", "feature.txt"]).await.unwrap();
    git::commit(&path, "PR change", "").await.unwrap();
    git::run(&path, ["checkout", "main"]).await.unwrap();
    fs::write(path.join("base-only.txt"), "base advanced\n").unwrap();
    git::run(&path, ["add", "--", "base-only.txt"])
        .await
        .unwrap();
    git::commit(&path, "Base change", "").await.unwrap();
    git::run(&path, ["update-ref", "refs/treerail/pr/123/base", "main"])
        .await
        .unwrap();
    git::run(&path, ["checkout", "review/pr-123"])
        .await
        .unwrap();
    fs::write(path.join("readme.txt"), "uncommitted edit\n").unwrap();
    let comparison = git::review_diff(&path, "review/pr-123").await.unwrap();
    assert!(comparison.contains("+PR feature"));
    assert!(!comparison.contains("base-only"));
    assert!(!comparison.contains("uncommitted edit"));
    assert_eq!(git::status(&path).await.unwrap().files.len(), 1);
}
