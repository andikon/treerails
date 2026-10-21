use crate::{
    agent::{Attention, Conversation},
    app::{Action, App, Confirm, FieldType, FormKind, Hit, Modal, Pane, View},
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use serde_json::Value;
use unicode_width::UnicodeWidthStr;

const BG: Color = Color::Rgb(17, 22, 28);
const SURFACE: Color = Color::Rgb(23, 32, 41);
const DIM: Color = Color::Rgb(28, 39, 49);
const FG: Color = Color::Rgb(220, 229, 231);
const MUTED: Color = Color::Rgb(157, 174, 184);
const LINE: Color = Color::Rgb(58, 75, 88);
const ACCENT: Color = Color::Rgb(109, 203, 217);
const SELECTED: Color = Color::Rgb(35, 63, 74);
const GREEN: Color = Color::Rgb(142, 210, 163);
const YELLOW: Color = Color::Rgb(242, 205, 127);
const RED: Color = Color::Rgb(241, 155, 157);

pub fn draw(frame: &mut Frame, app: &mut App) {
    app.hits.clear();
    app.terminal_area = Rect::default();
    let area = frame.area();
    frame.render_widget(Block::new().style(Style::default().bg(BG).fg(FG)), area);
    if area.width < 60 || area.height < 16 {
        frame.render_widget(Paragraph::new("Tree Rail\n\nEnlarge the terminal to at least 60 columns × 16 rows.\nYour conversations and Git operations keep running.").wrap(Wrap{trim:false}).style(Style::default().fg(MUTED)),area);
        return;
    }
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(area);
    header(frame, app, rows[0]);
    let columns = Layout::horizontal([
        Constraint::Percentage(22),
        Constraint::Percentage(22),
        Constraint::Fill(1),
    ])
    .split(rows[1]);
    worktrees(frame, app, columns[0]);
    middle(frame, app, columns[1]);
    detail(frame, app, columns[2]);
    statusline(frame, app, rows[2]);
    if let Some(modal) = app.modal.clone() {
        app.hits.clear();
        overlay(frame, app, area, modal);
    }
}
fn style(color: Color) -> Style {
    Style::default().fg(color)
}
fn hit(app: &mut App, rect: Rect, action: Action) {
    if rect.width > 0 && rect.height > 0 {
        app.hits.push(Hit { rect, action });
    }
}
fn label(
    frame: &mut Frame,
    app: &mut App,
    rect: Rect,
    text: impl Into<String>,
    color: Color,
    action: Action,
) {
    frame.render_widget(Paragraph::new(text.into()).style(style(color)), rect);
    hit(app, rect, action);
}
fn fit(text: &str, width: usize) -> String {
    let clean = plain(text);
    let text = clean.as_str();
    if text.width() <= width {
        return text.into();
    }
    if width == 0 {
        return String::new();
    }
    let mut result = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        used += w;
        result.push(c);
    }
    result.push('…');
    result
}
fn header(frame: &mut Frame, app: &mut App, area: Rect) {
    frame.render_widget(
        Block::new()
            .borders(Borders::BOTTOM)
            .border_style(style(LINE))
            .style(Style::default().bg(SURFACE)),
        area,
    );
    let right = area.right().saturating_sub(24);
    let mut x = area.x;
    let current = app.selected_tab;
    // Keep the selected project reachable even when many tabs are open.
    let tabs = app
        .tabs
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let active = app
                .state
                .conversations
                .iter()
                .filter(|c| {
                    crate::config::same_path(&c.project, &t.project.path) && c.attention.busy()
                })
                .count();
            (i, t.project.name.clone(), active)
        })
        .collect::<Vec<_>>();
    let total = tabs
        .iter()
        .map(|(_, name, active)| name.width() + if *active > 0 { 7 } else { 4 })
        .sum::<usize>();
    let start = if total > right.saturating_sub(x) as usize {
        current
    } else {
        0
    };
    for (i, name, active) in tabs.into_iter().skip(start) {
        let width = (name.width() + if active > 0 { 7 } else { 4 }).min(32) as u16;
        if x + width > right.saturating_sub(2) {
            break;
        }
        let rect = Rect::new(x, area.y, width, 1);
        let selected = i == current;
        frame.render_widget(
            Block::new().style(Style::default().bg(if selected { SELECTED } else { SURFACE })),
            rect,
        );
        let text = if active > 0 {
            format!(
                " {} ●{}",
                fit(&name, width.saturating_sub(7) as usize),
                active
            )
        } else {
            format!(" {}", fit(&name, width.saturating_sub(4) as usize))
        };
        label(
            frame,
            app,
            Rect::new(x, area.y, width.saturating_sub(2), 1),
            text,
            if selected { ACCENT } else { FG },
            Action::Tab(i),
        );
        label(
            frame,
            app,
            Rect::new(x + width - 2, area.y, 2, 1),
            "× ",
            MUTED,
            Action::Close(i),
        );
        x += width;
    }
    label(
        frame,
        app,
        Rect::new(x.min(right), area.y, 2, 1),
        " +",
        MUTED,
        Action::Projects,
    );
    let changes = app.tab().is_some_and(|t| t.view == View::Changes);
    let session = Rect::new(right, area.y, 10, 1);
    let files = Rect::new(right + 10, area.y, 10, 1);
    for (rect, selected, text, view) in [
        (session, !changes, " Sessions ", View::Sessions),
        (files, changes, " Changes ", View::Changes),
    ] {
        frame.render_widget(
            Paragraph::new(text).style(
                Style::default()
                    .bg(if selected { SELECTED } else { SURFACE })
                    .fg(if selected { ACCENT } else { FG }),
            ),
            rect,
        );
        hit(app, rect, Action::Switch(view));
    }
    label(
        frame,
        app,
        Rect::new(area.right() - 3, area.y, 3, 1),
        " ? ",
        MUTED,
        Action::Help,
    );
}
fn pane(frame: &mut Frame, area: Rect, title: &str, focused: bool) -> Rect {
    frame.render_widget(
        Block::new()
            .borders(Borders::RIGHT)
            .border_style(style(LINE)),
        area,
    );
    let inner = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    let head = Rect::new(inner.x, inner.y, inner.width, 2);
    frame.render_widget(
        Block::new()
            .borders(Borders::BOTTOM)
            .border_style(style(if focused { ACCENT } else { LINE })),
        head,
    );
    frame.render_widget(
        Paragraph::new(format!(" {title}")).style(style(if focused { ACCENT } else { FG })),
        Rect::new(head.x, head.y, head.width, 1),
    );
    Rect::new(
        inner.x,
        inner.y + 2,
        inner.width,
        inner.height.saturating_sub(2),
    )
}
fn worktrees(frame: &mut Frame, app: &mut App, area: Rect) {
    let body = pane(frame, area, "Worktrees", app.pane == Pane::Worktrees);
    label(
        frame,
        app,
        Rect::new(body.right().saturating_sub(2), area.y, 2, 1),
        "+ ",
        MUTED,
        Action::NewTree,
    );
    let Some(tab) = app.tab() else {
        frame.render_widget(
            Paragraph::new("\n Choose a project\n from the explorer").style(style(MUTED)),
            body,
        );
        return;
    };
    let selected = tab.selected;
    let trees = tab.trees.clone();
    let page = (body.height as usize / 4).max(1);
    let start = selected.saturating_sub(page - 1);
    for (i, tree) in trees.iter().enumerate().skip(start).take(page) {
        let y = body.y + ((i - start) * 4) as u16;
        let rect = Rect::new(
            body.x,
            y,
            body.width,
            4.min(body.bottom().saturating_sub(y)),
        );
        let active = app
            .state
            .conversations
            .iter()
            .find(|c| crate::config::same_path(&c.worktree, &tree.path) && c.attention.busy());
        let name = if tree.original {
            tree.branch.clone()
        } else {
            tree.path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into()
        };
        let count = app
            .state
            .conversations
            .iter()
            .filter(|c| crate::config::same_path(&c.worktree, &tree.path) && !c.archived)
            .count();
        let files = app
            .snapshots
            .get(&tree.path)
            .map(|s| {
                if s.files.is_empty() {
                    "clean".into()
                } else {
                    format!("{} files", s.files.len())
                }
            })
            .unwrap_or_else(|| {
                if tree.prunable {
                    "history only".into()
                } else {
                    "…".into()
                }
            });
        let description = if tree.prunable {
            "Removed checkout".into()
        } else if tree.original {
            "Original checkout".into()
        } else {
            format!("{count} conversations")
        };
        let text = vec![
            Line::from(vec![
                Span::styled(
                    format!(" {}", fit(&name, body.width.saturating_sub(4) as usize)),
                    style(FG).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    if let Some(c) = active {
                        if c.attention == Attention::NeedsInput {
                            " !"
                        } else {
                            " ●"
                        }
                    } else {
                        ""
                    },
                    style(
                        if active.is_some_and(|c| c.attention == Attention::NeedsInput) {
                            YELLOW
                        } else {
                            GREEN
                        },
                    ),
                ),
            ]),
            Line::styled(
                format!(
                    " {}",
                    fit(&tree.branch, body.width.saturating_sub(2) as usize)
                ),
                style(MUTED),
            ),
            Line::styled(
                format!(
                    " {}",
                    fit(
                        &format!("{description} · {files}"),
                        body.width.saturating_sub(2) as usize
                    )
                ),
                style(MUTED),
            ),
            Line::from(
                app.snapshots
                    .get(&tree.path)
                    .map(|s| {
                        vec![
                            Span::styled(format!(" +{}", s.additions), style(GREEN)),
                            Span::styled(format!(" -{}", s.deletions), style(RED)),
                        ]
                    })
                    .unwrap_or_default(),
            ),
        ];
        frame.render_widget(
            Paragraph::new(text).style(Style::default().bg(if i == selected {
                SELECTED
            } else {
                BG
            })),
            rect,
        );
        hit(app, rect, Action::Tree(i));
    }
}
fn middle(frame: &mut Frame, app: &mut App, area: Rect) {
    let changes = app.tab().is_some_and(|t| t.view == View::Changes);
    let body = pane(
        frame,
        area,
        if changes { "Changes" } else { "Sessions" },
        app.pane == Pane::List,
    );
    if changes {
        let files = app.snapshot().map(|s| s.files.clone()).unwrap_or_default();
        let selected = app.tab().map_or(0, |t| t.file);
        let page = (body.height as usize / 3).max(1);
        let start = selected.saturating_sub(page - 1);
        for (i, file) in files.iter().enumerate().skip(start).take(page) {
            let y = body.y + ((i - start) * 3) as u16;
            let rect = Rect::new(
                body.x,
                y,
                body.width,
                3.min(body.bottom().saturating_sub(y)),
            );
            let mark = if app.marked.contains(&file.path) {
                "×"
            } else {
                " "
            };
            let state = if file.untracked {
                "Untracked"
            } else if file.staged() && file.unstaged() {
                "Staged + unstaged"
            } else if file.staged() {
                "Staged"
            } else {
                "Unstaged"
            };
            let text = vec![
                Line::styled(
                    format!(" [{mark}] {} {state}", file.code()),
                    style(if file.untracked { GREEN } else { YELLOW }),
                ),
                Line::from(format!(
                    " {}",
                    fit(
                        &file.path.to_string_lossy(),
                        body.width.saturating_sub(2) as usize
                    )
                )),
            ];
            frame.render_widget(
                Paragraph::new(text).style(Style::default().bg(if i == selected {
                    SELECTED
                } else {
                    BG
                })),
                rect,
            );
            hit(app, rect, Action::File(i));
        }
        if files.is_empty() {
            frame.render_widget(
                Paragraph::new("\n No changed files").style(style(MUTED)),
                body,
            );
        }
    } else {
        label(
            frame,
            app,
            Rect::new(body.right().saturating_sub(2), area.y, 2, 1),
            "+ ",
            MUTED,
            Action::NewSession,
        );
        let sessions = app.sessions().into_iter().cloned().collect::<Vec<_>>();
        let current = app.tab().and_then(|t| t.conversation.clone());
        let selected = sessions
            .iter()
            .position(|s| Some(&s.id) == current.as_ref())
            .unwrap_or(0);
        let page = (body.height as usize / 3).max(1);
        let start = selected.saturating_sub(page - 1);
        for (i, session) in sessions.iter().enumerate().skip(start).take(page) {
            let y = body.y + ((i - start) * 3) as u16;
            let rect = Rect::new(
                body.x,
                y,
                body.width,
                3.min(body.bottom().saturating_sub(y)),
            );
            let color = attention_color(session.attention);
            let text = vec![
                Line::styled(
                    format!(
                        " {}",
                        fit(&session.title, body.width.saturating_sub(2) as usize)
                    ),
                    style(FG).add_modifier(Modifier::BOLD),
                ),
                Line::styled(
                    format!(
                        " {} {}{}",
                        attention_icon(session.attention),
                        session.attention.label(),
                        if session.archived { " · archived" } else { "" }
                    ),
                    style(color),
                ),
            ];
            frame.render_widget(
                Paragraph::new(text).style(Style::default().bg(
                    if Some(&session.id) == current.as_ref() {
                        SELECTED
                    } else {
                        BG
                    },
                )),
                rect,
            );
            hit(app, rect, Action::Session(session.id.clone()));
        }
        if sessions.is_empty() {
            frame.render_widget(
                Paragraph::new("\n No conversations yet\n\n Use + to start one")
                    .style(style(MUTED)),
                body,
            );
        }
    }
}
fn detail(frame: &mut Frame, app: &mut App, area: Rect) {
    let changes = app.tab().is_some_and(|t| t.view == View::Changes);
    let heading = if changes {
        app.selected_file()
            .map(|f| {
                format!(
                    "{} · {}",
                    f.path.display(),
                    if app.diff_staged {
                        "staged"
                    } else {
                        "unstaged"
                    }
                )
            })
            .unwrap_or("Changes".into())
    } else {
        format!(
            "Copilot CLI / {}",
            app.conversation()
                .map(|c| c.title.as_str())
                .unwrap_or("New conversation")
        )
    };
    let body = pane(
        frame,
        area,
        &fit(&heading, area.width.saturating_sub(5) as usize),
        app.pane == Pane::Detail,
    );
    if changes {
        let lines = app
            .diff
            .lines()
            .map(|text| {
                let color = if text.starts_with('+') && !text.starts_with("+++") {
                    GREEN
                } else if text.starts_with('-') && !text.starts_with("---") {
                    RED
                } else if text.starts_with("@@") {
                    ACCENT
                } else {
                    FG
                };
                Line::styled(plain(text), style(color))
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(lines)
                .scroll((app.diff_scroll, 0))
                .wrap(Wrap { trim: false }),
            body,
        );
        hit(app, body, Action::Scroll(0));
        return;
    }
    app.terminal_area = body;
    if let Some(session) = app.conversation().cloned() {
        if let Some(terminal) = app.terminals.get_mut(&session.id) {
            terminal.resize(body.height, body.width);
            frame.render_widget(crate::cli_terminal::TerminalWidget(terminal.screen()), body);
            if !terminal.started && terminal.exit.is_none() {
                frame.render_widget(
                    Paragraph::new("Starting Copilot CLI…").style(style(MUTED)),
                    body,
                );
            }
            if app.cli_focused
                && app.modal.is_none()
                && terminal.running()
                && !terminal.screen().hide_cursor()
            {
                let (row, col) = terminal.screen().cursor_position();
                if row < body.height && col < body.width {
                    frame.set_cursor_position((body.x + col, body.y + row));
                }
            }
            if let Some(exit) = &terminal.exit {
                frame.render_widget(
                    Paragraph::new(format!("{exit} · Resume to reopen")).style(style(MUTED)),
                    Rect::new(body.x, body.bottom().saturating_sub(1), body.width, 1),
                );
            }
            hit(app, body, Action::FocusTerminal);
            label(
                frame,
                app,
                Rect::new(area.right().saturating_sub(3), area.y, 2, 1),
                "× ",
                MUTED,
                Action::CloseTerminal,
            );
            return;
        }
        if session.chat_uri.is_none()
            && session.attention == Attention::Running
            && !session.creation_failed
            && session.error.is_none()
        {
            frame.render_widget(
                Paragraph::new("Starting Copilot CLI…").style(style(MUTED)),
                body,
            );
            return;
        }
        let mut lines = conversation_lines(&session, app.expanded);
        if let Some(error) = &session.error {
            lines.push(Line::styled(plain(error), style(RED)));
        }
        if lines.is_empty() {
            lines.push(Line::styled(
                "Saved conversation · open Copilot CLI to continue",
                style(MUTED),
            ));
        }
        let content = Rect::new(body.x, body.y, body.width, body.height.saturating_sub(2));
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((app.transcript_scroll, 0)),
            content,
        );
        if session.worktree.is_dir() {
            label(
                frame,
                app,
                Rect::new(
                    body.x + 1,
                    body.bottom().saturating_sub(1),
                    body.width.saturating_sub(2),
                    1,
                ),
                if session.creation_failed {
                    "[ Retry session creation ]"
                } else {
                    "[ Open Copilot CLI ]"
                },
                ACCENT,
                if session.creation_failed {
                    Action::Retry
                } else {
                    Action::Resume
                },
            );
        } else {
            frame.render_widget(
                Paragraph::new("Checkout missing · saved history is read-only").style(style(MUTED)),
                Rect::new(body.x, body.bottom().saturating_sub(1), body.width, 1),
            );
        }
    } else {
        frame.render_widget(Paragraph::new("\n Start a session with +.\n\n The real Copilot CLI runs here, beside your worktrees and Git changes.").style(style(MUTED)).wrap(Wrap{trim:false}),body);
    }
}
pub fn conversation_lines(session: &Conversation, expanded: bool) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let Some(chat) = &session.chat else {
        return lines;
    };
    for turn in &chat.turns {
        append_turn(
            &mut lines,
            &turn.message.text,
            &turn.response_parts,
            expanded,
        );
    }
    if let Some(turn) = &chat.active_turn {
        append_turn(
            &mut lines,
            &turn.message.text,
            &turn.response_parts,
            expanded,
        );
    }
    lines
}
fn append_turn(
    lines: &mut Vec<Line<'static>>,
    prompt: &str,
    parts: &[ahp_types::state::ResponsePart],
    expanded: bool,
) {
    lines.push(Line::styled("YOU", style(ACCENT)));
    markdown(lines, prompt);
    lines.push(Line::from(""));
    lines.push(Line::styled("COPILOT", style(ACCENT)));
    for part in parts {
        let value = serde_json::to_value(part).unwrap_or(Value::Null);
        match part {
            ahp_types::state::ResponsePart::Markdown(text) => markdown(lines, &text.content),
            ahp_types::state::ResponsePart::Reasoning(_) => {
                lines.push(Line::styled(
                    if expanded {
                        "▾ Reasoning"
                    } else {
                        "▸ Reasoning"
                    },
                    style(MUTED),
                ));
                if expanded {
                    markdown(lines, value["content"].as_str().unwrap_or(""));
                }
            }
            ahp_types::state::ResponsePart::ToolCall(_) => {
                let tool = &value["toolCall"];
                let name = tool["displayName"].as_str().unwrap_or("Tool");
                let status = tool["status"].as_str().unwrap_or("working");
                lines.push(Line::styled(
                    format!("{} {name} · {status}", if expanded { "▾" } else { "▸" }),
                    style(if status.contains("confirmation") {
                        YELLOW
                    } else {
                        MUTED
                    }),
                ));
                if expanded {
                    if let Some(input) = tool["input"].as_str() {
                        markdown(lines, input);
                    }
                    if let Some(result) = tool.get("result") {
                        markdown(
                            lines,
                            &serde_json::to_string_pretty(result).unwrap_or_default(),
                        );
                    }
                }
            }
            ahp_types::state::ResponsePart::InputRequest(_) => {
                lines.push(Line::styled(
                    "! Copilot needs an answer · open Help → Respond",
                    style(YELLOW),
                ));
                if expanded {
                    markdown(
                        lines,
                        &serde_json::to_string_pretty(&value).unwrap_or_default(),
                    );
                }
            }
            ahp_types::state::ResponsePart::Error(_) => markdown(
                lines,
                &format!(
                    "Error: {}",
                    value["error"]["message"].as_str().unwrap_or("Agent error")
                ),
            ),
            _ => {
                if expanded {
                    markdown(
                        lines,
                        &serde_json::to_string_pretty(&value).unwrap_or_default(),
                    );
                }
            }
        }
    }
    lines.push(Line::from(""));
}
fn markdown(lines: &mut Vec<Line<'static>>, content: &str) {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};
    let clean = plain(content);
    let mut spans = vec![];
    let (mut code, mut bold, mut italic, mut heading) = (false, false, false, false);
    fn flush(lines: &mut Vec<Line<'static>>, spans: &mut Vec<Span<'static>>) {
        if !spans.is_empty() {
            lines.push(Line::from(std::mem::take(spans)));
        }
    }
    for event in Parser::new(&clean) {
        match event {
            Event::Start(Tag::CodeBlock(language)) => {
                flush(lines, &mut spans);
                if let pulldown_cmark::CodeBlockKind::Fenced(lang) = language
                    && !lang.is_empty()
                {
                    lines.push(Line::styled(format!("  {lang}"), style(MUTED)));
                }
                code = true;
            }
            Event::End(TagEnd::CodeBlock) => {
                flush(lines, &mut spans);
                code = false;
            }
            Event::Start(Tag::Strong) => bold = true,
            Event::End(TagEnd::Strong) => bold = false,
            Event::Start(Tag::Emphasis) => italic = true,
            Event::End(TagEnd::Emphasis) => italic = false,
            Event::Start(Tag::Heading { .. }) => {
                flush(lines, &mut spans);
                heading = true;
            }
            Event::End(TagEnd::Heading(_)) => {
                flush(lines, &mut spans);
                heading = false;
            }
            Event::Start(Tag::Item) => {
                flush(lines, &mut spans);
                spans.push(Span::styled(" • ", style(MUTED)));
            }
            Event::Text(text) | Event::Code(text) => {
                let mut text_style = style(if code {
                    GREEN
                } else if heading {
                    ACCENT
                } else {
                    FG
                });
                if bold || heading {
                    text_style = text_style.add_modifier(Modifier::BOLD);
                }
                if italic {
                    text_style = text_style.add_modifier(Modifier::ITALIC);
                }
                for (i, line) in text.split('\n').enumerate() {
                    if i > 0 {
                        flush(lines, &mut spans);
                    }
                    spans.push(Span::styled(line.to_owned(), text_style));
                }
            }
            Event::SoftBreak | Event::HardBreak => flush(lines, &mut spans),
            Event::End(TagEnd::Paragraph | TagEnd::Item) => {
                flush(lines, &mut spans);
                lines.push(Line::from(""));
            }
            Event::Rule => {
                flush(lines, &mut spans);
                lines.push(Line::styled("────────", style(LINE)));
            }
            _ => {}
        }
    }
    flush(lines, &mut spans);
}
fn plain(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect::<String>()
        .replace('\t', "    ")
}
fn statusline(frame: &mut Frame, app: &mut App, area: Rect) {
    let tree = app.tree().cloned();
    let snapshot = app.snapshot();
    let branch = tree
        .as_ref()
        .map(|t| {
            if t.branch.is_empty() {
                "HEAD"
            } else {
                &t.branch
            }
        })
        .unwrap_or("Tree Rail");
    let tracking = if let Some(s) = snapshot {
        if s.upstream.is_some() {
            format!("↑{} ↓{}", s.ahead, s.behind)
        } else {
            "unpublished".into()
        }
    } else if tree.as_ref().is_some_and(|t| t.prunable) {
        "missing".into()
    } else {
        "Git".into()
    };
    let activity = tree
        .as_ref()
        .and_then(|t| {
            app.state
                .conversations
                .iter()
                .find(|c| crate::config::same_path(&c.worktree, &t.path) && c.attention.busy())
        })
        .map(|c| c.attention);
    let all_running = app
        .state
        .conversations
        .iter()
        .filter(|c| c.attention == Attention::Running)
        .count();
    let all_input = app
        .state
        .conversations
        .iter()
        .filter(|c| c.attention == Attention::NeedsInput)
        .count();
    let width = area.width;
    let columns = Layout::horizontal([
        Constraint::Length((branch.width() as u16 + 3).min(width / 4)),
        Constraint::Length((tracking.width() as u16 + 3).min(width / 5)),
        Constraint::Fill(1),
        Constraint::Length(if width >= 130 { 9 } else { 0 }),
        Constraint::Length(if width >= 110 { 9 } else { 3 }),
        Constraint::Length(7),
        Constraint::Length(if width >= 120 { 10 } else { 2 }),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(format!(
            " {}",
            fit(branch, columns[0].width.saturating_sub(2) as usize)
        ))
        .style(
            Style::default()
                .bg(ACCENT)
                .fg(BG)
                .add_modifier(Modifier::BOLD),
        ),
        columns[0],
    );
    frame.render_widget(
        Paragraph::new(format!(" {tracking} ›")).style(Style::default().bg(SELECTED).fg(FG)),
        columns[1],
    );
    let context = if let Some((path, action)) = &app.mutation {
        format!("{action} · {}", path.display())
    } else if let Some((message, _, _)) = &app.notice {
        message.clone()
    } else {
        tree.as_ref()
            .map(|t| t.path.display().to_string())
            .unwrap_or_else(|| app.config_path.display().to_string())
    };
    frame.render_widget(
        Paragraph::new(format!(
            " {}",
            fit(&context, columns[2].width.saturating_sub(1) as usize)
        ))
        .style(Style::default().bg(DIM).fg(
            if app.notice.as_ref().is_some_and(|(_, error, _)| *error) {
                RED
            } else {
                MUTED
            },
        )),
        columns[2],
    );
    hit(
        app,
        columns[2],
        if app.notice.is_some() {
            Action::Details
        } else {
            Action::WorktreeInfo
        },
    );
    if let Some(tree) = tree {
        frame.render_widget(
            Paragraph::new(format!(
                " {}",
                tree.head.chars().take(7).collect::<String>()
            ))
            .style(Style::default().bg(DIM).fg(MUTED)),
            columns[3],
        );
    }
    let label = if let Some(attention) = activity {
        if width >= 110 {
            format!(
                " {} {}",
                attention_icon(attention),
                if attention == Attention::NeedsInput {
                    "Input"
                } else {
                    "Active"
                }
            )
        } else {
            format!(" {}", attention_icon(attention))
        }
    } else if width >= 110 {
        " ○ Ready".into()
    } else {
        " ○".into()
    };
    frame.render_widget(
        Paragraph::new(label).style(
            Style::default()
                .bg(DIM)
                .fg(activity.map(attention_color).unwrap_or(MUTED)),
        ),
        columns[4],
    );
    frame.render_widget(
        Paragraph::new(format!(" ●{all_running} !{all_input}")).style(
            Style::default()
                .bg(DIM)
                .fg(if all_input > 0 { YELLOW } else { GREEN }),
        ),
        columns[5],
    );
    hit(app, columns[5], Action::Activity);
    let provider = if width >= 120 {
        if app.connected {
            " Copilot ✓"
        } else {
            " Copilot ○"
        }
    } else if app.connected {
        " ✓"
    } else {
        " ○"
    };
    frame.render_widget(
        Paragraph::new(provider).style(Style::default().bg(SURFACE).fg(MUTED)),
        columns[6],
    );
    hit(app, columns[6], Action::Host);
}
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(4));
    let height = height.min(area.height.saturating_sub(2));
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}
fn modal_frame(frame: &mut Frame, area: Rect, title: &str) -> Rect {
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .title(format!(" {title} "))
        .border_style(style(ACCENT))
        .style(Style::default().bg(SURFACE).fg(FG));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}
