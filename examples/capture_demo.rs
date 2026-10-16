//! Capture the real UI and keyboard handlers against disposable Git repositories.
//! No GitHub connection, Copilot launch, or changes to user repositories occur.
use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Terminal,
    backend::TestBackend,
    style::{Color, Modifier},
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use treerail::{
    app::{App, Completion, Job, Modal},
    config::{Config, State},
    git, ui,
};

const WIDTH: u16 = 144;
const HEIGHT: u16 = 34;
#[derive(Serialize)]
struct Cell {
    text: String,
    fg: Option<[u8; 3]>,
    bg: Option<[u8; 3]>,
    bold: bool,
}
#[derive(Serialize)]
struct Frame {
    title: String,
    detail: String,
    keys: String,
    duration_ms: u32,
    still: Option<String>,
    cells: Vec<Cell>,
}
#[derive(Serialize)]
struct Recording {
    columns: u16,
    rows: u16,
    frames: Vec<Frame>,
}

fn key(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}
fn control(app: &mut App, c: char) {
    app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
}
async fn settle(app: &mut App) -> Result<()> {
    for _ in 0..20 {
        let jobs = std::mem::take(&mut app.jobs);
        if jobs.is_empty() {
            return Ok(());
        }
        for job in jobs {
            let token = job.token();
            let epoch = app.epoch;
            let result = job.execute().await;
            if let Err(error) = &result {
                anyhow::bail!("Demo job {token}: {error:#}");
            }
            app.accept(Completion {
                epoch,
                token,
                result,
            });
        }
    }
    anyhow::bail!("Demo did not settle")
}
async fn refresh(app: &mut App) -> Result<()> {
    for tree in app.trees.clone() {
        app.enqueue(Job::Status(tree.path));
    }
    settle(app).await
}
async fn repository(root: &Path, name: &str) -> Result<PathBuf> {
    let path = root.join(name);
    fs::create_dir_all(path.join("src/routes"))?;
    fs::create_dir_all(path.join("notes"))?;
    git::run(&path, ["init", "--initial-branch=main"]).await?;
    for (setting, value) in [
        ("user.name", "Tree Rail Demo"),
        ("user.email", "demo@example.invalid"),
        ("commit.gpgsign", "false"),
        ("core.autocrlf", "false"),
    ] {
        git::run(&path, ["config", setting, value]).await?;
    }
    let hooks = path.join(".git/empty-hooks");
    fs::create_dir_all(&hooks)?;
    git::run(
        &path,
        [
            "config",
            "core.hooksPath",
            hooks.to_str().context("Demo path is not UTF-8")?,
        ],
    )
    .await?;
    fs::write(
        path.join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
    )?;
    fs::write(
        path.join("src/main.rs"),
        "fn main() {\n    println!(\"Service ready\");\n}\n",
    )?;
    fs::write(
        path.join("src/routes/health.rs"),
        "pub struct Health {\n    pub status: &'static str,\n}\n\npub fn health() -> Health {\n    Health { status: \"ok\" }\n}\n",
    )?;
    fs::write(
        path.join("notes/legacy-probe.txt"),
        "The original health probe.\n",
    )?;
    git::run(&path, ["add", "."]).await?;
    git::commit(&path, "Initial service", "").await?;
    Ok(path)
}
fn display_path(path: &Path, root: &Path) -> PathBuf {
    path.strip_prefix(root)
        .map(|p| {
            PathBuf::from(format!(
                r"C:\Code\{}",
                p.to_string_lossy().replace('/', r"\")
            ))
        })
        .unwrap_or_else(|_| path.to_owned())
}
fn capture(
    app: &mut App,
    root: &Path,
    frames: &mut Vec<Frame>,
    text: (&str, &str, &str),
    ms: u32,
    still: Option<&str>,
) -> Result<()> {
    let (title, detail, keys) = text;
    // Keep the real Git state and UI, but replace temporary/personal paths before rendering.
    let projects = app.projects.clone();
    let project = app.project.clone();
    let trees = app.trees.clone();
    let snapshots = std::mem::take(&mut app.snapshots);
    let modal = app.modal.clone();
    for p in &mut app.projects {
        p.path = display_path(&p.path, root);
    }
    if let Some(p) = &mut app.project {
        p.path = display_path(&p.path, root);
    }
    for tree in &mut app.trees {
        tree.path = display_path(&tree.path, root);
    }
    app.snapshots = snapshots
        .iter()
        .map(|(p, s)| (display_path(p, root), s.clone()))
        .collect::<BTreeMap<_, _>>();
    if let Some(Modal::Form(form)) = &mut app.modal {
        for field in &mut form.fields {
            let at_end = field.cursor == field.value.len();
            if field.label == "Worktree directory" {
                field.value = display_path(Path::new(&field.value), root)
                    .to_string_lossy()
                    .into_owned();
            }
            field.value = field
                .value
                .replace(root.to_string_lossy().as_ref(), r"C:\Code")
                .replace(&root.to_string_lossy().replace('\\', "/"), "C:/Code");
            if at_end {
                field.cursor = field.value.len();
            }
        }
    }
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT))?;
    let result = terminal.draw(|frame| ui::draw(frame, app));
    app.projects = projects;
    app.project = project;
    app.trees = trees;
    app.snapshots = snapshots;
    app.modal = modal;
    result?;
    let cells = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| Cell {
            text: cell.symbol().to_owned(),
            fg: rgb(cell.fg),
            bg: rgb(cell.bg),
            bold: cell.modifier.contains(Modifier::BOLD),
        })
        .collect();
    frames.push(Frame {
        title: title.into(),
        detail: detail.into(),
        keys: keys.into(),
        duration_ms: ms,
        still: still.map(str::to_owned),
        cells,
    });
    Ok(())
}
fn rgb(color: Color) -> Option<[u8; 3]> {
    let palette = [
        [11, 17, 28],
        [236, 115, 136],
        [134, 239, 172],
        [253, 230, 138],
        [122, 162, 247],
        [192, 153, 232],
        [125, 211, 252],
        [194, 203, 216],
        [108, 122, 143],
        [253, 164, 175],
        [166, 245, 189],
        [255, 238, 170],
        [160, 192, 255],
        [218, 181, 255],
        [165, 232, 255],
        [240, 244, 250],
    ];
    Some(match color {
        Color::Reset => return None,
        Color::Rgb(r, g, b) => [r, g, b],
        Color::Indexed(i) => {
            if i < 16 {
                palette[i as usize]
            } else if i < 232 {
                let n = i - 16;
                let c = [0, 95, 135, 175, 215, 255];
                [
                    c[(n / 36) as usize],
                    c[((n / 6) % 6) as usize],
                    c[(n % 6) as usize],
                ]
            } else {
                let n = 8 + (i - 232) * 10;
                [n, n, n]
            }
        }
        Color::Black => palette[0],
        Color::Red => palette[1],
        Color::Green => palette[2],
        Color::Yellow => palette[3],
        Color::Blue => palette[4],
        Color::Magenta => palette[5],
        Color::Cyan => palette[6],
        Color::Gray => palette[7],
        Color::DarkGray => palette[8],
        Color::LightRed => palette[9],
        Color::LightGreen => palette[10],
        Color::LightYellow => palette[11],
        Color::LightBlue => palette[12],
        Color::LightMagenta => palette[13],
        Color::LightCyan => palette[14],
        Color::White => palette[15],
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/readme-frames.json"));
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let repo = repository(root, "atlas-api").await?;
    repository(root, "client-portal").await?;
    repository(root, "infra-kit").await?;
    git::run(&repo, ["checkout", "-b", "develop"]).await?;
    fs::write(repo.join("src/routes/mod.rs"), "pub mod health;\n")?;
    git::run(&repo, ["add", "."]).await?;
    git::commit(&repo, "Prepare development routes", "").await?;
    git::run(&repo, ["checkout", "main"]).await?;
    let bare = root.join("origin.git");
    fs::create_dir(&bare)?;
    git::run(&bare, ["init", "--bare"]).await?;
    git::run(
        &repo,
        [
            "remote",
            "add",
            "origin",
            bare.to_str().context("Demo path is not UTF-8")?,
        ],
    )
    .await?;
    git::run(&repo, ["push", "origin", "main", "develop"]).await?;
    git::run(
        &repo,
        [
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/develop",
        ],
    )
    .await?;
    let parallel = root.join("atlas-api.worktrees/feature-session-cache");
    git::create_worktree(
        &repo,
        "feature/session-cache",
        "origin/develop",
        &parallel,
        false,
        false,
    )
    .await?;
    fs::write(
        parallel.join("src/cache.rs"),
        "pub const CACHE_TTL: u64 = 300;\n",
    )?;
    let fix = root.join("atlas-api.worktrees/fix-timeout-budget");
    git::create_worktree(&repo, "fix/timeout-budget", "main", &fix, false, false).await?;
    fs::write(
        fix.join("src/main.rs"),
        "fn main() {\n    println!(\"Timeout budget: 30s\");\n}\n",
    )?;
    let mut app = App::new(
        Config::default(),
        State::default(),
        PathBuf::from(r"C:\Code\treerail.toml"),
        root.join("state.json"),
        vec![root.into()],
        false,
    );
    app.viewport_height = HEIGHT as usize;
    let mut frames = Vec::new();
    settle(&mut app).await?;
    capture(
        &mut app,
        root,
        &mut frames,
        (
            "Find your next project",
            "Repositories and parent folders, in one searchable explorer.",
            "treerail C:\\Code",
        ),
        1800,
        Some("projects"),
    )?;
    key(&mut app, KeyCode::Enter);
    settle(&mut app).await?;
    refresh(&mut app).await?;
    key(&mut app, KeyCode::Char('f'));
    settle(&mut app).await?;
    refresh(&mut app).await?;
    capture(
        &mut app,
        root,
        &mut frames,
        (
            "Make room for parallel work",
            "Keep your original checkout and existing worktrees together.",
            "Enter",
        ),
        1700,
        None,
    )?;
    key(&mut app, KeyCode::Char('a'));
    for chunk in ["feature/", "health-", "checks"] {
        app.paste(chunk);
        capture(
            &mut app,
            root,
            &mut frames,
            (
                "Create a focused workspace",
                "Name the branch; Tree Rail fills the worktree directory.",
                "a  →  feature/health-checks",
            ),
            250,
            None,
        )?;
    }
    key(&mut app, KeyCode::Tab);
    control(&mut app, 'b');
    key(&mut app, KeyCode::Char('/'));
    app.paste("origin/develop");
    capture(
        &mut app,
        root,
        &mut frames,
        (
            "Start from the right branch",
            "Select a remote branch, local branch, tag, or explicit commit.",
            "Ctrl+b  →  /origin/develop",
        ),
        1800,
        None,
    )?;
    key(&mut app, KeyCode::Enter);
    capture(
        &mut app,
        root,
        &mut frames,
        (
            "Choose the base, branch, and directory",
            "The new worktree starts at origin/develop, independently of HEAD.",
            "Ctrl+s creates the worktree",
        ),
        2400,
        Some("create-worktree"),
    )?;
    control(&mut app, 's');
    settle(&mut app).await?;
    refresh(&mut app).await?;
    capture(
        &mut app,
        root,
        &mut frames,
        (
            "Your worktree is ready",
            "Parallel branches stay isolated, with a shared Git repository.",
            "Ctrl+s",
        ),
        1500,
        None,
    )?;
    let path = app
        .selected_tree()
        .context("Created worktree missing")?
        .path
        .clone();
    fs::create_dir_all(path.join("docs"))?;
    fs::write(
        path.join("docs/health-checks.md"),
        "# Health checks\n\nGET /health reports service readiness.\n",
    )?;
    fs::remove_file(path.join("notes/legacy-probe.txt"))?;
    fs::write(
        path.join("src/routes/health.rs"),
        "pub struct Health {\n    pub status: &'static str,\n    pub service: &'static str,\n}\n\npub fn health() -> Health {\n    Health {\n        status: \"ready\",\n        service: \"atlas-api\",\n    }\n}\n",
    )?;
    refresh(&mut app).await?;
    key(&mut app, KeyCode::Char('l'));
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('j'));
    settle(&mut app).await?;
    capture(
        &mut app,
        root,
        &mut frames,
        (
            "Every worktree, one clear view",
            "Live file status and readable diffs, alongside your parallel branches.",
            "j / k navigate    h / l focus panes",
        ),
        3200,
        Some("workspace"),
    )?;
    key(&mut app, KeyCode::Char('s'));
    settle(&mut app).await?;
    refresh(&mut app).await?;
    key(&mut app, KeyCode::Char('t'));
    settle(&mut app).await?;
    capture(
        &mut app,
        root,
        &mut frames,
        (
            "Stage only what belongs in the commit",
            "Switch between staged and unstaged changes without leaving the view.",
            "s stage    t toggle staged diff",
        ),
        2200,
        Some("staged-diff"),
    )?;
    key(&mut app, KeyCode::Char('c'));
    for chunk in ["Add service ", "metadata to ", "health checks"] {
        app.paste(chunk);
        capture(
            &mut app,
            root,
            &mut frames,
            (
                "Commit from the keyboard",
                "Write a subject and body using the built-in commit form.",
                "c commit    Tab next field",
            ),
            250,
            None,
        )?;
    }
    key(&mut app, KeyCode::Tab);
    app.paste("Return readiness details for local probes.\nKeep the existing endpoint contract.");
    capture(
        &mut app,
        root,
        &mut frames,
        (
            "Commit without losing your place",
            "Git hooks and signing keep using your normal repository configuration.",
            "Ctrl+s commit staged changes",
        ),
        2400,
        Some("commit"),
    )?;
    control(&mut app, 's');
    settle(&mut app).await?;
    refresh(&mut app).await?;
    key(&mut app, KeyCode::Char('O'));
    capture(
        &mut app,
        root,
        &mut frames,
        (
            "Your workspace. Your tools.",
            "Choose Copilot, Neovim, a shell, or a custom launch profile.",
            "O choose profile    Enter launches in a new terminal",
        ),
        3200,
        Some("launch-profiles"),
    )?;
    key(&mut app, KeyCode::Esc);
    app.say(
        "Ready for the next ticket. Tree Rail stays open while you work.",
        false,
    );
    capture(
        &mut app,
        root,
        &mut frames,
        (
            "Find a project. Pick a branch. Get to work.",
            "Tree Rail · a native terminal explorer for parallel development.",
            "treerail C:\\Code",
        ),
        2000,
        None,
    )?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        &output,
        serde_json::to_vec(&Recording {
            columns: WIDTH,
            rows: HEIGHT,
            frames,
        })?,
    )?;
    println!("Captured real UI frames: {}", output.display());
    Ok(())
}
