#![cfg(windows)]
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
use treerail::{
    host::runtime::{Client, HostCommand, Response},
    hosted, workflow,
};

struct Source;
impl workflow::IssueSource for Source {
    async fn repository(&self, _: &Path) -> anyhow::Result<String> {
        Ok("owner/repo".into())
    }
    async fn list(&self, _: &Path) -> anyhow::Result<Vec<workflow::Issue>> {
        Ok(vec![])
    }
    async fn get(&self, _: &Path, number: u64) -> anyhow::Result<workflow::Issue> {
        Ok(workflow::Issue {
            number,
            title: "Prepare work".into(),
            body: "A simple task".into(),
            state: "OPEN".into(),
            url: format!("https://github.com/owner/repo/issues/{number}"),
        })
    }
}
fn git(path: &Path, args: &[&str]) {
    let result = Command::new("git")
        .current_dir(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
struct Host(std::process::Child);
impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Gate(PathBuf);
impl Drop for Gate {
    fn drop(&mut self) {
        let _ = std::fs::write(&self.0, "release");
    }
}

#[tokio::test]
async fn a_waiting_git_hook_does_not_starve_live_host_commands() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    for args in [
        &["init", "--initial-branch=main"][..],
        &["config", "user.name", "Host test"],
        &["config", "user.email", "test@example.invalid"],
        &["config", "commit.gpgsign", "false"],
        &["commit", "--allow-empty", "-m", "initial"],
    ] {
        git(&project, args);
    }
    let config = temp.path().join("config.toml");
    let flow = workflow::Workflow::new(&config);
    flow.execute_with(&project, workflow::Command::Open { number: 1 }, &Source)
        .await
        .unwrap();
    let entered = temp.path().join("hook-entered");
    let release = temp.path().join("hook-release");
    let script = format!(
        "#!/bin/sh\nprintf ready > '{}'\nwhile [ ! -f '{}' ]; do sleep 0.05; done\n",
        entered.to_string_lossy().replace('\\', "/"),
        release.to_string_lossy().replace('\\', "/")
    );
    std::fs::write(project.join(".git/hooks/post-checkout"), script).unwrap();
    let endpoint = format!("workflow-{}", uuid::Uuid::new_v4());
    let _host = Host(
        Client::launch(
            Path::new(env!("CARGO_BIN_EXE_treerail-session-host")),
            &endpoint,
        )
        .unwrap(),
    );
    let client = Client::new(&endpoint);
    let deadline = Instant::now() + Duration::from_secs(10);
    while client.execute(HostCommand::Ping).is_err() {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let Response::Started { session, .. } = client
        .execute(HostCommand::Start {
            request_id: uuid::Uuid::new_v4().to_string(),
            checkout: project.clone(),
            program: "cmd.exe".into(),
            args: vec!["/D".into(), "/Q".into()],
            rows: 20,
            cols: 80,
        })
        .unwrap()
    else {
        panic!("Child did not start")
    };
    let destination = temp.path().join("linked");
    let requested_destination = destination.clone();
    let task_endpoint = endpoint.clone();
    let task = std::thread::spawn(move || {
        Client::new(task_endpoint).execute(HostCommand::Ticket {
            config_path: config,
            command: hosted::Command::Workflow {
                project,
                command: workflow::Command::Prepare {
                    ticket: "owner/repo#1".into(),
                    preparation: workflow::Preparation::Create {
                        branch: "feature".into(),
                        base: "main".into(),
                        destination: requested_destination,
                        fetch: false,
                    },
                },
            },
        })
    });
    let gate = Gate(release);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !entered.exists() {
        assert!(
            !task.is_finished(),
            "Preparation ended before entering the hook"
        );
        assert!(Instant::now() < deadline, "Hook did not start");
        std::thread::yield_now();
    }
    assert!(matches!(
        client.execute(HostCommand::Ping).unwrap(),
        Response::Pong { .. }
    ));
    assert!(matches!(
        client
            .execute(HostCommand::Snapshot {
                session: session.clone()
            })
            .unwrap(),
        Response::Terminal(_)
    ));
    assert!(
        !task.is_finished(),
        "Git preparation is still gated independently of host access"
    );
    drop(gate);
    assert!(matches!(task.join().unwrap().unwrap(), Response::Ticket(_)));
    assert!(destination.is_dir());
    client.execute(HostCommand::Stop { session }).unwrap();
    client.execute(HostCommand::StopHost).unwrap();
}