fn overlay(frame: &mut Frame, app: &mut App, area: Rect, modal: Modal) {
    match modal {
        Modal::Branches {
            form,
            query,
            selected,
        } => {
            let branches = app.matching_branches(&form, &query.value);
            let rect = centered(area, 82, 22);
            let inner = modal_frame(frame, rect, "Choose starting point");
            frame.render_widget(
                Paragraph::new(format!(" Search branches: {}▌", query.value)).style(style(ACCENT)),
                Rect::new(inner.x, inner.y, inner.width, 1),
            );
            let body = Rect::new(
                inner.x,
                inner.y + 2,
                inner.width,
                inner.height.saturating_sub(4),
            );
            let start = selected.saturating_sub(body.height.saturating_sub(1) as usize);
            for (i, branch) in branches
                .iter()
                .enumerate()
                .skip(start)
                .take(body.height as usize)
            {
                let row = Rect::new(body.x, body.y + (i - start) as u16, body.width, 1);
                frame.render_widget(
                    Paragraph::new(format!(
                        " {}  · {}",
                        fit(&branch.name, body.width.saturating_sub(14) as usize),
                        if branch.remote { "remote" } else { "local" }
                    ))
                    .style(Style::default().fg(FG).bg(if i == selected {
                        SELECTED
                    } else {
                        SURFACE
                    })),
                    row,
                );
                hit(app, row, Action::ChooseBranch(i));
            }
            if branches.is_empty() {
                frame.render_widget(
                    Paragraph::new(if app.branch_loading(&form) {
                        " Fetching branches…"
                    } else {
                        " No matching branches"
                    })
                    .style(style(MUTED)),
                    body,
                );
            }
            frame.render_widget(
                Paragraph::new(if app.branch_loading(&form) {
                    " Fetching latest branches…"
                } else {
                    " ↑ ↓ choose · Enter select · Esc back"
                })
                .style(style(MUTED)),
                Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
            );
        }
        Modal::Projects { query, selected } => {
            let projects = app.visible_projects(&query.value);
            let rect = centered(area, 82, 22);
            let inner = modal_frame(frame, rect, "Projects");
            frame.render_widget(
                Paragraph::new(format!(" Search: {}▌", query.value)).style(style(ACCENT)),
                Rect::new(inner.x, inner.y, inner.width, 1),
            );
            let body = Rect::new(
                inner.x,
                inner.y + 2,
                inner.width,
                inner.height.saturating_sub(4),
            );
            let page = (body.height as usize / 3).max(1);
            let start = selected.saturating_sub(page - 1);
            for (pos, index) in projects.iter().enumerate().skip(start).take(page) {
                let project = &app.projects[*index];
                let y = body.y + ((pos - start) * 3) as u16;
                let rect = Rect::new(body.x, y, body.width, 2);
                frame.render_widget(
                    Paragraph::new(vec![
                        Line::from(format!(" {}", project.name)),
                        Line::styled(
                            format!(
                                " {}",
                                fit(
                                    &project.path.to_string_lossy(),
                                    body.width.saturating_sub(2) as usize
                                )
                            ),
                            style(MUTED),
                        ),
                    ])
                    .style(Style::default().bg(if pos == selected {
                        SELECTED
                    } else {
                        SURFACE
                    })),
                    rect,
                );
                hit(app, rect, Action::Project(*index));
            }
            if projects.is_empty() {
                frame.render_widget(
                    Paragraph::new(" No projects found. Add a repository or discovery folder.")
                        .wrap(Wrap { trim: false })
                        .style(style(MUTED)),
                    body,
                );
            }
            label(
                frame,
                app,
                Rect::new(inner.x, inner.bottom() - 1, inner.width / 2, 1),
                " + Remember a path",
                ACCENT,
                Action::Git("add-path"),
            );
            label(
                frame,
                app,
                Rect::new(
                    inner.x + inner.width / 2,
                    inner.bottom() - 1,
                    inner.width / 2,
                    1,
                ),
                "Close",
                MUTED,
                Action::Cancel,
            );
        }
        Modal::Help(selected) => {
            let actions = app.help_actions();
            let rect = centered(area, 78, actions.len() as u16 + 6);
            let inner = modal_frame(frame, rect, "Help & actions");
            let busy = app.tree().is_some_and(|t| app.busy(&t.path));
            let hint = if busy {
                " Agent active · Git writes paused"
            } else {
                " Commands apply to the selected worktree"
            };
            frame.render_widget(
                Paragraph::new(hint).style(style(if busy { YELLOW } else { MUTED })),
                Rect::new(inner.x, inner.y, inner.width, 1),
            );
            let list = Rect::new(
                inner.x,
                inner.y + 2,
                inner.width,
                inner.height.saturating_sub(4),
            );
            let start = selected.saturating_sub(list.height.saturating_sub(1) as usize);
            for (i, (text, key, action)) in actions
                .into_iter()
                .enumerate()
                .skip(start)
                .take(list.height as usize)
            {
                let rect = Rect::new(list.x, list.y + (i - start) as u16, list.width, 1);
                let disabled = matches!(
                    action,
                    Action::Git("stage" | "unstage" | "commit" | "pull" | "push" | "remove")
                ) && busy;
                let pad = (list.width as usize).saturating_sub(text.width() + key.width() + 3);
                frame.render_widget(
                    Paragraph::new(format!(" {text}{}{key}", " ".repeat(pad))).style(
                        Style::default()
                            .bg(if i == selected { SELECTED } else { SURFACE })
                            .fg(if disabled { LINE } else { FG }),
                    ),
                    rect,
                );
                if !disabled {
                    hit(app, rect, action);
                }
            }
            frame.render_widget(
                Paragraph::new(" Tab focus · Enter execute · Esc close").style(style(MUTED)),
                Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
            );
        }
        Modal::Form(form) => {
            let visible = form.visible();
            let height = visible
                .iter()
                .map(|i| {
                    if matches!(form.fields[*i].kind, FieldType::Toggle) {
                        2
                    } else if matches!(form.fields[*i].kind, FieldType::Multiline) {
                        5
                    } else {
                        3
                    }
                })
                .sum::<u16>()
                + 6;
            let rect = centered(area, 82, height);
            let inner = modal_frame(frame, rect, &form.title);
            let bottom = inner.bottom().saturating_sub(3);
            let mut y = inner.y;
            // Start at the focused field on terminals too short for the form.
            let total = height.saturating_sub(6);
            let available = bottom.saturating_sub(y);
            let start = if total > available {
                visible.iter().position(|i| *i == form.focused).unwrap_or(0)
            } else {
                0
            };
            for index in visible.into_iter().skip(start) {
                let field = &form.fields[index];
                let toggle = matches!(field.kind, FieldType::Toggle);
                let lines = if toggle {
                    2
                } else if matches!(field.kind, FieldType::Multiline) {
                    5
                } else {
                    3
                };
                if y + lines > bottom {
                    break;
                }
                let focused = form.focused == index;
                let color = if focused { ACCENT } else { MUTED };
                if toggle {
                    let mark = if field.input.value == "yes" {
                        "×"
                    } else {
                        " "
                    };
                    label(
                        frame,
                        app,
                        Rect::new(inner.x + 1, y, inner.width.saturating_sub(2), 1),
                        format!("[{mark}] {}", field.label),
                        color,
                        Action::Field(index),
                    );
                } else {
                    frame.render_widget(
                        Paragraph::new(fit(&field.label, inner.width.saturating_sub(2) as usize))
                            .style(style(color)),
                        Rect::new(inner.x + 1, y, inner.width.saturating_sub(2), 1),
                    );
                    let input = Rect::new(
                        inner.x + 1,
                        y + 1,
                        inner
                            .width
                            .saturating_sub(if matches!(field.kind, FieldType::Branch) {
                                12
                            } else {
                                2
                            }),
                        lines - 2,
                    );
                    let shown = if field.input.value.is_empty() {
                        "…".to_owned()
                    } else {
                        format!(
                            "{}{}",
                            field.input.value,
                            if matches!(field.kind, FieldType::Choice(_)) {
                                "  ◂ ▸"
                            } else {
                                ""
                            }
                        )
                    };
                    frame.render_widget(
                        Paragraph::new(plain(&shown))
                            .wrap(Wrap { trim: false })
                            .style(
                                Style::default()
                                    .bg(if focused { SELECTED } else { DIM })
                                    .fg(FG),
                            ),
                        input,
                    );
                    hit(app, input, Action::Field(index));
                    if matches!(field.kind, FieldType::Branch) {
                        label(
                            frame,
                            app,
                            Rect::new(inner.right().saturating_sub(11), y + 1, 10, 1),
                            "[ Browse ]",
                            ACCENT,
                            Action::PickBranch,
                        );
                    }
                    if focused {
                        let before = &field.input.value[..field.input.cursor];
                        let col = before.lines().last().unwrap_or("").width() as u16;
                        let row = before.chars().filter(|c| *c == '\n').count() as u16;
                        frame.set_cursor_position((
                            input.x + col.min(input.width.saturating_sub(1)),
                            input.y + row.min(input.height.saturating_sub(1)),
                        ));
                    }
                }
                y += lines;
            }
            if form.kind == FormKind::Session {
                label(
                    frame,
                    app,
                    Rect::new(inner.x + 1, bottom, inner.width.saturating_sub(2), 1),
                    if form.advanced {
                        "▾ Provider options"
                    } else {
                        "▸ Provider options"
                    },
                    MUTED,
                    Action::ToggleAdvanced,
                );
            } else if form.kind == FormKind::Worktree {
                frame.render_widget(
                    Paragraph::new(if app.branch_loading(&form) {
                        " Fetching branches…"
                    } else {
                        " Fetches automatically · location from settings"
                    })
                    .style(style(MUTED)),
                    Rect::new(inner.x, bottom, inner.width, 1),
                );
            }
            let row = inner.bottom() - 1;
            label(
                frame,
                app,
                Rect::new(inner.right().saturating_sub(26), row, 12, 1),
                "[ Cancel ]",
                MUTED,
                Action::Cancel,
            );
            let submit = Rect::new(inner.right().saturating_sub(14), row, 14, 1);
            if (form.kind == FormKind::Worktree
                || (form.kind == FormKind::Session && form.value("new_tree") == "yes"))
                && app.branch_loading(&form)
            {
                frame.render_widget(Paragraph::new("[ Fetching… ]").style(style(MUTED)), submit);
            } else {
                label(frame, app, submit, "[ Continue ]", ACCENT, Action::Confirm);
            }
        }
        Modal::Confirm(kind) => {
            let (title, message) = match &kind {
                Confirm::Trust { .. } => ("Trust folder for Copilot?", ""),
                Confirm::Close(_) => (
                    "Close project tab?",
                    "Its sessions are currently running. Close the tab and keep them running? Reopen the project through agent activity.",
                ),
                Confirm::Quit => (
                    "Quit Tree Rail?",
                    "Stop Tree Rail-owned agent work and exit? Conversations stay saved. No agent will restart automatically next time.",
                ),
                Confirm::Remove => (
                    "Remove worktree?",
                    "Remove this clean linked checkout and keep its branch? Saved conversations remain readable. The directory will not be recreated automatically.",
                ),
            };
            let message = match &kind {
                Confirm::Trust { path, .. } => format!(
                    "{}\n\nAllow Copilot to load this folder's instructions, hooks and tools? Saves this exact folder in Copilot's trusted folders. Cancel keeps your prompt unsent.",
                    path.display()
                ),
                _ => message.to_owned(),
            };
            let rect = if matches!(kind, Confirm::Trust { .. }) {
                centered(area, 80, 14)
            } else {
                centered(area, 68, 10)
            };
            let inner = modal_frame(frame, rect, title);
            frame.render_widget(
                Paragraph::new(message).wrap(Wrap { trim: false }),
                Rect::new(
                    inner.x + 1,
                    inner.y + 1,
                    inner.width.saturating_sub(2),
                    inner.height.saturating_sub(3),
                ),
            );
            label(
                frame,
                app,
                Rect::new(inner.right() - 28, inner.bottom() - 1, 13, 1),
                "[ Cancel ]",
                if app.confirm_yes { MUTED } else { ACCENT },
                Action::Cancel,
            );
            label(
                frame,
                app,
                Rect::new(inner.right() - 15, inner.bottom() - 1, 15, 1),
                "[ Confirm ]",
                if app.confirm_yes { YELLOW } else { MUTED },
                Action::Confirm,
            );
        }
        Modal::Activity(selected) => {
            let rect = centered(area, 76, 18);
            let inner = modal_frame(frame, rect, "Agent activity");
            let items = app
                .state
                .conversations
                .iter()
                .filter(|c| c.attention.busy())
                .map(|c| {
                    ListItem::new(format!(
                        "{} · {} · {}",
                        c.project.file_name().unwrap_or_default().to_string_lossy(),
                        c.title,
                        c.attention.label()
                    ))
                })
                .collect::<Vec<_>>();
            let mut state = ListState::default().with_selected(Some(selected));
            frame.render_stateful_widget(
                List::new(items).highlight_style(Style::default().bg(SELECTED)),
                inner,
                &mut state,
            );
        }
        Modal::Host => {
            let rect = centered(area, 88, 13);
            let inner = modal_frame(frame, rect, "AHP connection · private");
            let text = if let Some(url) = &app.connection_url {
                format!(
                    "Use this authenticated endpoint in another AHP client.\nThe token grants access to this host; keep it private.\n\n{url}\n\nTree Rail-owned hosts stop when Tree Rail exits.\nExternal hosts remain owned by their original application."
                )
            } else {
                "Copilot is not connected. Start a session or Refresh to connect.\n\nThe official pinned runtime is used by default.\nSet [agent].runtime or [agent].endpoint in config.toml to override.".into()
            };
            frame.render_widget(
                Paragraph::new(text)
                    .wrap(Wrap { trim: false })
                    .style(style(MUTED)),
                inner,
            );
        }
        Modal::Approval(request) => {
            let rect = centered(area, 88, 24);
            let inner = modal_frame(frame, rect, "Copilot · approval required");
            let worktree = app
                .conversation()
                .map(|c| c.worktree.display().to_string())
                .unwrap_or_default();
            let tool = &request["toolCall"];
            let text = format!(
                "{}\n{}\n\n{}",
                tool["displayName"].as_str().unwrap_or("Tool request"),
                worktree,
                serde_json::to_string_pretty(tool).unwrap_or_default()
            );
            let options = tool["options"].as_array();
            let option_count = options.map_or(2, Vec::len).min(9) as u16;
            frame.render_widget(
                Paragraph::new(text).wrap(Wrap { trim: false }),
                Rect::new(
                    inner.x + 1,
                    inner.y,
                    inner.width.saturating_sub(2),
                    inner.height.saturating_sub(option_count + 1),
                ),
            );
            let y = inner.bottom().saturating_sub(option_count);
            if let Some(options) = options {
                for (i, option) in options.iter().take(9).enumerate() {
                    let approved = option["kind"] == "approve";
                    label(
                        frame,
                        app,
                        Rect::new(inner.x + 1, y + i as u16, inner.width.saturating_sub(2), 1),
                        format!(
                            "[{}] {}",
                            i + 1,
                            option["label"].as_str().unwrap_or("Choose")
                        ),
                        if approved { YELLOW } else { MUTED },
                        Action::Approve(approved, option["id"].as_str().map(str::to_owned)),
                    );
                }
            } else {
                label(
                    frame,
                    app,
                    Rect::new(inner.x + 1, y, inner.width.saturating_sub(2), 1),
                    "[y] Allow once",
                    YELLOW,
                    Action::Approve(true, None),
                );
                label(
                    frame,
                    app,
                    Rect::new(inner.x + 1, y + 1, inner.width.saturating_sub(2), 1),
                    "[n] Deny",
                    MUTED,
                    Action::Approve(false, None),
                );
            }
        }
        Modal::Completions { items, selected } => {
            let rect = centered(area, 76, items.len().min(12) as u16 + 4);
            let inner = modal_frame(frame, rect, "Copilot completions");
            let start = selected.saturating_sub(inner.height.saturating_sub(1) as usize);
            for (i, item) in items
                .iter()
                .enumerate()
                .skip(start)
                .take(inner.height as usize)
            {
                let label_text = item["attachment"]["label"]
                    .as_str()
                    .or_else(|| item["insertText"].as_str())
                    .unwrap_or("Completion");
                let rect = Rect::new(inner.x, inner.y + (i - start) as u16, inner.width, 1);
                frame.render_widget(
                    Paragraph::new(format!(
                        " {}",
                        fit(label_text, inner.width.saturating_sub(2) as usize)
                    ))
                    .style(
                        Style::default()
                            .bg(if i == selected { SELECTED } else { SURFACE })
                            .fg(FG),
                    ),
                    rect,
                );
                hit(app, rect, Action::Completion(i));
            }
        }
        Modal::Sources(selected) => {
            let rect = centered(area, 88, 18);
            let inner = modal_frame(frame, rect, "Remembered locations");
            frame.render_widget(
                Paragraph::new(" Forget removes the remembered entry and keeps the directory.")
                    .style(style(MUTED)),
                Rect::new(inner.x, inner.y, inner.width, 1),
            );
            let start = selected.saturating_sub(inner.height.saturating_sub(5) as usize);
            let paths = app.state.remembered_sources.clone();
            for (i, path) in paths
                .iter()
                .enumerate()
                .skip(start)
                .take(inner.height.saturating_sub(4) as usize)
            {
                let row = Rect::new(inner.x, inner.y + 2 + (i - start) as u16, inner.width, 1);
                frame.render_widget(
                    Paragraph::new(format!(
                        " {}",
                        fit(
                            &path.display().to_string(),
                            inner.width.saturating_sub(5) as usize
                        )
                    ))
                    .style(
                        Style::default()
                            .bg(if i == selected { SELECTED } else { SURFACE })
                            .fg(FG),
                    ),
                    row,
                );
                label(
                    frame,
                    app,
                    Rect::new(inner.right() - 3, row.y, 3, 1),
                    " × ",
                    MUTED,
                    Action::ForgetSource(i),
                );
            }
            label(
                frame,
                app,
                Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
                "+ Remember a path",
                ACCENT,
                Action::Git("add-path"),
            );
        }
        Modal::Notice(message) => {
            let rect = centered(area, 88, 18);
            let inner = modal_frame(frame, rect, "Details");
            frame.render_widget(Paragraph::new(message).wrap(Wrap { trim: false }), inner);
        }
    }
    if let Some((message, true, _)) = &app.notice {
        let rect = Rect::new(
            area.x + 2,
            area.bottom().saturating_sub(2),
            area.width.saturating_sub(4),
            1,
        );
        frame.render_widget(
            Paragraph::new(fit(message, rect.width as usize))
                .style(Style::default().bg(SURFACE).fg(RED)),
            rect,
        );
    }
}
fn attention_color(attention: Attention) -> Color {
    match attention {
        Attention::Running => GREEN,
        Attention::NeedsInput => YELLOW,
        Attention::Interrupted | Attention::Unavailable => RED,
        Attention::Ready => MUTED,
    }
}
fn attention_icon(attention: Attention) -> &'static str {
    match attention {
        Attention::Running => "●",
        Attention::NeedsInput => "!",
        Attention::Interrupted | Attention::Unavailable => "×",
        Attention::Ready => "○",
    }
}
