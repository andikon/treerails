#![cfg(windows)]
use std::{
    path::Path,
    process::Command,
    time::{Duration, Instant},
};
use treerail::host::runtime::{Client, HostCommand, Response};

struct Host(std::process::Child);
impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn a_single_downloaded_executable_hosts_managed_children_without_a_companion() {
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("treerail.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_treerail"), &executable).unwrap();
    assert!(!temp.path().join("treerail-session-host.exe").exists());
    assert!(
        Command::new("git")
            .args(["init"])
            .current_dir(temp.path())
            .output()
            .unwrap()
            .status
            .success()
    );
    let endpoint = format!("standalone-{}", uuid::Uuid::new_v4());
    let mut host = Host(Client::launch(Path::new(&executable), &endpoint).unwrap());
    let client = Client::new(&endpoint);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match client.execute(HostCommand::Ping) {
            Ok(Response::Pong { .. }) => break,
            result => {
                assert!(
                    host.0.try_wait().unwrap().is_none(),
                    "Standalone host exited: {result:?}"
                );
                assert!(
                    Instant::now() < deadline,
                    "Standalone host did not start: {result:?}"
                );
                std::thread::yield_now();
            }
        }
    }
    let Response::Started { session, .. } = client
        .execute(HostCommand::Start {
            request_id: uuid::Uuid::new_v4().to_string(),
            checkout: temp.path().into(),
            program: "cmd.exe".into(),
            args: vec!["/D".into(), "/Q".into()],
            rows: 20,
            cols: 80,
        })
        .unwrap()
    else {
        panic!("Managed child did not start")
    };
    let Response::Terminal(snapshot) = client
        .execute(HostCommand::Attach {
            session: session.clone(),
            client: "standalone-test".into(),
            takeover: false,
        })
        .unwrap()
    else {
        panic!("Attach failed")
    };
    let token = snapshot.input_token.unwrap();
    client
        .execute(HostCommand::Input {
            session: session.clone(),
            client: "standalone-test".into(),
            token,
            bytes: b"echo standalone-native-child\r\n".to_vec(),
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let Response::Terminal(snapshot) = client
            .execute(HostCommand::Snapshot {
                session: session.clone(),
            })
            .unwrap()
        else {
            panic!("Snapshot failed")
        };
        if String::from_utf8_lossy(&snapshot.formatted).contains("standalone-native-child") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Managed child did not produce its native terminal screen"
        );
        std::thread::yield_now();
    }
    client.execute(HostCommand::Stop { session }).unwrap();
    client.execute(HostCommand::StopHost).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while host.0.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "Standalone host did not stop");
        std::thread::yield_now();
    }
}
