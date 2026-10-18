use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use treerail::{
    app::{App, Completion, Job, Payload},
    browser,
    config::{Config, State},
    git::Project,
    ticket_browser::{self as data, IssueCard, Page, RepositoryViews},
    workflow::{self, Issue, Progress, Task, TicketRecord},
};

fn app() -> App {
    let mut app = App::new(
        Config::default(),
        State::default(),
        "unused.toml".into(),
        vec![],
        false,
    );
    app.project = Some(Project {
        name: "repo".into(),
        path: "repo".into(),
    });
    app.browser.repository = "github.com/owner/repo".into();
    app.browser.request = 1;
    app.jobs.clear();
    app
}
fn issue(number: u64) -> Issue {
    Issue {
        number,
        title: format!("Ticket {number}"),
        body: format!("# Heading {number}\n- [ ] Work\n> rationale\n```rust\nfn main() {{}}\n```"),
        state: "OPEN".into(),
        url: format!("https://github.com/owner/repo/issues/{number}"),
    }
}
fn card(number: u64) -> IssueCard {
    let mut card = IssueCard::from(issue(number));
    card.labels = vec![data::Label {
        name: "backend 🦀".into(),
        color: "ffffff".into(),
    }];
    card.author = "person".into();
    card
}
fn page(numbers: &[u64], page: u32, more: bool) -> Page {
    Page {
        issues: numbers.iter().map(|number| card(*number)).collect(),
        page,
        has_more: more,
        total: Some(103),
        limited: false,
        fetched_at: 12,
        warning: None,
    }
}
fn event(app: &mut App, event: data::Event) {
    app.accept(Completion {
        epoch: app.epoch,
        token: "browser:test".into(),
        result: Ok(Payload::Workflow(workflow::Event::Browser(event))),
    });
}
fn loaded(app: &mut App, numbers: &[u64]) {
    event(
        app,
        data::Event::Loaded {
            repository: "github.com/owner/repo".into(),
            request: 1,
            views: RepositoryViews::default(),
            page: page(numbers, 1, true),
        },
    );
}
fn key(app: &mut App, ch: char) {
    app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
}
fn screen(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| treerail::ui::draw(frame, app))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}
fn record(number: u64, progress: Progress) -> TicketRecord {
    TicketRecord {
        repository: "owner/repo".into(),
        project: "repo".into(),
        issue: issue(number),
        fetched_at: 1,
        prepared_issue: None,
        reviewed_issue: None,
        github_error: None,
        last_attempt_at: None,
        tasks: vec![Task {
            id: "task".into(),
            title: "Implement".into(),
            progress,
        }],
        next_task_id: 2,
        selected_task: None,
        worktree: None,
        checkout_removed_at: None,
    }
}

#[test]
fn browsing_and_paging_are_passive_and_late_completions_cannot_steal_selection() {
    let mut app = app();
    loaded(&mut app, &[1, 2]);
    assert!(app.state.workflow.tickets.is_empty());
    assert!(app.ticket.is_none());
    assert_eq!(app.browser.selected, Some(1));
    let first_request = app.browser.preview_request;
    key(&mut app, 'j');
    assert_eq!(app.browser.selected, Some(2));
    assert!(app.jobs.iter().any(|job| matches!(
        job,
        Job::Workflow(
            _,
            _,
            workflow::Command::Browser(data::Command::Page { page: 2, .. })
        )
    )));
    event(
        &mut app,
        data::Event::Discussion {
            repository: "github.com/owner/repo".into(),
            request: first_request,
            number: 1,
            discussion: data::Discussion {
                card: card(1),
                comments: vec![],
                complete: true,
                fetched_at: 2,
                warning: None,
            },
        },
    );
    assert!(app.browser.discussion.is_none());
    event(
        &mut app,
        data::Event::Page {
            repository: "github.com/owner/repo".into(),
            request: 1,
            page: page(&[2, 101], 2, false),
        },
    );
    assert_eq!(app.browser.cards.len(), 3);
    assert_eq!(app.browser.cards[2].issue.number, 101);
    assert!(app.state.workflow.tickets.is_empty());
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.jobs.iter().any(|job| matches!(
        job,
        Job::Workflow(_, _, workflow::Command::Open { number: 2 })
    )));
}

#[test]
fn repository_and_query_revisions_reject_stale_pages() {
    let mut app = app();
    loaded(&mut app, &[1]);
    event(
        &mut app,
        data::Event::Page {
            repository: "github.com/other/repo".into(),
            request: 1,
            page: page(&[2], 1, false),
        },
    );
    event(
        &mut app,
        data::Event::Page {
            repository: "github.com/owner/repo".into(),
            request: 0,
            page: page(&[3], 1, false),
        },
    );
    assert_eq!(app.browser.cards[0].issue.number, 1);
}

