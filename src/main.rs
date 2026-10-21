use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event,
    },
    execute,
};
use std::{
    io::{self, IsTerminal},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;
use treerail::{
    agent,
    app::{App, Completion},
    discover,
    startup::Options,
    ui,
};

#[tokio::main]
async fn main() -> Result<()> {
    let options = Options::parse();
    let listing = options.list;
    if !listing {
        anyhow::ensure!(
            io::stdin().is_terminal() && io::stdout().is_terminal(),
            "Tree Rail requires an interactive terminal. Use --list for noninteractive discovery."
        );
    }
    // Config, state, known projects and supplied paths are loaded before the
    // terminal is initialized. Discovery and provider startup happen afterwards.
    let session = options.prepare()?;
    if listing {
        let (projects, warnings) = discover::discover(session.roots, session.recursive).await?;
        println!("{}", serde_json::to_string_pretty(&projects)?);
        for warning in warnings {
            eprintln!("{warning}");
        }
        return Ok(());
    }
    let mut app = App::new(session);
    let mut terminal = ratatui::init();
    let guard = TerminalGuard;
    execute!(io::stdout(), EnableBracketedPaste, EnableMouseCapture)?;
    let result = run(&mut terminal, &mut app).await;
    drop(guard);
    let saved = app.save();
    result?;
    saved?;
    Ok(())
}
struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableBracketedPaste, DisableMouseCapture);
        ratatui::restore();
    }
}
async fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<(String, Result<Completion>)>();
    let (event_tx, mut event_rx) = mpsc::channel::<Result<Event>>(64);
    let (feedback_tx, mut feedback_rx) = mpsc::channel(128);
    let (agent_tx, mut agent_rx) = mpsc::unbounded_channel();
    let (commands, mut provider) = agent::spawn(
        app.config.agent_for_sources(&app.roots, &app.projects),
        agent_tx,
    );
    let running = Arc::new(AtomicBool::new(true));
    let input_running = running.clone();
    let input = std::thread::spawn(move || {
        while input_running.load(Ordering::Relaxed) {
            match event::poll(Duration::from_millis(50)) {
                Ok(true) => {
                    let event = event::read().map_err(Into::into);
                    if event_tx.blocking_send(event).is_err() {
                        break;
                    }
                }
                Ok(false) => {}
                Err(e) => {
                    let _ = event_tx.blocking_send(Err(e.into()));
                    break;
                }
            }
        }
    });
    let mut jobs = tokio::task::JoinSet::new();
    let mut ticker = tokio::time::interval(Duration::from_millis(33));
    let result:Result<()>=async{
        while !app.quit{
            for job in std::mem::take(&mut app.jobs){let sender=tx.clone();let token=job.token();let feedback=feedback_tx.clone();jobs.spawn(async move{
                let result=if token=="mutation"{treerail::feedback::scope(treerail::feedback::Reporter{id:0,sender:feedback},job.execute()).await}else{job.execute().await};
                let _=sender.send((token,result));
            });}
            for command in std::mem::take(&mut app.agent_commands){let _=commands.send(command);}
            terminal.draw(|frame|ui::draw(frame,app))?;
            tokio::select!{
                Some(event)=event_rx.recv()=>match event?{Event::Key(key)=>app.key(key),Event::Paste(text)=>app.paste(&text),Event::Mouse(mouse)=>app.mouse(mouse),_=>{}},
                Some((token,result))=rx.recv()=>app.accept(token,result),
                Some(event)=agent_rx.recv()=>app.agent_event(event),
                Some(event)=feedback_rx.recv()=>{match event.update{treerail::feedback::Update::Output{text,..}=>{app.operation_output.push_str(&text);if app.operation_output.len()>65536{let mut split=app.operation_output.len()-65536;while !app.operation_output.is_char_boundary(split){split+=1;}app.operation_output.drain(..split);}},treerail::feedback::Update::Phase(phase)=>{if let Some((_,label))=&mut app.mutation{*label=phase;}},_=>{}}},
                _=ticker.tick()=>app.tick(),
            }
            while jobs.try_join_next().is_some(){}
        }
        Ok(())
    }.await;
    running.store(false, Ordering::Relaxed);
    drop(event_rx);
    let _ = input.join();
    // Disconnect the terminal clients before shutting down their shared host.
    app.terminals.shutdown();
    // A Git mutation is allowed to finish rather than being killed halfway
    // through a commit, publication, or checkout removal on UI exit.
    if app.mutation.is_none() {
        jobs.abort_all();
    }
    while jobs.join_next().await.is_some() {}
    while let Ok((token, result)) = rx.try_recv() {
        app.accept(token, result);
    }
    for conversation in app
        .state
        .conversations
        .iter()
        .filter(|c| c.owned && c.attention.busy())
    {
        if let Some(turn) = conversation
            .chat
            .as_ref()
            .and_then(|c| c.active_turn.as_ref())
            && let Some(channel) = &conversation.chat_uri
        {
            let _=commands.send(agent::Command::Dispatch{channel:channel.clone(),worktree:None,action:serde_json::json!({"type":"chat/turnCancelled","turnId":turn.id,"duration":0})});
        }
    }
    let stop_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while app.state.conversations.iter().any(|c| {
        c.owned
            && c.attention.busy()
            && c.chat
                .as_ref()
                .is_some_and(|chat| chat.active_turn.is_some())
    }) {
        match tokio::time::timeout_at(stop_deadline, agent_rx.recv()).await {
            Ok(Some(event)) => app.agent_event(event),
            _ => break,
        }
    }
    let _ = commands.send(agent::Command::Shutdown);
    if tokio::time::timeout(Duration::from_secs(30), &mut provider)
        .await
        .is_err()
    {
        provider.abort();
        let _ = provider.await;
    }
    while let Ok(event) = agent_rx.try_recv() {
        app.agent_event(event);
    }
    result
}
