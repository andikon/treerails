use super::*;
use crate::{browser::Editor, ticket_browser::IssueCard};

fn overflow(text: String, width: u16) -> String {
    if text.width() <= usize::from(width) {
        return text;
    }
    let mut shown = String::new();
    for value in text.chars() {
        if shown.width() + value.to_string().width() + 1 > usize::from(width) {
            break;
        }
        shown.push(value);
    }
    shown.push('…');
    shown
}
fn freshness(fetched_at: u64) -> String {
    let seconds = crate::ticket_browser::now().saturating_sub(fetched_at);
    if seconds < 60 {
        "just now".into()
    } else if seconds < 3600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86400 {
        format!("{}h ago", seconds / 3600)
    } else {
        format!("{}d ago", seconds / 86400)
    }
}
fn card(card: &IssueCard, detail: Option<&str>, width: u16) -> ListItem<'static> {
    let mut lines = vec![Line::from(format!(
        "#{} [{}] {}",
        card.issue.number, card.issue.state, card.issue.title
    ))];
    if !card.labels.is_empty() {
        lines.push(Line::from(
            card.labels
                .iter()
                .map(|label| format!("[{}]", label.name))
                .collect::<Vec<_>>()
                .join(" "),
        ));
    }
    if let Some(detail) = detail {
        lines.push(Line::styled(
            detail.to_owned(),
            Style::default().fg(Color::Yellow),
        ));
    }
    ListItem::new(
        lines
            .into_iter()
            .map(|line| Line::from(overflow(line.to_string(), width)))
            .collect::<Vec<_>>(),
    )
}

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let narrow = area.width < 90;
    let panes =
        Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)]).split(area);
    if !narrow || !app.browser.preview_focus {
        column(frame, app, if narrow { area } else { panes[0] });
    }
    if !narrow || app.browser.preview_focus {
        preview(frame, app, if narrow { area } else { panes[1] });
    }
    if let Some(editor) = &app.browser.editor {
        edit(frame, app, area, editor);
    }
}

fn column(frame: &mut Frame, app: &App, area: Rect) {
    let active = app.active_browser_tickets();
    let active_height = if active.is_empty() {
        0
    } else if app.browser.active_collapsed {
        3
    } else {
        (active.len() as u16 * 3 + 2).min((area.height / 3).max(3))
    };
    let rows =
        Layout::vertical([Constraint::Length(active_height), Constraint::Min(2)]).split(area);
    if !active.is_empty() {
        let attention = active.iter().filter(|ticket| ticket.rank == 0).count();
        let title = format!(
            " {} Active: {} · attention {} ",
            if app.browser.active_collapsed {
                "▸"
            } else {
                "▾"
            },
            active.len(),
            attention
        );
        if app.browser.active_collapsed {
            frame.render_widget(
                Paragraph::new("a expand active work").block(border(title, false, app)),
                rows[0],
            );
        } else {
            let items = active
                .iter()
                .map(|entry| {
                    let metadata = app
                        .browser
                        .cards
                        .iter()
                        .find(|card| card.issue.number == entry.ticket.issue.number)
                        .or_else(|| app.browser.active_cards.get(&entry.ticket.issue.number))
                        .cloned()
                        .unwrap_or_else(|| entry.ticket.issue.clone().into());
                    let matches = app
                        .browser
                        .cards
                        .iter()
                        .any(|card| card.issue.number == entry.ticket.issue.number);
                    let completed = entry
                        .ticket
                        .tasks
                        .iter()
                        .filter(|task| task.progress == crate::workflow::Progress::Done)
                        .count();
                    card(
                        &metadata,
                        Some(&format!(
                            "{} · tasks {}/{}{}",
                            entry.attention,
                            completed,
                            entry.ticket.tasks.len(),
                            if matches { " · matches view" } else { "" }
                        )),
                        area.width.saturating_sub(2),
                    )
                })
                .collect::<Vec<_>>();
            let selected = active
                .iter()
                .position(|ticket| Some(ticket.ticket.issue.number) == app.browser.selected);
            frame.render_stateful_widget(
                List::new(items)
                    .block(border(
                        title,
                        !app.browser.preview_focus && selected.is_some(),
                        app,
                    ))
                    .highlight_style(Style::default().bg(Color::DarkGray)),
                rows[0],
                &mut ListState::default().with_selected(selected),
            );
        }
    }
    let filtered = app
        .browser
        .cards
        .iter()
        .filter(|card| {
            !active
                .iter()
                .any(|entry| entry.ticket.issue.number == card.issue.number)
        })
        .collect::<Vec<_>>();
    let selected = filtered
        .iter()
        .position(|card| Some(card.issue.number) == app.browser.selected);
    let status = if app.browser.loading {
        "loading"
    } else if !app.browser.warning.is_empty() {
        "stale / unavailable"
    } else if app.browser.has_more {
        "more below"
    } else {
        "end"
    };
    let title = format!(" {} ", app.browser.view_name());
    let counts = format!(
        "{} loaded{} · {} in queue · {}{}",
        app.browser.cards.len(),
        app.browser
            .total
            .map(|total| format!(" / {total} matches"))
            .unwrap_or_default(),
        filtered.len(),
        status,
        if app.browser.limited {
            " · LIMIT: narrow filters"
        } else {
            ""
        }
    );
    let rows = Layout::vertical([Constraint::Length(4), Constraint::Min(1)]).split(rows[1]);
    frame.render_widget(
        Paragraph::new(format!(
            "{}\n{}\n{}",
            filter_summary(app),
            counts,
            app.browser.warning
        ))
        .wrap(Wrap { trim: false }),
        rows[0],
    );
    if filtered.is_empty() {
        let message = if app.browser.loading {
            "Loading matching tickets…"
        } else if !app.browser.warning.is_empty() {
            "Ticket query unavailable. Active work remains accessible."
        } else {
            "No matching tickets"
        };
        frame.render_widget(
            Paragraph::new(message)
                .wrap(Wrap { trim: false })
                .block(border(title, !app.browser.preview_focus, app)),
            rows[1],
        );
    } else {
        frame.render_stateful_widget(
            List::new(
                filtered
                    .iter()
                    .map(|issue| card(issue, None, area.width.saturating_sub(2)))
                    .collect::<Vec<_>>(),
            )
            .block(border(title, !app.browser.preview_focus, app))
            .highlight_style(Style::default().bg(Color::DarkGray)),
            rows[1],
            &mut ListState::default().with_selected(selected),
        );
    }
}

