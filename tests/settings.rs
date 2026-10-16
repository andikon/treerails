use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use treerail::{app::App, config, ui};
fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}
fn render(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}
#[test]
fn settings_are_available_in_an_empty_explorer_and_keep_an_unsaved_source() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let (config, state) = config::open(&path, true, None).unwrap();
    let mut app = App::new(config, state, path.clone(), vec![], false);
    app.key(key(','));
    assert!(render(&app, 120, 35).contains("Settings"));
    app.key(key('a'));
    app.paste(temp.path().to_str().unwrap());
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.config.project_roots.is_empty());
    app.key(key(','));
    assert!(render(&app, 120, 35).contains("Unsaved"));
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert_eq!(
        config::Config::load(&path).unwrap().project_roots,
        [temp.path().to_path_buf()]
    );
}

#[test]
fn invalid_sources_keep_the_draft_editable_and_settings_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let (config, state) = config::open(&path, true, None).unwrap();
    let original = std::fs::read(&path).unwrap();
    let mut app = App::new(config, state, path.clone(), vec![], false);
    app.key(key(','));
    app.key(key('a'));
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(render(&app, 120, 35).contains("Enter a project source directory"));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    app.paste(temp.path().join("does-not-exist").to_str().unwrap());
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(render(&app, 120, 35).contains("Source directory is unavailable"));
    assert!(app.config.project_roots.is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

#[test]
fn conflicting_settings_offer_a_choice_and_keep_unrelated_editor_changes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    std::fs::write(&path,"# user notes\nproject_roots = [] # sources\n[profiles.editor]\nprogram = 'nvim'\nenv = {KEEP = 'original'} # launcher note\n[state]\nlast_project = 'first'\n").unwrap();
    let (config, state) = config::open(&path, false, None).unwrap();
    let mut app = App::new(config, state, path.clone(), vec![], false);
    app.key(key(','));
    app.key(key('a'));
    app.paste(temp.path().to_str().unwrap());
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.key(key('r'));
    let external = temp.path().join("external");
    std::fs::create_dir(&external).unwrap();
    config::Config::save_roots(&path, std::slice::from_ref(&external)).unwrap();
    let edited = std::fs::read_to_string(&path)
        .unwrap()
        .replace(
            "project_roots =",
            "default_profile = 'editor'\nproject_roots =",
        )
        .replace("'original'", "'edited'")
        .replace("'first'", "'new selection'");
    std::fs::write(&path, &edited).unwrap();
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    let text = render(&app, 120, 35);
    assert!(text.contains("Changed elsewhere: project_roots"), "{text}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
    assert!(app.config.project_roots.is_empty());
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(render(&app, 120, 35).contains("Unsaved"));
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    app.key(key('m'));
    let saved = config::Config::load(&path).unwrap();
    assert_eq!(saved.project_roots, [temp.path().to_path_buf()]);
    assert!(saved.recursive);
    assert_eq!(saved.default_profile, "editor");
    assert_eq!(saved.profiles["editor"].env["KEEP"], "edited");
    assert_eq!(saved.state.last_project, Some("new selection".into()));
    let raw = std::fs::read_to_string(&path).unwrap();
    for comment in ["# user notes", "# sources", "# launcher note"] {
        assert!(raw.contains(comment), "{raw}");
    }
}

#[test]
fn keeping_external_conflicts_still_applies_nonconflicting_draft_changes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let (config, state) = config::open(&path, true, None).unwrap();
    let mut app = App::new(config, state, path.clone(), vec![], false);
    app.key(key(','));
    app.key(key('a'));
    app.paste(temp.path().to_str().unwrap());
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.key(key('r'));
    let external = temp.path().join("external");
    std::fs::create_dir(&external).unwrap();
    config::Config::save_roots(&path, std::slice::from_ref(&external)).unwrap();
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    app.key(key('e'));
    let saved = config::Config::load(&path).unwrap();
    assert_eq!(saved.project_roots, [external]);
    assert!(saved.recursive);
}

#[test]
fn quitting_requires_a_choice_before_discarding_a_settings_draft() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let (config, state) = config::open(&path, true, None).unwrap();
    let mut app = App::new(config, state, path.clone(), vec![], false);
    app.key(key(','));
    app.key(key('r'));
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.key(key('q'));
    assert!(!app.quit);
    assert!(render(&app, 100, 25).contains("Discard unsaved settings and quit"));
    app.key(key('n'));
    assert!(!app.quit);
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.key(key('q'));
    app.key(key('y'));
    assert!(app.quit);
    assert!(!config::Config::load(&path).unwrap().recursive);
}

