use super::*;
use crate::{app::hosted_ui::Dialog, copilot::AttentionState};

fn terminal_color(value: vt100::Color) -> Color {
    match value {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(index) => Color::Indexed(index),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}
pub fn terminal(frame: &mut Frame, app: &App, area: Rect) {
    let Some(snapshot) = &app.hosted.terminal else {
        return;
    };
    let mode = if app.hosted.input {
        "AGENT INPUT"
    } else {
        "NAVIGATION"
    };
    let mode_color = if app.hosted.input {
        color(&app.config.theme.modified, Color::Yellow)
    } else {
        accent(app)
    };
    let attention = if snapshot.exited {
        "Terminal ended"
    } else if !snapshot.attention.healthy {
        "Status unavailable"
    } else {
        match snapshot.attention.value {
            Some(AttentionState::Running) => "Running",
            Some(AttentionState::NeedsInput) => "Needs input",
            Some(AttentionState::Ready) => "Ready",
            None => "Status unavailable",
        }
    };
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(3),
    ])
    .split(area);
    let identity = app
        .hosted
        .record
        .as_ref()
        .map(|record| {
            format!(
                "{} · {}\n{}",
                record.ticket, record.task, record.provider_session_id
            )
        })
        .unwrap_or_default();
    frame.render_widget(
        Paragraph::new(identity)
            .style(Style::default().fg(mode_color))
            .block(
                Block::bordered()
                    .title(format!(" {mode} · {attention} "))
                    .border_style(Style::default().fg(mode_color)),
            ),
        rows[0],
    );
    let block = Block::bordered().border_style(Style::default().fg(mode_color));
    let inner = block.inner(rows[1]);
    frame.render_widget(block, rows[1]);
    let mut parser = vt100::Parser::new(snapshot.rows, snapshot.cols, 0);
    parser.process(&snapshot.formatted);
    for row in 0..inner.height.min(snapshot.rows) {
        for col in 0..inner.width.min(snapshot.cols) {
            if let Some(cell) = parser.screen().cell(row, col) {
                if cell.is_wide_continuation() {
                    continue;
                }
                let mut style = Style::default()
                    .fg(terminal_color(cell.fgcolor()))
                    .bg(terminal_color(cell.bgcolor()));
                for (enabled, modifier) in [
                    (cell.bold(), Modifier::BOLD),
                    (cell.italic(), Modifier::ITALIC),
                    (cell.underline(), Modifier::UNDERLINED),
                    (cell.inverse(), Modifier::REVERSED),
                ] {
                    if enabled {
                        style = style.add_modifier(modifier);
                    }
                }
                let contents = cell.contents();
                let symbol = if contents.is_empty() { " " } else { contents };
                frame.buffer_mut().set_stringn(
                    inner.x + col,
                    inner.y + row,
                    symbol,
                    (inner.width - col) as usize,
                    style,
                );
            }
        }
    }
    if app.hosted.input
        && app.hosted.dialog.is_none()
        && !snapshot.cursor_hidden
        && snapshot.cursor.0 < inner.height
        && snapshot.cursor.1 < inner.width
    {
        frame.set_cursor_position((inner.x + snapshot.cursor.1, inner.y + snapshot.cursor.0));
    }
    let others = attention_text(app, true);
    frame.render_widget(
        Paragraph::new(format!(
            "{}\n{}\n{}",
            if app.hosted.input {
                "Keys go to Copilot"
            } else {
                "Tree Rail controls the keyboard"
            },
            if snapshot.attention.healthy {
                ""
            } else {
                &snapshot.attention.reason
            },
            others
        ))
        .style(Style::default().fg(mode_color)),
        rows[2],
    );
}

