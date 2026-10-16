use crate::app::{App, Modal};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

fn color(value: &str, fallback: Color) -> Color {
    value.parse().unwrap_or(fallback)
}
fn accent(app: &App) -> Color {
    color(&app.config.theme.accent, Color::Cyan)
}
fn border<'a>(title: impl Into<Line<'a>>, active: bool, app: &App) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(if active { accent(app) } else { Color::DarkGray }))
}
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let rows = Layout::vertical([
        Constraint::Length(if app.latest_operation.is_some() { 4 } else { 2 }),
        Constraint::Min(3),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .split(area);
    let location = app
        .project
        .as_ref()
        .map(|p| format!(" / {}", p.name))
        .unwrap_or_else(|| " / projects".into());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " TREE RAIL ",
                Style::default()
                    .fg(Color::Black)
                    .bg(accent(app))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(location, Style::default().fg(Color::White)),
            Span::styled(
                if app.busy { "   RUNNING" } else { "" },
                Style::default().fg(Color::Yellow),
            ),
        ])),
        rows[0],
    );
    if let Some(record) = &app.latest_operation {
        let status = if record.progress.is_empty() {
            record.summary()
        } else {
            format!("{}\n{}", record.summary(), record.progress)
        };
        frame.render_widget(
            Paragraph::new(status)
                .style(
                    Style::default().fg(if record.result.as_ref().is_some_and(Result::is_err) {
                        Color::Red
                    } else {
                        Color::Yellow
                    }),
                )
                .wrap(Wrap { trim: false }),
            Rect::new(
                rows[0].x,
                rows[0].y.saturating_add(1),
                rows[0].width,
                rows[0].height.saturating_sub(1),
            ),
        );
    }
    if app.settings_open {
        settings_view(frame, app, rows[1]);
    } else if app.project.is_none() {
        projects(frame, app, rows[1]);
    } else {
        workspace(frame, app, rows[1]);
    }
    let prompt = if app.searching {
        format!("/{}", app.filter)
    } else if !app.filter.is_empty() {
        format!("filter: {} · Esc clears", app.filter)
    } else {
        app.notice.clone()
    };
    frame.render_widget(
        Paragraph::new(prompt)
            .style(Style::default().fg(if app.error && !app.searching {
                color(&app.config.theme.deleted, Color::Red)
            } else {
                Color::Gray
            }))
            .wrap(Wrap { trim: false }),
        rows[2],
    );
    let hints = if app.settings_open {
        " a add · Enter edit · d remove · r recursive · Ctrl+s apply · i output · Esc explorer · D discard "
    } else if app.project.is_none() {
        " j/k navigate   / search   Enter explore   , settings   R reload   i output   ? help   q quit "
    } else {
        " a worktree  b branches  o Copilot  O profiles  s/u stage  c commit  p/P push/pull  , settings  i output  ? help "
    };
    frame.render_widget(
        Paragraph::new(hints).style(Style::default().fg(Color::Black).bg(accent(app))),
        rows[3],
    );
    if let Some(modal) = &app.modal {
        modal_view(frame, app, modal, area);
    }
}
fn settings_view(frame: &mut Frame, app: &App, area: Rect) {
    let Some(draft) = &app.settings else {
        return;
    };
    let block = border(
        format!(
            " Settings · {} ",
            if draft.dirty() {
                "Unsaved draft"
            } else {
                "Saved"
            }
        ),
        true,
        app,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(if draft.input.is_some() { 3 } else { 0 }),
        Constraint::Length(3),
    ])
    .split(inner);
    frame.render_widget(
        Paragraph::new(format!("Config: {}", app.config_path.display())).wrap(Wrap { trim: false }),
        rows[0],
    );
    let mut items: Vec<ListItem> = draft
        .config
        .project_roots
        .iter()
        .map(|path| {
            ListItem::new(format!(
                " {}{}",
                path.display(),
                if path.is_dir() {
                    ""
                } else {
                    "  [unavailable; kept]"
                }
            ))
        })
        .collect();
    items.push(ListItem::new(format!(
        " Recursive discovery: {}  [r toggles]",
        if draft.config.recursive { "on" } else { "off" }
    )));
    let mut state = ListState::default().with_selected(Some(draft.cursor));
    frame.render_stateful_widget(
        List::new(items)
            .highlight_symbol("▸ ")
            .highlight_style(Style::default().bg(Color::Rgb(28, 43, 58))),
        rows[1],
        &mut state,
    );
    if let Some(input) = &draft.input {
        let block = border(format!(" {} · Enter accept ", input.field.label), true, app);
        let rect = block.inner(rows[2]);
        let column = input.field.value[..input.field.cursor].width();
        let offset = column.saturating_sub(rect.width.saturating_sub(1) as usize);
        frame.render_widget(
            Paragraph::new(input.field.value.as_str())
                .scroll((0, offset.min(u16::MAX as usize) as u16))
                .block(block),
            rows[2],
        );
        if rect.width > 0 && rect.height > 0 {
            frame.set_cursor_position((rect.x + (column - offset) as u16, rect.y));
        }
    }
    let message = match &draft.mode {
        crate::settings::Mode::Edit => draft.message.clone(),
        crate::settings::Mode::Discard { quit } => format!(
            "Discard unsaved settings{}? y discard · any other key keep editing",
            if *quit { " and quit" } else { "" }
        ),
        crate::settings::Mode::Conflict(fields) => format!(
            "Changed elsewhere: {}\nm use my changes · e keep external changes · Esc continue editing",
            fields.join(", ")
        ),
    };
    frame.render_widget(
        Paragraph::new(message)
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(if draft.error {
                Color::Red
            } else {
                Color::Yellow
            })),
        rows[3],
    );
}
fn projects(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .visible_projects()
        .iter()
        .map(|p| {
            ListItem::new(vec![
                Line::from(Span::styled(
                    format!(" {}", p.name),
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    format!(" {}", p.path.display()),
                    Style::default().fg(Color::DarkGray),
                )),
            ])
        })
        .collect();
    if items.is_empty() {
        frame.render_widget(Paragraph::new("\n  No projects found.\n\n  Pass a repository or its parent directory:\n  treerail \"C:\\Code\"\n\n  R reloads sources; ? opens help.").block(border(" Projects ",true,app)),area);
        return;
    }
    let mut state = ListState::default().with_selected(Some(app.project_cursor));
    frame.render_stateful_widget(
        List::new(items)
            .block(border(
                format!(" Projects · {} ", app.projects.len()),
                true,
                app,
            ))
            .highlight_style(Style::default().bg(Color::Rgb(28, 43, 58)))
            .highlight_symbol("▸ "),
        area,
        &mut state,
    );
}
fn workspace(frame: &mut Frame, app: &App, area: Rect) {
    let panes = if area.width >= 110 {
        Layout::horizontal([
            Constraint::Percentage(29),
            Constraint::Percentage(28),
            Constraint::Percentage(43),
        ])
        .split(area)
        .to_vec()
    } else {
        vec![area; 3]
    };
    if area.width >= 110 || app.focus == 0 {
        let items: Vec<ListItem> = app
            .visible_trees()
            .iter()
            .map(|w| {
                let (a, m, d) = app
                    .snapshots
                    .get(&w.path)
                    .map(|s| s.counts())
                    .unwrap_or_default();
                let mut title = vec![Span::styled(
                    format!(" {}", w.branch),
                    Style::default().add_modifier(Modifier::BOLD),
                )];
                if w.original {
                    title.push(Span::styled(
                        " [original]",
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                if w.locked {
                    title.push(Span::styled(
                        " [locked]",
                        Style::default().fg(Color::Yellow),
                    ));
                }
                if w.prunable {
                    title.push(Span::styled(" [missing]", Style::default().fg(Color::Red)));
                }
                let counts = Line::from(vec![
                    Span::raw(" "),
                    Span::styled(
                        format!("+{a} "),
                        Style::default().fg(color(&app.config.theme.added, Color::Green)),
                    ),
                    Span::styled(
                        format!("~{m} "),
                        Style::default().fg(color(&app.config.theme.modified, Color::Yellow)),
                    ),
                    Span::styled(
                        format!("-{d}"),
                        Style::default().fg(color(&app.config.theme.deleted, Color::Red)),
                    ),
                ]);
                ListItem::new(vec![
                    Line::from(title),
                    counts,
                    Line::from(Span::styled(
                        format!(" {}", w.path.display()),
                        Style::default().fg(Color::DarkGray),
                    )),
                ])
            })
            .collect();
        let mut state = ListState::default().with_selected(Some(app.tree));
        frame.render_stateful_widget(
            List::new(items)
                .block(border(" 1 · Worktrees ", app.focus == 0, app))
                .highlight_style(Style::default().bg(Color::Rgb(28, 43, 58)))
                .highlight_symbol("▸ "),
            panes[0],
            &mut state,
        );
    }
    if area.width >= 110 || app.focus == 1 {
        let items: Vec<ListItem> = app
            .visible_files()
            .iter()
            .map(|file| {
                let fg = match file.code() {
                    '+' => color(&app.config.theme.added, Color::Green),
                    '-' => color(&app.config.theme.deleted, Color::Red),
                    _ => color(&app.config.theme.modified, Color::Yellow),
                };
                let mark = if app.marked.contains(&file.path) {
                    "●"
                } else {
                    " "
                };
                ListItem::new(Line::from(vec![
                    Span::raw(format!("{mark} ")),
                    Span::styled(format!("{} ", file.code()), Style::default().fg(fg)),
                    Span::styled(
                        format!("{}{} ", file.index, file.work),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::raw(file.path.display().to_string()),
                ]))
            })
            .collect();
        let status = app
            .snapshot()
            .map(|s| format!("↑{} ↓{}", s.ahead, s.behind))
            .unwrap_or_default();
        let mut state = ListState::default().with_selected(Some(app.file));
        frame.render_stateful_widget(
            List::new(items)
                .block(border(
                    format!(" 2 · Files · {status} "),
                    app.focus == 1,
                    app,
                ))
                .highlight_style(Style::default().bg(Color::Rgb(28, 43, 58)))
                .highlight_symbol("▸ "),
            panes[1],
            &mut state,
        );
    }
    if area.width >= 110 || app.focus == 2 {
        let lines: Vec<Line> = app
            .diff
            .lines()
            .skip(app.diff_scroll)
            .take(panes[2].height.saturating_sub(2) as usize)
            .map(|line| {
                let fg = if line.starts_with("@@") {
                    accent(app)
                } else if line.starts_with('+') {
                    color(&app.config.theme.added, Color::Green)
                } else if line.starts_with('-') {
                    color(&app.config.theme.deleted, Color::Red)
                } else if line.starts_with("diff ") {
                    Color::White
                } else {
                    Color::Gray
                };
                Line::from(Span::styled(line.to_owned(), Style::default().fg(fg)))
            })
            .collect();
        let label = if app.pr_diff {
            "PR comparison"
        } else if app.staged {
            "staged"
        } else {
            "unstaged"
        };
        let lines = if lines.is_empty() {
            vec![
                Line::from(""),
                Line::from(" Select a changed file."),
                Line::from(" t switches staged/unstaged."),
                Line::from(" v shows the PR comparison."),
            ]
        } else {
            lines
        };
        frame.render_widget(
            Paragraph::new(lines).block(border(
                format!(" 3 · Diff · {label} "),
                app.focus == 2,
                app,
            )),
            panes[2],
        );
    }
}
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}
fn modal_view(frame: &mut Frame, app: &App, modal: &Modal, area: Rect) {
    match modal {
        Modal::OperationDetails => {
            let Some(record) = &app.latest_operation else {
                return;
            };
            let rect = centered(area, 112, area.height.saturating_sub(2).max(3));
            frame.render_widget(Clear, rect);
            let block = border(
                " Operation details · j/k scroll · g/G first/last · Esc close ",
                true,
                app,
            );
            let inner = block.inner(rect);
            frame.render_widget(block, rect);
            let rows = Layout::vertical([Constraint::Length(5), Constraint::Min(1)]).split(inner);
            let outcome = match &record.result {
                Some(Ok(message)) => format!("Succeeded: {message}"),
                Some(Err(message)) => format!("Failed: {message}"),
                None => "Running · i/Esc closes details; the operation continues".into(),
            };
            frame.render_widget(
                Paragraph::new(format!(
                    "{}\nProject: {}\n{}\n{}",
                    record.summary(),
                    record.project.display(),
                    outcome,
                    if record.truncated {
                        "Earlier output discarded (64 KiB limit)."
                    } else {
                        "Output from Git and hooks:"
                    }
                ))
                .wrap(Wrap { trim: false }),
                rows[0],
            );
            let visible = rows[1].height as usize;
            let lines = record.display_lines(rows[1].width as usize);
            let count = lines.len();
            let start = app.operation_scroll.min(count.saturating_sub(visible));
            frame.render_widget(
                Paragraph::new(lines.into_iter().skip(start).collect::<Vec<_>>().join("\n")),
                rows[1],
            );
        }
        Modal::Help => {
            let rect = centered(area, 94, 28);
            frame.render_widget(Clear, rect);
            let help = format!(
                "\n  NAVIGATION\n  j/k or arrows · gg/G first/last · Ctrl+u/d half-page\n  h/l or Tab focus panes · Ctrl+w then h/j/k/l focus panes\n  / search · n/N next/previous result · Esc back · q back/quit\n\n  WORKTREES & GIT\n  a new worktree · b branch picker (a new branch, d delete)\n  Space mark files · s stage · u unstage · t staged/unstaged diff\n  c commit · f fetch · P fast-forward pull · p push/publish\n  d remove clean worktree (branch retained)\n  r pull requests · v PR comparison · w open PR in browser\n\n  i live operation details · j/k scroll · G follow output\n\n  TOOLS & SETTINGS\n  o launch default · O choose profile · R refresh · , settings\n  Settings: a add · Enter edit · d remove · r recursive\n  Ctrl+s apply · Esc explorer (keeps draft) · D discard\n  Forms: Tab next field · Ctrl+b base picker · Ctrl+s submit\n  Pickers: j/k navigate · / filter · arrows or Ctrl+n/p move\n\n  Config: {}\n  Press Esc, q, Enter or ? to close.",
                app.config_path.display()
            );
            frame.render_widget(
                Paragraph::new(help)
                    .block(border(" Tree Rail · keyboard guide ", true, app))
                    .wrap(Wrap { trim: false }),
                rect,
            );
        }
        Modal::Confirm(message, _) => {
            let rect = centered(area, 85, 11);
            frame.render_widget(Clear, rect);
            frame.render_widget(
                Paragraph::new(format!(
                    "\n{message}\n\nEnter / y confirm    Esc / n cancel"
                ))
                .wrap(Wrap { trim: false })
                .block(border(" Confirm ", true, app)),
                rect,
            );
        }
        Modal::Picker(picker) => {
            let rect = centered(area, 100, 24);
            frame.render_widget(Clear, rect);
            let block = border(format!(" {} ", picker.title), true, app);
            let inner = block.inner(rect);
            frame.render_widget(block, rect);
            let rows = Layout::vertical([
                Constraint::Length(2),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(inner);
            frame.render_widget(
                Paragraph::new(format!(
                    " /{}  [{}]",
                    picker.query,
                    if picker.editing { "filter" } else { "navigate" }
                ))
                .style(Style::default().fg(accent(app))),
                rows[0],
            );
            let items: Vec<ListItem> = picker
                .visible()
                .iter()
                .map(|e| ListItem::new(e.0.clone()))
                .collect();
            let mut state = ListState::default().with_selected(Some(picker.selected));
            frame.render_stateful_widget(
                List::new(items)
                    .highlight_symbol("▸ ")
                    .highlight_style(Style::default().bg(Color::Rgb(28, 43, 58))),
                rows[1],
                &mut state,
            );
            let hint = " j/k move · / filter · Enter select · Esc back";
            frame.render_widget(
                Paragraph::new(hint).style(Style::default().fg(Color::DarkGray)),
                rows[2],
            );
        }
        Modal::Form(form) => {
            let height = 5 + form
                .fields
                .iter()
                .map(|f| if f.multiline { 8 } else { 3 })
                .sum::<u16>();
            let rect = centered(area, 96, height);
            frame.render_widget(Clear, rect);
            let block = border(format!(" {} ", form.title), true, app);
            let inner = block.inner(rect);
            frame.render_widget(block, rect);
            let mut constraints: Vec<Constraint> = form
                .fields
                .iter()
                .map(|f| Constraint::Length(if f.multiline { 8 } else { 3 }))
                .collect();
            constraints.push(Constraint::Min(1));
            let rows = Layout::vertical(constraints).split(inner);
            for (index, field) in form.fields.iter().enumerate() {
                let active = index == form.active;
                let block = border(format!(" {} ", field.label), active, app);
                let text_rect = block.inner(rows[index]);
                let before = &field.value[..field.cursor];
                let row = before.bytes().filter(|b| *b == b'\n').count();
                let column = before.rsplit('\n').next().unwrap_or("").width();
                let sy = row.saturating_sub(text_rect.height.saturating_sub(1) as usize);
                let sx = column.saturating_sub(text_rect.width.saturating_sub(1) as usize);
                frame.render_widget(
                    Paragraph::new(field.value.as_str())
                        .scroll((
                            sy.min(u16::MAX as usize) as u16,
                            sx.min(u16::MAX as usize) as u16,
                        ))
                        .block(block),
                    rows[index],
                );
                if active && text_rect.width > 0 && text_rect.height > 0 {
                    frame.set_cursor_position((
                        text_rect.x + (column - sx) as u16,
                        text_rect.y + (row - sy) as u16,
                    ));
                }
            }
            frame.render_widget(
                Paragraph::new(
                    " Tab next field · Ctrl+b starting point · Ctrl+s submit · Esc cancel",
                )
                .style(Style::default().fg(Color::DarkGray)),
                rows[form.fields.len()],
            );
        }
    }
}