#[tokio::test]
async fn unavailable_sources_are_kept_and_removing_sources_does_not_delete_directories() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let missing = temp.path().join("offline");
    config::Config::save_roots(&path, &[temp.path().to_path_buf(), missing.clone()]).unwrap();
    let (config, state) = config::open(&path, false, None).unwrap();
    let mut app = App::new(
        config,
        state,
        path.clone(),
        vec![temp.path().to_path_buf(), missing.clone()],
        false,
    );
    let result = app.jobs.remove(0).execute().await;
    app.accept(treerail::app::Completion {
        epoch: app.epoch,
        token: "discover".into(),
        result,
    });
    app.key(key(','));
    assert!(render(&app, 120, 35).contains("unavailable; kept"));
    app.key(key('d'));
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert_eq!(
        config::Config::load(&path).unwrap().project_roots,
        [missing]
    );
    assert!(temp.path().is_dir());
    assert!(path.is_file());
    for (width, height) in [(140, 40), (80, 24), (30, 10), (8, 3)] {
        let _ = render(&app, width, height);
    }
}

#[test]
fn vim_half_page_navigation_does_not_remove_saved_sources() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    config::Config::save_roots(&path, &[first.clone(), second.clone()]).unwrap();
    let (config, state) = config::open(&path, false, None).unwrap();
    let mut app = App::new(
        config,
        state,
        path.clone(),
        vec![first.clone(), second.clone()],
        false,
    );
    app.key(key(','));
    app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert_eq!(
        config::Config::load(&path).unwrap().project_roots,
        [first, second]
    );
}

#[tokio::test]
async fn applying_settings_ignores_stale_discovery_and_preserves_valid_project_and_worktree() {
    use treerail::{
        app::{Completion, Job, Payload},
        git::{Project, Worktree},
    };
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let source = temp.path().join("sources");
    let project_path = source.join("project");
    std::fs::create_dir_all(&project_path).unwrap();
    treerail::git::run(&project_path, ["init", "--initial-branch=main"])
        .await
        .unwrap();
    config::Config::save_roots(&path, std::slice::from_ref(&source)).unwrap();
    let (config, state) = config::open(&path, false, None).unwrap();
    let mut app = App::new(config, state, path, vec![source], false);
    app.accept(Completion {
        epoch: app.epoch,
        token: "discover".into(),
        result: Ok(Payload::Projects(
            vec![Project {
                path: project_path.clone(),
                name: "project".into(),
            }],
            vec![],
        )),
    });
    app.open_project();
    app.accept(Completion {
        epoch: app.epoch,
        token: "trees".into(),
        result: Ok(Payload::Trees(
            project_path.clone(),
            vec![
                Worktree {
                    path: project_path.clone(),
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
            ],
            "main".into(),
        )),
    });
    app.key(key('j'));
    assert_eq!(app.selected_tree().unwrap().branch, "feature");
    let old_epoch = app.epoch;
    app.key(key(','));
    app.key(key('r'));
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    app.accept(Completion {
        epoch: old_epoch,
        token: "discover".into(),
        result: Ok(Payload::Projects(vec![], vec![])),
    });
    assert!(app.project.is_some());
    let index = app
        .jobs
        .iter()
        .position(|job| matches!(job, Job::Discover(..)))
        .unwrap();
    let result = app.jobs.remove(index).execute().await;
    app.accept(Completion {
        epoch: app.epoch,
        token: "discover".into(),
        result,
    });
    assert_eq!(app.project.as_ref().unwrap().path, project_path);
    assert_eq!(app.selected_tree().unwrap().branch, "feature");
    // Removing the sole source takes the project out of the explorer, without deleting it.
    app.key(key('d'));
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    let index = app
        .jobs
        .iter()
        .position(|job| matches!(job, Job::Discover(..)))
        .unwrap();
    let result = app.jobs.remove(index).execute().await;
    app.accept(Completion {
        epoch: app.epoch,
        token: "discover".into(),
        result,
    });
    assert!(app.project.is_none());
    assert!(project_path.join(".git").is_dir());
}

#[test]
fn independent_settings_drafts_merge_changes_to_different_fields() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let (first_config, first_state) = config::open(&path, true, None).unwrap();
    let (second_config, second_state) = config::open(&path, false, None).unwrap();
    let mut first = App::new(first_config, first_state, path.clone(), vec![], false);
    let mut second = App::new(second_config, second_state, path.clone(), vec![], false);
    first.key(key(','));
    second.key(key(','));
    first.key(key('r'));
    first.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    second.key(key('a'));
    second.paste(temp.path().to_str().unwrap());
    second.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    let saved = config::Config::load(&path).unwrap();
    assert!(saved.recursive);
    assert_eq!(saved.project_roots, [temp.path().to_path_buf()]);
    assert!(render(&second, 120, 35).contains("Saved & applied"));
}
