//! Native AHP conversations. The SDK only owns the temporary Copilot runtime;
//! protocol state, streaming, approvals and resume all cross the AHP interface.
mod sealed_auth;
use crate::config::AgentConfig;
use ahp::{Client, ClientConfig, SubscriptionEvent};
use ahp_types::{
    actions::StateAction,
    state::{ChatState, SessionState, SnapshotState},
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Attention {
    Running,
    NeedsInput,
    #[default]
    Ready,
    Interrupted,
    Unavailable,
}
impl Attention {
    pub fn busy(self) -> bool {
        matches!(self, Self::Running | Self::NeedsInput | Self::Unavailable)
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::NeedsInput => "Needs input",
            Self::Ready => "Ready",
            Self::Interrupted => "Interrupted",
            Self::Unavailable => "Unavailable",
        }
    }
    pub fn from_status(status: u32) -> Self {
        if status & 24 == 24 {
            Self::NeedsInput
        } else if status & 8 != 0 {
            Self::Running
        } else if status & 2 != 0 {
            Self::Interrupted
        } else {
            Self::Ready
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub project: PathBuf,
    pub worktree: PathBuf,
    pub title: String,
    pub session_uri: String,
    #[serde(default)]
    pub chat_uri: Option<String>,
    #[serde(default)]
    pub attention: Attention,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub draft: String,
    #[serde(default)]
    pub chat: Option<ChatState>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub pending: Vec<Value>,
    #[serde(default)]
    pub options: Value,
    #[serde(default)]
    pub owned: bool,
    #[serde(default)]
    pub initial_prompt: Option<String>,
    #[serde(default)]
    pub creation_failed: bool,
    #[serde(default)]
    pub model: Option<String>,
}
#[derive(Debug, Clone)]
pub enum Command {
    Discover,
    Create {
        id: String,
        worktree: PathBuf,
        prompt: String,
        config: Value,
    },
    Observe {
        id: String,
    },
    Dispatch {
        channel: String,
        action: Value,
        worktree: Option<PathBuf>,
    },
    Complete {
        channel: String,
        text: String,
    },
    Options {
        worktree: PathBuf,
    },
    TrustWorkspace {
        id: String,
        approved: bool,
    },
    Shutdown,
}
#[derive(Debug, Clone)]
pub enum Event {
    FolderTrust {
        id: String,
        path: PathBuf,
    },
    Connected {
        connection_url: String,
        agents: Value,
    },
    Catalog(Vec<Value>),
    Session {
        id: String,
        state: Box<SessionState>,
    },
    Chat {
        session: String,
        state: Box<ChatState>,
    },
    Completions(Value),
    Agents(Value),
    Options(Value),
    Error {
        id: Option<String>,
        message: String,
    },
    Disconnected,
    LaunchFailed {
        id: String,
        message: String,
    },
}
pub fn spawn(
    config: AgentConfig,
    output: mpsc::UnboundedSender<Event>,
) -> (mpsc::UnboundedSender<Command>, tokio::task::JoinHandle<()>) {
    let (sender, mut input) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        let mut pending = VecDeque::new();
        let mut waiting: BTreeMap<String, Command> = BTreeMap::new();
        let mut connection: Option<Connection> = None;
        let mut watched = BTreeSet::new();
        let (finished_tx, mut finished_rx) = mpsc::unbounded_channel::<String>();
        let leases = Arc::new(tokio::sync::Mutex::new(BTreeMap::new()));
        let mut jobs = tokio::task::JoinSet::new();
        while let Some(command) = match pending.pop_front() {
            Some(command) => Some(command),
            None => input.recv().await,
        } {
            while let Ok(id) = finished_rx.try_recv() {
                watched.remove(&id);
            }
            if matches!(command, Command::Shutdown) {
                break;
            }
            if let Command::TrustWorkspace { id, approved } = &command {
                let Some(original) = waiting.remove(id) else {
                    continue;
                };
                let result = if *approved {
                    if let Command::Create { worktree, .. } = &original {
                        match connection.as_ref().and_then(|c| c.owner.as_ref()) {
                            Some(owner) => trust_workspace(owner, worktree).await,
                            None => {
                                Err(anyhow::anyhow!("Folder trust belongs to the external host"))
                            }
                        }
                    } else {
                        Err(anyhow::anyhow!("No pending session to authorize"))
                    }
                } else {
                    Err(anyhow::anyhow!(
                        "Folder trust cancelled. Your prompt was not sent"
                    ))
                };
                match result {
                    Ok(()) => pending.push_back(original),
                    Err(error) => {
                        let _ = output.send(Event::LaunchFailed {
                            id: id.clone(),
                            message: format!("{error:#}"),
                        });
                    }
                }
                continue;
            }
            if let Command::Observe { id } = &command
                && watched.contains(id)
            {
                continue;
            }
            if connection.is_none() {
                match Connection::open(&config, &output).await {
                    Ok(c) => connection = Some(c),
                    Err(e) => {
                        let event = if let Command::Create { id, .. } = &command {
                            Event::LaunchFailed {
                                id: id.clone(),
                                message: format!("Copilot AHP unavailable: {e:#}"),
                            }
                        } else {
                            Event::Error {
                                id: command_id(&command),
                                message: format!("Copilot AHP unavailable: {e:#}"),
                            }
                        };
                        let _ = output.send(event);
                        continue;
                    }
                }
            }
            if let Command::Create { id, worktree, .. } = &command
                && let Some(owner) = connection.as_ref().and_then(|c| c.owner.as_ref())
            {
                match workspace_trusted(owner, worktree).await {
                    Ok(true) => {}
                    Ok(false) => {
                        waiting.insert(id.clone(), command.clone());
                        let _ = output.send(Event::FolderTrust {
                            id: id.clone(),
                            path: worktree.clone(),
                        });
                        continue;
                    }
                    Err(error) => {
                        let _ = output.send(Event::LaunchFailed {
                            id: id.clone(),
                            message: format!("Check Copilot folder trust: {error:#}"),
                        });
                        continue;
                    }
                }
            }
            let client = connection.as_ref().expect("connected").client.clone();
            if matches!(
                &command,
                Command::Create { .. } | Command::Observe { .. } | Command::Options { .. }
            ) {
                let endpoint = connection.as_ref().expect("connected").endpoint.clone();
                if let Err(error) = authenticate_github(
                    &client,
                    &endpoint,
                    &connection.as_ref().expect("connected").initialize_meta,
                )
                .await
                {
                    let message = format!("{error:#}");
                    let event = if let Command::Create { id, .. } = &command {
                        Event::LaunchFailed {
                            id: id.clone(),
                            message,
                        }
                    } else {
                        Event::Error {
                            id: command_id(&command),
                            message,
                        }
                    };
                    let _ = output.send(event);
                    continue;
                }
            }
            let tx = output.clone();
            let turns = leases.clone();
            let finished = finished_tx.clone();
            match command {
                Command::Discover => {
                    jobs.spawn(async move {
                        if let Err(e) = catalog(&client, &tx).await {
                            report(&tx, None, e);
                        }
                    });
                }
                Command::Create {
                    id,
                    worktree,
                    prompt,
                    mut config,
                } => {
                    watched.insert(id.clone());
                    jobs.spawn(async move {
                        let result: Result<()> = async {
                            let model=config.as_object_mut().and_then(|c|c.remove("_treerailModel")).and_then(|m|m.as_str().map(str::to_owned));
                            let title=config.as_object_mut().and_then(|c|c.remove("_treerailName")).and_then(|m|m.as_str().map(str::to_owned));
                            let directory = url::Url::from_directory_path(&worktree).map_err(|_| anyhow::anyhow!("Invalid worktree path"))?;
                            if let Err(e)=client.request::<_,Value>("createSession", json!({"channel":id,"provider":"copilot","workingDirectories":[directory.as_str()],"config":config})).await {
                                let hint = if matches!(&e, ahp::ClientError::Rpc(rpc) if rpc.code == -32009) { ". Check the host workspace scope: remember this project and reopen Tree Rail, or set agent.host_directory to a containing folder" } else { "" };
                                let _=tx.send(Event::LaunchFailed{id:id.clone(),message:format!("Create Copilot session: {e}{hint}")});
                                return Ok(());
                            }
                            if let Some(title)=title{client.dispatch(id.clone(),serde_json::from_value(json!({"type":"session/titleChanged","title":title}))?).await?;}
                            watch_session(client, id.clone(), (!prompt.is_empty()).then_some((prompt,model)), tx.clone(),turns).await
                        }.await;
                        if let Err(e) = result { report(&tx, Some(id.clone()), e); }
                        let _=finished.send(id);
                    });
                }
                Command::Observe { id } if watched.insert(id.clone()) => {
                    jobs.spawn(async move {
                        if let Err(e) =
                            watch_session(client, id.clone(), None, tx.clone(), turns).await
                        {
                            report(&tx, Some(id.clone()), e);
                        }
                        let _ = finished.send(id);
                    });
                }
                Command::Dispatch {
                    channel,
                    action,
                    worktree,
                } => {
                    jobs.spawn(async move {
                        let result: Result<()> = async {
                            dispatch(&client, channel.clone(), action, worktree, &turns).await?;
                            Ok(())
                        }
                        .await;
                        if let Err(e) = result {
                            report(&tx, Some(channel), e);
                        }
                    });
                }
                Command::Complete { channel, text } => {
                    jobs.spawn(async move {
                    match client.request::<_, Value>("completions", json!({"channel":channel,"kind":"userMessage","text":text,"offset":text.encode_utf16().count()})).await {
                        Ok(value) => { let _ = tx.send(Event::Completions(value)); }, Err(e) => report(&tx, None, e.into()),
                    }
                });
                }
                Command::Options { worktree } => {
                    let worktree =
                        if let Some(owner) = connection.as_ref().and_then(|c| c.owner.as_ref()) {
                            workspace_trusted(owner, &worktree)
                                .await
                                .ok()
                                .filter(|v| *v)
                                .map(|_| worktree)
                        } else {
                            Some(worktree)
                        };
                    jobs.spawn(async move {
                    if let Ok(snapshot)=client.request::<_,ahp_types::commands::SubscribeResult>("subscribe",json!({"channel":ahp_types::ROOT_RESOURCE_URI})).await&&let Some(snapshot)=snapshot.snapshot&&let SnapshotState::Root(root)=snapshot.state{let _=tx.send(Event::Agents(serde_json::to_value(root.agents).unwrap_or(json!([]))));}
                    let directory = worktree.and_then(|p|url::Url::from_directory_path(p).ok()).map(String::from);
                    match client.request::<_, Value>("resolveSessionConfig", json!({"channel":ahp_types::ROOT_RESOURCE_URI,"provider":"copilot","workingDirectory":directory})).await {
                        Ok(value) => { let _ = tx.send(Event::Options(value)); }, Err(e) => report(&tx, None, e.into()),
                    }
                });
                }
                _ => {}
            }
            while jobs.try_join_next().is_some() {}
        }
        jobs.abort_all();
        while jobs.join_next().await.is_some() {}
        if let Some(c) = connection {
            c.close().await;
        }
        let _ = output.send(Event::Disconnected);
    });
    (sender, task)
}
fn command_id(command: &Command) -> Option<String> {
    match command {
        Command::Create { id, .. } | Command::Observe { id } => Some(id.clone()),
        _ => None,
    }
}
fn report(output: &mpsc::UnboundedSender<Event>, id: Option<String>, error: anyhow::Error) {
    let _ = output.send(Event::Error {
        id,
        message: format!("{error:#}"),
    });
}
async fn workspace_trusted(
    owner: &github_copilot_sdk::Client,
    path: &std::path::Path,
) -> Result<bool> {
    let result = owner
        .call(
            "permissions.folderTrust.checkTrust",
            Some(json!({"path":path})),
        )
        .await?;
    result["trusted"]
        .as_bool()
        .context("Copilot returned no folder trust verdict")
}
/// Grant native Copilot folder trust only after explicit user confirmation.
/// The compatibility probe uses this with a disposable, isolated Copilot home.
async fn trust_workspace(owner: &github_copilot_sdk::Client, path: &std::path::Path) -> Result<()> {
    // A short-lived, instruction-free broker makes the runtime's own trust API
    // available without sending a model turn or hand-editing Copilot settings.
    let config = github_copilot_sdk::SessionConfig::default()
        .with_session_id(format!("treerail-trust-{}", uuid::Uuid::new_v4()))
        .with_enable_config_discovery(false)
        .with_skip_custom_instructions(true)
        .with_enable_session_store(false)
        .with_request_extensions(false);
    let broker = owner.create_session(config).await?;
    let result = owner
        .call(
            "session.permissions.folderTrust.addTrusted",
            Some(json!({"sessionId":broker.id(),"path":path})),
        )
        .await;
    let _ = broker.disconnect().await;
    result?;
    anyhow::ensure!(
        workspace_trusted(owner, path).await?,
        "Copilot did not grant folder trust"
    );
    Ok(())
}
struct Connection {
    client: Client,
    endpoint: String,
    initialize_meta: Value,
    owner: Option<github_copilot_sdk::Client>,
    host: Option<github_copilot_sdk::AhpHost>,
}
impl Connection {
    async fn open(config: &AgentConfig, output: &mpsc::UnboundedSender<Event>) -> Result<Self> {
        let (mut endpoint, token, owner, host) = if let Some(endpoint) = &config.endpoint {
            (
                endpoint.clone(),
                config
                    .token_env
                    .as_ref()
                    .and_then(|name| std::env::var(name).ok()),
                None,
                None,
            )
        } else {
            let mut options = github_copilot_sdk::ClientOptions::default();
            options.base_directory = config.home.clone();
            if let Some(directory) = &config.host_directory {
                options.working_directory = directory.clone();
            }
            if let Some(path) = &config.runtime {
                options.program = github_copilot_sdk::CliProgram::Path(path.clone());
            } else {
                // The full native CLI bootstrap also supplies its bundled
                // extension SDK path. The bare runtime wrapper does not.
                let path = github_copilot_sdk::install_bundled_cli()
                    .context("Extract the compatible bundled Copilot CLI")?;
                options.program = github_copilot_sdk::CliProgram::Path(path);
            }
            let owner = github_copilot_sdk::Client::start(options)
                .await
                .context("Start the compatible Copilot runtime")?;
            let host = match owner
                .start_ahp_host(
                    github_copilot_sdk::AhpHostOptions::default()
                        .with_local_server(Default::default()),
                )
                .await
            {
                Ok(host) => host,
                Err(e) => {
                    let _ = owner.stop().await;
                    return Err(e).context(
                        "This Copilot runtime does not provide a compatible AHP listener",
                    );
                }
            };
            (
                host.url
                    .clone()
                    .context("AHP host returned no local endpoint")?,
                host.token.clone(),
                Some(owner),
                Some(host),
            )
        };
        if let Some(token) = token {
            let mut url = url::Url::parse(&endpoint)?;
            url.query_pairs_mut().append_pair("tkn", &token);
            endpoint = url.into();
        }
        let result: Result<(Client, Value, Value)> = async {
            let transport = ahp_ws::WebSocketTransport::connect(&endpoint)
                .await
                .context("Connect to the AHP listener")?;
            let client = Client::connect(
                transport,
                ClientConfig {
                    subscription_buffer: 8192,
                    ..Default::default()
                },
            )
            .await?;
            let init = client
                .initialize(
                    format!("treerail-{}", uuid::Uuid::new_v4()),
                    ahp_types::SUPPORTED_PROTOCOL_VERSIONS
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    vec![ahp_types::ROOT_RESOURCE_URI.into()],
                )
                .await?;
            let agents = init
                .snapshots
                .iter()
                .find_map(|s| {
                    if let SnapshotState::Root(root) = &s.state {
                        serde_json::to_value(&root.agents).ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(json!([]));
            Ok((client, agents, serde_json::to_value(init.meta)?))
        }
        .await;
        match result {
            Ok((client, agents, initialize_meta)) => {
                let _ = output.send(Event::Connected {
                    connection_url: endpoint.clone(),
                    agents,
                });
                Ok(Self {
                    client,
                    endpoint,
                    initialize_meta,
                    owner,
                    host,
                })
            }
            Err(e) => {
                if let Some(host) = host {
                    let _ = host.dispose().await;
                }
                if let Some(owner) = owner {
                    let _ = owner.stop().await;
                }
                Err(e)
            }
        }
    }
    async fn close(self) {
        self.client.shutdown().await;
        if let Some(host) = self.host {
            let _ = host.dispose().await;
        }
        if let Some(owner) = self.owner {
            let _ = owner.stop().await;
        }
    }
}
/// Forward only a GitHub credential to the advertised GitHub protected resource.
/// No credential enters application state, process arguments, logs, or UI events.
pub async fn authenticate_github(
    client: &Client,
    endpoint: &str,
    initialize_meta: &Value,
) -> Result<()> {
    let snapshot: ahp_types::commands::SubscribeResult = client
        .request("subscribe", json!({"channel":ahp_types::ROOT_RESOURCE_URI}))
        .await?;
    let Some(snapshot) = snapshot.snapshot else {
        return Ok(());
    };
    let SnapshotState::Root(root) = snapshot.state else {
        return Ok(());
    };
    let required = root
        .agents
        .iter()
        .filter(|a| a.provider == "copilot")
        .flat_map(|a| a.protected_resources.iter().flatten())
        .any(|r| r.resource == "https://api.github.com");
    if !required {
        return Ok(());
    }
    let url = url::Url::parse(endpoint)?;
    let loopback = match url.host() {
        Some(url::Host::Domain("localhost")) => true,
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    anyhow::ensure!(
        url.scheme() == "wss" || (url.scheme() == "ws" && loopback),
        "GitHub authentication requires an encrypted or loopback AHP connection"
    );
    let mut token = ["COPILOT_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|s| !s.trim().is_empty());
    if token.is_none() {
        let mut command = tokio::process::Command::new("gh");
        command
            .args(["auth", "token", "--hostname", "github.com"])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        if let Ok(Ok(output)) = tokio::time::timeout(Duration::from_secs(5), command.output()).await
            && output.status.success()
        {
            token = String::from_utf8(output.stdout)
                .ok()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty());
        }
    }
    let token=token.context("Sign in with 'gh auth login', or set COPILOT_GITHUB_TOKEN, then retry. AHP requires a GitHub credential for this connection")?;
    let sealed = sealed_auth::token(
        initialize_meta,
        &serde_json::to_value(&root.meta)?,
        "https://api.github.com",
        &token,
    )?;
    let result:Result<Value,_>=client.request("authenticate",json!({"channel":ahp_types::ROOT_RESOURCE_URI,"resource":"https://api.github.com","token":sealed})).await;
    if let Err(error) = result {
        let message = error
            .to_string()
            .replace(&token, "[redacted]")
            .replace(&sealed, "[redacted]");
        anyhow::bail!(
            "GitHub sign-in failed: {message}. Refresh your GitHub login or COPILOT_GITHUB_TOKEN and retry"
        );
    }
    Ok(())
}
async fn catalog(client: &Client, output: &mpsc::UnboundedSender<Event>) -> Result<()> {
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let result: Value = client
            .request(
                "listSessions",
                json!({"channel":ahp_types::ROOT_RESOURCE_URI,"cursor":cursor}),
            )
            .await?;
        items.extend(result["items"].as_array().into_iter().flatten().cloned());
        cursor = result["nextCursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    let _ = output.send(Event::Catalog(items));
    Ok(())
}
async fn watch_session(
    client: Client,
    id: String,
    mut prompt: Option<(String, Option<String>)>,
    output: mpsc::UnboundedSender<Event>,
    leases: TurnLeases,
) -> Result<()> {
    let (snapshot, mut subscription) = client.subscribe(id.clone()).await?;
    let snapshot = snapshot
        .snapshot
        .context("Session subscription has no snapshot")?;
    let SnapshotState::Session(mut state) = snapshot.state else {
        anyhow::bail!("Expected session snapshot");
    };
    let mut sequence = snapshot.from_seq;
    let mut chats = BTreeSet::new();
    let mut watchers = tokio::task::JoinSet::new();
    let mut refresh = tokio::time::interval(Duration::from_secs(5));
    loop {
        let _ = output.send(Event::Session {
            id: id.clone(),
            state: state.clone(),
        });
        if state.lifecycle == ahp_types::state::SessionLifecycle::Failed {
            anyhow::bail!("Session creation failed: {:?}", state.creation_error);
        }
        for chat in &state.chats {
            if chats.insert(chat.resource.clone()) {
                let first_prompt = if Some(&chat.resource) == state.default_chat.as_ref() {
                    prompt.take()
                } else {
                    None
                };
                let c = client.clone();
                let tx = output.clone();
                let session = id.clone();
                let uri = chat.resource.clone();
                let turns = leases.clone();
                let worktree = state
                    .working_directories
                    .as_ref()
                    .and_then(|d| d.first())
                    .and_then(|d| url::Url::parse(d).ok())
                    .and_then(|d| d.to_file_path().ok());
                watchers.spawn(async move {
                    watch_chat(c, session, uri, first_prompt, tx, worktree, turns).await
                });
            }
        }
        tokio::select! {
            Some(result) = watchers.join_next() => { result??; },
            event = subscription.recv() => match event {
                Some(SubscriptionEvent::Action(envelope)) if envelope.server_seq > sequence as u64 => { if let Some(reason)=envelope.rejection_reason{report(&output,Some(id.clone()),anyhow::anyhow!("Agent rejected the action: {reason}"));}else{ahp::reducers::apply_action_to_session(&mut state, &envelope.action);} sequence = i64::try_from(envelope.server_seq)?; },
                Some(SubscriptionEvent::AuthRequired(_)) => anyhow::bail!("Copilot authentication is required; refresh 'gh auth login' or COPILOT_GITHUB_TOKEN and resume"),
                None => anyhow::bail!("AHP session connection closed"), _ => {}
            },
            _ = refresh.tick() => {
                let fresh: ahp_types::commands::SubscribeResult = client.request("subscribe", json!({"channel":id})).await?;
                if let Some(snapshot) = fresh.snapshot && snapshot.from_seq >= sequence && let SnapshotState::Session(s) = snapshot.state { state = s; sequence = snapshot.from_seq; }
            }
        }
    }
}
async fn watch_chat(
    client: Client,
    session: String,
    uri: String,
    prompt: Option<(String, Option<String>)>,
    output: mpsc::UnboundedSender<Event>,
    worktree: Option<PathBuf>,
    leases: TurnLeases,
) -> Result<()> {
    let (snapshot, mut subscription) = client.subscribe(uri.clone()).await?;
    let snapshot = snapshot
        .snapshot
        .context("Chat subscription has no snapshot")?;
    let SnapshotState::Chat(mut state) = snapshot.state else {
        anyhow::bail!("Expected chat snapshot");
    };
    let mut sequence = snapshot.from_seq;
    if let Some((prompt, model)) = prompt {
        let mut action = turn_action(&prompt, None);
        if let Some(model) = model {
            action["message"]["model"] = json!({"id":model});
        }
        dispatch(&client, uri.clone(), action, worktree.clone(), &leases).await?;
    }
    let mut observed = None;
    let mut refresh = tokio::time::interval(Duration::from_secs(5));
    loop {
        // CLI and VS Code turns arrive directly through AHP. Guard them once
        // observed too; this is not an atomic fence against an external client.
        if let Some(turn) = &state.active_turn
            && observed.as_deref() != Some(turn.id.as_str())
            && !leases.lock().await.contains_key(&uri)
            && let Some(path) = &worktree
        {
            observed = Some(turn.id.clone());
            match crate::checkout::Lease::acquire(path).await {
                Ok(lease) => {
                    leases.lock().await.insert(
                        uri.clone(),
                        TurnLease {
                            id: turn.id.clone(),
                            _lease: lease,
                        },
                    );
                }
                Err(error) => report(
                    &output,
                    Some(session.clone()),
                    error.context(
                        "AHP turn active; checkout guard belongs to another Tree Rail operation",
                    ),
                ),
            }
        }
        {
            let mut held = leases.lock().await;
            if held
                .get(&uri)
                .is_some_and(|turn| state.turns.iter().any(|t| t.id == turn.id))
            {
                held.remove(&uri);
            }
        }
        let _ = output.send(Event::Chat {
            session: session.clone(),
            state: state.clone(),
        });
        tokio::select! {
            event = subscription.recv() => match event {
                Some(SubscriptionEvent::Action(envelope)) if envelope.server_seq > sequence as u64 => { if let Some(reason)=envelope.rejection_reason{if let StateAction::ChatTurnStarted(turn)=&envelope.action{let mut held=leases.lock().await;if held.get(&uri).is_some_and(|lease|lease.id==turn.turn_id){held.remove(&uri);}}report(&output,Some(session.clone()),anyhow::anyhow!("Agent rejected the action: {reason}"));}else{ahp::reducers::apply_action_to_chat(&mut state, &envelope.action);} sequence = i64::try_from(envelope.server_seq)?; },
                None => anyhow::bail!("AHP conversation connection closed"), _ => {}
            },
            _ = refresh.tick() => {
                let fresh: ahp_types::commands::SubscribeResult = client.request("subscribe", json!({"channel":uri})).await?;
                if let Some(snapshot) = fresh.snapshot && snapshot.from_seq >= sequence && let SnapshotState::Chat(s) = snapshot.state { state = s; sequence = snapshot.from_seq; }
            }
        }
    }
}
struct TurnLease {
    id: String,
    _lease: crate::checkout::Lease,
}
type TurnLeases = Arc<tokio::sync::Mutex<BTreeMap<String, TurnLease>>>;
async fn dispatch(
    client: &Client,
    channel: String,
    action: Value,
    worktree: Option<PathBuf>,
    leases: &TurnLeases,
) -> Result<()> {
    let parsed = serde_json::from_value::<StateAction>(action.clone())?;
    if action["type"] == "chat/turnStarted" {
        let path = worktree.context("A worktree is required to start a turn")?;
        let lease = crate::checkout::Lease::acquire(&path).await?;
        leases.lock().await.insert(
            channel.clone(),
            TurnLease {
                id: action["turnId"]
                    .as_str()
                    .context("Turn has no identity")?
                    .into(),
                _lease: lease,
            },
        );
    }
    client.dispatch(channel, parsed).await?;
    Ok(())
}
pub fn turn_action(text: &str, attachments: Option<Value>) -> Value {
    json!({"type":"chat/turnStarted","turnId":uuid::Uuid::new_v4().to_string(),"startedAt":timestamp(),"message":{"text":text,"origin":{"kind":"user"},"attachments":attachments}})
}
fn timestamp() -> String {
    jiff::Timestamp::now().to_string()
}
