use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use tempfile::TempDir;
use tokio::sync::mpsc;
use treerail::{
    app::{App, Completion, Operation},
    config::{Config, State},
    git, ui,
};

async fn repo(parent: &Path) -> PathBuf {
    let path = parent.join("project");
    fs::create_dir_all(&path).unwrap();
    git::run(&path, ["init", "--initial-branch=main"])
        .await
        .unwrap();
    for (key, value) in [
        ("user.name", "Feedback Test"),
        ("user.email", "test@example.invalid"),
        ("commit.gpgsign", "false"),
        ("core.autocrlf", "false"),
    ] {
        git::run(&path, ["config", key, value]).await.unwrap();
    }
    let hooks = path.join(".git/hooks");
    git::run(&path, ["config", "core.hooksPath", hooks.to_str().unwrap()])
        .await
        .unwrap();
    fs::write(path.join("file.txt"), "hello").unwrap();
    git::run(&path, ["add", "file.txt"]).await.unwrap();
    path
}
fn rendered(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}
async fn explorer(temp: &TempDir, path: &Path) -> App {
    let mut app = App::new(
        Config::default(),
        State::default(),
        temp.path().join("config.toml"),
        temp.path().join("state.json"),
        vec![path.into()],
        false,
    );
    app.projects.push(git::project(path).await.unwrap());
    app.open_project();
    app.trees = git::worktrees(path).await.unwrap();
    app.snapshots
        .insert(path.into(), git::status(path).await.unwrap());
    app
}
async fn finish(app: &mut App) {
    let job = app.jobs.pop().expect("Operation scheduled");
    let token = job.token();
    let (tx, mut rx) = mpsc::channel(128);
    let task = tokio::spawn(job.execute_with_feedback(tx));
    while let Some(event) = rx.recv().await {
        app.accept_feedback(event);
    }
    app.accept(Completion {
        epoch: app.epoch,
        token,
        result: task.await.unwrap(),
    });
}
#[tokio::test]
async fn commit_hook_output_is_visible_before_exit_and_commit_stdin_is_preserved() {
    let temp = TempDir::new().unwrap();
    let path = repo(temp.path()).await;
    let hook = path.join(".git/hooks/pre-commit");
    fs::write(&hook, "#!/bin/sh\nprintf 'stdout fragment'\nprintf 'checking 12%%\\rchecking 43%%' >&2\nwhile [ ! -f .git/release-hook ]; do sleep 0.02; done\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut app = App::new(
        Config::default(),
        State::default(),
        temp.path().join("config.toml"),
        temp.path().join("state.json"),
        vec![path.clone()],
        false,
    );
    app.projects.push(git::project(&path).await.unwrap());
    app.open_project();
    app.trees = git::worktrees(&path).await.unwrap();
    app.action("commit");
    app.paste("Keep subject");
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.paste("body from stdin");
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    app.action("operation_details");
    let job = app.jobs.pop().unwrap();
    let token = job.token();
    let (tx, mut rx) = mpsc::channel(128);
    let task = tokio::spawn(job.execute_with_feedback(tx));
    let observed = tokio::time::timeout(Duration::from_secs(15), async {
        while let Some(event) = rx.recv().await {
            app.accept_feedback(event);
            let text = rendered(&app);
            if text.contains("stdout fragment") && text.contains("checking 43%") {
                return text;
            }
        }
        panic!("Git exited before streaming the hook output");
    })
    .await;
    // Release the real hook even when an assertion fails, avoiding a stranded process.
    fs::write(path.join(".git/release-hook"), "release").unwrap();
    let result = tokio::time::timeout(Duration::from_secs(15), task)
        .await
        .unwrap()
        .unwrap();
    let text = observed.expect("Hook output must be inspectable before the hook is released");
    assert!(text.contains("Commit"));
    while let Ok(event) = rx.try_recv() {
        app.accept_feedback(event);
    }
    app.accept(Completion {
        epoch: app.epoch,
        token,
        result,
    });
    let text = rendered(&app);
    assert!(text.contains("Succeeded"), "{text}");
    assert_eq!(
        git::text(&path, &["log", "-1", "--format=%B"])
            .await
            .unwrap(),
        "Keep subject\n\nbody from stdin"
    );
}

