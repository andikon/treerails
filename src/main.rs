use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event},
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
use tokio::sync::{Semaphore, mpsc};
use treerail::{
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
    let session = options.prepare()?;
    if listing {
        let (projects, warnings) = discover::discover(session.roots, session.recursive).await?;
        println!("{}", serde_json::to_string_pretty(&projects)?);
        for warning in warnings {
            eprintln!("{warning}");
        }
        return Ok(());
    }
    let mut app = App::new(
        session.config,
        session.state,
        session.config_path,
        session.roots,
        session.recursive,
    );
    let mut terminal = ratatui::init();
    let guard = TerminalGuard;
    execute!(io::stdout(), EnableBracketedPaste)?;
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
        let _ = execute!(io::stdout(), DisableBracketedPaste);
        ratatui::restore();
    }
}
async fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    let (tx, mut rx) = mpsc::channel::<Completion>(64);
    let (event_tx, mut event_rx) = mpsc::channel::<Result<Event>>(64);
    let running = Arc::new(AtomicBool::new(true));
    let input_running = running.clone();
    let input = std::thread::spawn(move || {
        while input_running.load(Ordering::Relaxed) {
            match event::poll(Duration::from_millis(100)) {
                Ok(true) => match event::read() {
                    Ok(event) => {
                        if event_tx.blocking_send(Ok(event)).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = event_tx.blocking_send(Err(e.into()));
                        break;
                    }
                },
                Ok(false) => {}
                Err(e) => {
                    let _ = event_tx.blocking_send(Err(e.into()));
                    break;
                }
            }
        }
    });
    let semaphore = Arc::new(Semaphore::new(4));
    let mut ticker = tokio::time::interval(Duration::from_millis(100));
    let result:Result<()>=async{
        while !app.quit{
            for job in std::mem::take(&mut app.jobs){
                let sender=tx.clone();let limiter=semaphore.clone();let epoch=app.epoch;let token=job.token();
                tokio::spawn(async move{let _permit=limiter.acquire_owned().await;let result=job.execute().await;let _=sender.send(Completion{epoch,token,result}).await;});
            }
            app.viewport_height=terminal.size()?.height as usize;
            terminal.draw(|frame|ui::draw(frame,app))?;
            tokio::select!{
                Some(event)=event_rx.recv()=>match event?{Event::Key(key)=>app.key(key),Event::Paste(text)=>app.paste(&text),_=>{}},
                Some(done)=rx.recv()=>app.accept(done),
                _=ticker.tick()=>app.tick(),
            }
        }
        Ok(())
    }.await;
    running.store(false, Ordering::Relaxed);
    drop(event_rx);
    let _ = input.join();
    result
}
