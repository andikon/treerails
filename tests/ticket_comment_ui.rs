use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use treerail::{
    app::{App, Completion, Job, Payload},
    config::{Config, State},
    git::Project,
    ticket_comments::{self, Command, Event, Review},
    ui, workflow,
};
fn app(path: &std::path::Path) -> App {
    let mut app = App::new(
        Config::default(),
        State::default(),
        path.join("settings.toml"),
        vec![],
        false,
    );
    app.project = Some(Project {
        name: "repo".into(),
        path: path.into(),
    });
    app.browser.repository = "github.com/owner/repo".into();
    app.browser.selected = Some(7);
    app
}
fn key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    app.key(KeyEvent::new(code, modifiers));
}
#[test]
fn comment_input_owns_workflow_letters_paste_and_persists_when_leaving() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    key(&mut app, KeyCode::Char('c'), KeyModifiers::NONE);
    assert!(app.comment_editor.is_some());
    for ch in "Eyof?".chars() {
        key(&mut app, KeyCode::Char(ch), KeyModifiers::NONE);
    }
    app.paste("\nPasted body");
    assert!(app.ticket_mode);
    assert!(app.ticket.is_none());
    assert_eq!(
        app.comment_editor.as_ref().unwrap().draft.text,
        "Eyof?\nPasted body"
    );
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.comment_editor.is_none());
    assert_eq!(
        ticket_comments::load(&app.config_path, "github.com/owner/repo", 7)
            .unwrap()
            .text,
        "Eyof?\nPasted body"
    );
    key(&mut app, KeyCode::Char('c'), KeyModifiers::NONE);
    assert_eq!(
        app.comment_editor.as_ref().unwrap().draft.text,
        "Eyof?\nPasted body"
    );
}
#[test]
fn only_review_screen_accepts_y_as_publication_and_displays_fixed_target() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    key(&mut app, KeyCode::Char('c'), KeyModifiers::NONE);
    app.paste("Approved text");
    app.jobs.clear();
    key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
    assert!(matches!(
        app.jobs.last(),
        Some(Job::Workflow(
            _,
            _,
            workflow::Command::Comment(Command::Review { number: 7, .. })
        ))
    ));
    let editor = app.comment_editor.as_ref().unwrap();
    let review = Review {
        repository: editor.repository.clone(),
        number: 7,
        login: "posting-user".into(),
        text: editor.draft.text.clone(),
        revision: editor.draft.revision,
    };
    app.accept(Completion {
        epoch: app.epoch,
        token: "workflow".into(),
        result: Ok(Payload::Workflow(workflow::Event::Comment(Event::Review(
            review,
        )))),
    });
    app.browser.selected = Some(99);
    for width in [45, 120] {
        let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
        terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("posting-user"));
        assert!(text.contains("#7"));
        assert!(text.contains("Approved text"));
    }
    app.jobs.clear();
    key(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
    assert!(
        matches!(app.jobs.last(), Some(Job::Workflow(_, _, workflow::Command::Comment(Command::Send { review, retry_uncertain: false }))) if review.number == 7)
    );
}
#[test]
fn unsaved_conflicting_text_cannot_review_an_older_disk_draft() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = app(temp.path());
    key(&mut app, KeyCode::Char('c'), KeyModifiers::NONE);
    app.paste("Original");
    let saved = app.comment_editor.as_ref().unwrap().draft.clone();
    ticket_comments::save_text(
        &app.config_path,
        "github.com/owner/repo",
        7,
        "Other client's text".into(),
        saved.revision,
    )
    .unwrap();
    key(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
    app.jobs.clear();
    key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
    assert!(app.jobs.is_empty());
    assert_eq!(app.comment_editor.as_ref().unwrap().draft.text, "Originalx");
    assert!(
        app.comment_editor
            .as_ref()
            .unwrap()
            .status
            .starts_with("Draft not saved")
    );
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.comment_editor.is_some());
}
