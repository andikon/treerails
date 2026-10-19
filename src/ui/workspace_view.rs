use super::*;
use crate::app::workspace_ui::WorkspaceTab;
pub(super) fn work(frame: &mut Frame, app: &App, area: Rect) {
    let rows = app
        .project
        .as_ref()
        .map(|p| {
            crate::work::rows_with_attention(&app.state.workflow, &p.path, &app.hosted.summaries)
        })
        .unwrap_or_default();
    let items = rows
        .iter()
        .map(|row| {
            let source = row
                .issue
                .as_ref()
                .map(|issue| format!("#{}", issue.number))
                .unwrap_or_else(|| "Local".into());
            let branch = row
                .checkout
                .as_ref()
                .map(|c| c.branch.as_str())
                .unwrap_or("No checkout");
            ListItem::new(vec![
                Line::from(format!(
                    "{} / {} / {}",
                    row.workspace_title, row.title, row.status
                )),
                Line::styled(
                    format!("  {source} / {branch} / task {:?}", row.progress),
                    Style::default().fg(Color::Gray),
                ),
                Line::from(""),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_stateful_widget(
        List::new(items)
            .block(border(
                " Work / W Work / T Tickets / w Worktrees ",
                true,
                app,
            ))
            .highlight_style(Style::default().bg(Color::DarkGray)),
        area,
        &mut ListState::default().with_selected(Some(app.work_cursor)),
    );
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new("No tasks yet. Find a ticket or open a worktree, then start a task.")
                .wrap(Wrap { trim: false }),
            Rect::new(
                area.x + 1,
                area.y + 2,
                area.width.saturating_sub(2),
                area.height.saturating_sub(3),
            ),
        );
    }
}
pub(super) fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let rows = Layout::vertical([Constraint::Length(4), Constraint::Min(1)]).split(area);
    let workspace = app.current_workspace();
    let title = workspace
        .as_ref()
        .map(|w| w.title.clone())
        .unwrap_or_else(|| "Checkout workspace / no local task yet".into());
    let task = workspace
        .as_ref()
        .and_then(|w| w.tasks.get(app.task_cursor))
        .filter(|t| t.title != title)
        .map(|t| format!(" / {}", t.title))
        .unwrap_or_default();
    let checkout = workspace
        .as_ref()
        .and_then(|w| w.worktree.as_ref())
        .map(|c| format!("{} / {}", c.branch, c.path.display()))
        .or_else(|| {
            app.selected_tree()
                .map(|t| format!("{} / {}", t.branch, t.path.display()))
        })
        .unwrap_or_else(|| "Checkout not prepared".into());
    frame.render_widget(
        Paragraph::new(format!(
            "{title}{task}\n{checkout}\n1 Overview / 2 Changes / 3 Agent / 4 Discussion / 5 Activity   [{:?}]",
            app.workspace_tab
        ))
        .style(Style::default().fg(accent(app))),
        rows[0],
    );
    match app.workspace_tab {
        WorkspaceTab::Changes => {
            if app.workspace_changes_ready() {
                super::workspace(frame, app, rows[1]);
            } else {
                frame.render_widget(Paragraph::new("This workspace checkout is missing or not loaded. Prepare or adopt a checkout in Overview before Git actions.").wrap(Wrap{trim:false}).block(border(" Changes unavailable ",true,app)),rows[1]);
            }
        }
        WorkspaceTab::Agent => {
            if app.hosted.terminal.is_some()
                && app
                    .hosted
                    .record
                    .as_ref()
                    .is_some_and(|record| app.workspace_session_matches(record))
            {
                super::hosted_view::terminal(frame, app, rows[1]);
            } else {
                frame.render_widget(Paragraph::new("s Continue / start agent / n New conversation\nReview the starting context before launch.\nAn existing checkout can use a local task without a GitHub issue.").wrap(Wrap{trim:false}).block(border(" Agent ",true,app)),rows[1]);
            }
        }
        WorkspaceTab::Discussion => {
            if app.ticket.is_some() {
                super::browser_view::preview(frame, app, rows[1]);
            } else {
                frame.render_widget(Paragraph::new("This local workspace has no linked GitHub issue. Its task objective supplies agent context.\nL links this work to an already opened GitHub ticket, preserving its tasks and conversations.").wrap(Wrap{trim:false}).block(border(" Discussion ",true,app)),rows[1]);
            }
        }
        WorkspaceTab::Activity => {
            let text = app
                .operation_records()
                .into_iter()
                .filter(|r| {
                    workspace
                        .as_ref()
                        .and_then(|w| w.worktree.as_ref())
                        .is_some_and(|c| crate::config::same_path(&r.target, &c.path))
                })
                .map(|r| r.headline())
                .collect::<Vec<_>>()
                .join("\n");
            frame.render_widget(
                Paragraph::new(if text.is_empty() {
                    "No Git operations recorded for this checkout in this UI session.".into()
                } else {
                    text
                })
                .wrap(Wrap { trim: false })
                .block(border(
                    " Activity / E operation details / H history ",
                    true,
                    app,
                )),
                rows[1],
            );
        }
        WorkspaceTab::Overview => {
            if let Some(w) = workspace {
                let checkout = w
                    .worktree
                    .as_ref()
                    .map(|c| format!("{} / {}", c.branch, c.path.display()))
                    .unwrap_or_else(|| "No checkout / c create or a adopt".into());
                let mut lines = vec![Line::from(checkout), Line::from("")];
                for (index, t) in w.tasks.iter().enumerate() {
                    lines.push(Line::from(format!(
                        "{} {} / {:?}",
                        if index == app.task_cursor { ">" } else { " " },
                        t.title,
                        t.progress
                    )));
                }
                if let Some(t) = w.tasks.get(app.task_cursor) {
                    let objective = app
                        .state
                        .workflow
                        .local_work
                        .get(&w.id)
                        .and_then(|local| local.objectives.get(&t.id))
                        .or_else(|| {
                            app.state
                                .workflow
                                .task_objectives
                                .get(&format!("{}/{}", w.id, t.id))
                        })
                        .cloned()
                        .unwrap_or_else(|| t.title.clone());
                    lines.push(Line::from(""));
                    lines.push(Line::from("Selected objective"));
                    lines.extend(objective.lines().map(|line| Line::from(line.to_owned())));
                    if let Some(row) = crate::work::rows_with_attention(
                        &app.state.workflow,
                        &w.project,
                        &app.hosted.summaries,
                    )
                    .iter()
                    .find(|row| row.workspace == w.id && row.task == t.id)
                    {
                        lines.push(Line::from(
                            if row
                                .session
                                .as_ref()
                                .is_some_and(|s| s.health == crate::hosted::Health::Stopped)
                            {
                                "Agent: stopped conversation".into()
                            } else if row.session.is_some() {
                                format!("Agent: {}", row.status)
                            } else {
                                "Agent: no recorded conversation".into()
                            },
                        ));
                    }
                }
                frame.render_widget(
                    Paragraph::new(lines)
                        .wrap(Wrap { trim: false })
                        .block(border(" Task workspace / p progress / s agent ", true, app)),
                    rows[1],
                );
            } else {
                frame.render_widget(Paragraph::new("Inspect changes in this checkout or choose s to create a local task and start an agent.").wrap(Wrap{trim:false}).block(border(" Workspace ",true,app)),rows[1]);
            }
        }
    }
}
