use super::*;
pub(super) fn draw(frame: &mut Frame, app: &App) {
    let Some(editor) = &app.comment_editor else {
        return;
    };
    let area = frame.area();
    frame.render_widget(Clear, area);
    let title = format!(" Comment · {} #{} ", editor.repository, editor.number);
    let (content, footer) = if let Some(review) = &editor.review {
        (
            format!(
                "REVIEW · posting as {}\nTarget: {} #{}\n\n{}\n\n{}",
                review.login, review.repository, review.number, review.text, editor.status
            ),
            "y post · Esc edit",
        )
    } else if let Some(pending) = &editor.draft.pending {
        let mut content = format!(
            "CONFIRMATION UNKNOWN · posting as {}\n{}\n\nSubmitted text:\n{}\n\nDiscussion candidates (inspect author/time/URL; similarity is not proof):\n",
            pending.login, editor.status, pending.text
        );
        if let Some(discussion) = &app.browser.discussion {
            for comment in discussion
                .comments
                .iter()
                .filter(|comment| comment.body == pending.text)
            {
                content.push_str(&format!(
                    "\n{} · {}\n{}\n{}\n",
                    comment.author, comment.created_at, comment.url, comment.body
                ));
            }
            if !discussion.complete {
                content
                    .push_str("\nDiscussion is incomplete; absence here does not prove failure.\n");
            }
        }
        (
            content,
            "R refresh · p verified posted · t review retry · Esc keep",
        )
    } else {
        (
            format!(
                "Draft saved per issue · nothing posts until reviewed\n\n{}\n\n{}",
                editor.draft.text, editor.status
            ),
            "Ctrl+s review · Ctrl+d discard · Esc keep draft",
        )
    };
    let rows = Layout::vertical([Constraint::Min(1), Constraint::Length(2)]).split(area);
    let width = rows[0].width.saturating_sub(2).max(1) as usize;
    let lines = content
        .lines()
        .map(|line| line.width().max(1).div_ceil(width))
        .sum::<usize>();
    let maximum = lines
        .saturating_sub(rows[0].height.saturating_sub(2) as usize)
        .min(u16::MAX as usize) as u16;
    let paragraph = Paragraph::new(content)
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(paragraph.scroll((editor.scroll.min(maximum), 0)), rows[0]);
    frame.render_widget(
        Paragraph::new(if editor.posting {
            "Waiting for GitHub…"
        } else {
            footer
        })
        .wrap(Wrap { trim: false }),
        rows[1],
    );
}
