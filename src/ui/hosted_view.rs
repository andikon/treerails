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
        Constraint::Length(2),
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
        && !snapshot.cursor_hidden
        && snapshot.cursor.0 < inner.height
        && snapshot.cursor.1 < inner.width
    {
        frame.set_cursor_position((inner.x + snapshot.cursor.1, inner.y + snapshot.cursor.0));
    }
    let exit = app
        .config
        .keys
        .iter()
        .find(|(_, action)| action.as_str() == "agent_navigation")
        .map(|(key, _)| key.as_str())
        .unwrap_or("ctrl+]");
    frame.render_widget(
        Paragraph::new(format!(
            "Enter focus · {exit} navigation · d detach · x stop\n{}",
            if snapshot.attention.healthy {
                ""
            } else {
                &snapshot.attention.reason
            }
        ))
        .style(Style::default().fg(mode_color)),
        rows[2],
    );
}

pub fn attention(frame: &mut Frame, app: &App, area: Rect) {
    let text = app
        .hosted
        .summaries
        .iter()
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
                "status unavailable"
            } else {
                match summary.attention.value {
                    Some(AttentionState::NeedsInput) => "Needs input",
                    Some(AttentionState::Running) => "Running",
                    Some(AttentionState::Ready) => "Ready",
                    None => "status unavailable",
                }
            };
            format!(
                "{label}: {status}{}",
                if summary.notification_error.is_some() {
                    " (notification unavailable)"
                } else {
                    ""
                }
            )
        })
        .collect::<Vec<_>>()
        .join(" · ");
    frame.render_widget(
        Paragraph::new(text).style(Style::default().fg(Color::Yellow)),
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
    let (title,text)=match dialog {
        Dialog::Start {brief,..}=>("Inspect starting brief · y start native Copilot · any other key cancel".to_string(),brief.clone()),
        Dialog::Stop(_)=>("Stop agent session? · y confirm · any other key cancel".into(),"Stop the managed process and its descendants. The recorded conversation and branch remain available. This is separate from detaching the interface.".into()),
        Dialog::StopHost=>("Stop session host? · y confirm · any other key cancel".into(),"The host refuses to stop while agents are alive. Stop each session explicitly first.".into()),
        Dialog::Takeover=>("Take terminal control? · y confirm · any other key cancel".into(),"This revokes the previous UI client's keyboard and resize authority.".into()),
        Dialog::Sessions {records,cursor}=>{
            let items=records.iter().enumerate().map(|(index,record)|format!("{} {} · {:?} · {}",if index==*cursor {">"} else {" "},record.provider_session_id,record.health,if record.current {"current"} else {"recorded"})).collect::<Vec<_>>().join("\n");
            ("Recorded conversations · j/k select · Enter continue · Esc cancel".into(),if items.is_empty() {"No recorded conversations. n starts a new session after brief review.".into()} else {items})
        }
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