fn attention_text(app: &App, other_only: bool) -> String {
    let current = app
        .hosted
        .terminal
        .as_ref()
        .map(|terminal| terminal.session.as_str());
    let mut summaries = app
        .hosted
        .summaries
        .iter()
        .filter(|summary| !other_only || Some(summary.session.as_str()) != current)
        .collect::<Vec<_>>();
    summaries.sort_by_key(
        |summary| match (summary.attention.healthy, summary.attention.value) {
            (_, Some(AttentionState::NeedsInput)) => 0,
            (false, _) => 1,
            _ if summary.notification_error.is_some() => 1,
            _ => 2,
        },
    );
    summaries
        .into_iter()
        .map(|summary| {
            let record = app
                .state
                .workflow
                .sessions
                .values()
                .find(|record| record.host_session.as_ref() == Some(&summary.session));
            let label = record
                .map(|record| record.ticket.as_str())
                .unwrap_or("Session");
            let status = if summary.exited {
                "ended"
            } else if !summary.attention.healthy {
                "Status unavailable"
            } else {
                match summary.attention.value {
                    Some(AttentionState::NeedsInput) => "Needs input",
                    Some(AttentionState::Running) => "Running",
                    Some(AttentionState::Ready) => "Ready",
                    None => "Status unavailable",
                }
            };
            format!(
                "{label}: {status}{}",
                if summary.notification_error.is_some() {
                    " (notification unavailable · E details)"
                } else {
                    ""
                }
            )
        })
        .collect::<Vec<_>>()
        .join(" · ")
}
pub fn attention(frame: &mut Frame, app: &App, area: Rect) {
    frame.render_widget(
        Paragraph::new(attention_text(app, false)).style(Style::default().fg(Color::Yellow)),
        area,
    );
}