#[test]
fn active_membership_excludes_history_and_projects_and_keeps_closed_manual_work() {
    let mut state = workflow::WorkflowState::default();
    let mut closed = record(1, Progress::InProgress);
    closed.issue.state = "CLOSED".into();
    state.tickets.insert(closed.id(), closed);
    let idle = record(2, Progress::Done);
    state.tickets.insert(idle.id(), idle);
    let mut other = record(3, Progress::InProgress);
    other.repository = "other/repo".into();
    state.tickets.insert(other.id(), other);
    let active = browser::active_tickets(&state, "github.com/owner/repo", &[]);
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].ticket.issue.number, 1);
    state.tickets.get_mut("owner/repo#1").unwrap().tasks[0].progress = Progress::Done;
    assert!(browser::active_tickets(&state, "owner/repo", &[]).is_empty());
}

#[test]
fn ready_checkout_owners_and_recovery_count_but_historical_failures_do_not() {
    let mut state = workflow::WorkflowState::default();
    for number in 1..=3 {
        let ticket = record(number, Progress::Pending);
        state.tickets.insert(ticket.id(), ticket);
    }
    for (number, health, current) in [
        (1, "Live", true),
        (2, "Failed", false),
        (3, "Interrupted", true),
    ] {
        let session=serde_json::from_value(serde_json::json!({"id":format!("s{number}"),"ticket":format!("owner/repo#{number}"),"task":"task","checkout":"repo","provider_session_id":"provider","starting_brief":"brief","created_at":1,"host_session":format!("h{number}"),"health":health,"current":current,"error":null})).unwrap();
        state.sessions.insert(format!("s{number}"), session);
    }
    let attention = treerail::copilot::Attention {
        value: Some(treerail::copilot::AttentionState::Ready),
        healthy: true,
        ..Default::default()
    };
    let summaries = vec![treerail::host::runtime::SessionSummary {
        session: "h1".into(),
        provider_session_id: None,
        attention,
        exited: false,
        notification_error: None,
    }];
    let active = browser::active_tickets(&state, "owner/repo", &summaries);
    assert_eq!(
        active
            .iter()
            .map(|active| active.ticket.issue.number)
            .collect::<Vec<_>>(),
        vec![3, 1]
    );
    assert!(active[1].attention.contains("Ready"));
    let mut unhealthy = summaries.clone();
    unhealthy[0].attention.healthy = false;
    let unavailable = browser::active_tickets(&state, "owner/repo", &unhealthy);
    assert!(
        unavailable
            .iter()
            .any(|entry| entry.ticket.issue.number == 1
                && entry.rank == 0
                && entry.attention.contains("unavailable"))
    );
    let stopped = state.sessions.get_mut("s2").unwrap();
    stopped.health = treerail::hosted::Health::Stopped;
    stopped.current = true;
    stopped.start_intent = Some(treerail::hosted::StartIntent {
        handoff: "prior launch context".into(),
        rows: 24,
        cols: 80,
    });
    assert_eq!(
        browser::active_tickets(&state, "owner/repo", &summaries).len(),
        2
    );
}

#[test]
fn view_mutations_are_serialized_and_old_revisions_cannot_undo_new_selection() {
    let mut app = app();
    loaded(&mut app, &[1]);
    app.browser.views.views.push(data::SavedView {
        id: "backend".into(),
        name: "Backend".into(),
        filter: data::Filter::default(),
    });
    key(&mut app, 'v');
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    key(&mut app, 'v');
    key(&mut app, 'j');
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.browser.edit_queue.len(), 1);
    assert_eq!(
        app.jobs
            .iter()
            .filter(|job| matches!(
                job,
                Job::Workflow(
                    _,
                    _,
                    workflow::Command::Browser(data::Command::SelectView { .. })
                )
            ))
            .count(),
        1
    );
    let mut first = app.browser.views.clone();
    first.revision = 1;
    event(
        &mut app,
        data::Event::Views {
            repository: "github.com/owner/repo".into(),
            views: first.clone(),
        },
    );
    assert!(app.browser.edit_queue.is_empty());
    let mut second = first.clone();
    second.revision = 2;
    second.active = "backend".into();
    event(
        &mut app,
        data::Event::Views {
            repository: "github.com/owner/repo".into(),
            views: second,
        },
    );
    event(
        &mut app,
        data::Event::Views {
            repository: "github.com/owner/repo".into(),
            views: first,
        },
    );
    assert_eq!(app.browser.views.active, "backend");
}

#[test]
fn refresh_reloads_all_previously_loaded_pages_and_preserves_reading_position() {
    let mut app = app();
    loaded(&mut app, &[1]);
    event(
        &mut app,
        data::Event::Page {
            repository: "github.com/owner/repo".into(),
            request: 1,
            page: page(&[2], 2, false),
        },
    );
    app.browser.selected = Some(2);
    app.browser.preview_scroll = 6;
    key(&mut app, 'R');
    let request = app.browser.request;
    event(
        &mut app,
        data::Event::Page {
            repository: "github.com/owner/repo".into(),
            request,
            page: page(&[1], 1, true),
        },
    );
    assert!(app.browser.loading);
    assert!(app.jobs.iter().any(|job|matches!(job,Job::Workflow(_,_,workflow::Command::Browser(data::Command::Page{request:r,page:2,..})) if *r==request)));
    event(
        &mut app,
        data::Event::Page {
            repository: "github.com/owner/repo".into(),
            request,
            page: page(&[2], 2, false),
        },
    );
    assert_eq!(app.browser.cards.len(), 2);
    assert_eq!(app.browser.selected, Some(2));
    assert_eq!(app.browser.preview_scroll, 6);
}

