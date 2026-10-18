use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use treerail::{
    app::{App, Modal},
    config,
    git::Project,
    ui,
};
fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}
fn ctrl(app: &mut App, c: char) {
    app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
}
fn enter(app: &mut App) {
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
}
fn tab(app: &mut App) {
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
}
fn replace(app: &mut App, text: &str) {
    ctrl(app, 'u');
    app.paste(text);
}
fn render(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(120, 35)).unwrap();
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
fn global_defaults_apply_to_the_next_worktree_form_without_restart() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let project = temp.path().join("app");
    std::fs::create_dir(&project).unwrap();
    let (config, state) = config::open(&path, true, None).unwrap();
    let mut app = App::new(config, state, path.clone(), vec![], true);
    app.projects.push(Project {
        path: project.clone(),
        name: "app".into(),
    });
    app.open_basic_project();
    app.key(key(','));
    app.key(key('2'));
    assert!(render(&app).contains("Worktrees"));
    let epoch = app.epoch;
    enter(&mut app);
    replace(&mut app, "custom/{branch_slug}");
    tab(&mut app);
    replace(&mut app, "terminal");
    ctrl(&mut app, 's');
    assert!(
        app.recursive,
        "temporary recursive discovery override was lost"
    );
    assert_eq!(app.epoch, epoch, "unrelated settings triggered a rescan");
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.key(key('a'));
    app.paste("feature/test");
    let Some(Modal::Form(form)) = &app.modal else {
        panic!("worktree form missing")
    };
    assert_eq!(
        std::path::PathBuf::from(&form.fields[2].value),
        project.join("custom/feature-test")
    );
    assert_eq!(
        config::Config::load(&path).unwrap().default_profile,
        "terminal"
    );
}

#[test]
fn project_defaults_merge_by_identity_and_field_while_preserving_other_projects() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    let quoted =
        |path: &std::path::Path| toml::Value::String(path.display().to_string()).to_string();
    let original = format!(
        "# my projects\n[[projects]]\npath = {}\nbase = 'old' # first base\ndefault_profile = 'editor' # first profile\n\n# second project notes\n[[projects]]\npath = {}\nbase = 'keep' # second base\n",
        quoted(&first),
        quoted(&second)
    );
    std::fs::write(&path, &original).unwrap();
    let (config, mut state) = config::open(&path, false, None).unwrap();
    state
        .bases
        .insert(first.display().to_string(), "remembered".into());
    let mut app = App::new(config, state, path.clone(), vec![], false);
    app.projects.push(Project {
        path: first.clone(),
        name: "first".into(),
    });
    app.open_basic_project();
    app.key(key(','));
    app.key(key('2'));
    app.key(key('j'));
    enter(&mut app);
    replace(&mut app, "origin/develop");
    tab(&mut app);
    replace(&mut app, "custom/{branch_slug}");
    std::fs::write(
        &path,
        original
            .replace("'keep'", "'externally-updated'")
            .replace("'editor'", "'terminal'"),
    )
    .unwrap();
    ctrl(&mut app, 's');
    let saved = config::Config::load(&path).unwrap();
    assert_eq!(
        saved.project(&first).unwrap().base.as_deref(),
        Some("origin/develop")
    );
    assert_eq!(
        saved.project(&first).unwrap().default_profile.as_deref(),
        Some("terminal")
    );
    assert_eq!(
        saved.project(&second).unwrap().base.as_deref(),
        Some("externally-updated")
    );
    let raw = std::fs::read_to_string(&path).unwrap();
    for comment in [
        "# my projects",
        "# first base",
        "# first profile",
        "# second project notes",
        "# second base",
    ] {
        assert!(raw.contains(comment), "{raw}");
    }
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.key(key('a'));
    app.paste("feature/project");
    let Some(Modal::Form(form)) = &app.modal else {
        panic!("form missing")
    };
    assert_eq!(form.fields[1].value, "origin/develop");
    assert_eq!(
        std::path::PathBuf::from(&form.fields[2].value),
        first.join("custom/feature-project")
    );
}

