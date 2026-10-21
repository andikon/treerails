//! Render the production Ratatui UI with illustrative data, without starting
//! providers, writing application state, or changing any repository.
use anyhow::Result;
use ratatui::{Terminal, backend::TestBackend, style::Color};
use serde_json::json;
use std::{fs, path::PathBuf};
use treerail::{
    agent::Conversation,
    app::{Action, App, Completion, View},
    config::{Config, State},
    git::{ChangedFile, Project, Snapshot, Worktree},
    startup::Session,
    ui,
};
fn fixture() -> App {
    let project = Project {
        name: "treerails".into(),
        path: PathBuf::from("C:/Code/treerails"),
    };
    let path = PathBuf::from("C:/Code/treerails.worktrees/worktree-search");
    let mut app = App::new(Session {
        config: Config::default(),
        state: State::default(),
        config_path: PathBuf::from("illustrative/config.toml"),
        roots: vec![],
        recursive: false,
    });
    app.projects = vec![
        project.clone(),
        Project {
            name: "shop-api".into(),
            path: "C:/Code/shop-api".into(),
        },
    ];
    app.open_project(0);
    let trees = vec![
        Worktree {
            path: project.path.clone(),
            branch: "develop".into(),
            head: "a1e35cbb".into(),
            original: true,
            locked: false,
            prunable: false,
        },
        Worktree {
            path: path.clone(),
            branch: "feat/worktree-search".into(),
            head: "b7c42de0".into(),
            original: false,
            locked: false,
            prunable: false,
        },
        Worktree {
            path: "C:/Code/treerails.worktrees/config-paths".into(),
            branch: "fix/config-paths".into(),
            head: "f03be231".into(),
            original: false,
            locked: false,
            prunable: false,
        },
    ];
    app.accept(
        format!("trees:{}", project.path.display()),
        Ok(Completion::Trees(project.clone(), trees)),
    );
    app.action(Action::Tree(1));
    let conversation:Conversation=serde_json::from_value(json!({"id":"ahp-session:/example","project":project.path,"worktree":path,"title":"Add worktree search","session_uri":"ahp-session:/example","chat_uri":"ahp-chat:/example","attention":"Running","owned":true,"chat":{"resource":"ahp-chat:/example","title":"Add worktree search","status":8,"modifiedAt":"2026-10-21T10:00:00Z","turns":[],"activeTurn":{"id":"turn-1","startedAt":"2026-10-21T10:00:00Z","message":{"text":"Add fuzzy search to worktree names and branches. Keep the selected checkout stable.","origin":{"kind":"user"}},"responseParts":[{"kind":"markdown","id":"response-1","content":"I’ll use the existing worktree list, match names and branches, and preserve selection during refresh.\n\n```rust\nlet filtered = visible_worktrees\n    .iter()\n    .filter(|tree| matches_query(tree, query));\n```\n\nRunning focused tests…"}]}}})).unwrap();
    app.state.conversations.push(conversation);
    let mut second = app.state.conversations[0].clone();
    second.id = "review".into();
    second.title = "Review search UX".into();
    second.attention = treerail::agent::Attention::Ready;
    second.chat = None;
    app.state.conversations.push(second);
    app.action(Action::Session("ahp-session:/example".into()));
    app.connected = true;
    app.snapshots.insert(
        path,
        Snapshot {
            branch: "feat/worktree-search".into(),
            upstream: None,
            ahead: 2,
            behind: 0,
            additions: 42,
            deletions: 9,
            files: vec![
                ChangedFile {
                    path: "src/ui/worktrees.rs".into(),
                    original: None,
                    index: 'M',
                    work: ' ',
                    untracked: false,
                },
                ChangedFile {
                    path: "src/app.rs".into(),
                    original: None,
                    index: ' ',
                    work: 'M',
                    untracked: false,
                },
                ChangedFile {
                    path: "tests/worktree_search.rs".into(),
                    original: None,
                    index: '?',
                    work: '?',
                    untracked: true,
                },
            ],
        },
    );
    app
}
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn color(value: Color, background: bool) -> String {
    match value {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Black => "#000000".into(),
        Color::White => "#ffffff".into(),
        _ => if background { "#11161c" } else { "#dce5e7" }.into(),
    }
}
fn capture(app: &mut App, dir: &std::path::Path, name: &str) -> Result<()> {
    let mut terminal = Terminal::new(TestBackend::new(128, 36))?;
    terminal.draw(|frame| ui::draw(frame, app))?;
    let buffer = terminal.backend().buffer();
    let mut svg = String::from(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1280\" height=\"720\" viewBox=\"0 0 1280 720\"><title>Tree Rail native TUI</title><rect width=\"1280\" height=\"720\" fill=\"#11161c\"/><g font-family=\"Cascadia Mono,Consolas,monospace\" font-size=\"15\">",
    );
    for y in 0..36u16 {
        for x in 0..128u16 {
            let cell = &buffer[(x, y)];
            let bg = color(cell.bg, true);
            if bg != "#11161c" {
                svg.push_str(&format!(
                    "<rect x=\"{}\" y=\"{}\" width=\"10\" height=\"20\" fill=\"{}\"/>",
                    x * 10,
                    y * 20,
                    bg
                ));
            }
            if cell.symbol() != " " {
                svg.push_str(&format!(
                    "<text x=\"{}\" y=\"{}\" fill=\"{}\">{}</text>",
                    x * 10,
                    y * 20 + 15,
                    color(cell.fg, false),
                    xml(cell.symbol())
                ));
            }
        }
    }
    svg.push_str("</g></svg>");
    fs::write(dir.join(format!("worktree-{name}.svg")), svg)?;
    Ok(())
}
fn main() -> Result<()> {
    let directory = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| "dist/previews".into());
    fs::create_dir_all(&directory)?;
    let mut app = fixture();
    capture(&mut app, &directory, "sessions")?;
    app.action(Action::Switch(View::Changes));
    app.diff_staged = true;
    app.diff="@@ -218,8 +218,13 @@ impl App {\n fn filter_worktrees(&mut self, query: &str) {\n-    self.visible = self.worktrees.clone();\n+    self.visible = self.worktrees.iter()\n+        .filter(|tree| matches_query(tree, query))\n+        .cloned()\n+        .collect();\n+    self.selection.retain_checkout(&self.visible);\n }".into();
    capture(&mut app, &directory, "changes")?;
    app.action(Action::Switch(View::Sessions));
    app.action(Action::NewSessionOptions);
    capture(&mut app, &directory, "create-session")?;
    println!("Rendered native TUI previews in {}", directory.display());
    Ok(())
}
