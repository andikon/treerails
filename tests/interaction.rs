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
    app.open_basic_project();
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
    let token = app.jobs.last().unwrap().token();
    app.accept(Completion {
        epoch: app.epoch,
        token,
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
    let token = app.jobs.last().unwrap().token();
    app.action("back");
    app.accept(Completion {
        epoch: app.epoch,
        token,
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

#[test]
fn operation_feedback_stays_attributed_during_navigation_and_search() {
    let mut app = app();
    app.action("fetch");
    let token = app.jobs.last().unwrap().token();
    app.action("back");
    app.action("search");
    app.key(key('x'));
    app.say("Unrelated refresh notice", false);
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(text.contains("Fetch"), "{text}");
    assert!(text.contains("/code/app"), "{text}");
    app.accept(Completion {
        epoch: 0,
        token: token.clone(),
        result: Err(anyhow::anyhow!("Connection refused")),
    });
    assert!(!app.busy);
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.action("operation_details");
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(text.contains("Connection refused"));
    app.accept(Completion {
        epoch: 0,
        token,
        result: Ok(Payload::Done(
            "/code/app".into(),
            None,
            "stale success".into(),
        )),
    });
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(text.contains("Connection refused"));
    assert!(!text.contains("stale success"));
}

#[test]
fn progress_belongs_to_current_phase_and_stale_events_cannot_change_a_new_operation() {
    use treerail::feedback::{Event, Update};
    let mut app = app();
    app.action("fetch");
    let token = app.jobs.last().unwrap().token();
    let id = token.strip_prefix("operation:").unwrap().parse().unwrap();
    for update in [
        Update::Phase("Fetching remotes".into()),
        Update::Output {
            stderr: true,
            text: "Receiving objects: 10% (1/10)\\rReceiving objects: ".replace("\\r", "\r"),
        },
        Update::Output {
            stderr: true,
            text: "100% (10/10), 24 KiB | 2 MiB/s, done.\r".into(),
        },
    ] {
        app.accept_feedback(Event { id, update });
    }
    let mut terminal = Terminal::new(TestBackend::new(140, 30)).unwrap();
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(text.contains("100% (10/10), 24 KiB"));
    app.accept_feedback(Event {
        id,
        update: Update::Phase("Creating worktree".into()),
    });
    app.accept_feedback(Event {
        id,
        update: Update::Output {
            stderr: true,
            text: "Preparing worktree\n".into(),
        },
    });
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(
        !text.contains("100%"),
        "A completed fetch percentage must not describe worktree creation"
    );
    app.accept(Completion {
        epoch: 0,
        token: token.clone(),
        result: Ok(Payload::Done("/code/app".into(), None, "Fetched".into())),
    });
    app.action("fetch");
    app.accept_feedback(Event {
        id,
        update: Update::Output {
            stderr: true,
            text: "stale output".into(),
        },
    });
    app.accept(Completion {
        epoch: 0,
        token,
        result: Err(anyhow::anyhow!("stale failure")),
    });
    assert!(app.busy);
    app.action("operation_details");
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(!text.contains("stale output"));
    assert!(!text.contains("stale failure"));
}

#[test]
fn details_bound_long_output_and_keep_navigation_usable_at_supported_sizes() {
    use treerail::feedback::{Event, Update};
    let mut app = app();
    app.action("fetch");
    let token = app.jobs.last().unwrap().token();
    let id = token.strip_prefix("operation:").unwrap().parse().unwrap();
    app.accept_feedback(Event {
        id,
        update: Update::Output {
            stderr: true,
            text: format!(
                "discarded beginning\n{}\nlatest output",
                "line of output\n".repeat(6000)
            ),
        },
    });
    app.action("pull");
    assert_eq!(
        app.jobs.last().unwrap().token(),
        token,
        "Only one mutation can be scheduled"
    );
    app.action("operation_details");
    app.key(key('G'));
    for (width, height) in [(140, 40), (80, 24), (30, 10), (8, 3)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| ui::draw(f, &app)).unwrap();
        if width >= 80 {
            let text = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>();
            assert!(text.contains("latest output"));
            assert!(text.contains("Earlier output discarded"));
            assert!(!text.contains("discarded beginning"));
        }
    }
    app.key(key('q'));
    assert!(app.modal.is_none());
    assert!(app.busy, "Closing details does not cancel Git");
}

#[test]
fn history_keeps_an_older_result_selected_while_a_new_operation_reports_progress() {
    use treerail::feedback::{Event, Update};
    let mut app = app();
    app.action("fetch");
    let first = app.jobs.last().unwrap().token();
    let first_id = first.strip_prefix("operation:").unwrap().parse().unwrap();
    app.accept_feedback(Event {
        id: first_id,
        update: Update::Output {
            stderr: true,
            text: "first operation output\n".into(),
        },
    });
    app.accept(Completion {
        epoch: 0,
        token: first,
        result: Ok(Payload::Done(
            "/code/app".into(),
            None,
            "Fetched first target".into(),
        )),
    });
    let mut second = app.trees[0].clone();
    second.path = "/code/app.worktrees/second".into();
    second.branch = "second".into();
    second.original = false;
    app.trees.push(second);
    app.key(key('j'));
    app.action("pull");
    let second = app.jobs.last().unwrap().token();
    app.accept(Completion {
        epoch: 0,
        token: second,
        result: Err(anyhow::anyhow!("Second target cannot fast-forward")),
    });
    app.key(key('H'));
    app.key(key('j'));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(text.contains("first operation output"), "{text}");
    app.operation(treerail::app::Operation::Push);
    let third = app.jobs.last().unwrap().token();
    let third_id = third.strip_prefix("operation:").unwrap().parse().unwrap();
    app.accept_feedback(Event {
        id: third_id,
        update: Update::Phase("Pushing to remote".into()),
    });
    app.accept_feedback(Event {
        id: third_id,
        update: Update::Output {
            stderr: true,
            text: "Writing objects: 43% (3/7)\rthird private output".into(),
        },
    });
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let buffer = &terminal.backend().buffer().content;
    let text = buffer.iter().map(|c| c.symbol()).collect::<String>();
    let header = buffer
        .iter()
        .take(120 * 4)
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(header.contains("Push"), "{header}");
    assert!(header.contains("Pushing to remote"), "{header}");
    assert!(header.contains("43%"), "{header}");
    assert!(text.contains("first operation output"));
    assert!(!text.contains("third private output"));
}

#[test]
fn history_limits_are_visible_and_the_inspected_result_survives_eviction() {
    use treerail::feedback::{Event, Update};
    let mut app = app();
    for number in 1..=25 {
        app.action("fetch");
        let token = app.jobs.last().unwrap().token();
        let id = token.strip_prefix("operation:").unwrap().parse().unwrap();
        app.accept_feedback(Event {
            id,
            update: Update::Output {
                stderr: false,
                text: format!("Distinct output {number}\n"),
            },
        });
        app.accept(Completion {
            epoch: 0,
            token,
            result: Ok(Payload::Done(
                "/code/app".into(),
                None,
                format!("Result {number}"),
            )),
        });
    }
    app.key(key('H'));
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(text.contains("20 retained · 5 older discarded"), "{text}");
    app.key(key('G'));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(
        text.contains("Distinct output 6"),
        "Oldest retained result must be operation 6: {text}"
    );
    for number in 26..=50 {
        app.operation(treerail::app::Operation::Fetch);
        let token = app.jobs.last().unwrap().token();
        app.accept(Completion {
            epoch: 0,
            token,
            result: Ok(Payload::Done(
                "/code/app".into(),
                None,
                format!("Result {number}"),
            )),
        });
    }
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(
        text.contains("Distinct output 6"),
        "Pinned inspection must remain stable: {text}"
    );
    app.key(key('H'));
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(text.contains("20 retained · 30 older discarded"));
    app.key(key('/'));
    app.paste("Result 6");
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(text.contains("Distinct output 6"));
}

#[test]
fn a_fresh_session_has_no_operation_history_and_config_never_contains_output() {
    use treerail::{
        config,
        feedback::{Event, Update},
    };
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    config::open(&path, true, None).unwrap();
    let mut app = app();
    app.config_path = path.clone();
    app.action("fetch");
    let token = app.jobs.last().unwrap().token();
    let id = token.strip_prefix("operation:").unwrap().parse().unwrap();
    app.accept_feedback(Event {
        id,
        update: Update::Output {
            stderr: true,
            text: "SESSION OUTPUT MUST NOT BE SAVED".into(),
        },
    });
    app.accept(Completion {
        epoch: 0,
        token,
        result: Ok(Payload::Done(
            "/code/app".into(),
            None,
            "PRIVATE RESULT".into(),
        )),
    });
    app.save().unwrap();
    let document = std::fs::read_to_string(&path).unwrap();
    assert!(!document.contains("SESSION OUTPUT"));
    assert!(!document.contains("PRIVATE RESULT"));
    let (config, state) = config::open(&path, true, None).unwrap();
    let mut fresh = App::new(config, state, path, vec![], false);
    fresh.key(key('H'));
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &fresh)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(text.contains("No Git operations yet"), "{text}");
    assert!(text.contains("0 retained"));
    assert!(!text.contains("PRIVATE RESULT"));
}

#[test]
fn long_platform_paths_do_not_hide_active_status_above_older_details() {
    use treerail::feedback::{Event, Update};
    let mut app = app();
    app.action("fetch");
    let token = app.jobs.last().unwrap().token();
    app.accept(Completion {
        epoch: 0,
        token,
        result: Ok(Payload::Done(
            "/code/app".into(),
            None,
            "Earlier fetch finished".into(),
        )),
    });
    app.key(key('i'));
    app.trees[0].path = PathBuf::from(format!(
        "C:/Users/runner/AppData/Local/Temp/{}/repository.worktrees/feature",
        "long-directory-name/".repeat(8)
    ));
    app.operation(treerail::app::Operation::Push);
    let token = app.jobs.last().unwrap().token();
    let id = token.strip_prefix("operation:").unwrap().parse().unwrap();
    for update in [
        Update::Destination("origin/main".into()),
        Update::Phase("Pushing to remote".into()),
        Update::Output {
            stderr: true,
            text: "Writing objects: 43% (3/7)\r".into(),
        },
    ] {
        app.accept_feedback(Event { id, update });
    }
    for (width, height) in [(140, 40), (80, 24), (30, 10), (8, 3)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
        if width >= 80 {
            let text = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>();
            let header = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .take(width as usize * 4)
                .map(|c| c.symbol())
                .collect::<String>();
            assert!(header.contains("Push → origin/main"), "{header}");
            assert!(header.contains("Pushing to remote"), "{header}");
            assert!(header.contains("0s"), "{header}");
            assert!(header.contains("43%"), "{header}");
            assert!(header.contains("repository.worktrees/feature"), "{header}");
            assert!(text.contains("Earlier fetch finished"), "{text}");
        }
    }
}

#[test]
fn a_failed_commit_does_not_replace_older_details_and_its_message_can_be_reopened() {
    let mut app = app();
    app.action("fetch");
    let token = app.jobs.last().unwrap().token();
    app.accept(Completion {
        epoch: 0,
        token,
        result: Ok(Payload::Done(
            "/code/app".into(),
            None,
            "Earlier output stays selected".into(),
        )),
    });
    app.key(key('c'));
    app.paste("Retain this commit subject");
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    let token = app.jobs.last().unwrap().token();
    app.key(key('H'));
    app.key(key('j'));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.accept(Completion {
        epoch: 0,
        token,
        result: Err(anyhow::anyhow!("Commit hook refused")),
    });
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(text.contains("Earlier output stays selected"), "{text}");
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.key(key('f'));
    let token = app.jobs.last().unwrap().token();
    app.accept(Completion {
        epoch: 0,
        token,
        result: Ok(Payload::Done(
            "/code/app".into(),
            None,
            "Unrelated fetch finished".into(),
        )),
    });
    app.key(key('c'));
    let Some(Modal::Form(form)) = app.modal else {
        panic!("Commit form missing")
    };
    assert_eq!(form.fields[0].value, "Retain this commit subject");
}

#[test]
fn changing_worktrees_does_not_offer_a_failed_commit_draft_for_the_new_target() {
    let mut app = app();
    app.key(key('c'));
    app.paste("Draft for original checkout");
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    let token = app.jobs.last().unwrap().token();
    let mut second = app.trees[0].clone();
    second.path = "/code/app.worktrees/other".into();
    second.branch = "other".into();
    app.trees.push(second);
    app.key(key('j'));
    app.accept(Completion {
        epoch: 0,
        token,
        result: Err(anyhow::anyhow!("Commit hook refused")),
    });
    assert!(
        app.modal.is_none(),
        "Failure in the original worktree must not reopen its form in another"
    );
    app.key(key('c'));
    let Some(Modal::Form(form)) = app.modal else {
        panic!("Commit form missing")
    };
    assert!(form.fields[0].value.is_empty());
}

#[test]
fn running_feedback_and_details_remain_accessible_while_editing_settings() {
    use treerail::feedback::{Event, Update};
    let mut app = app();
    app.action("fetch");
    let id = app
        .jobs
        .last()
        .unwrap()
        .token()
        .strip_prefix("operation:")
        .unwrap()
        .parse()
        .unwrap();
    app.action("settings");
    app.accept_feedback(Event {
        id,
        update: Update::Phase("Fetching remote objects".into()),
    });
    let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Fetching remote objects"));
    assert!(text.contains("Settings"));
    app.key(key('i'));
    assert!(matches!(app.modal, Some(Modal::OperationDetails)));
    app.key(key('q'));
    assert!(app.settings_open);
    assert!(app.busy);
    app.key(key('H'));
    assert!(
        matches!(&app.modal, Some(Modal::Picker(picker)) if matches!(picker.kind, treerail::app::PickerKind::Operations))
    );
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Fetching remote objects"));
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.settings_open);
}
