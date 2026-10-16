use anyhow::{Context, Result};
use clap::Parser;
use crossterm::{
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event},
    execute,
};
use std::{
    io::{self, IsTerminal},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Semaphore, mpsc};
use treerail::{
    app::{App, Completion},
    config::{self, Config, State},
    discover, ui,
};

#[derive(Parser, Debug)]
#[command(version, about = "A keyboard-first project and Git worktree explorer")]
struct Args {
    /// Repositories or parent folders to explore (combined with saved roots)
    paths: Vec<PathBuf>,
    /// File containing one project source directory per line
    #[arg(long)]
    paths_file: Option<PathBuf>,
    /// Discover repositories recursively (excluding .worktrees folders)
    #[arg(long)]
    recursive: bool,
    /// Configuration file; defaults to the OS application configuration directory
    #[arg(long)]
    config: Option<PathBuf>,
    /// Override remembered-state file
    #[arg(long)]
    state: Option<PathBuf>,
    /// Print discovered projects as JSON without opening the TUI
    #[arg(long)]
    list: bool,
}
#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let (default_config, default_state) = config::paths()?;
    let config_path = args.config.unwrap_or(default_config);
    let state_path = args.state.unwrap_or(default_state);
    let config = Config::load(&config_path)?;
    let recursive = args.recursive || config.recursive;
    let mut roots = config.project_roots.clone();
    roots.extend(args.paths);
    if let Some(path) = args.paths_file {
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Read paths file {}", path.display()))?;
        let parent = path.parent().unwrap_or(std::path::Path::new("."));
        roots.extend(
            content
                .trim_start_matches('\u{feff}')
                .lines()
                .map(str::trim)
                .filter(|s| !s.is_empty() && !s.starts_with('#'))
                .map(|s| {
                    let p = PathBuf::from(s);
                    if p.is_absolute() { p } else { parent.join(p) }
                }),
        );
    }
    if roots.is_empty() {
        roots.push(std::env::current_dir()?);
    }
    let cwd = std::env::current_dir()?;
    for root in &mut roots {
        if root.is_relative() {
            *root = cwd.join(&*root);
        }
    }
    if args.list {
        let (projects, warnings) = discover::discover(roots, recursive).await?;
        println!("{}", serde_json::to_string_pretty(&projects)?);
        for warning in warnings {
            eprintln!("{warning}");
        }
        return Ok(());
    }
    anyhow::ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "Tree Rail requires an interactive terminal. Use --list for noninteractive discovery."
    );
    let state = State::load(&state_path)?;
    let mut app = App::new(config, state, config_path, state_path, roots, recursive);
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
    let (feedback_tx, mut feedback_rx) = mpsc::channel(128);
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
                let sender=tx.clone();let limiter=semaphore.clone();let epoch=app.epoch;let token=job.token();let feedback=feedback_tx.clone();
                tokio::spawn(async move{let _permit=limiter.acquire_owned().await;let result=job.execute_with_feedback(feedback).await;let _=sender.send(Completion{epoch,token,result}).await;});
            }
            app.viewport_height=terminal.size()?.height as usize;
            app.viewport_width=terminal.size()?.width as usize;
            terminal.draw(|frame|ui::draw(frame,app))?;
            tokio::select!{
                Some(event)=event_rx.recv()=>match event?{Event::Key(key)=>app.key(key),Event::Paste(text)=>app.paste(&text),_=>{}},
                Some(done)=rx.recv()=>{while let Ok(event)=feedback_rx.try_recv(){app.accept_feedback(event);}app.accept(done)},
                Some(event)=feedback_rx.recv()=>app.accept_feedback(event),
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
