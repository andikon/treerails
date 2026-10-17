use std::{
    io::Cursor,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};
use treerail::host::{
    PROTOCOL_VERSION, handshake, read_frame,
    runtime::{Client, HostCommand, Response, TerminalSnapshot},
    write_frame,
};

fn execute(client: &Client, mut command: impl FnMut() -> HostCommand) -> Response {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match client.execute(command()) {
            Ok(response) => return response,
            Err(error)
                if Instant::now() < deadline
                    && (format!("{error:#}").contains("Connect to local session host")) =>
            {
                std::thread::yield_now()
            }
            Err(error) => panic!("{error:#}"),
        }
    }
}
fn snapshot(client: &Client, id: &str) -> TerminalSnapshot {
    match execute(client, || HostCommand::Snapshot { session: id.into() }) {
        Response::Terminal(snapshot) => snapshot,
        value => panic!("{value:?}"),
    }
}
fn wait_text(client: &Client, id: &str, text: &str) -> TerminalSnapshot {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let state = snapshot(client, id);
        if String::from_utf8_lossy(&state.formatted).contains(text) {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "Terminal never contained {text}: {:?}",
            state
        );
        std::thread::yield_now();
    }
}
fn git(path: &Path, args: &[&str]) {
    assert!(
        Command::new("git")
            .current_dir(path)
            .args(args)
            .output()
            .unwrap()
            .status
            .success()
    );
}
struct HostGuard(std::process::Child);
impl Drop for HostGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn bounded_frames_and_versions_fail_before_any_mutation() {
    assert!(handshake(PROTOCOL_VERSION + 1).is_err());
    let mut bytes = Vec::new();
    write_frame(&mut bytes, &vec!["value"]).unwrap();
    assert_eq!(
        read_frame::<_, Vec<String>>(&mut Cursor::new(bytes)).unwrap(),
        vec!["value"]
    );
    assert!(read_frame::<_, String>(&mut Cursor::new(u32::MAX.to_be_bytes())).is_err());
    assert!(read_frame::<_, String>(&mut Cursor::new([0, 0, 0, 8, b'"', b'x'])).is_err());
}

#[test]
fn actual_host_retains_terminal_across_client_exit_and_enforces_takeover() {
    let temp = tempfile::tempdir().unwrap();
    git(temp.path(), &["init"]);
    let endpoint = format!(
        "test-{}-{}",
        std::process::id(),
        temp.path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .replace('.', "-")
    );
    let mut host = HostGuard(
        Client::launch(
            Path::new(env!("CARGO_BIN_EXE_treerail-session-host")),
            &endpoint,
        )
        .unwrap(),
    );
    let client = Client::new(&endpoint);
    assert!(matches!(
        execute(&client, || HostCommand::Ping),
        Response::Pong { .. }
    ));
    #[cfg(windows)]
    let (program, args) = ("cmd.exe", vec!["/D".into(), "/Q".into()]);
    #[cfg(unix)]
    let (program, args) = ("sh", vec![]);
    let start = || HostCommand::Start {
        request_id: "start-one".into(),
        checkout: temp.path().into(),
        program: program.into(),
        args: args.clone(),
        rows: 24,
        cols: 80,
    };
    let (id, pid) = match execute(&client, start) {
        Response::Started {
            session,
            process_id,
        } => (session, process_id),
        value => panic!("{value:?}"),
    };
    assert!(pid.is_some());
    match execute(&client, start) {
        Response::Started {
            session,
            process_id,
        } => assert_eq!((session, process_id), (id.clone(), pid)),
        value => panic!("{value:?}"),
    };
    let token = match execute(&client, || HostCommand::Attach {
        session: id.clone(),
        client: "first".into(),
        takeover: false,
    }) {
        Response::Terminal(state) => state.input_token.unwrap(),
        value => panic!("{value:?}"),
    };
    execute(&client, || HostCommand::Input {
        session: id.clone(),
        client: "first".into(),
        token,
        bytes: b"echo before-detach\r\n".to_vec(),
    });
    wait_text(&client, &id, "before-detach");
    execute(&client, || HostCommand::Detach {
        session: id.clone(),
        client: "first".into(),
        token,
    });
    drop(client); // No UI or connection remains; PTY and process belong to the host.
    let client = Client::new(&endpoint);
    assert_eq!(snapshot(&client, &id).process_id, pid);
    let token2 = match execute(&client, || HostCommand::Attach {
        session: id.clone(),
        client: "second".into(),
        takeover: false,
    }) {
        Response::Terminal(state) => state.input_token.unwrap(),
        value => panic!("{value:?}"),
    };
    assert!(
        client
            .execute(HostCommand::Input {
                session: id.clone(),
                client: "first".into(),
                token,
                bytes: b"echo invalid\r\n".to_vec()
            })
            .is_err()
    );
    let token3 = match execute(&client, || HostCommand::Attach {
        session: id.clone(),
        client: "third".into(),
        takeover: true,
    }) {
        Response::Terminal(state) => state.input_token.unwrap(),
        value => panic!("{value:?}"),
    };
    assert!(
        client
            .execute(HostCommand::Input {
                session: id.clone(),
                client: "second".into(),
                token: token2,
                bytes: b"echo invalid\r\n".to_vec()
            })
            .is_err()
    );
    execute(&client, || HostCommand::Resize {
        session: id.clone(),
        client: "third".into(),
        token: token3,
        rows: 30,
        cols: 90,
    });
    assert_eq!(
        (snapshot(&client, &id).rows, snapshot(&client, &id).cols),
        (30, 90)
    );
    execute(&client, || HostCommand::Input {
        session: id.clone(),
        client: "third".into(),
        token: token3,
        bytes: b"echo after-reattach\r\n".to_vec(),
    });
    wait_text(&client, &id, "after-reattach");
    assert!(
        client
            .execute(HostCommand::Start {
                request_id: "duplicate-checkout".into(),
                checkout: temp.path().join("."),
                program: program.into(),
                args: args.clone(),
                rows: 24,
                cols: 80
            })
            .is_err()
    );
    assert!(client.execute(HostCommand::StopHost).is_err());
    execute(&client, || HostCommand::Stop {
        session: id.clone(),
    });
    execute(&client, || HostCommand::StopHost);
    let deadline = Instant::now() + Duration::from_secs(5);
    while host.0.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
}