#[test]
fn active_deduplicates_query_and_remains_reachable_offline() {
    let mut app = app();
    let ticket = record(1, Progress::InProgress);
    app.state.workflow.tickets.insert(ticket.id(), ticket);
    loaded(&mut app, &[1, 2]);
    assert_eq!(app.browser_numbers(), vec![1, 2]);
    app.browser.cards.clear();
    app.browser.warning = "offline".into();
    app.browser.selected = Some(1);
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.jobs.iter().any(|job|matches!(job,Job::Workflow(_,_,workflow::Command::Reopen{ticket}) if ticket=="owner/repo#1")));
    let rendered = screen(&app, 120, 30);
    assert!(rendered.contains("Active: 1"));
    assert!(rendered.contains("unavailable"));
}

#[test]
fn focused_pane_narrow_layout_markdown_and_overflow_are_readable() {
    let mut app = app();
    loaded(&mut app, &[1]);
    app.browser.cards[0].labels[0].name = "后端".repeat(40);
    let rendered = screen(&app, 70, 30);
    assert!(rendered.contains('…'));
    assert!(!rendered.contains("Ticket preview"));
    let request = app.browser.preview_request;
    event(
        &mut app,
        data::Event::Discussion {
            repository: "github.com/owner/repo".into(),
            request,
            number: 1,
            discussion: data::Discussion {
                card: card(1),
                comments: vec![data::Comment {
                    id: 4,
                    author: "reviewer".into(),
                    body: "Looks good".into(),
                    created_at: "today".into(),
                    updated_at: "tomorrow".into(),
                    url: String::new(),
                }],
                complete: true,
                fetched_at: 10,
                warning: None,
            },
        },
    );
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    let rendered = screen(&app, 70, 40);
    assert!(rendered.contains("Ticket preview"));
    assert!(rendered.contains("Heading 1"));
    assert!(rendered.contains("☐ Work"));
    assert!(rendered.contains("@reviewer"));
    assert!(rendered.contains("edited"));
    app.browser.preview_scroll = u16::MAX;
    assert!(screen(&app, 70, 40).contains("Looks good"));
}

#[test]
fn filter_typing_owns_letters_paste_and_cancel_without_saving() {
    let mut app = app();
    loaded(&mut app, &[1]);
    key(&mut app, 'f');
    let jobs = app.jobs.len();
    key(&mut app, 'E');
    app.paste("label\ntext");
    assert!(app.hosted.dialog.is_none());
    assert_eq!(app.jobs.len(), jobs);
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.browser.editor.is_none());
    assert_eq!(app.browser.filter(), data::Filter::default());
}

#[test]
fn background_refresh_retains_preview_but_explicit_filter_switch_reselects() {
    let mut app = app();
    loaded(&mut app, &[1]);
    app.browser.preview_scroll = 4;
    event(
        &mut app,
        data::Event::Page {
            repository: "github.com/owner/repo".into(),
            request: 1,
            page: page(&[2], 1, false),
        },
    );
    assert_eq!(app.browser.selected, Some(1));
    assert_eq!(app.browser.preview_scroll, 4);
    let mut views = app.browser.views.clone();
    views.views[0].filter.state = data::IssueState::Closed;
    event(
        &mut app,
        data::Event::Views {
            repository: "github.com/owner/repo".into(),
            views,
        },
    );
    let request = app.browser.request;
    event(
        &mut app,
        data::Event::Page {
            repository: "github.com/owner/repo".into(),
            request,
            page: page(&[2], 1, false),
        },
    );
    assert_eq!(app.browser.selected, Some(2));
}

#[test]
fn bounded_active_column_leaves_queue_visible_and_collapse_is_persisted() {
    let mut app = app();
    for number in 1..=20 {
        let ticket = record(number, Progress::InProgress);
        app.state.workflow.tickets.insert(ticket.id(), ticket);
    }
    loaded(&mut app, &[100]);
    let rendered = screen(&app, 120, 24);
    assert!(rendered.contains("Active: 20"));
    assert!(rendered.contains("Ticket 100"));
    key(&mut app, 'a');
    assert_eq!(app.browser_numbers(), vec![100]);
    assert!(app.jobs.iter().any(|job| matches!(
        job,
        Job::Workflow(
            _,
            _,
            workflow::Command::Browser(data::Command::SetActiveCollapsed {
                collapsed: true,
                ..
            })
        )
    )));
}
