use std::{
    fs,
    path::{Path, PathBuf},
};
use tempfile::TempDir;
use treerail::{discover, git};
async fn repo(parent: &Path, name: &str) -> PathBuf {
    let path = parent.join(name);
    fs::create_dir_all(&path).unwrap();
    git::run(&path, ["init", "--initial-branch=develop"])
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
    fs::write(path.join("file.txt"), "initial\n").unwrap();
    git::run(&path, ["add", "."]).await.unwrap();
    git::commit(&path, "Initial", "").await.unwrap();
    path
}
#[tokio::test]
async fn line_counts_compare_checkout_with_head_and_include_untracked_text() {
    let temp = TempDir::new().unwrap();
    let path = repo(temp.path(), "line-counts").await;
    fs::write(path.join("file.txt"), "staged\nextra\n").unwrap();
    git::run(&path, ["add", "file.txt"]).await.unwrap();
    // Net against HEAD, rather than adding staged and unstaged diffs twice.
    fs::write(path.join("file.txt"), "initial\nextra\n").unwrap();
    fs::write(path.join("untracked file.txt"), "one\ntwo").unwrap();
    fs::write(path.join("image.bin"), [0, 1, 2, 3]).unwrap();
    let status = git::status(&path).await.unwrap();
    assert_eq!((status.additions, status.deletions), (3, 0));
    git::run(&path, ["mv", "file.txt", "renamed file.txt"])
        .await
        .unwrap();
    let status = git::status(&path).await.unwrap();
    assert_eq!((status.additions, status.deletions), (3, 0));
    fs::remove_file(path.join("renamed file.txt")).unwrap();
    let status = git::status(&path).await.unwrap();
    assert_eq!((status.additions, status.deletions), (2, 1));
}

