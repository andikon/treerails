use std::io::Cursor;
use treerail::host::{PROTOCOL_VERSION, handshake, read_frame, write_frame};
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

#[cfg(windows)]
mod native {
    use std::{
        path::Path,
        process::Command,
        time::{Duration, Instant},
    };
    use treerail::host::{
        PROTOCOL_VERSION, read_frame,
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
        // Deliberately lose a successful Start result after receiving only its header.
        // Retrying the same request must return the existing process, never spawn twice.
        {
            let mut pipe = treerail::host::ipc::connect(&endpoint).unwrap();
            write_frame(
                &mut pipe,
                &treerail::host::runtime::Request {
                    version: PROTOCOL_VERSION,
                    command: start(),
                },
            )
            .unwrap();
            let mut header = [0; 4];
            std::io::Read::read_exact(&mut pipe, &mut header).unwrap();
        }
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
        assert!(matches!(execute(&client,start),Response::Ended {session} if session==id));
        execute(&client, || HostCommand::StopHost);
        let deadline = Instant::now() + Duration::from_secs(5);
        while host.0.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
    }

    #[cfg(windows)]
    #[test]
    fn concurrent_discovery_keeps_one_host_and_incompatible_clients_cannot_replace_it() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config.toml");
        let executable = Path::new(env!("CARGO_BIN_EXE_treerail-session-host"));
        let results = std::thread::scope(|scope| {
            let first = scope.spawn(|| Client::connect_or_launch(executable, &config).unwrap());
            let second = scope.spawn(|| Client::connect_or_launch(executable, &config).unwrap());
            (first.join().unwrap(), second.join().unwrap())
        });
        assert_eq!(results.0.endpoint(), results.1.endpoint());
        let mut pipe = treerail::host::ipc::connect(results.0.endpoint()).unwrap();
        write_frame(
            &mut pipe,
            &treerail::host::runtime::Request {
                version: PROTOCOL_VERSION + 1,
                command: HostCommand::StopHost,
            },
        )
        .unwrap();
        assert!(matches!(
            read_frame::<_, Response>(&mut pipe).unwrap(),
            Response::Error(_)
        ));
        drop(pipe);
        assert!(matches!(
            execute(&results.1, || HostCommand::Ping),
            Response::Pong { .. }
        ));
        execute(&results.0, || HostCommand::StopHost);
    }

    #[cfg(windows)]
    #[test]
    fn stopping_session_terminates_descendants_before_releasing_checkout() {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::{
            Foundation::WAIT_OBJECT_0,
            System::Threading::{OpenProcess, WaitForSingleObject},
        };
        let temp = tempfile::tempdir().unwrap();
        git(temp.path(), &["init"]);
        let endpoint = format!(
            "descendants-{}-{}",
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
        let instance = match execute(&client, || HostCommand::Ping) {
            Response::Pong { instance, .. } => instance,
            value => panic!("{value:?}"),
        };
        let request_id = uuid::Uuid::new_v4().to_string();
        let expected_group =
            treerail::host::execution::execution_group_identity(&instance, &request_id).unwrap();
        let script = "$p=Start-Process powershell.exe -ArgumentList '-NoProfile','-Command','Start-Sleep -Seconds 60' -WindowStyle Hidden -PassThru; $p.Id | Set-Content descendant.pid; Start-Sleep -Seconds 60";
        let id = match execute(&client, || HostCommand::Start {
            request_id: request_id.clone(),
            checkout: temp.path().into(),
            program: "powershell.exe".into(),
            args: vec!["-NoProfile".into(), "-Command".into(), script.into()],
            rows: 24,
            cols: 80,
        }) {
            Response::Started { session, .. } => session,
            value => panic!("{value:?}"),
        };
        assert_eq!(
            snapshot(&client, &id).execution_group.as_deref(),
            Some(expected_group.as_str())
        );
        assert!(!treerail::host::execution::execution_group_stopped(&expected_group).unwrap());
        let deadline = Instant::now() + Duration::from_secs(15);
        let child_pid = loop {
            if let Ok(text) = std::fs::read_to_string(temp.path().join("descendant.pid"))
                && let Ok(pid) = text.trim().parse::<u32>()
            {
                break pid;
            }
            assert!(Instant::now() < deadline, "Descendant did not start");
            std::thread::yield_now();
        };
        let handle = unsafe { OpenProcess(0x00100000, 0, child_pid) };
        assert!(!handle.is_null());
        let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
        execute(&client, || HostCommand::Stop {
            session: id.clone(),
        });
        assert_eq!(
            unsafe { WaitForSingleObject(handle.as_raw_handle(), 5000) },
            WAIT_OBJECT_0
        );
        assert!(treerail::host::execution::execution_group_stopped(&expected_group).unwrap());
        let _lease = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(treerail::host::CheckoutLease::acquire(temp.path()))
            .unwrap();
        drop(_lease);
        std::fs::remove_file(temp.path().join("descendant.pid")).unwrap();
        execute(&client, || HostCommand::Start {
            request_id: "crash-tree".into(),
            checkout: temp.path().into(),
            program: "powershell.exe".into(),
            args: vec!["-NoProfile".into(), "-Command".into(), script.into()],
            rows: 24,
            cols: 80,
        });
        let deadline = Instant::now() + Duration::from_secs(15);
        let child_pid = loop {
            if let Ok(text) = std::fs::read_to_string(temp.path().join("descendant.pid"))
                && let Ok(pid) = text.trim().parse::<u32>()
            {
                break pid;
            }
            assert!(Instant::now() < deadline, "Second descendant did not start");
            std::thread::yield_now();
        };
        let handle = unsafe { OpenProcess(0x00100000, 0, child_pid) };
        assert!(!handle.is_null());
        let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
        host.0.kill().unwrap();
        host.0.wait().unwrap();
        assert_eq!(
            unsafe { WaitForSingleObject(handle.as_raw_handle(), 5000) },
            WAIT_OBJECT_0,
            "Host failure must close its job and terminate managed descendants"
        );
        let _recovered_lease = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(treerail::host::CheckoutLease::acquire(temp.path()))
            .unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn partial_client_frame_has_a_deadline_and_does_not_replace_host() {
        let temp = tempfile::tempdir().unwrap();
        let endpoint = format!(
            "slow-{}-{}",
            std::process::id(),
            temp.path()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .replace('.', "-")
        );
        let _host = HostGuard(
            Client::launch(
                Path::new(env!("CARGO_BIN_EXE_treerail-session-host")),
                &endpoint,
            )
            .unwrap(),
        );
        let client = Client::new(&endpoint);
        let instance = match execute(&client, || HostCommand::Ping) {
            Response::Pong { instance, .. } => instance,
            value => panic!("{value:?}"),
        };
        let mut pipe = treerail::host::ipc::connect(&endpoint).unwrap();
        std::io::Write::write_all(&mut pipe, &1000u32.to_be_bytes()).unwrap();
        let began = Instant::now();
        let instance_after = match execute(&client, || HostCommand::Ping) {
            Response::Pong { instance, .. } => instance,
            value => panic!("{value:?}"),
        };
        assert!(began.elapsed() < Duration::from_secs(8));
        assert_eq!(instance, instance_after);
        drop(pipe);
        execute(&client, || HostCommand::StopHost);
    }

    #[cfg(windows)]
    #[test]
    fn hosted_ui_client_fixture() {
        let Ok(endpoint) = std::env::var("TREERAIL_TEST_ENDPOINT") else {
            return;
        };
        let session = std::env::var("TREERAIL_TEST_SESSION").unwrap();
        let client = Client::new(endpoint);
        let token = match execute(&client, || HostCommand::Attach {
            session: session.clone(),
            client: "crashed-ui".into(),
            takeover: false,
        }) {
            Response::Terminal(snapshot) => snapshot.input_token.unwrap(),
            value => panic!("{value:?}"),
        };
        execute(&client, || HostCommand::Input {
            session: session.clone(),
            client: "crashed-ui".into(),
            token,
            bytes: b"echo fixture-client-connected\r\n".to_vec(),
        });
        std::fs::write(
            std::env::var_os("TREERAIL_TEST_MARKER").unwrap(),
            b"attached",
        )
        .unwrap();
        loop {
            std::thread::park();
        }
    }

    #[cfg(windows)]
    #[test]
    fn crashed_ui_releases_keyboard_while_detached_heavy_output_continues() {
        let temp = tempfile::tempdir().unwrap();
        git(temp.path(), &["init"]);
        let endpoint = format!(
            "client-crash-{}-{}",
            std::process::id(),
            temp.path()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .replace('.', "-")
        );
        let _host = HostGuard(
            Client::launch(
                Path::new(env!("CARGO_BIN_EXE_treerail-session-host")),
                &endpoint,
            )
            .unwrap(),
        );
        let client = Client::new(&endpoint);
        execute(&client, || HostCommand::Ping);
        let (id, pid) = match execute(&client, || HostCommand::Start {
            request_id: "crashed-ui-start".into(),
            checkout: temp.path().into(),
            program: "cmd.exe".into(),
            args: vec!["/D".into(), "/Q".into()],
            rows: 24,
            cols: 80,
        }) {
            Response::Started {
                session,
                process_id,
            } => (session, process_id),
            value => panic!("{value:?}"),
        };
        let marker = temp.path().join("client-attached");
        let mut ui = HostGuard(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "native::hosted_ui_client_fixture", "--nocapture"])
                .env("TREERAIL_TEST_ENDPOINT", &endpoint)
                .env("TREERAIL_TEST_SESSION", &id)
                .env("TREERAIL_TEST_MARKER", &marker)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        while !marker.exists() {
            assert!(Instant::now() < deadline);
            assert!(ui.0.try_wait().unwrap().is_none());
            std::thread::yield_now();
        }
        wait_text(&client, &id, "fixture-client-connected");
        ui.0.kill().unwrap();
        ui.0.wait().unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let token = loop {
            match client.execute(HostCommand::Attach {
                session: id.clone(),
                client: "replacement-ui".into(),
                takeover: false,
            }) {
                Ok(Response::Terminal(state)) => break state.input_token.unwrap(),
                Err(error) => {
                    assert!(
                        Instant::now() < deadline,
                        "Crashed client retained control: {error:#}"
                    );
                    std::thread::yield_now();
                }
                value => panic!("{value:?}"),
            }
        };
        assert_eq!(snapshot(&client, &id).process_id, pid);
        let burst=b"for /L %G in (1,1,12000) do @echo 012345678901234567890123456789012345678901234567890123456789\r\necho finished>detached.finished\r\necho detached-output-done\r\n";
        execute(&client, || HostCommand::Input {
            session: id.clone(),
            client: "replacement-ui".into(),
            token,
            bytes: burst.to_vec(),
        });
        execute(&client, || HostCommand::Detach {
            session: id.clone(),
            client: "replacement-ui".into(),
            token,
        });
        let deadline = Instant::now() + Duration::from_secs(20);
        while !temp.path().join("detached.finished").exists() {
            assert!(
                Instant::now() < deadline,
                "Headless output blocked the child before its completion marker"
            );
            std::thread::yield_now();
        }
        let state = wait_text(&client, &id, "detached-output-done");
        assert_eq!(state.process_id, pid);
        assert!(!state.exited);
        assert!(
            state.formatted.len() < 65536,
            "Snapshot must retain a bounded screen rather than replay all output"
        );
        execute(&client, || HostCommand::Stop {
            session: id.clone(),
        });
        execute(&client, || HostCommand::StopHost);
    }
    #[test]
    fn simultaneous_clients_preserve_one_host_and_process() {
        let temp = tempfile::tempdir().unwrap();
        git(temp.path(), &["init"]);
        let endpoint = format!(
            "stress-{}-{}",
            std::process::id(),
            temp.path()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .replace('.', "-")
        );
        let _host = HostGuard(
            Client::launch(
                Path::new(env!("CARGO_BIN_EXE_treerail-session-host")),
                &endpoint,
            )
            .unwrap(),
        );
        let client = Client::new(&endpoint);
        execute(&client, || HostCommand::Ping);
        let id = match execute(&client, || HostCommand::Start {
            request_id: "stress-start".into(),
            checkout: temp.path().into(),
            program: "cmd.exe".into(),
            args: vec!["/D".into(), "/Q".into()],
            rows: 24,
            cols: 80,
        }) {
            Response::Started { session, .. } => session,
            value => panic!("{value:?}"),
        };
        let pid = snapshot(&client, &id).process_id;
        let mut partial = treerail::host::ipc::connect(&endpoint).unwrap();
        std::io::Write::write_all(&mut partial, &1000u32.to_be_bytes()).unwrap();
        std::thread::scope(|scope| {
            for _ in 0..12 {
                let endpoint = &endpoint;
                let id = &id;
                scope.spawn(move || {
                    let client = Client::new(endpoint);
                    for _ in 0..25 {
                        assert!(matches!(
                            client.execute(HostCommand::Ping).unwrap(),
                            Response::Pong { .. }
                        ));
                        assert_eq!(snapshot(&client, id).process_id, pid);
                    }
                });
            }
        });
        drop(partial);
        assert!(
            matches!(execute(&client,||HostCommand::FindStart{request_id:"not-started".into()}),Response::NotStarted{request_id} if request_id=="not-started")
        );
        execute(&client, || HostCommand::Stop {
            session: id.clone(),
        });
        execute(&client, || HostCommand::StopHost);
    }
}
