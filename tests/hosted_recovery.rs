#![cfg(windows)]
//! Real host/ConPTY failure injection; the child is cmd.exe, never Copilot.
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
use treerail::{
    host::{
        execution::execution_group_stopped,
        runtime::{Client, HostCommand, Response},
    },
    hosted, workflow,
};
struct Owner(std::process::Child);
impl Drop for Owner {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn launch() -> (Owner, Client, String) {
    let endpoint = format!("ticket-test-{}", treerail::copilot::new_session_id());
    let owner = Owner(
        Client::launch(
            Path::new(env!("CARGO_BIN_EXE_treerail-session-host")),
            &endpoint,
        )
        .unwrap(),
    );
    let client = Client::new(endpoint);
    let deadline = Instant::now() + Duration::from_secs(10);
    let instance = loop {
        match client.execute(HostCommand::Ping) {
            Ok(Response::Pong { instance, .. }) => break instance,
            _ => {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    };
    (owner, client, instance)
}
struct Fixture {
    temp: tempfile::TempDir,
    path: PathBuf,
    record: hosted::SessionRecord,
}
impl Fixture {
    fn new(client: &Client, instance: String) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let checkout = temp.path().join("checkout");
        std::fs::create_dir(&checkout).unwrap();
        assert!(
            Command::new("git")
                .current_dir(&checkout)
                .args(["init", "--initial-branch=feature"])
                .output()
                .unwrap()
                .status
                .success()
        );
        let request_id = treerail::copilot::new_session_id();
        let Response::Started { session, .. } = client
            .execute(HostCommand::Start {
                request_id: request_id.clone(),
                checkout: checkout.clone(),
                program: "cmd.exe".into(),
                args: vec!["/D".into(), "/Q".into()],
                rows: 24,
                cols: 80,
            })
            .unwrap()
        else {
            panic!()
        };
        let Response::Terminal(terminal) = client
            .execute(HostCommand::Snapshot {
                session: session.clone(),
            })
            .unwrap()
        else {
            panic!()
        };
        let group = terminal.execution_group.unwrap();
        assert!(!execution_group_stopped(&group).unwrap());
        // Only execution lifecycle is under test. No fixture pretends to observe native attention.
        let record:hosted::SessionRecord=serde_json::from_value(serde_json::json!({
            "id":request_id,"ticket":"owner/repo#1","task":"implementation","checkout":checkout,
            "provider_session_id":treerail::copilot::new_session_id(),"starting_brief":"Retained original brief",
            "created_at":1,"host_session":session,"health":"Live","current":true,"error":null,
            "host_instance":instance,"branch":"feature","execution_group":group
        })).unwrap();
        let path = temp.path().join("treerail.toml");
        workflow::Workflow::new(&path).update(|state| {
            let ticket:workflow::TicketRecord=serde_json::from_value(serde_json::json!({"repository":"owner/repo","project":checkout,"issue":{"number":1,"title":"Deliver","body":"Acceptance criteria","state":"OPEN","url":"https://github.com/owner/repo/issues/1"},"fetched_at":1,"tasks":[{"id":"implementation","title":"Implement","progress":"in_progress"}],"next_task_id":1,"selected_task":"implementation","worktree":{"path":checkout,"branch":"feature","original":false}}))?;
            state.tickets.insert(record.ticket.clone(),ticket);state.sessions.insert(record.id.clone(),record.clone());Ok(())
        }).unwrap();
        Self { temp, path, record }
    }
    fn ticket(&self, client: &Client, command: hosted::Command) -> anyhow::Result<hosted::Event> {
        match client.execute(HostCommand::Ticket {
            config_path: self.path.clone(),
            command,
        })? {
            Response::Ticket(event) => Ok(*event),
            value => anyhow::bail!("Unexpected ticket event: {value:?}"),
        }
    }
    fn record(&self) -> hosted::SessionRecord {
        workflow::Workflow::new(&self.path)
            .snapshot()
            .unwrap()
            .sessions[&self.record.id]
            .clone()
    }
}
#[test]
fn another_host_cannot_reconcile_or_stop_a_live_owner() {
    let (_owner, client, instance) = launch();
    let fixture = Fixture::new(&client, instance);
    let (_other, other, _) = launch();
    fixture.ticket(&other, hosted::Command::Snapshot).unwrap();
    assert_eq!(fixture.record().health, hosted::Health::Interrupted);
    assert!(
        fixture
            .ticket(
                &other,
                hosted::Command::Reconcile {
                    session: fixture.record.id.clone()
                }
            )
            .is_err()
    );
    assert!(
        fixture
            .ticket(
                &other,
                hosted::Command::Stop {
                    session: fixture.record.id.clone()
                }
            )
            .is_err()
    );
    assert!(!execution_group_stopped(fixture.record.execution_group.as_ref().unwrap()).unwrap());
    let Response::Terminal(terminal) = client
        .execute(HostCommand::Snapshot {
            session: fixture.record.host_session.clone().unwrap(),
        })
        .unwrap()
    else {
        panic!()
    };
    assert!(!terminal.exited);
    assert!(matches!(
        fixture
            .ticket(
                &client,
                hosted::Command::Stop {
                    session: fixture.record.id.clone()
                }
            )
            .unwrap(),
        hosted::Event::Stopped(_)
    ));
    assert!(execution_group_stopped(fixture.record.execution_group.as_ref().unwrap()).unwrap());
    assert_eq!(
        fixture.record().starting_brief,
        fixture.record.starting_brief
    );
    assert_eq!(fixture.record().health, hosted::Health::Stopped);
    other.execute(HostCommand::StopHost).unwrap();
    client.execute(HostCommand::StopHost).unwrap();
}
#[test]
fn host_crash_keeps_records_interrupted_until_named_group_is_gone_and_reconcile_is_explicit() {
    let (mut owner, client, instance) = launch();
    let fixture = Fixture::new(&client, instance);
    owner.0.kill().unwrap();
    owner.0.wait().unwrap();
    let (_replacement, replacement, _) = launch();
    fixture
        .ticket(&replacement, hosted::Command::Snapshot)
        .unwrap();
    assert_eq!(fixture.record().health, hosted::Health::Interrupted);
    assert!(
        matches!(replacement.execute(HostCommand::Summary).unwrap(),Response::Summary(values) if values.is_empty())
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while !execution_group_stopped(fixture.record.execution_group.as_ref().unwrap()).unwrap() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let event = fixture
        .ticket(
            &replacement,
            hosted::Command::Reconcile {
                session: fixture.record.id.clone(),
            },
        )
        .unwrap();
    assert!(matches!(event, hosted::Event::Stopped(_)));
    assert_eq!(fixture.record().health, hosted::Health::Stopped);
    // No saved provider record was created by the controlled child. Explicit Continue must fail,
    // rather than launch a new native conversation or replay the retained brief.
    let error = fixture
        .ticket(
            &replacement,
            hosted::Command::Continue {
                session: fixture.record.id.clone(),
                rows: 24,
                cols: 80,
            },
        )
        .unwrap_err();
    assert!(format!("{error:#}").contains("missing"));
    assert!(
        matches!(replacement.execute(HostCommand::Summary).unwrap(),Response::Summary(values) if values.is_empty())
    );
    let record = fixture.record();
    assert_eq!(
        record.provider_session_id,
        fixture.record.provider_session_id
    );
    assert_eq!(record.starting_brief, fixture.record.starting_brief);
    assert_eq!(record.checkout, fixture.record.checkout);
    assert!(fixture.temp.path().is_dir());
    replacement.execute(HostCommand::StopHost).unwrap();
}