pub fn dialog(frame: &mut Frame, app: &App, area: Rect) {
    let Some(dialog) = &app.hosted.dialog else {
        return;
    };
    let area = Rect::new(
        area.x + 1,
        area.y + 1,
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    frame.render_widget(Clear, area);
    match dialog {
        Dialog::Start {
            brief,
            handoff,
            editing,
            scroll,
            from_session,
            ..
        } => {
            let block = border(
                if from_session.is_some() {
                    " Inspect handover starting brief "
                } else {
                    " Inspect starting brief "
                },
                true,
                app,
            );
            let inner = block.inner(area);
            frame.render_widget(block, area);
            let rows = Layout::vertical([
                Constraint::Length(if from_session.is_some() { 3 } else { 1 }),
                Constraint::Min(1),
                Constraint::Length(6),
                Constraint::Length(2),
            ])
            .split(inner);
            frame.render_widget(Paragraph::new(from_session.as_ref().map(|owner|format!("Explicit handover from {owner}: verify stop before starting the selected task. A failed stop keeps checkout ownership.")).unwrap_or_else(||"No agent starts until y confirms this brief.".into())).wrap(Wrap{trim:false}),rows[0]);
            let text = crate::app::hosted_ui::draft_brief(brief, &handoff.value);
            frame.render_widget(
                Paragraph::new(text)
                    .scroll((*scroll, 0))
                    .wrap(Wrap { trim: false })
                    .block(border(" Exact starting brief ", false, app)),
                rows[1],
            );
            let block = border(
                if *editing {
                    " OPTIONAL HANDOFF · editing · Tab review "
                } else {
                    " Optional multiline handoff · Tab edit "
                },
                *editing,
                app,
            );
            let text_area = block.inner(rows[2]);
            let before = &handoff.value[..handoff.cursor];
            let line = before.chars().filter(|c| *c == '\n').count();
            let column = before.rsplit('\n').next().unwrap_or("").width();
            let sy = line.saturating_sub(text_area.height.saturating_sub(1) as usize);
            let sx = column.saturating_sub(text_area.width.saturating_sub(1) as usize);
            frame.render_widget(
                Paragraph::new(handoff.value.as_str())
                    .scroll((
                        sy.min(u16::MAX as usize) as u16,
                        sx.min(u16::MAX as usize) as u16,
                    ))
                    .block(block),
                rows[2],
            );
            if *editing && text_area.width > 0 && text_area.height > 0 {
                frame.set_cursor_position((
                    text_area.x + (column - sx) as u16,
                    text_area.y + (line - sy) as u16,
                ));
            }
            frame.render_widget(
                Paragraph::new(if *editing {
                    "Ctrl+u clear handoff · Tab/Esc return to review (does not start)"
                } else {
                    "Tab edit handoff · y confirm launch"
                })
                .wrap(Wrap { trim: false }),
                rows[3],
            );
            return;
        }
        Dialog::Sessions { records, cursor } => {
            let items = records
                .iter()
                .map(|record| {
                    ListItem::new(format!(
                        "{} · {:?} · {}",
                        record.provider_session_id,
                        record.health,
                        if record.current {
                            "selected"
                        } else {
                            "recorded"
                        }
                    ))
                })
                .collect::<Vec<_>>();
            frame.render_stateful_widget(
                List::new(items)
                    .block(border(
                        " Conversations · Enter selects only · c continues ",
                        true,
                        app,
                    ))
                    .highlight_style(Style::default().bg(Color::DarkGray)),
                area,
                &mut ListState::default().with_selected(Some(*cursor)),
            );
            return;
        }
        Dialog::RetryStart { scroll } => {
            let details = match &app.hosted.pending_launch {
                Some(crate::hosted::Command::StartRequest {
                    ticket,
                    task,
                    request_id,
                    expected_brief,
                    ..
                }) => format!(
                    "Launch response is unresolved for {ticket} · {task}.\nRetrying uses the same request identity {request_id} and discovers existing work before any new start.\n\nCaptured starting brief:\n{}",
                    expected_brief.as_deref().unwrap_or("No captured brief")
                ),
                _ => "No unresolved local launch request.".into(),
            };
            frame.render_widget(Paragraph::new(details).scroll((*scroll,0)).wrap(Wrap{trim:false}).block(border(" Unresolved launch · y retry same request · f forget local reminder · Esc close ",true,app)),area);
            return;
        }
        Dialog::Diagnostics { scroll } => {
            let mut lines = app.hosted.errors.iter().rev().cloned().collect::<Vec<_>>();
            for summary in &app.hosted.summaries {
                if let Some(error) = &summary.notification_error {
                    lines.push(format!(
                        "Desktop notification for {}: {error}",
                        summary.session
                    ));
                }
                if !summary.attention.healthy {
                    lines.push(format!(
                        "Agent status for {}: {}",
                        summary.session, summary.attention.reason
                    ));
                }
            }
            if let Some(record) = &app.hosted.record
                && let Some(error) = &record.error
            {
                lines.push(format!("Recorded conversation {}: {error}", record.id));
            }
            if lines.is_empty() {
                lines.push("No host or desktop notification errors recorded.".into());
            }
            frame.render_widget(
                Paragraph::new(lines.join("\n\n"))
                    .scroll((*scroll, 0))
                    .wrap(Wrap { trim: false })
                    .block(border(" Host and notification diagnostics ", true, app)),
                area,
            );
            return;
        }
        _ => {}
    }
    let (title,text):(String,String)=match dialog {
        Dialog::Reconcile(_)=>("Examine recorded execution? · y reconcile · Esc cancel".into(),"Inspect the exact saved conversation and native execution without restarting an agent. Uncertain execution remains Interrupted and continues blocking checkout ownership until verified. Resume stays an explicit action after reconciliation.".into()),
        Dialog::Stop(_)=>("Stop agent session? · y confirm · any other key cancel".into(),"Stop the managed process and its descendants. The recorded conversation and branch remain available. This is separate from detaching the interface.".into()),
        Dialog::StopHost=>("Stop session host? · y confirm · any other key cancel".into(),"The host refuses to stop while agents are alive. Stop each session explicitly first.".into()),
        Dialog::Takeover=>("Take terminal control? · y confirm · any other key cancel".into(),"This revokes the previous UI client's keyboard and resize authority.".into()),
        _=>unreachable!(),
    };
    frame.render_widget(
        Paragraph::new(text).wrap(Wrap { trim: false }).block(
            Block::bordered()
                .title(title)
                .border_style(Style::default().fg(accent(app))),
        ),
        area,
    );
}
