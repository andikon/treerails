use ratatui::{Terminal, backend::TestBackend};
use treerail::{
    app::App,
    config::{Config, State},
    git::Project,
    settings::{Draft, Section},
    ui,
};

fn app() -> App {
    App::new(
        Config::default(),
        State::default(),
        "unused.toml".into(),
        vec![],
        false,
    )
}

fn render(app: &App, width: u16) -> (String, String) {
    let mut terminal = Terminal::new(TestBackend::new(width, 40)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer();
    let screen = buffer.content.iter().map(|c| c.symbol()).collect();
    let footer = buffer.content[39 * width as usize..]
        .iter()
        .map(|c| c.symbol())
        .collect::<String>()
        .trim()
        .to_owned();
    (screen, footer)
}

#[test]
fn explorer_legends_prioritize_actions_and_fit_whole_hints_at_narrow_widths() {
    let app = app();
    let (_, footer) = render(&app, 120);
    assert!(footer.contains("w basic worktrees"));
    assert!(footer.contains(", settings"));
    for routine in ["j/k", "h/l", "/ search", "Enter", "navigate"] {
        assert!(
            !footer.contains(routine),
            "Unexpected routine hint: {footer}"
        );
    }
    assert_eq!(render(&app, 8).1, "?");
    for width in [9, 20, 40, 60] {
        let (_, footer) = render(&app, width);
        assert!(
            footer.ends_with("? actions"),
            "Guide was clipped at {width}: {footer}"
        );
        assert!(!footer.ends_with('·'));
        assert!(!footer.contains("w basic") || footer.contains("w basic worktrees"));
    }
}

#[test]
fn basic_pane_actions_follow_focus_and_respect_explorer_overrides() {
    let mut app = app();
    app.project = Some(Project {
        name: "repo".into(),
        path: ".".into(),
    });
    app.ticket_mode = false;
    app.config.keys.insert("a".into(), "commit".into());
    app.config
        .keys
        .insert("alt+a".into(), "create_worktree".into());
    let (_, trees) = render(&app, 140);
    assert!(trees.contains("Alt+a worktree"));
    assert!(!trees.contains("s stage"));
    app.focus = 1;
    let (_, files) = render(&app, 140);
    assert!(files.contains("s stage"));
    assert!(files.contains("u unstage"));
    assert!(!files.contains("Alt+a worktree"));
    app.focus = 2;
    let (_, diff) = render(&app, 140);
    assert!(diff.contains("v PR comparison"));
    assert!(!diff.contains("s stage"));
}

#[test]
fn ticket_legends_follow_preparation_and_the_action_guide_covers_hosted_work() {
    let mut app = app();
    app.project = Some(Project {
        name: "repo".into(),
        path: ".".into(),
    });
    let (_, issues) = render(&app, 120);
    assert!(!issues.contains("s continue"));
    app.ticket = Some(serde_json::from_value(serde_json::json!({
        "repository":"owner/repo", "project":".",
        "issue":{"number":1,"title":"Implement feature","body":"Requirements","state":"OPEN","url":"https://github.com/owner/repo/issues/1"},
        "fetched_at":1, "tasks":[{"id":"task-1","title":"Implement","progress":"pending"}],
        "next_task_id":2,"selected_task":"task-1","worktree":null
    })).unwrap());
    let (_, unprepared) = render(&app, 120);
    assert!(unprepared.contains("c create checkout"));
    assert!(!unprepared.contains("s continue"));
    app.ticket.as_mut().unwrap().worktree = Some(treerail::workflow::Checkout {
        path: ".".into(),
        branch: "ticket/1".into(),
        original: false,
    });
    let (_, prepared) = render(&app, 120);
    assert!(prepared.contains("s continue"));
    assert!(!prepared.contains("c create checkout"));
    app.action("help");
    let (help, _) = render(&app, 120);
    for action in [
        "TICKET WORKFLOW",
        "b handover",
        "x stop",
        "Ctrl+]",
        "D preview",
    ] {
        assert!(help.contains(action), "Missing action {action}");
    }
    assert!(!help.contains("j/k"));
}

#[test]
fn settings_and_forms_show_only_their_special_editing_actions() {
    let mut app = app();
    app.settings = Some(Draft::new(app.config.clone()));
    app.settings_open = true;
    app.settings.as_mut().unwrap().section = Section::Notifications;
    let (_, footer) = render(&app, 120);
    assert!(footer.contains("Ctrl+s apply"));
    assert!(!footer.contains("a add"));
    assert!(!footer.contains("Enter"));
    app.action("help");
    assert!(render(&app, 120).0.contains("4 Notifications"));
    app.modal = None;
    app.settings_open = false;
    app.project = Some(Project {
        name: "repo".into(),
        path: ".".into(),
    });
    app.ticket_mode = false;
    app.action("commit");
    let (commit, footer) = render(&app, 120);
    assert!(footer.contains("Ctrl+s submit"));
    assert!(
        !commit.contains("Ctrl+b"),
        "Commit advertised an unavailable base picker"
    );
    app.modal = None;
    app.create_form("feature/test".into(), false, false);
    assert!(render(&app, 120).0.contains("Ctrl+b choose starting point"));
}
