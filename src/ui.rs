use crate::app::{App, Modal, PickerKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;
mod hosted_view;

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
        let path = record.target.display().to_string();
        let length = rows[0].width.saturating_sub(2) as usize;
        let path = if path.chars().count() > length {
            format!(
                "…{}",
                path.chars()
                    .rev()
                    .take(length.saturating_sub(1))
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
            )
        } else {
            path
        };
        let summary = format!("{}\n{}", record.headline(), path);
        let status = if record.progress.is_empty() {
            summary
        } else {
            format!("{}\n{}", summary, record.progress)
        };
        frame.render_widget(
            Paragraph::new(status).style(Style::default().fg(
                if record.result.as_ref().is_some_and(Result::is_err) {
                    Color::Red
                } else {
                    Color::Yellow
                },
            )),
            Rect::new(
                rows[0].x,
                rows[0].y.saturating_add(1),
                rows[0].width,
                rows[0].height.saturating_sub(1),
            ),
        );
    }
    if app.latest_operation.is_none() {
        hosted_view::attention(
            frame,
            app,
            Rect::new(rows[0].x, rows[0].y.saturating_add(1), rows[0].width, 1),
        );
    }
    if app.settings_open {
        settings_view(frame, app, rows[1]);
    } else if app.project.is_none() {
        projects(frame, app, rows[1]);
    } else if app.ticket_mode {
        tickets(frame, app, rows[1]);
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
    let hints = if app.settings_open
        && app
            .settings
            .as_ref()
            .is_some_and(|draft| draft.input.is_some())
    {
        " Tab next field · Ctrl+u clear · Enter accept · Ctrl+s apply · Esc explorer (keeps draft) "
    } else if app.settings_open {
        " 1/2/3/4 sections · h/l switch · Enter edit · Ctrl+s apply · Esc explorer · D discard "
    } else if app.project.is_none() {
        " j/k navigate   / search   Enter tickets   w worktrees   , settings   R reload   i output · H history   ? help   q quit "
    } else if app.ticket_mode {
        " s Continue · n new session · Enter input · x stop · d detach · ! takeover · g conversations · c create · a adopt · w basic · Esc back "
    } else {
        " T tickets · a worktree  b branches  o Copilot  O profiles  s/u stage  c commit  p/P push/pull  , settings  i output · H history  ? help "
    };
    frame.render_widget(
        Paragraph::new(hints).style(Style::default().fg(Color::Black).bg(accent(app))),
        rows[3],
    );
    if let Some(modal) = &app.modal {
        let history =
            matches!(modal, Modal::Picker(picker) if matches!(picker.kind, PickerKind::Operations));
        let area = if history || matches!(modal, Modal::OperationDetails) {
            Rect::new(
                area.x,
                rows[0].bottom(),
                area.width,
                area.bottom().saturating_sub(rows[0].bottom()),
            )
        } else {
            area
        };
        modal_view(frame, app, modal, area);
    }
    hosted_view::dialog(frame, app, area);
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
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(3),
    ])
    .split(inner);
    let introduction = draft
        .input
        .as_ref()
        .map(|editor| format!("{}\nConfig: {}", editor.title, app.config_path.display()))
        .unwrap_or_else(|| format!("Config: {}", app.config_path.display()));
    frame.render_widget(
        Paragraph::new(introduction).wrap(Wrap { trim: false }),
        rows[0],
    );
    let sections = [
        crate::settings::Section::Sources,
        crate::settings::Section::Worktrees,
        crate::settings::Section::Profiles,
        crate::settings::Section::Notifications,
    ];
    frame.render_widget(
        Paragraph::new(Line::from(
            sections
                .iter()
                .enumerate()
                .map(|(index, section)| {
                    Span::styled(
                        format!(" {} {} ", index + 1, section.title()),
                        if *section == draft.section {
                            Style::default().fg(Color::Black).bg(accent(app))
                        } else {
                            Style::default().fg(Color::Gray)
                        },
                    )
                })
                .collect::<Vec<_>>(),
        )),
        rows[1],
    );
    if let Some(editor) = &draft.input {
        let visible = (rows[2].height as usize / 3).max(1);
        let start = editor.active.saturating_sub(visible - 1);
        for (position, (index, field)) in editor
            .fields
            .iter()
            .enumerate()
            .skip(start)
            .take(visible)
            .enumerate()
        {
            let rect = Rect::new(
                rows[2].x,
                rows[2].y + (position * 3) as u16,
                rows[2].width,
                3.min(rows[2].height.saturating_sub((position * 3) as u16)),
            );
            let active = index == editor.active;
            let block = border(
                format!(" {}/{} · {} ", index + 1, editor.fields.len(), field.label),
                active,
                app,
            );
            let inner = block.inner(rect);
            let column = field.value[..field.cursor].width();
            let offset = if active {
                column.saturating_sub(inner.width.saturating_sub(1) as usize)
            } else {
                0
            };
            frame.render_widget(
                Paragraph::new(field.value.as_str())
                    .scroll((0, offset.min(u16::MAX as usize) as u16))
                    .block(block),
                rect,
            );
            if active && inner.width > 0 && inner.height > 0 {
                frame.set_cursor_position((inner.x + (column - offset) as u16, inner.y));
            }
        }
    } else {
        let items = draft
            .rows()
            .into_iter()
            .map(ListItem::new)
            .collect::<Vec<_>>();
        let mut state = ListState::default().with_selected(Some(draft.cursor));
        frame.render_stateful_widget(
            List::new(items)
                .highlight_symbol("▸ ")
                .highlight_style(Style::default().bg(Color::Rgb(28, 43, 58))),
            rows[2],
            &mut state,
        );
    }
    let message = match &draft.mode {
        crate::settings::Mode::Edit => draft.message.clone(),
        crate::settings::Mode::Discard { quit } => format!(
            "Discard unsaved settings{}? y discard · any other key keep editing",
            if *quit { " and quit" } else { "" }
        ),
        crate::settings::Mode::Conflict(fields) => {
            let fields = fields
                .iter()
                .map(|field| {
                    let Some((path, setting)) = field
                        .strip_prefix("projects.")
                        .and_then(|field| field.rsplit_once('.'))
                    else {
                        return field.clone();
                    };
                    let name = std::path::Path::new(path)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy();
                    let name = if name.chars().count() > 24 {
                        format!("{}…", name.chars().take(23).collect::<String>())
                    } else {
                        name.into_owned()
                    };
                    format!("{setting} ({name})")
                })
                .collect::<Vec<_>>()
                .join(", ");
            let choices = if rows[3].width >= 64 {
                "m use my changes · e keep external changes · Esc continue editing"
            } else {
                "m mine · e external · Esc edit"
            };
            format!("{choices}\nChanged elsewhere: {fields}")
        }
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
        Modal::Requirements(review) => requirements_review(frame, app, review, area),
        Modal::OperationDetails => {
            let Some(record) = app.inspected_operation() else {
                return;
            };
            let rect = centered(area, 112, area.height);
            frame.render_widget(Clear, rect);
            let block = border(
                format!(
                    " Operation #{} · j/k scroll · g/G first/last · Esc close ",
                    record.id
                ),
                true,
                app,
            );
            let inner = block.inner(rect);
            frame.render_widget(block, rect);
            let rows = Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).split(inner);
            frame.render_widget(
                Paragraph::new(format!(
                    "{}\nProject: {} · Target: {}\n{}",
                    record.headline(),
                    record
                        .project
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy(),
                    record
                        .target
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy(),
                    if record.truncated {
                        "Earlier output discarded (64 KiB limit)."
                    } else {
                        "H history · g full paths/result · session only"
                    }
                )),
                rows[0],
            );
            let visible = rows[1].height as usize;
            let lines = record.details_lines(rows[1].width as usize);
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
                "\n  NAVIGATION\n  j/k or arrows · gg/G first/last · Ctrl+u/d half-page\n  h/l or Tab focus panes · Ctrl+w then h/j/k/l focus panes\n  / search · n/N next/previous result · Esc back · q back/quit\n\n  WORKTREES & GIT\n  a new worktree · b branch picker (a new branch, d delete)\n  Space mark files · s stage · u unstage · t staged/unstaged diff\n  c commit · f fetch · P fast-forward pull · p push/publish\n  d remove clean worktree (branch retained)\n  r pull requests · v PR comparison · w open PR in browser\n\n  i live details · H session history · j/k scroll · G follow\n\n  TOOLS & SETTINGS\n  o launch default · O choose profile · R refresh · , settings\n  Settings: 1/2/3 sections · h/l switch · a add · Enter edit\n  Sources: d remove · r recursive · fields: Tab move, Ctrl+u clear\n  Ctrl+s apply · Esc explorer (keeps draft) · D discard\n  Forms: Tab next field · Ctrl+b base picker · Ctrl+s submit\n  Pickers: j/k navigate · / filter · arrows or Ctrl+n/p move\n\n  Config: {}\n  Press Esc, q, Enter or ? to close.",
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
            let history = matches!(picker.kind, PickerKind::Operations);
            let rect = centered(area, 100, if history { area.height } else { 24 });
            frame.render_widget(Clear, rect);
            let block = border(format!(" {} ", picker.title), true, app);
            let inner = block.inner(rect);
            frame.render_widget(block, rect);
            let rows = Layout::vertical([
                Constraint::Length(if history { 4 } else { 2 }),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(inner);
            let mut prompt = format!(
                " /{}  [{}]",
                picker.query,
                if picker.editing { "filter" } else { "navigate" }
            );
            if history {
                prompt.push_str(&format!(
                    "\n Session only · {} retained · {} older discarded\n {}",
                    app.operation_records().len(),
                    app.history_evicted,
                    if app.latest_operation.is_none() {
                        "No Git operations yet. Run an action to see its result here."
                    } else {
                        "Up to 20 results; the inspected result stays available."
                    }
                ));
            }
            frame.render_widget(
                Paragraph::new(prompt).style(Style::default().fg(accent(app))),
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

fn tickets(frame: &mut Frame, app: &App, area: Rect) {
    let area = if app.hosted.terminal.is_some() {
        let panes = Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)])
            .split(area);
        hosted_view::terminal(frame, app, panes[1]);
        panes[0]
    } else {
        area
    };
    if let Some(ticket) = &app.ticket {
        let rows = Layout::vertical([
            Constraint::Length(6),
            Constraint::Percentage(45),
            Constraint::Min(3),
        ])
        .split(area);
        let checkout = ticket
            .worktree
            .as_ref()
            .map(|wt| {
                format!(
                    "{} · {}{}",
                    wt.branch,
                    wt.path.display(),
                    if wt.original {
                        " [original checkout]"
                    } else {
                        ""
                    }
                )
            })
            .unwrap_or_else(|| "No shared worktree · c create or a adopt".into());
        frame.render_widget(
            Paragraph::new(format!(
                "{} #{} · {} [{}]{}\n{}\nLast sync Unix {} · {}\n{}",
                ticket.repository,
                ticket.issue.number,
                ticket.issue.title,
                ticket.issue.state,
                if ticket.issue_updated() {
                    " · Issue updated · v inspect"
                } else {
                    ""
                },
                ticket.issue.url,
                ticket.fetched_at,
                checkout,
                ticket
                    .github_error
                    .as_deref()
                    .unwrap_or("GitHub issue cache · provider health is reported separately")
            ))
            .wrap(Wrap { trim: false })
            .block(border(" Ticket overview ", true, app)),
            rows[0],
        );
        frame.render_widget(
            Paragraph::new(ticket.issue.body.clone())
                .scroll((app.ticket_scroll, 0))
                .wrap(Wrap { trim: false })
                .block(border(
                    " GitHub requirements · PgUp/PgDn scroll ",
                    false,
                    app,
                )),
            rows[1],
        );
        let items: Vec<ListItem> = ticket
            .tasks
            .iter()
            .map(|task| {
                ListItem::new(format!(
                    "{} {} · {:?} · {}",
                    if ticket.selected_task.as_ref() == Some(&task.id) {
                        "●"
                    } else {
                        " "
                    },
                    task.id,
                    task.progress,
                    task.title
                ))
            })
            .collect();
        frame.render_stateful_widget(
            List::new(items)
                .block(border(
                    " Tasks · Enter select · p change progress ",
                    true,
                    app,
                ))
                .highlight_style(Style::default().bg(Color::DarkGray)),
            rows[2],
            &mut ListState::default().with_selected(Some(app.task_cursor)),
        );
    } else {
        let items: Vec<ListItem> = app
            .issues
            .iter()
            .map(|issue| {
                ListItem::new(format!(
                    "#{} [{}] {}",
                    issue.number, issue.state, issue.title
                ))
            })
            .collect();
        frame.render_stateful_widget(
            List::new(items)
                .block(border(
                    if app.issue_warning.is_empty() {
                        " GitHub tickets · Enter inspect · w basic worktrees "
                    } else {
                        " Cached GitHub tickets · stale · Enter inspect · w basic "
                    },
                    true,
                    app,
                ))
                .highlight_style(Style::default().bg(Color::DarkGray)),
            area,
            &mut ListState::default().with_selected(Some(app.issue_cursor)),
        );
    }
}

fn requirements_review(
    frame: &mut Frame,
    app: &App,
    review: &crate::workflow::RequirementsReview,
    area: Rect,
) {
    frame.render_widget(Clear, area);
    let block = border(
        " Issue requirements · j/k/PgUp/PgDn scroll · Esc cancel ",
        true,
        app,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = Layout::vertical([Constraint::Min(1), Constraint::Length(5)]).split(inner);
    let columns =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(rows[0]);
    frame.render_widget(
        Paragraph::new(format!(
            "{}\n\n{}",
            review.previous.title, review.previous.body
        ))
        .scroll((app.ticket_scroll, 0))
        .wrap(Wrap { trim: false })
        .block(border(" Previously reviewed requirements ", false, app)),
        columns[0],
    );
    frame.render_widget(
        Paragraph::new(review.instructions())
            .scroll((app.ticket_scroll, 0))
            .wrap(Wrap { trim: false })
            .block(border(
                " Current requirements · exact text sent by s ",
                true,
                app,
            )),
        columns[1],
    );
    frame.render_widget(Paragraph::new(format!("Task merge preview: retain {} local tasks with their IDs, progress and sessions; add: {}\nm apply task additions explicitly · s send displayed revision to selected live task/session\nExisting sessions keep their original starting briefs. No conversation is created by this action.", review.retained_tasks.len(), if review.additions.is_empty() { "none".into() } else { review.additions.join(", ") })).wrap(Wrap {trim:false}), rows[1]);
}
