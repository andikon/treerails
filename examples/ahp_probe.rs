//! AHP compatibility probe. By default, reads metadata only. An optional VS Code
//! CLI path also creates, lists and disposes an idle session. Never sends a turn.
use anyhow::{Context, Result};
use std::time::Duration;
use tokio::sync::mpsc;
use treerail::{
    agent::{self, Command, Event},
    config::AgentConfig,
};
#[path = "support/capture.rs"]
mod capture;
#[tokio::main]
async fn main() -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let provider_home = tempfile::TempDir::new()?;
    let config = AgentConfig {
        home: Some(provider_home.path().to_path_buf()),
        host_directory: Some(std::env::temp_dir()),
        ..Default::default()
    };
    let (commands, task) = agent::spawn(config, tx);
    commands.send(Command::Discover)?;
    let mut connection_url = String::new();
    let result = tokio::time::timeout(Duration::from_secs(60), async {
        while let Some(event) = rx.recv().await {
            match event {
                Event::Connected {
                    agents,
                    connection_url: url,
                } => {
                    connection_url = url;
                    println!(
                        "AHP negotiated; {} provider(s)",
                        agents.as_array().map_or(0, Vec::len)
                    );
                }
                Event::Catalog(items) => {
                    println!(
                        "Catalog readable; {} conversation(s). No prompt sent.",
                        items.len()
                    );
                    commands.send(Command::Options {
                        worktree: std::env::current_dir()?,
                    })?;
                }
                Event::Options(options) => {
                    println!(
                        "Provider configuration readable; {} option(s). No session created.",
                        options["schema"]["properties"]
                            .as_object()
                            .map_or(0, |p| p.len())
                    );
                    if let Some(properties) = options["schema"]["properties"].as_object() {
                        println!(
                            "Provider option names: {}",
                            properties.keys().cloned().collect::<Vec<_>>().join(", ")
                        );
                    }
                    if let Some(cli) = std::env::args().nth(1) {
                        verify_vscode(
                            &connection_url,
                            &cli,
                            &commands,
                            &mut rx,
                            provider_home.path(),
                        )
                        .await?;
                    }
                    return Ok(());
                }
                Event::Error { message, .. } => anyhow::bail!("{message}"),
                _ => {}
            }
        }
        anyhow::bail!("Provider closed before catalog arrived")
    })
    .await;
    let _ = commands.send(Command::Shutdown);
    let _ = task.await;
    result??;
    Ok(())
}
async fn verify_vscode(
    endpoint: &str,
    cli: &str,
    commands: &mpsc::UnboundedSender<Command>,
    rx: &mut mpsc::UnboundedReceiver<Event>,
    provider_home: &std::path::Path,
) -> Result<()> {
    use serde_json::{Value, json};
    let temp = tempfile::TempDir::new()?;
    let scratch = std::env::current_dir()?.join("dist");
    std::fs::create_dir_all(&scratch)?;
    let project = tempfile::TempDir::new_in(scratch)?;
    treerail::git::run(project.path(), ["init", "--initial-branch=develop"]).await?;
    treerail::git::run(
        project.path(),
        [
            "-c",
            "user.name=Tree Rail Probe",
            "-c",
            "user.email=probe@example.invalid",
            "commit",
            "--allow-empty",
            "-m",
            "Disposable probe",
        ],
    )
    .await?;
    let checkout = temp.path().join("checkout");
    treerail::git::run(
        project.path(),
        [
            "worktree",
            "add",
            "-b",
            "probe",
            checkout.to_str().context("Probe checkout path")?,
        ],
    )
    .await?;
    let id = format!("ahp-session:/{}", uuid::Uuid::new_v4());
    commands.send(Command::Create {
        id: id.clone(),
        worktree: checkout.clone(),
        prompt: String::new(),
        config: json!({}),
    })?;
    let mut granted_endpoint = endpoint.to_owned();
    loop {
        match rx
            .recv()
            .await
            .context("Provider closed during idle session creation")?
        {
            Event::Connected { connection_url, .. } => granted_endpoint = connection_url,
            Event::FolderTrust { id, path } => {
                anyhow::ensure!(path == checkout, "Unexpected trust request");
                // This host uses only the disposable COPILOT_HOME supplied by main.
                anyhow::ensure!(provider_home.is_dir(), "Missing isolated provider home");
                commands.send(Command::TrustWorkspace { id, approved: true })?;
            }
            Event::Session { id: session, state }
                if session == id
                    && state.lifecycle == ahp_types::state::SessionLifecycle::Ready =>
            {
                break;
            }
            Event::LaunchFailed { message, .. } | Event::Error { message, .. } => {
                anyhow::bail!("{message}")
            }
            _ => {}
        }
    }
    let endpoint = granted_endpoint.as_str();
    if std::env::args().any(|arg| arg == "--terminal") {
        let mut terminal = treerail::cli_terminal::Terminal::copilot(
            treerail::cli_terminal::CopilotLaunch {
                program: None,
                directory: checkout.clone(),
                home: Some(provider_home.to_path_buf()),
                endpoint: endpoint.to_owned(),
                session: id.clone(),
                model: None,
            },
            32,
            100,
        );
        let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
        let mut input_checked = false;
        let mut resized = false;
        loop {
            terminal.poll();
            let contents = terminal.screen().contents();
            if !resized && contents.contains("remote") && contents.contains('❯') {
                terminal.resize(24, 68);
                terminal.paste("/help");
                resized = true;
            }
            if resized && !input_checked && contents.contains("/help") {
                input_checked = true;
                terminal.key(crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Esc,
                    crossterm::event::KeyModifiers::NONE,
                ));
                terminal.key(crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Char('u'),
                    crossterm::event::KeyModifiers::CONTROL,
                ));
            }
            if terminal.exit.is_some()
                || (input_checked && contents.contains("remote"))
                || tokio::time::Instant::now() >= deadline
            {
                let mut safe = contents.replace(endpoint, "[private host]");
                if let Ok(url) = url::Url::parse(endpoint) {
                    for (key, value) in url.query_pairs() {
                        if key == "tkn" {
                            safe = safe.replace(value.as_ref(), "[redacted]");
                        }
                    }
                }
                println!("CLI terminal snapshot:\n{safe}");
                anyhow::ensure!(
                    terminal.exit.is_none(),
                    "CLI exited before the terminal probe completed"
                );
                anyhow::ensure!(
                    contents.contains("Copilot")
                        && contents.contains("remote")
                        && contents.contains('❯')
                        && !contents.contains("Failed to connect"),
                    "CLI did not render its interactive interface"
                );
                anyhow::ensure!(input_checked, "CLI did not accept typing after resize");
                if let Some(path) = std::env::args()
                    .find_map(|arg| arg.strip_prefix("--capture=").map(std::path::PathBuf::from))
                {
                    capture_terminal(terminal, project.path(), &checkout, &id, &path).await?;
                } else {
                    terminal.close();
                }
                break;
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    }
    let client = ahp::Client::connect(
        ahp_ws::WebSocketTransport::connect(endpoint).await?,
        Default::default(),
    )
    .await?;
    let initialized = client
        .initialize(
            format!("treerail-verify-{}", uuid::Uuid::new_v4()),
            ahp_types::SUPPORTED_PROTOCOL_VERSIONS
                .iter()
                .map(ToString::to_string)
                .collect(),
            vec![],
        )
        .await?;
    agent::authenticate_github(&client, endpoint, &serde_json::to_value(initialized.meta)?).await?;
    let result: Result<()> = async {
        let (session, _) = client.subscribe(id.clone()).await?;
        let session = session.snapshot.context("Missing session snapshot")?;
        if let ahp_types::state::SnapshotState::Session(session) = session.state {
            let chat = session.default_chat.context("Missing default chat")?;
            let (chat, _) = client.subscribe(chat).await?;
            let chat = chat.snapshot.context("Missing chat snapshot")?;
            if let ahp_types::state::SnapshotState::Chat(chat) = chat.state {
                anyhow::ensure!(
                    chat.turns.is_empty() && chat.active_turn.is_none(),
                    "The idle probe unexpectedly started a model turn"
                );
            } else {
                anyhow::bail!("Expected chat state");
            }
        } else {
            anyhow::bail!("Expected session state");
        }
        let output = tokio::process::Command::new(cli)
            .args([
                "--cli-data-dir",
                temp.path().to_str().unwrap_or("."),
                "agent",
                "ps",
                "--address",
                endpoint,
                "--json",
                "--all",
            ])
            .output()
            .await?;
        anyhow::ensure!(
            output.status.success(),
            "VS Code AHP client could not list the test host (exit {})",
            output.status
        );
        let response: Value = serde_json::from_slice(&output.stdout)?;
        anyhow::ensure!(
            response
                .to_string()
                .contains(id.trim_start_matches("ahp-session:/")),
            "VS Code did not discover the idle test conversation"
        );
        println!(
            "VS Code's AHP client discovered the same idle Copilot session. No model turn was sent."
        );
        Ok(())
    }
    .await;
    let disposal: Result<Value, _> = client
        .request("disposeSession", json!({"channel":id}))
        .await;
    client.shutdown().await;
    result?;
    disposal.context("Dispose the idle test session")?;
    Ok(())
}