#[test]
fn a_new_profile_preserves_argument_arrays_environment_and_custom_terminal_launch_planning() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let tree = temp.path().join("work tree");
    std::fs::create_dir_all(tree.join("sub dir")).unwrap();
    std::fs::write(&path,"# custom adapter\n[terminals.custom]\nprogram = 'test-terminal'\nargs = ['--cwd', '{cwd}', '--', '{command}']\n[keys]\n'alt+o' = 'profiles'\n[theme]\naccent = '#aabbcc'\n").unwrap();
    let (config, state) = config::open(&path, false, None).unwrap();
    let mut app = App::new(config, state, path.clone(), vec![], false);
    app.key(key(','));
    app.key(key('3'));
    app.key(key('a'));
    for (index, value) in [
        "review agent",
        "nvim",
        "['a file.txt', '--branch', '{branch}', 'j k q']",
        "{worktree}/sub dir",
        "{ TASK = 'j k q', BRANCH = '{branch}' }",
        "custom",
    ]
    .iter()
    .enumerate()
    {
        if index > 0 {
            tab(&mut app);
        }
        replace(&mut app, value);
    }
    enter(&mut app);
    assert!(!app.quit);
    app.key(key('2'));
    enter(&mut app);
    tab(&mut app);
    replace(&mut app, "review agent");
    ctrl(&mut app, 's');
    let saved = config::Config::load(&path).unwrap();
    assert_eq!(saved.default_profile, "review agent");
    let plan = treerail::launch::plan(
        &app.config,
        &app.config.profiles["review agent"],
        &tree,
        "feature/test",
    )
    .unwrap();
    assert_eq!(plan.program, "test-terminal");
    assert_eq!(plan.cwd, tree.join("sub dir"));
    assert_eq!(plan.args[0], "--cwd");
    assert_eq!(
        std::path::PathBuf::from(&plan.args[1]),
        tree.join("sub dir")
    );
    assert_eq!(
        plan.args[2..],
        [
            "--",
            "nvim",
            "a file.txt",
            "--branch",
            "feature/test",
            "j k q"
        ]
    );
    assert_eq!(plan.env["TASK"], "j k q");
    assert_eq!(plan.env["BRANCH"], "feature/test");
    assert_eq!(saved.keys["alt+o"], "profiles");
    assert_eq!(saved.theme.accent, "#aabbcc");
}

#[test]
fn editing_an_implicit_builtin_profile_keeps_its_default_program() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    std::fs::write(&path, "# minimal configuration\n").unwrap();
    let (config, state) = config::open(&path, false, None).unwrap();
    let mut app = App::new(config, state, path.clone(), vec![], false);
    app.key(key(','));
    app.key(key('3'));
    app.key(key('j'));
    enter(&mut app);
    tab(&mut app);
    replace(&mut app, "['a file.txt']");
    ctrl(&mut app, 's');
    let saved = config::Config::load(&path).unwrap();
    assert_eq!(saved.profiles["editor"].program, "nvim");
    assert_eq!(saved.profiles["editor"].args, ["a file.txt"]);
}

#[test]
fn deleted_profiles_require_a_choice_and_keep_mine_restores_the_complete_draft() {
    for choice in ['m', 'e'] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        let adapter = "[terminals.custom]\nprogram = 'test-terminal'\nargs = ['{command}']\n";
        std::fs::write(&path,format!("{adapter}[profiles.custom]\nprogram = 'nvim'\nargs = ['old']\ncwd = '{{worktree}}/subdir'\nterminal = 'custom'\nenv = {{KEEP = 'value'}}\n")).unwrap();
        let (config, state) = config::open(&path, false, None).unwrap();
        let mut app = App::new(config, state, path.clone(), vec![], false);
        app.key(key(','));
        app.key(key('3'));
        app.key(key('j'));
        enter(&mut app);
        tab(&mut app);
        replace(&mut app, "['new file.txt']");
        std::fs::write(&path, adapter).unwrap();
        ctrl(&mut app, 's');
        assert!(render(&app).contains("Changed elsewhere"));
        app.key(key(choice));
        let saved = config::Config::load(&path).unwrap();
        if choice == 'm' {
            let profile = &saved.profiles["custom"];
            assert_eq!(profile.program, "nvim");
            assert_eq!(profile.args, ["new file.txt"]);
            assert_eq!(profile.cwd, "{worktree}/subdir");
            assert_eq!(profile.env["KEEP"], "value");
            assert_eq!(profile.terminal, "custom");
        } else {
            assert!(!saved.profiles.contains_key("custom"));
        }
    }
}

