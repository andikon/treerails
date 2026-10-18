use std::{
    fs,
    path::{Path, PathBuf},
};
use tempfile::TempDir;
use treerail::{
    config::{self, Config},
    git,
    workflow::{self, Command, Event, Issue, IssueSource, Preparation, Progress, Workflow},
};
struct Source;
impl IssueSource for Source {
    async fn repository(&self, _: &Path) -> anyhow::Result<String> {
        Ok("owner/project".into())
    }
    async fn list(&self, _: &Path) -> anyhow::Result<Vec<Issue>> {
        Ok(vec![issue(1)])
    }
    async fn get(&self, _: &Path, number: u64) -> anyhow::Result<Issue> {
        Ok(issue(number))
    }
}
fn issue(number: u64) -> Issue {
    Issue {
        number,
        title: format!("Deliver {number}"),
        body:
            "## Acceptance criteria\n- [ ] Never a task\n## Tasks\n- [ ] Build it\n- [x] Review it"
                .into(),
        state: "OPEN".into(),
        url: format!("https://github.com/owner/project/issues/{number}"),
    }
}
async fn repo(temp: &TempDir) -> PathBuf {
    let project = temp.path().join("project");
    fs::create_dir(&project).unwrap();
    for args in [
        vec!["init", "--initial-branch=main"],
        vec!["config", "user.name", "Workflow Test"],
        vec!["config", "user.email", "test@example.invalid"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["commit", "--allow-empty", "-m", "initial"],
    ] {
        git::run(&project, args).await.unwrap();
    }
    project
}
fn ticket(event: Event) -> workflow::TicketRecord {
    let Event::Ticket(ticket) = event else {
        panic!("ticket expected")
    };
    ticket
}
#[tokio::test]
async fn project_issue_task_create_and_reopen_use_one_checkout() {
    let temp = TempDir::new().unwrap();
    let project = repo(&temp).await;
    let config = temp.path().join("config.toml");
    let workflow = Workflow::new(&config);
    assert!(matches!(
        workflow
            .execute_with(&project, Command::List, &Source)
            .await
            .unwrap(),
        Event::Issues(_)
    ));
    assert!(!config.exists(), "Listing is read-only");
    let record = ticket(
        workflow
            .execute_with(&project, Command::Open { number: 1 }, &Source)
            .await
            .unwrap(),
    );
    assert_eq!(record.tasks.len(), 1);
    assert!(record.worktree.is_none());
    assert_eq!(git::worktrees(&project).await.unwrap().len(), 1);
    let id = record.id();
    let candidates = workflow::import_candidates(&record.issue.body);
    assert_eq!(candidates, vec!["Build it", "Review it"]);
    let record = ticket(
        workflow
            .execute_with(
                &project,
                Command::ImportTasks {
                    ticket: id.clone(),
                    titles: candidates.clone(),
                },
                &Source,
            )
            .await
            .unwrap(),
    );
    let build = record.tasks[1].id.clone();
    workflow
        .execute_with(
            &project,
            Command::ImportTasks {
                ticket: id.clone(),
                titles: candidates,
            },
            &Source,
        )
        .await
        .unwrap();
    workflow
        .execute_with(
            &project,
            Command::SelectTask {
                ticket: id.clone(),
                task: build.clone(),
            },
            &Source,
        )
        .await
        .unwrap();
    workflow
        .execute_with(
            &project,
            Command::SetProgress {
                ticket: id.clone(),
                task: build.clone(),
                progress: Progress::InProgress,
            },
            &Source,
        )
        .await
        .unwrap();
    let destination = temp.path().join("ticket");
    let record = ticket(
        workflow
            .execute_with(
                &project,
                Command::Prepare {
                    ticket: id.clone(),
                    preparation: Preparation::Create {
                        branch: "ticket/1".into(),
                        base: "main".into(),
                        destination: destination.clone(),
                        fetch: false,
                    },
                },
                &Source,
            )
            .await
            .unwrap(),
    );
    assert_eq!(record.tasks.len(), 3);
    assert_eq!(record.selected_task, Some(build));
    assert_eq!(record.tasks[1].progress, Progress::InProgress);
    assert!(!record.worktree.as_ref().unwrap().original);
    let reopened = Workflow::new(&config);
    let same = ticket(
        reopened
            .execute_with(&project, Command::Reopen { ticket: id.clone() }, &Source)
            .await
            .unwrap(),
    );
    assert_eq!(record, same);
    let same = ticket(
        reopened
            .execute_with(
                &project,
                Command::Prepare {
                    ticket: id,
                    preparation: Preparation::Adopt {
                        path: project.clone(),
                    },
                },
                &Source,
            )
            .await
            .unwrap(),
    );
    assert_eq!(same.worktree, record.worktree);
    assert_eq!(git::worktrees(&project).await.unwrap().len(), 2);
    assert!(workflow::claim_path(&destination).await.unwrap().is_file());
}
#[tokio::test]
async fn adoption_rejects_conflicts_across_configurations_and_path_aliases() {
    let temp = TempDir::new().unwrap();
    let project = repo(&temp).await;
    let first = Workflow::new(temp.path().join("one.toml"));
    let second = Workflow::new(temp.path().join("two.toml"));
    let one = ticket(
        first
            .execute_with(&project, Command::Open { number: 1 }, &Source)
            .await
            .unwrap(),
    );
    let adopted = ticket(
        first
            .execute_with(
                &project,
                Command::Prepare {
                    ticket: one.id(),
                    preparation: Preparation::Adopt {
                        path: project.join("."),
                    },
                },
                &Source,
            )
            .await
            .unwrap(),
    );
    assert!(adopted.worktree.unwrap().original);
    let two = ticket(
        second
            .execute_with(&project, Command::Open { number: 2 }, &Source)
            .await
            .unwrap(),
    );
    assert!(
        second
            .execute_with(
                &project,
                Command::Prepare {
                    ticket: two.id(),
                    preparation: Preparation::Adopt {
                        path: project.clone()
                    }
                },
                &Source
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("already belongs")
    );
    assert!(
        ticket(
            second
                .execute_with(&project, Command::Reopen { ticket: two.id() }, &Source)
                .await
                .unwrap()
        )
        .worktree
        .is_none()
    );
}
#[tokio::test]
async fn preparation_refuses_occupied_paths_and_reuses_checked_out_branches() {
    let temp = TempDir::new().unwrap();
    let project = repo(&temp).await;
    let workflow = Workflow::new(temp.path().join("config.toml"));
    let record = ticket(
        workflow
            .execute_with(&project, Command::Open { number: 1 }, &Source)
            .await
            .unwrap(),
    );
    let id = record.id();
    let occupied = temp.path().join("occupied");
    fs::create_dir(&occupied).unwrap();
    fs::write(occupied.join("precious"), "keep").unwrap();
    assert!(
        workflow
            .execute_with(
                &project,
                Command::Prepare {
                    ticket: id.clone(),
                    preparation: Preparation::Create {
                        branch: "ticket/1".into(),
                        base: "main".into(),
                        destination: occupied.clone(),
                        fetch: false
                    }
                },
                &Source
            )
            .await
            .is_err()
    );
    assert_eq!(
        fs::read_to_string(occupied.join("precious")).unwrap(),
        "keep"
    );
    let record = ticket(
        workflow
            .execute_with(
                &project,
                Command::Prepare {
                    ticket: id,
                    preparation: Preparation::Create {
                        branch: "main".into(),
                        base: "main".into(),
                        destination: temp.path().join("unused"),
                        fetch: false,
                    },
                },
                &Source,
            )
            .await
            .unwrap(),
    );
    assert!(record.worktree.unwrap().original);
    assert!(!temp.path().join("unused").exists());
}
#[tokio::test]
async fn updates_preserve_comments_settings_other_tasks_and_malformed_input() {
    let temp = TempDir::new().unwrap();
    let project = repo(&temp).await;
    let path = temp.path().join("config.toml");
    fs::write(&path, "# user notes\nrecursive = true # keep preference\n").unwrap();
    let a = Workflow::new(&path);
    let b = Workflow::new(&path);
    let record = ticket(
        a.execute_with(&project, Command::Open { number: 1 }, &Source)
            .await
            .unwrap(),
    );
    let id = record.id();
    let text = fs::read_to_string(&path).unwrap().replace(
        "progress = \"pending\"",
        "progress = \"pending\" # task comment",
    );
    fs::write(&path, text).unwrap();
    b.execute_with(
        &project,
        Command::ImportTasks {
            ticket: id.clone(),
            titles: vec!["Second".into()],
        },
        &Source,
    )
    .await
    .unwrap();
    a.execute_with(
        &project,
        Command::SetProgress {
            ticket: id.clone(),
            task: "task-1".into(),
            progress: Progress::Done,
        },
        &Source,
    )
    .await
    .unwrap();
    let saved = fs::read_to_string(&path).unwrap();
    assert!(saved.contains("# task comment"));
    assert!(saved.contains("# user notes"));
    assert!(saved.contains("# keep preference"));
    assert_eq!(a.snapshot().unwrap().tickets[&id].tasks.len(), 2);
    assert!(Config::load(&path).unwrap().recursive);
    let (_, mut state) = config::open(&path, false, None).unwrap();
    state.last_project = Some(project.clone());
    state.save(&path).unwrap();
    assert_eq!(a.snapshot().unwrap().tickets[&id].tasks.len(), 2);
    fs::write(&path, "recursive = [oops").unwrap();
    let bad = fs::read(&path).unwrap();
    assert!(
        a.execute_with(
            &project,
            Command::SetProgress {
                ticket: id,
                task: "task-1".into(),
                progress: Progress::Done
            },
            &Source
        )
        .await
        .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), bad);
}
#[test]
fn task_import_ignores_acceptance_criteria_prose_and_code_fences() {
    assert_eq!(
        workflow::import_candidates(
            "## Acceptance criteria\n- [ ] no\n## Tasks\n- [ ] yes\n```\n- [ ] no code\n```\n### Detail\n* [X] also\n## Other\n- [ ] no"
        ),
        vec!["yes", "also"]
    );
}
#[tokio::test]
async fn terminal_default_browsing_and_basic_escape_share_production_events() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};
    use treerail::{
        app::{App, Completion, Job, Payload},
        ui,
    };
    let temp = TempDir::new().unwrap();
    let project = repo(&temp).await;
    let path = temp.path().join("config.toml");
    let (config, state) = config::open(&path, true, None).unwrap();
    let mut app = App::new(config, state, path.clone(), vec![], false);
    app.projects.push(git::project(&project).await.unwrap());
    app.jobs.clear();
    app.open_project();
    assert!(app.ticket_mode);
    assert!(
        app.jobs
            .iter()
            .any(|job| matches!(job, Job::Workflow(_, _, Command::List)))
    );
    app.accept(Completion {
        epoch: app.epoch,
        token: "workflow".into(),
        result: Ok(Payload::Workflow(Event::Issues(vec![issue(1)]))),
    });
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(
        app.jobs.last().unwrap(),
        Job::Workflow(_, _, Command::Open { number: 1 })
    ));
    let record = Workflow::new(&path)
        .execute_with(&project, Command::Open { number: 1 }, &Source)
        .await
        .unwrap();
    app.accept(Completion {
        epoch: app.epoch,
        token: "workflow".into(),
        result: Ok(Payload::Workflow(record)),
    });
    let mut terminal = Terminal::new(TestBackend::new(120, 35)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Ticket overview"));
    assert!(text.contains("Acceptance criteria"));
    assert!(text.contains("No shared worktree"));
    app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
    assert!(!app.ticket_mode);
    app.key(KeyEvent::new(KeyCode::Char('T'), KeyModifiers::NONE));
    assert!(app.ticket_mode);
    assert_eq!(git::worktrees(&project).await.unwrap().len(), 1);
}
#[tokio::test]
async fn simultaneous_adoption_has_one_physical_winner() {
    let temp = TempDir::new().unwrap();
    let project = repo(&temp).await;
    let one = Workflow::new(temp.path().join("one.toml"));
    let two = Workflow::new(temp.path().join("two.toml"));
    let first = ticket(
        one.execute_with(&project, Command::Open { number: 1 }, &Source)
            .await
            .unwrap(),
    );
    let second = ticket(
        two.execute_with(&project, Command::Open { number: 2 }, &Source)
            .await
            .unwrap(),
    );
    let (a, b) = tokio::join!(
        one.execute_with(
            &project,
            Command::Prepare {
                ticket: first.id(),
                preparation: Preparation::Adopt {
                    path: project.clone()
                }
            },
            &Source
        ),
        two.execute_with(
            &project,
            Command::Prepare {
                ticket: second.id(),
                preparation: Preparation::Adopt {
                    path: project.clone()
                }
            },
            &Source
        )
    );
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(
        one.snapshot()
            .unwrap()
            .tickets
            .values()
            .filter(|ticket| ticket.worktree.is_some())
            .count()
            + two
                .snapshot()
                .unwrap()
                .tickets
                .values()
                .filter(|ticket| ticket.worktree.is_some())
                .count(),
        1
    );
}