#[tokio::test]
async fn line_counts_work_before_the_first_commit() {
    let temp = TempDir::new().unwrap();
    git::run(temp.path(), ["init", "--initial-branch=develop"])
        .await
        .unwrap();
    fs::write(temp.path().join("new.txt"), "one\ntwo\n").unwrap();
    git::run(temp.path(), ["add", "new.txt"]).await.unwrap();
    fs::write(temp.path().join("new.txt"), "one\n").unwrap();
    let status = git::status(temp.path()).await.unwrap();
    assert_eq!((status.additions, status.deletions), (1, 0));
}
async fn remote_repo(temp: &TempDir) -> (PathBuf, PathBuf) {
    let source = repo(temp.path(), "source").await;
    let remote = temp.path().join("remote.git");
    git::run(
        temp.path(),
        [
            "clone",
            "--bare",
            source.to_str().unwrap(),
            remote.to_str().unwrap(),
        ],
    )
    .await
    .unwrap();
    git::run(
        &source,
        ["remote", "add", "origin", remote.to_str().unwrap()],
    )
    .await
    .unwrap();
    git::run(&source, ["fetch", "origin"]).await.unwrap();
    (source, remote)
}
#[tokio::test]
async fn default_worktree_uses_fresh_actual_remote_default_not_selected_head() {
    let temp = TempDir::new().unwrap();
    let (source, remote) = remote_repo(&temp).await;
    git::run(&source, ["switch", "-c", "feature"])
        .await
        .unwrap();
    let feature = git::text(&source, &["rev-parse", "HEAD"]).await.unwrap();
    let author = temp.path().join("author");
    git::run(
        temp.path(),
        ["clone", remote.to_str().unwrap(), author.to_str().unwrap()],
    )
    .await
    .unwrap();
    git::run(&author, ["config", "user.name", "Test"])
        .await
        .unwrap();
    git::run(&author, ["config", "user.email", "test@example.invalid"])
        .await
        .unwrap();
    git::run(&author, ["config", "commit.gpgsign", "false"])
        .await
        .unwrap();
    fs::write(author.join("new.txt"), "latest\n").unwrap();
    git::run(&author, ["add", "."]).await.unwrap();
    git::commit(&author, "Latest remote default", "")
        .await
        .unwrap();
    git::run(&author, ["push", "origin", "develop"])
        .await
        .unwrap();
    let expected = git::text(&author, &["rev-parse", "HEAD"]).await.unwrap();
    assert_ne!(feature, expected);
    let base = git::fresh_default_base(&source, None).await.unwrap();
    assert_eq!(base, "origin/develop");
    let destination = temp.path().join("new checkout");
    git::create_worktree(&source, "feat/new", &base, &destination, false, false)
        .await
        .unwrap();
    assert_eq!(
        git::text(&destination, &["rev-parse", "HEAD"])
            .await
            .unwrap(),
        expected
    );
    assert!(git::status(&destination).await.unwrap().upstream.is_none());
}
#[tokio::test]
async fn new_branch_push_publishes_and_sets_upstream() {
    let temp = TempDir::new().unwrap();
    let (source, remote) = remote_repo(&temp).await;
    git::run(&source, ["switch", "-c", "feat/published"])
        .await
        .unwrap();
    git::push_or_publish(&source, None).await.unwrap();
    assert_eq!(
        git::status(&source).await.unwrap().upstream.as_deref(),
        Some("origin/feat/published")
    );
    assert!(
        git::text(&remote, &["rev-parse", "refs/heads/feat/published"])
            .await
            .is_ok()
    );
    git::push_or_publish(&source, None).await.unwrap();
}
#[tokio::test]
async fn removal_refuses_dirty_and_original_checkouts_and_retains_branch() {
    let temp = TempDir::new().unwrap();
    let source = repo(temp.path(), "source").await;
    let tree = temp.path().join("linked");
    git::create_worktree(&source, "feature", "HEAD", &tree, false, false)
        .await
        .unwrap();
    fs::write(tree.join("dirty.txt"), "keep me").unwrap();
    assert!(git::remove_worktree(&source, &tree).await.is_err());
    assert!(tree.exists());
    assert!(git::remove_worktree(&source, &source).await.is_err());
    fs::remove_file(tree.join("dirty.txt")).unwrap();
    git::remove_worktree(&source, &tree).await.unwrap();
    assert!(!tree.exists());
    assert!(
        git::text(&source, &["rev-parse", "refs/heads/feature"])
            .await
            .is_ok()
    );
}
#[tokio::test]
async fn explicit_head_does_not_copy_uncommitted_changes_or_adopt_existing_branch() {
    let temp = TempDir::new().unwrap();
    let source = repo(temp.path(), "with spaces").await;
    fs::write(source.join("file.txt"), "dirty\n").unwrap();
    let destination = temp.path().join("fresh tree");
    let head = git::text(&source, &["rev-parse", "HEAD"]).await.unwrap();
    git::create_worktree(&source, "feat/new", &head, &destination, false, false)
        .await
        .unwrap();
    assert_eq!(
        fs::read_to_string(destination.join("file.txt")).unwrap(),
        "initial\n"
    );
    assert!(
        git::create_worktree(
            &source,
            "feat/new",
            "HEAD",
            &temp.path().join("duplicate"),
            false,
            false
        )
        .await
        .is_err()
    );
    assert!(!temp.path().join("duplicate").exists());
}
#[tokio::test]
async fn staging_and_unstaging_preserve_spaces_and_diff_sides() {
    let temp = TempDir::new().unwrap();
    let source = repo(temp.path(), "source").await;
    fs::write(source.join("a file.txt"), "new\n").unwrap();
    let files = git::status(&source).await.unwrap().files;
    git::stage(&source, &files, false).await.unwrap();
    let file = git::status(&source).await.unwrap().files.remove(0);
    assert!(file.staged());
    assert!(
        git::diff(&source, &file, true)
            .await
            .unwrap()
            .contains("+new")
    );
    git::stage(&source, &[file], true).await.unwrap();
    assert!(git::status(&source).await.unwrap().files[0].untracked);
}
#[tokio::test]
async fn ambiguous_remote_requires_explicit_choice() {
    let temp = TempDir::new().unwrap();
    let source = repo(temp.path(), "source").await;
    for name in ["one", "two"] {
        git::run(&source, ["remote", "add", name, "unused"])
            .await
            .unwrap();
    }
    assert!(git::choose_remote(&source, None).await.is_err());
    assert_eq!(
        git::choose_remote(&source, Some("two")).await.unwrap(),
        "two"
    );
    assert!(git::choose_remote(&source, Some("missing")).await.is_err());
}
#[tokio::test]
async fn discovery_deduplicates_linked_worktrees() {
    let temp = TempDir::new().unwrap();
    let source = repo(temp.path(), "source").await;
    let tree = temp.path().join("source.worktrees").join("feature");
    git::create_worktree(&source, "feature", "HEAD", &tree, false, false)
        .await
        .unwrap();
    let (projects, warnings) =
        discover::discover(vec![temp.path().into(), source.clone(), tree], true)
            .await
            .unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(projects.len(), 1);
    assert!(treerail::config::same_path(&projects[0].path, &source));
}