#[tokio::test]
async fn publish_up_to_date_and_composite_worktree_failures_have_attributed_results() {
    let temp = TempDir::new().unwrap();
    let path = repo(temp.path()).await;
    git::commit(&path, "Initial", "").await.unwrap();
    let bare = temp.path().join("remote.git");
    fs::create_dir(&bare).unwrap();
    git::run(&bare, ["init", "--bare"]).await.unwrap();
    git::run(&path, ["remote", "add", "origin", bare.to_str().unwrap()])
        .await
        .unwrap();
    let mut app = explorer(&temp, &path).await;
    app.operation(Operation::Publish(
        "origin".into(),
        "feature/test-123".into(),
    ));
    finish(&mut app).await;
    app.action("operation_details");
    let text = rendered(&app);
    assert!(text.contains("Publish → origin/feature/test-123"));
    assert!(text.contains("Succeeded"));
    assert_eq!(
        git::status(&path).await.unwrap().upstream.as_deref(),
        Some("origin/feature/test-123")
    );
    app.operation(Operation::Push);
    finish(&mut app).await;
    let text = rendered(&app);
    assert!(text.contains("Succeeded: Everything up-to-date"), "{text}");
    let tree = temp.path().join("project.worktrees/feature");
    app.operation(Operation::Create {
        branch: "feature".into(),
        base: "origin/feature/test-123".into(),
        destination: tree.clone(),
        existing: false,
        fetch: true,
    });
    finish(&mut app).await;
    let output = &app.latest_operation.as_ref().unwrap().output;
    assert!(output.contains("Fetching remotes"));
    assert!(output.contains("Creating worktree"));
    assert!(tree.join("file.txt").exists());
    git::run(
        &path,
        [
            "remote",
            "set-url",
            "origin",
            temp.path().join("missing.git").to_str().unwrap(),
        ],
    )
    .await
    .unwrap();
    let blocked = temp.path().join("project.worktrees/blocked");
    app.operation(Operation::Create {
        branch: "blocked".into(),
        base: "main".into(),
        destination: blocked.clone(),
        existing: false,
        fetch: true,
    });
    finish(&mut app).await;
    let record = app.latest_operation.as_ref().unwrap();
    assert!(record.result.as_ref().unwrap().is_err());
    assert!(record.output.contains("Fetching remotes"));
    assert!(!record.output.contains("Creating worktree"));
    assert_eq!(record.target, blocked);
    assert!(!blocked.exists());
}

#[tokio::test]
async fn rejected_push_and_protected_cleanup_keep_useful_failure_details() {
    let temp = TempDir::new().unwrap();
    let path = repo(temp.path()).await;
    git::commit(&path, "Initial", "").await.unwrap();
    let bare = temp.path().join("remote.git");
    fs::create_dir(&bare).unwrap();
    git::run(&bare, ["init", "--bare"]).await.unwrap();
    git::run(&path, ["remote", "add", "origin", bare.to_str().unwrap()])
        .await
        .unwrap();
    git::publish(&path, "origin", "main").await.unwrap();
    let hook = bare.join("hooks/pre-receive");
    fs::write(
        &hook,
        "#!/bin/sh\necho 'Server policy rejected this push' >&2\nexit 1\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(path.join("file.txt"), "changed").unwrap();
    git::run(&path, ["add", "file.txt"]).await.unwrap();
    git::commit(&path, "Feature", "").await.unwrap();
    let mut app = explorer(&temp, &path).await;
    app.action("push");
    finish(&mut app).await;
    app.action("operation_details");
    let text = rendered(&app);
    assert!(text.contains("Push → origin/main"));
    assert!(text.contains("Failed"));
    assert!(text.contains("Server policy rejected this push"));
    assert!(!app.busy);
    assert_eq!(git::status(&path).await.unwrap().ahead, 1);
    app.operation(Operation::Remove(path.clone()));
    finish(&mut app).await;
    let text = rendered(&app);
    assert!(
        text.contains("original checkout cannot be removed"),
        "{text}"
    );
    assert!(path.exists());
}