fn filter_summary(app: &App) -> String {
    let filter = app.browser.filter();
    use crate::ticket_browser::{AssigneeFilter, Sort};
    let assignees = match &filter.assignees {
        AssigneeFilter::Anyone => "anyone".into(),
        AssigneeFilter::Me => "me".into(),
        AssigneeFilter::Unassigned => "unassigned".into(),
        AssigneeFilter::Users(users) => users.join(", "),
    };
    let mut chips = vec![
        format!("{:?}", filter.state),
        assignees,
        match filter.sort {
            Sort::Updated => "recently updated",
            Sort::Newest => "newest created",
            Sort::Oldest => "oldest created",
        }
        .into(),
    ];
    if !filter.labels.is_empty() {
        chips.push(format!(
            "{:?}: {}",
            filter.label_mode,
            filter.labels.join(", ")
        ));
    }
    if !filter.exclude_labels.is_empty() {
        chips.push(format!("exclude: {}", filter.exclude_labels.join(", ")));
    }
    if !filter.text.is_empty() || filter.include_comments {
        chips.push(format!(
            "{:?} in {}",
            filter.text,
            if filter.include_comments {
                "title/body/comments"
            } else {
                "title/body"
            }
        ));
    }
    chips.join(" · ")
}

pub(super) fn preview(frame: &mut Frame, app: &App, area: Rect) {
    let selected = app.browser.selected;
    let discussion = app
        .browser
        .discussion
        .as_ref()
        .filter(|discussion| Some(discussion.card.issue.number) == selected);
    let active = app.active_browser_tickets();
    let fallback = active
        .iter()
        .find(|entry| Some(entry.ticket.issue.number) == selected)
        .map(|entry| IssueCard::from(entry.ticket.issue.clone()));
    let card = discussion
        .map(|discussion| &discussion.card)
        .or_else(|| app.browser.selected_card())
        .or_else(|| selected.and_then(|number| app.browser.active_cards.get(&number)))
        .or(fallback.as_ref());
    let mut lines = vec![];
    if let Some(card) = card {
        lines.push(Line::styled(
            format!("#{} {}", card.issue.number, card.issue.title),
            Style::default().add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::from(format!(
            "{} · @{} · created {} · updated {}",
            card.issue.state, card.author, card.created_at, card.updated_at
        )));
        lines.push(Line::from(format!(
            "Labels: {}",
            card.labels
                .iter()
                .map(|label| label.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
        lines.push(Line::from(format!(
            "Assignees: {}",
            card.assignees.join(", ")
        )));
        lines.push(Line::from(card.issue.url.clone()));
        if !app
            .browser
            .cards
            .iter()
            .any(|entry| Some(entry.issue.number) == selected)
            && !active
                .iter()
                .any(|entry| Some(entry.ticket.issue.number) == selected)
        {
            lines.push(Line::styled(
                if app.browser.has_more || app.browser.loading {
                    "Not in loaded results · preview retained"
                } else {
                    "No longer matches this view"
                },
                Style::default().fg(Color::Yellow),
            ));
        }
        lines.push(Line::from(""));
        lines.extend(markdown(&card.issue.body));
        if let Some(discussion) = discussion {
            lines.push(Line::from(""));
            lines.push(Line::styled(
                format!(
                    "Discussion · {} comments · {} · fetched {}",
                    discussion.comments.len(),
                    if discussion.complete {
                        "complete"
                    } else {
                        "INCOMPLETE"
                    },
                    freshness(discussion.fetched_at)
                ),
                Style::default().add_modifier(Modifier::BOLD),
            ));
            for comment in &discussion.comments {
                lines.push(Line::from("────────────────"));
                lines.push(Line::styled(
                    format!(
                        "@{} · {}{}",
                        comment.author,
                        comment.created_at,
                        if comment.created_at != comment.updated_at {
                            " · edited"
                        } else {
                            ""
                        }
                    ),
                    Style::default().fg(Color::Cyan),
                ));
                lines.extend(markdown(&comment.body));
            }
            if let Some(warning) = &discussion.warning {
                lines.push(Line::styled(
                    warning.clone(),
                    Style::default().fg(Color::Yellow),
                ));
            }
        }
    } else {
        lines.push(Line::from(if app.browser.preview_loading {
            "Loading ticket…"
        } else {
            "Select a ticket to read its discussion"
        }));
    }
    if app.browser.preview_loading {
        lines.push(Line::from("Refreshing discussion…"));
    }
    if !app.browser.preview_warning.is_empty() {
        lines.push(Line::styled(
            app.browser.preview_warning.clone(),
            Style::default().fg(Color::Yellow),
        ));
    }
    let line_count = lines
        .iter()
        .map(|line| {
            line.width()
                .div_ceil(usize::from(area.width.saturating_sub(2)).max(1))
                .max(1)
        })
        .sum::<usize>();
    let scroll = usize::from(app.browser.preview_scroll)
        .min(line_count.saturating_sub(usize::from(area.height.saturating_sub(2))))
        as u16;
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0))
            .block(border(
                " Ticket preview · o GitHub ",
                app.browser.preview_focus,
                app,
            )),
        area,
    );
}

/// Terminal-safe Markdown preserves references while styling block structure.
pub fn markdown(text: &str) -> Vec<Line<'static>> {
    let mut code = false;
    text.lines()
        .map(|raw| {
            if raw.trim_start().starts_with("```") {
                code = !code;
                return Line::styled(raw.to_owned(), Style::default().fg(Color::DarkGray));
            }
            if code {
                return Line::styled(format!("  {raw}"), Style::default().fg(Color::Green));
            }
            if let Some(heading) = raw.trim_start().strip_prefix('#') {
                return Line::styled(
                    heading.trim_start_matches('#').trim().to_owned(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                );
            }
            if let Some(quote) = raw.strip_prefix('>') {
                return Line::styled(format!("│{}", quote), Style::default().fg(Color::Gray));
            }
            let text = raw
                .replace("- [ ] ", "☐ ")
                .replace("- [x] ", "☑ ")
                .replace("- [X] ", "☑ ");
            Line::from(text)
        })
        .collect()
}

fn edit(frame: &mut Frame, app: &App, area: Rect, editor: &Editor) {
    let rect = centered(area, 90, 85);
    let text = match editor {
        Editor::Filters { fields, cursor } => {
            let names = [
                "State: open / closed / all",
                "Assignees: anyone / me / unassigned / comma-separated users",
                "Include labels: comma-separated",
                "Label matching: all / any",
                "Exclude labels: comma-separated",
                "Text search",
                "Include comments: yes / no",
                "Sort: updated / newest / oldest",
            ];
            let metadata = app
                .browser
                .metadata
                .as_ref()
                .map(|metadata| {
                    format!(
                        "\nLabels: {}\nAssignees: {}{}",
                        metadata
                            .labels
                            .iter()
                            .map(|label| label.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                        metadata.assignees.join(", "),
                        metadata
                            .warning
                            .as_ref()
                            .map(|warning| format!("\n{warning}"))
                            .unwrap_or_default()
                    )
                })
                .unwrap_or_else(|| "\nLoading available labels and assignees…".into());
            names
                .iter()
                .zip(fields)
                .enumerate()
                .map(|(index, (name, value))| {
                    format!(
                        "{} {}\n  {}",
                        if index == *cursor { "▶" } else { " " },
                        name,
                        value
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
                + "\n\nCtrl+s Apply and save · Esc cancel · Ctrl+u clear field"
                + &metadata
        }
        Editor::Views { cursor } => {
            app.browser
                .views
                .views
                .iter()
                .enumerate()
                .map(|(index, view)| {
                    format!(
                        "{} {}{}",
                        if index == *cursor { "▶" } else { " " },
                        view.name,
                        if view.id == app.browser.views.active {
                            " · current"
                        } else {
                            ""
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
                + "\n\nn new · d duplicate · r rename · x delete · Esc close"
        }
        Editor::Name { action, text, .. } => {
            format!("{} view\n\n{}\n\nEnter save · Esc cancel", action, text)
        }
    };
    let target = text
        .lines()
        .take_while(|line| !line.starts_with('▶'))
        .map(|line| {
            line.width()
                .div_ceil(usize::from(rect.width.saturating_sub(2)).max(1))
                .max(1)
        })
        .sum::<usize>();
    let scroll = if matches!(editor, Editor::Name { .. }) {
        0
    } else {
        target.saturating_sub(usize::from(rect.height.saturating_sub(4))) / 2
    };
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(text)
            .scroll((scroll.min(u16::MAX as usize) as u16, 0))
            .wrap(Wrap { trim: false })
            .block(border(" Saved ticket views ", true, app)),
        rect,
    );
}