async fn capture_terminal(
    terminal: treerail::cli_terminal::Terminal,
    project_path: &std::path::Path,
    checkout: &std::path::Path,
    id: &str,
    path: &std::path::Path,
) -> Result<()> {
    use treerail::{
        app::{Action, App, Completion},
        config::{Config, State},
        git::{Project, Worktree},
        startup::Session,
    };
    let mut app = App::new(Session {
        config: Config::default(),
        state: State::default(),
        config_path: "unused/config.toml".into(),
        roots: vec![],
        recursive: false,
    });
    let project = Project {
        name: "Copilot CLI probe".into(),
        path: project_path.into(),
    };
    app.projects.push(project.clone());
    app.open_project(0);
    app.accept(
        format!("trees:{}", project_path.display()),
        Ok(Completion::Trees(
            project.clone(),
            vec![Worktree {
                path: checkout.into(),
                branch: "probe".into(),
                head: String::new(),
                original: false,
                locked: false,
                prunable: false,
            }],
        )),
    );
    app.state.conversations.push(serde_json::from_value(serde_json::json!({
        "id": id, "project": project_path, "worktree": checkout, "title": "Idle CLI compatibility check", "session_uri": id, "attention": "Ready", "owned": true
    }))?);
    app.action(Action::Session(id.into()));
    app.terminals.insert(id.into(), terminal);
    app.snapshots
        .insert(checkout.into(), treerail::git::status(checkout).await?);
    app.connected = true;
    app.cli_focused = true;
    let mut screen = ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 36))?;
    // Draw once to resize the real child, then allow its own layout to settle.
    screen.draw(|frame| treerail::ui::draw(frame, &mut app))?;
    for _ in 0..35 {
        tokio::time::sleep(Duration::from_millis(40)).await;
        app.terminals.poll();
    }
    screen.draw(|frame| treerail::ui::draw(frame, &mut app))?;
    capture::svg(screen.backend().buffer(), path)?;
    app.terminals.shutdown();
    println!(
        "Captured the actual CLI inside Tree Rail at {}",
        path.display()
    );
    Ok(())
}
