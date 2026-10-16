use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use std::{collections::BTreeSet, path::PathBuf};
use treerail::{
    app::{App, Completion, FormKind, Modal, Payload},
    config::{Config, State},
    git::{ChangedFile, Project, Snapshot, Worktree},
    ui,
};
fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}
fn app() -> App {
    let mut app = App::new(
        Config::default(),
        State::default(),
        "config.toml".into(),
        vec![],
        false,
    );
    app.projects = vec![Project {
        path: PathBuf::from("/code/app"),
        name: "app".into(),
    }];
    app.open_project();
    app.trees = vec![Worktree {
        path: "/code/app".into(),
        branch: "main".into(),
        head: "123".into(),
        original: true,
        locked: false,
        prunable: false,
    }];
    app.snapshots.insert(
        "/code/app".into(),
        Snapshot {
            branch: "main".into(),
            files: vec![ChangedFile {
                path: "test.txt".into(),
                original: None,
                index: ' ',
                work: 'M',
                untracked: false,
            }],
            ..Snapshot::default()
        },
    );
    app
}
#[test]
fn vim_navigation_and_form_input_are_separate() {
    let mut app = app();
    app.key(key('c'));
    for c in "joking about q".chars() {
        app.key(key(c));
    }
    let Some(Modal::Form(form)) = &app.modal else {
        panic!("Commit form missing")
    };
    assert!(matches!(form.kind, FormKind::Commit));
    assert_eq!(form.fields[0].value, "joking about q");
    assert!(!app.quit);
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.paste("one\r\ntwo");
    let Some(Modal::Form(form)) = &app.modal else {
        panic!()
    };
    assert_eq!(form.fields[1].value, "one\ntwo");
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    app.key(key('l'));
    assert_eq!(app.focus, 1);
}
#[test]
fn release_and_repeat_do_not_start_mutating_commands() {
    let mut app = app();
    let mut release = key('c');
    release.kind = KeyEventKind::Release;
    app.key(release);
    assert!(app.modal.is_none());
    release.kind = KeyEventKind::Repeat;
    app.key(release);
    assert!(app.modal.is_none());
    app.key(key('c'));
    assert!(app.modal.is_some());
}
#[test]
fn refresh_preserves_file_marks_and_scroll_and_ignores_stale_results() {
    let mut app = app();
    app.focus = 1;
    app.marked.insert("test.txt".into());
    app.diff_scroll = 20;
    let trees = app.trees.clone();
    app.accept(Completion {
        epoch: app.epoch,
        token: "trees".into(),
        result: Ok(Payload::Trees("/code/app".into(), trees, "main".into())),
    });
    assert_eq!(app.marked, BTreeSet::from([PathBuf::from("test.txt")]));
    assert_eq!(app.diff_scroll, 20);
    let epoch = app.epoch;
    app.epoch += 1;
    app.accept(Completion {
        epoch,
        token: "status".into(),
        result: Ok(Payload::Status("/code/app".into(), Snapshot::default())),
    });
    assert_eq!(app.snapshot().unwrap().files.len(), 1);
}
#[test]
fn paste_updates_the_default_worktree_path() {
    let mut app = app();
    app.key(key('a'));
    app.paste("feature/login");
    let Some(Modal::Form(form)) = &app.modal else {
        panic!()
    };
    assert!(
        form.fields[2]
            .value
            .ends_with("app.worktrees/feature-login")
            || form.fields[2]
                .value
                .ends_with("app.worktrees\\feature-login")
    );
}
#[test]
fn layouts_render_at_wide_narrow_and_tiny_terminal_sizes() {
    let mut app = app();
    app.diff = "diff --git a/test.txt b/test.txt\n-old\n+new".into();
    for (width, height) in [(140, 40), (80, 24), (30, 10), (8, 3)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| ui::draw(f, &app)).unwrap();
        if width >= 110 {
            let text = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>();
            assert!(text.contains("Worktrees"));
            assert!(text.contains("test.txt"));
            assert!(text.contains("+new"));
        }
        app.key(key('c'));
        terminal.draw(|f| ui::draw(f, &app)).unwrap();
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    }
}

#[test]
fn pickers_have_vim_navigation_and_explicit_text_entry() {
    let mut app = app();
    app.action("profiles");
    app.key(key('j'));
    let Some(Modal::Picker(p)) = &app.modal else {
        panic!()
    };
    assert_eq!(p.selected, 1);
    assert!(!p.editing);
    app.key(key('/'));
    app.key(key('k'));
    let Some(Modal::Picker(p)) = &app.modal else {
        panic!()
    };
    assert_eq!(p.query, "k");
    assert!(p.editing);
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.modal.is_some());
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.modal.is_none());
}

#[test]
fn clearing_a_worktree_filter_and_refresh_keep_the_selected_path() {
    let mut app = app();
    let mut feature = app.trees[0].clone();
    feature.path = "/code/app.worktrees/feature".into();
    feature.branch = "feature".into();
    feature.original = false;
    app.trees.push(feature);
    app.filter = "feature".into();
    app.tree = 0;
    let trees = app.trees.clone();
    app.accept(Completion {
        epoch: app.epoch,
        token: "trees".into(),
        result: Ok(Payload::Trees("/code/app".into(), trees, "main".into())),
    });
    assert_eq!(app.selected_tree().unwrap().branch, "feature");
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.selected_tree().unwrap().branch, "feature");
    assert_eq!(app.tree, 1);
}

#[test]
fn failed_commit_keeps_the_message_but_navigation_discards_recovery() {
    let mut app = app();
    app.action("commit");
    for c in "Keep this message".chars() {
        app.key(key(c));
    }
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(app.busy);
    app.accept(Completion {
        epoch: app.epoch,
        token: "operation".into(),
        result: Err(anyhow::anyhow!("Hook rejected commit")),
    });
    let Some(Modal::Form(form)) = &app.modal else {
        panic!()
    };
    assert_eq!(form.fields[0].value, "Keep this message");
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.action("commit");
    app.key(key('x'));
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    app.action("back");
    app.accept(Completion {
        epoch: app.epoch,
        token: "operation".into(),
        result: Err(anyhow::anyhow!("Hook rejected commit")),
    });
    assert!(app.modal.is_none());
}

#[test]
fn a_slow_pr_query_does_not_replace_a_form_being_edited() {
    let mut app = app();
    app.waiting_prs = true;
    app.action("commit");
    app.key(key('x'));
    app.accept(Completion {
        epoch: app.epoch,
        token: "prs".into(),
        result: Ok(Payload::Prs("/code/app".into(), vec![])),
    });
    let Some(Modal::Form(form)) = &app.modal else {
        panic!()
    };
    assert_eq!(form.fields[0].value, "x");
}
