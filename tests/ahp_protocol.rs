//! Exercise the production AHP adapter through a real loopback WebSocket host.
//! This host supplies deterministic responses; it never contacts a model.
use ahp_types::{actions::StateAction, state::ChatState};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tempfile::TempDir;
use tokio::{
    net::TcpListener,
    sync::{Mutex, mpsc},
};
use tokio_tungstenite::tungstenite::Message;
use treerail::{
    agent::{self, Command, Event},
    config::AgentConfig,
    git,
};

struct HostState {
    chat: ChatState,
    session: Value,
    sequence: u64,
}
async fn receive_until(
    rx: &mut mpsc::UnboundedReceiver<Event>,
    predicate: impl Fn(&Event) -> bool,
) -> Event {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let event = rx.recv().await.expect("adapter ended");
            if predicate(&event) {
                return event;
            }
            if let Event::Error { message, .. } = event {
                panic!("AHP adapter failed: {message}");
            }
        }
    })
    .await
    .expect("AHP event timed out")
}
#[tokio::test]
async fn create_stream_stop_and_resume_use_ahp_without_replaying_the_prompt() {
    let temp = TempDir::new().unwrap();
    git::run(temp.path(), ["init", "--initial-branch=develop"])
        .await
        .unwrap();
    let uri = "ahp-session:/fixture";
    let chat_uri = "ahp-chat:/fixture";
    let directory = url::Url::from_directory_path(temp.path())
        .unwrap()
        .to_string();
    let chat:ChatState=serde_json::from_value(json!({"resource":chat_uri,"title":"Fixture","status":1,"modifiedAt":"2026-10-21T10:00:00Z","turns":[]})).unwrap();
    let session = json!({"provider":"copilot","title":"Fixture","status":1,"lifecycle":"ready","activeClients":[],"workingDirectories":[directory],"chats":[{"resource":chat_uri,"title":"Fixture","status":1,"modifiedAt":"2026-10-21T10:00:00Z"}],"defaultChat":chat_uri});
    let state = Arc::new(Mutex::new(HostState {
        chat,
        session,
        sequence: 0,
    }));
    let prompts = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server_state = state.clone();
    let counter = prompts.clone();
    let server = tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let state = server_state.clone();
            let counter = counter.clone();
            tokio::spawn(async move {
                let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
                while let Some(Ok(Message::Text(text))) = ws.next().await {
                    let request: Value = serde_json::from_str(&text).unwrap();
                    let method = request["method"].as_str().unwrap_or("");
                    let params = &request["params"];
                    let mut state = state.lock().await;
                    let result = match method {
                        "initialize" => {
                            json!({"protocolVersion":ahp_types::SUPPORTED_PROTOCOL_VERSIONS[0],"serverSeq":state.sequence,"snapshots":[{"resource":"ahp-root://","state":{"agents":[]},"fromSeq":state.sequence}]})
                        }
                        "ping" => Value::Null,
                        "listSessions" => {
                            json!({"items":[{"resource":uri,"provider":"copilot","title":"Fixture","status":state.chat.status,"workingDirectories":state.session["workingDirectories"],"createdAt":"2026-10-21T10:00:00Z","modifiedAt":"2026-10-21T10:00:00Z","defaultChat":chat_uri}]})
                        }
                        "createSession" => {
                            assert!(params["config"].get("_treerailModel").is_none());
                            assert_eq!(
                                params["workingDirectories"],
                                state.session["workingDirectories"]
                            );
                            Value::Null
                        }
                        "subscribe" => {
                            let resource = params["channel"].as_str().unwrap();
                            let snapshot = if resource == "ahp-root://" {
                                json!({"agents":[]})
                            } else if resource == chat_uri {
                                serde_json::to_value(&state.chat).unwrap()
                            } else {
                                let mut s = state.session.clone();
                                s["status"] = json!(state.chat.status);
                                s
                            };
                            json!({"snapshot":{"resource":resource,"state":snapshot,"fromSeq":state.sequence}})
                        }
                        "dispatchAction" => {
                            let action: StateAction =
                                serde_json::from_value(params["action"].clone()).unwrap();
                            let first = matches!(&action, StateAction::ChatTurnStarted(_));
                            if first {
                                assert_eq!(
                                    params["action"]["message"]["model"]["id"],
                                    "fixture-model"
                                );
                                counter.fetch_add(1, Ordering::SeqCst);
                            }
                            ahp::reducers::apply_action_to_chat(&mut state.chat, &action);
                            state.sequence += 1;
                            let envelope = json!({"jsonrpc":"2.0","method":"action","params":{"channel":chat_uri,"serverSeq":state.sequence,"action":action}});
                            ws.send(Message::Text(envelope.to_string().into()))
                                .await
                                .unwrap();
                            if let StateAction::ChatTurnStarted(turn) = action {
                                let part = json!({"type":"chat/responsePart","turnId":turn.turn_id,"part":{"kind":"markdown","id":"text-1","content":"Hello "}});
                                let delta = json!({"type":"chat/delta","turnId":turn.turn_id,"partId":"text-1","content":"from AHP"});
                                for action in [part, delta] {
                                    let typed: StateAction =
                                        serde_json::from_value(action.clone()).unwrap();
                                    ahp::reducers::apply_action_to_chat(&mut state.chat, &typed);
                                    state.sequence += 1;
                                    ws.send(Message::Text(json!({"jsonrpc":"2.0","method":"action","params":{"channel":chat_uri,"serverSeq":state.sequence,"action":action}}).to_string().into())).await.unwrap();
                                }
                            }
                            continue;
                        }
                        _ => continue,
                    };
                    if let Some(id) = request.get("id") {
                        ws.send(Message::Text(
                            json!({"jsonrpc":"2.0","id":id,"result":result})
                                .to_string()
                                .into(),
                        ))
                        .await
                        .unwrap();
                    }
                }
            });
        }
    });
    let config = AgentConfig {
        endpoint: Some(format!("ws://{address}")),
        ..Default::default()
    };
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (commands, task) = agent::spawn(config.clone(), tx);
    commands
        .send(Command::Create {
            id: uri.into(),
            worktree: temp.path().into(),
            prompt: "The only initial prompt".into(),
            config: json!({"_treerailModel":"fixture-model"}),
        })
        .unwrap();
    let event=receive_until(&mut rx,|e|matches!(e,Event::Chat{state,..} if state.active_turn.as_ref().is_some_and(|t|t.response_parts.iter().any(|p|matches!(p,ahp_types::state::ResponsePart::Markdown(m) if m.content=="Hello from AHP"))))).await;
    let Event::Chat { state, .. } = event else {
        unreachable!()
    };
    let turn = state.active_turn.unwrap();
    assert!(
        git::stage(temp.path(), &[], false).await.is_err(),
        "Running agent must exclude Git writes across clients"
    );
    commands
        .send(Command::Dispatch {
            channel: chat_uri.into(),
            worktree: None,
            action: json!({"type":"chat/turnCancelled","turnId":turn.id,"duration":1}),
        })
        .unwrap();
    receive_until(&mut rx,|e|matches!(e,Event::Chat{state,..} if state.active_turn.is_none()&&!state.turns.is_empty())).await;
    git::stage(temp.path(), &[], false).await.unwrap();
    // A separate client represents the real CLI. Its turn bypasses the
    // adapter's dispatch path and is reconciled through the AHP snapshot.
    let remote = ahp::Client::connect(
        ahp_ws::WebSocketTransport::connect(&format!("ws://{address}"))
            .await
            .unwrap(),
        Default::default(),
    )
    .await
    .unwrap();
    remote
        .initialize(
            "cli-fixture".into(),
            ahp_types::SUPPORTED_PROTOCOL_VERSIONS
                .iter()
                .map(ToString::to_string)
                .collect(),
            vec![],
        )
        .await
        .unwrap();
    let mut action = agent::turn_action("CLI turn", None);
    action["message"]["model"] = json!({"id":"fixture-model"});
    let remote_turn = action["turnId"].as_str().unwrap().to_owned();
    remote
        .dispatch(chat_uri.into(), serde_json::from_value(action).unwrap())
        .await
        .unwrap();
    receive_until(&mut rx,|e|matches!(e,Event::Chat{state,..} if state.active_turn.as_ref().is_some_and(|t|t.id==remote_turn))).await;
    assert!(
        git::stage(temp.path(), &[], false).await.is_err(),
        "Observed CLI turns must hold the checkout guard"
    );
    remote
        .dispatch(
            chat_uri.into(),
            serde_json::from_value(
                json!({"type":"chat/turnCancelled","turnId":remote_turn,"duration":1}),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    receive_until(
        &mut rx,
        |e| matches!(e,Event::Chat{state,..} if state.active_turn.is_none()&&state.turns.len()==2),
    )
    .await;
    git::stage(temp.path(), &[], false).await.unwrap();
    remote.shutdown().await;
    commands.send(Command::Shutdown).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(prompts.load(Ordering::SeqCst), 2);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (commands, task) = agent::spawn(config, tx);
    commands.send(Command::Observe { id: uri.into() }).unwrap();
    receive_until(
        &mut rx,
        |e| matches!(e,Event::Chat{state,..} if !state.turns.is_empty()),
    )
    .await;
    assert_eq!(
        prompts.load(Ordering::SeqCst),
        2,
        "Resume must not replay the initial prompt"
    );
    commands.send(Command::Shutdown).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    server.abort();
}