#[test]
fn invalid_profile_fields_and_references_do_not_replace_active_settings() {
    for (tabs, value, expected) in [
        (1, "['ok', 42]", "Arguments must be a TOML array"),
        (2, "", "Working directory"),
        (3, "{ MODE = 123 }", "Environment must be a TOML map"),
        (4, "missing-adapter", "Unknown terminal adapter"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        let (config, state) = config::open(&path, true, None).unwrap();
        let original = std::fs::read(&path).unwrap();
        let mut app = App::new(config, state, path.clone(), vec![], false);
        app.key(key(','));
        app.key(key('3'));
        app.key(key('j'));
        enter(&mut app);
        for _ in 0..tabs {
            tab(&mut app);
        }
        replace(&mut app, value);
        ctrl(&mut app, 's');
        assert!(render(&app).contains(expected), "{}", render(&app));
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(app.config.profiles["editor"].program, "nvim");
        let correction = match tabs {
            1 => "['corrected']",
            2 => "{worktree}",
            3 => "{ MODE = 'dev' }",
            _ => "auto",
        };
        replace(&mut app, correction);
        ctrl(&mut app, 's');
        assert!(render(&app).contains("Saved & applied"), "{}", render(&app));
    }
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let (config, state) = config::open(&path, true, None).unwrap();
    let original = std::fs::read(&path).unwrap();
    let mut app = App::new(config, state, path.clone(), vec![], false);
    app.key(key(','));
    app.key(key('2'));
    enter(&mut app);
    tab(&mut app);
    replace(&mut app, "unknown-profile");
    ctrl(&mut app, 's');
    assert!(render(&app).contains("Unknown default_profile"));
    assert_eq!(std::fs::read(path).unwrap(), original);
}

#[test]
fn inline_project_defaults_keep_comments_and_deleted_project_defaults_restore_all_fields() {
    for deleted in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        let project = temp.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let quoted = toml::Value::String(project.display().to_string()).to_string();
        std::fs::write(&path,format!("projects = [{{path = {quoted}, base = 'old', path_template = 'trees/{{branch_slug}}', default_profile = 'editor'}}] # project note\n")).unwrap();
        let (config, state) = config::open(&path, false, None).unwrap();
        let mut app = App::new(config, state, path.clone(), vec![], false);
        app.projects.push(Project {
            path: project.clone(),
            name: "project".into(),
        });
        app.open_basic_project();
        app.key(key(','));
        app.key(key('2'));
        app.key(key('j'));
        enter(&mut app);
        replace(&mut app, "origin/develop");
        if deleted {
            std::fs::write(&path, "# project removed elsewhere\n").unwrap();
        }
        ctrl(&mut app, 's');
        if deleted {
            assert!(render(&app).contains("Changed elsewhere"));
            app.key(key('m'));
        }
        let saved = config::Config::load(&path).unwrap();
        let defaults = saved.project(&project).unwrap();
        assert_eq!(defaults.base.as_deref(), Some("origin/develop"));
        assert_eq!(
            defaults.path_template.as_deref(),
            Some("trees/{branch_slug}")
        );
        assert_eq!(defaults.default_profile.as_deref(), Some("editor"));
        if !deleted {
            assert!(
                std::fs::read_to_string(&path)
                    .unwrap()
                    .contains("# project note")
            );
        }
    }
}

#[test]
fn profile_text_and_unfinished_forms_survive_navigation_and_render_at_small_sizes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let (config, state) = config::open(&path, true, None).unwrap();
    let mut app = App::new(config, state, path.clone(), vec![], false);
    app.key(key(','));
    app.key(key('3'));
    app.key(key('a'));
    for c in "joking q".chars() {
        app.key(key(c));
    }
    tab(&mut app);
    for c in "j k q 2".chars() {
        app.key(key(c));
    }
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.quit);
    app.key(key(','));
    for (width, height) in [(140, 40), (80, 24), (30, 10), (8, 3)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    }
    ctrl(&mut app, 's');
    assert_eq!(
        config::Config::load(path.as_path()).unwrap().profiles["joking q"].program,
        "j k q 2"
    );
    assert!(!app.quit);
}

#[test]
fn another_projects_defaults_can_be_added_without_adding_a_discovery_source() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let project = temp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let (config, state) = config::open(&path, true, None).unwrap();
    let mut app = App::new(config, state, path.clone(), vec![], false);
    app.key(key(','));
    app.key(key('2'));
    app.key(key('a'));
    replace(&mut app, project.to_str().unwrap());
    tab(&mut app);
    replace(&mut app, "origin/develop");
    ctrl(&mut app, 's');
    let saved = config::Config::load(&path).unwrap();
    assert_eq!(
        saved.project(&project).unwrap().base.as_deref(),
        Some("origin/develop")
    );
    assert!(saved.project_roots.is_empty());
}
