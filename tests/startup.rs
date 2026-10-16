use clap::Parser;
use std::{ffi::OsString, fs, path::Path};
use treerail::{
    config::Config,
    discover, git,
    startup::{Options, Session},
};
fn prepare(config: &Path, extra: impl IntoIterator<Item = OsString>) -> anyhow::Result<Session> {
    let mut args = vec![
        OsString::from("treerail"),
        "--config".into(),
        config.as_os_str().to_owned(),
    ];
    args.extend(extra);
    Options::try_parse_from(args).unwrap().prepare()
}
#[test]
fn invalid_paths_files_do_not_create_a_partial_startup_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("new/config.toml");
    let result = prepare(
        &config,
        [
            "--paths-file".into(),
            temp.path().join("missing.txt").into_os_string(),
        ],
    );
    assert!(format!("{:#}", result.err().unwrap()).contains("Read paths file"));
    assert!(!config.parent().unwrap().exists());
}

async fn repository(path: &Path) {
    fs::create_dir_all(path).unwrap();
    git::run(path, ["init", "--initial-branch=main"])
        .await
        .unwrap();
}
#[tokio::test]
async fn positional_and_paths_file_sources_are_normalized_deduplicated_and_restored() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("settings/config.toml");
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    repository(&first.join("alpha")).await;
    repository(&second.join("beta")).await;
    repository(&first.join("ignored.worktrees/feature")).await;
    let list = temp.path().join("sources.txt");
    fs::write(&list, "\u{feff}# comments\nsecond\nfirst\n").unwrap();
    let session = prepare(
        &config,
        [
            first.clone().into_os_string(),
            "--paths-file".into(),
            list.into_os_string(),
        ],
    )
    .unwrap();
    assert_eq!(
        session.config.project_roots,
        [first.clone(), second.clone()]
    );
    assert!(
        session
            .config
            .project_roots
            .iter()
            .all(|path| path.is_absolute())
    );
    let projects = discover::discover(session.roots, session.recursive)
        .await
        .unwrap()
        .0;
    assert_eq!(
        projects
            .iter()
            .map(|project| project.name.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "beta"]
    );
    let restarted = prepare(&config, Vec::<OsString>::new()).unwrap();
    assert_eq!(restarted.roots, [first, second]);
    assert_eq!(
        discover::discover(restarted.roots, restarted.recursive)
            .await
            .unwrap()
            .0
            .len(),
        2
    );
}
#[tokio::test]
async fn temporary_sources_are_discovered_for_one_session_and_fallback_is_never_saved() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config.toml");
    let source = temp.path().join("sources");
    repository(&source.join("temporary")).await;
    let list = temp.path().join("sources.txt");
    fs::write(&list, "sources\n").unwrap();
    let session = prepare(
        &config,
        [
            "--temporary".into(),
            "--paths-file".into(),
            list.into_os_string(),
            source.into_os_string(),
        ],
    )
    .unwrap();
    assert_eq!(
        discover::discover(session.roots, false).await.unwrap().0[0].name,
        "temporary"
    );
    assert!(Config::load(&config).unwrap().project_roots.is_empty());
    let next = prepare(&config, Vec::<OsString>::new()).unwrap();
    assert_eq!(next.roots, [std::env::current_dir().unwrap()]);
    assert!(Config::load(&config).unwrap().project_roots.is_empty());
}
#[tokio::test]
async fn actual_cli_listing_reads_sources_without_changing_existing_or_missing_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("sources");
    repository(&source.join("listed")).await;
    let list = temp.path().join("sources.txt");
    fs::write(&list, "sources\n").unwrap();
    let existing = temp.path().join("config.toml");
    fs::write(&existing, "# do not rewrite me\nproject_roots = []\n").unwrap();
    let missing = temp.path().join("new/config.toml");
    for config in [&existing, &missing] {
        let before = fs::read(config).ok();
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_treerail"))
            .args(["--list", "--config"])
            .arg(config)
            .arg(&source)
            .arg("--paths-file")
            .arg(&list)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let projects: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0]["name"], "listed");
        assert_eq!(fs::read(config).ok(), before);
    }
    assert!(!missing.parent().unwrap().exists());
    assert!(!temp.path().join("config.toml.lock").exists());
}
#[test]
fn rejected_noninteractive_launch_does_not_persist_supplied_sources() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("new/config.toml");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_treerail"))
        .arg("--config")
        .arg(&config)
        .arg(temp.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires an interactive terminal"));
    assert!(!config.parent().unwrap().exists());
}
