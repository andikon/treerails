use super::*;
use crate::ticket_comments::{self, Command, Editor, Event};

impl App {
    pub(super) fn begin_comment(&mut self) {
        let Some(number) = self.browser.selected else {
            return;
        };
        let repository = self.browser.repository.clone();
        match ticket_comments::load(&self.config_path, &repository, number) {
            Ok(draft) => {
                self.comment_editor = Some(Editor {
                    repository,
                    number,
                    status: draft.error.clone().unwrap_or_default(),
                    draft,
                    review: None,
                    retry_uncertain: false,
                    posting: false,
                    scroll: 0,
                })
            }
            Err(error) => self.say(format!("Cannot open draft: {error:#}"), true),
        }
    }
    pub(super) fn comment_key(&mut self, key: KeyEvent) -> bool {
        let Some(mut editor) = self.comment_editor.take() else {
            return false;
        };
        if matches!(key.code, KeyCode::PageDown | KeyCode::PageUp) {
            editor.scroll = if key.code == KeyCode::PageDown {
                editor.scroll.saturating_add(10)
            } else {
                editor.scroll.saturating_sub(10)
            };
            self.comment_editor = Some(editor);
            return true;
        }
        let mut command = None;
        let mut close = false;
        if editor.posting {
            editor.status =
                "Waiting for GitHub; this send will never be retried automatically".into();
        } else if let Some(review) = editor.review.clone() {
            match key.code {
                KeyCode::Esc => editor.review = None,
                KeyCode::Char('y') => {
                    editor.posting = true;
                    command = Some(Command::Send {
                        review,
                        retry_uncertain: editor.retry_uncertain,
                    });
                }
                _ => {}
            }
        } else if let Some(pending) = &editor.draft.pending {
            match key.code {
                KeyCode::Esc => close = true,
                KeyCode::Char('p') => {
                    command = Some(Command::Resolve {
                        repository: editor.repository.clone(),
                        number: editor.number,
                        attempt: pending.attempt.clone(),
                        posted: true,
                    });
                    editor.posting = true;
                }
                KeyCode::Char('t') => {
                    editor.retry_uncertain = true;
                    command = Some(Command::Review {
                        repository: editor.repository.clone(),
                        number: editor.number,
                    });
                }
                KeyCode::Char('R') => {
                    self.browser.preview_request += 1;
                    self.browser.preview_page = 1;
                    self.browser.preview_loading = true;
                    self.workflow_command(WorkflowCommand::Browser(
                        crate::ticket_browser::Command::Discussion {
                            repository: editor.repository.clone(),
                            request: self.browser.preview_request,
                            number: editor.number,
                            page: 1,
                        },
                    ));
                    editor.status = "Refreshing discussion. Similar text is only a candidate, not proof of publication.".into();
                }
                _ => {}
            }
        } else if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
            if editor.status.starts_with("Draft not saved") {
                editor.status="Draft not saved: resolve the save conflict before reviewing; your current text is still here".into();
            } else {
                command = Some(Command::Review {
                    repository: editor.repository.clone(),
                    number: editor.number,
                });
            }
        } else if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('d') {
            command = Some(Command::Discard {
                repository: editor.repository.clone(),
                number: editor.number,
                expected_revision: editor.draft.revision,
            });
        } else {
            let mut text = editor.draft.text.clone();
            match key.code {
                KeyCode::Esc => close = true,
                KeyCode::Enter => text.push('\n'),
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::Char(ch)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    text.push(ch)
                }
                _ => {}
            }
            if text != editor.draft.text {
                match ticket_comments::save_text(
                    &self.config_path,
                    &editor.repository,
                    editor.number,
                    text.clone(),
                    editor.draft.revision,
                ) {
                    Ok(draft) => {
                        editor.draft = draft;
                        editor.status.clear();
                    }
                    Err(error) => {
                        editor.draft.text = text;
                        editor.status = format!("Draft not saved: {error:#}");
                    }
                }
            }
            if close && editor.status.starts_with("Draft not saved") {
                close = false;
            }
        }
        if !close {
            self.comment_editor = Some(editor);
        }
        if let Some(command) = command {
            self.workflow_command(WorkflowCommand::Comment(command));
        }
        true
    }
    pub(super) fn accept_comment(&mut self, event: Event) {
        match event {
            Event::Review(review) => {
                if let Some(editor) = &mut self.comment_editor
                    && editor.repository == review.repository
                    && editor.number == review.number
                {
                    editor.review = Some(review);
                    editor.posting = false;
                    editor.scroll = 0;
                }
            }
            Event::Draft {
                repository,
                number,
                draft,
            }
            | Event::Unknown {
                repository,
                number,
                draft,
            }
            | Event::Rejected {
                repository,
                number,
                draft,
            } => {
                if let Some(editor) = &mut self.comment_editor
                    && editor.repository == repository
                    && editor.number == number
                {
                    editor.status = draft.error.clone().unwrap_or_default();
                    editor.draft = draft;
                    editor.review = None;
                    editor.posting = false;
                    editor.retry_uncertain = false;
                }
            }
            Event::Posted {
                repository,
                number,
                comment,
            } => {
                self.comment_editor = None;
                if self.browser.repository == repository
                    && self.browser.selected == Some(number)
                    && let Some(discussion) = &mut self.browser.discussion
                {
                    discussion
                        .comments
                        .retain(|existing| existing.id != comment.id);
                    discussion.comments.push(comment);
                    self.browser.preview_scroll = u16::MAX;
                }
                self.say("Comment posted", false);
            }
        }
    }
    pub fn comment_paste(&mut self, text: &str) -> bool {
        let Some(editor) = &mut self.comment_editor else {
            return false;
        };
        if editor.review.is_some() || editor.draft.pending.is_some() || editor.posting {
            return true;
        }
        let mut body = editor.draft.text.clone();
        body.push_str(text);
        match ticket_comments::save_text(
            &self.config_path,
            &editor.repository,
            editor.number,
            body.clone(),
            editor.draft.revision,
        ) {
            Ok(draft) => {
                editor.draft = draft;
                editor.status.clear();
            }
            Err(error) => {
                editor.draft.text = body;
                editor.status = format!("Draft not saved: {error:#}");
            }
        }
        true
    }
}
