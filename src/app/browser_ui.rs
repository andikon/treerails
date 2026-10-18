use super::*;
use crate::{
    browser::Editor,
    ticket_browser::{
        self as browser, AssigneeFilter, Command, Event, Filter, IssueState, LabelMode, Sort,
    },
};
use std::{
    hash::{Hash, Hasher},
    path::Path,
};

pub(super) fn token(project: &Path, command: &Command) -> String {
    let suffix = match command {
        Command::Load { request } => format!("list:{request}:load"),
        Command::Page { request, page, .. } => format!("list:{request}:page:{page}"),
        Command::Discussion { request, page, .. } => format!("preview:{request}:page:{page}"),
        Command::Metadata { request, .. } => format!("metadata:{request}"),
        Command::Cards {
            request, numbers, ..
        } => format!("cards:{request}:{numbers:?}"),
        _ => {
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            format!("{command:?}").hash(&mut hash);
            format!("edit:{}", hash.finish())
        }
    };
    format!("browser:{suffix}:{}", project.display())
}

impl App {
    pub(super) fn browser_failed(&mut self, token: &str, message: String) -> bool {
        if token.starts_with("browser:list:") {
            if !token.starts_with(&format!("browser:list:{}:", self.browser.request)) {
                return false;
            }
            self.browser.loading = false;
            self.browser.warning = format!("GitHub unavailable: {message}");
        } else if token.starts_with("browser:preview:") {
            if !token.starts_with(&format!(
                "browser:preview:{}:",
                self.browser.preview_request
            )) {
                return false;
            }
            self.browser.preview_loading = false;
            self.browser.preview_warning = message;
        } else if token.starts_with("browser:edit:") {
            if self.browser.editor.is_none() {
                self.browser.editor = self.browser.pending_editor.take();
            }
            self.finish_browser_edit();
        }
        true
    }
    pub(super) fn browser_tick(&mut self) {
        let active = self
            .active_browser_tickets()
            .iter()
            .map(|entry| entry.ticket.issue.number)
            .collect::<Vec<_>>();
        if active != self.browser.active_numbers && !self.browser.repository.is_empty() {
            self.refresh_active_cards();
        }
        if self.ticket_mode
            && self.project.is_some()
            && self
                .browser
                .work_refreshed_at
                .is_none_or(|last| last.elapsed() > Duration::from_secs(2))
        {
            self.browser.work_refreshed_at = Some(Instant::now());
            self.enqueue(Job::LocalWorkflow(self.config_path.clone()));
        }
        if self.ticket_mode
            && self.ticket.is_none()
            && self.project.is_some()
            && !self.browser.repository.is_empty()
            && !self.browser.loading
            && self
                .browser
                .refreshed_at
                .is_some_and(|last| last.elapsed() > Duration::from_secs(60))
        {
            self.browser.refreshed_at = Some(Instant::now());
            self.query_browser();
            self.preview_selected();
        }
    }
    pub(super) fn load_browser(&mut self) {
        if self.browser.repository.is_empty()
            && let Some(project) = &self.project
            && let Some(ticket) = self
                .state
                .workflow
                .tickets
                .values()
                .find(|ticket| same_path(&ticket.project, &project.path))
        {
            self.browser.repository =
                crate::ticket_browser::normalize_repository(&ticket.repository);
        }
        if self.browser.selected.is_none() {
            self.browser.selected = self
                .active_browser_tickets()
                .first()
                .map(|entry| entry.ticket.issue.number);
        }
        self.browser.request += 1;
        self.browser.loading = true;
        self.workflow_command(WorkflowCommand::Browser(Command::Load {
            request: self.browser.request,
        }));
    }
    fn browser_command(&mut self, command: Command) {
        if matches!(
            command,
            Command::SaveView { .. }
                | Command::SelectView { .. }
                | Command::DeleteView { .. }
                | Command::DuplicateView { .. }
                | Command::SetActiveCollapsed { .. }
        ) {
            if self.browser.editing {
                self.browser.edit_queue.push_back(command);
                return;
            }
            self.browser.editing = true;
        }
        self.workflow_command(WorkflowCommand::Browser(command));
    }
    fn finish_browser_edit(&mut self) {
        self.browser.editing = false;
        if let Some(next) = self.browser.edit_queue.pop_front() {
            self.browser_command(next);
        }
    }
    fn refresh_active_cards(&mut self) {
        let numbers = self
            .active_browser_tickets()
            .iter()
            .map(|entry| entry.ticket.issue.number)
            .collect::<Vec<_>>();
        self.browser.active_numbers = numbers.clone();
        if !numbers.is_empty() {
            self.browser_command(Command::Cards {
                repository: self.browser.repository.clone(),
                request: self.browser.request,
                numbers,
            });
        }
    }
    pub fn active_browser_tickets(&self) -> Vec<crate::browser::ActiveTicket> {
        let repository = self
            .browser
            .repository
            .strip_prefix("github.com/")
            .unwrap_or(&self.browser.repository);
        crate::browser::active_tickets(&self.state.workflow, repository, &self.hosted.summaries)
    }
    pub fn browser_numbers(&self) -> Vec<u64> {
        let active = self.active_browser_tickets();
        let mut numbers = if self.browser.active_collapsed {
            vec![]
        } else {
            active
                .iter()
                .map(|active| active.ticket.issue.number)
                .collect::<Vec<_>>()
        };
        for card in &self.browser.cards {
            if !active
                .iter()
                .any(|active| active.ticket.issue.number == card.issue.number)
            {
                numbers.push(card.issue.number);
            }
        }
        numbers
    }
    fn preview_selected(&mut self) {
        let Some(number) = self.browser.selected else {
            return;
        };
        self.browser.preview_request += 1;
        self.browser.preview_page = 1;
        self.browser.preview_loading = true;
        self.browser.preview_warning.clear();
        self.browser_command(Command::Discussion {
            repository: self.browser.repository.clone(),
            request: self.browser.preview_request,
            number,
            page: 1,
        });
    }
    fn query_browser(&mut self) {
        self.browser.refresh_pages = if self.browser.reselect_after_query {
            1
        } else {
            self.browser.page.max(1)
        };
        self.browser.request += 1;
        self.browser.loading = true;
        self.browser_command(Command::Page {
            repository: self.browser.repository.clone(),
            request: self.browser.request,
            filter: self.browser.filter(),
            page: 1,
        });
    }
    pub(super) fn accept_browser(&mut self, event: Event) {
        match event {
            Event::Cards {
                repository,
                request,
                cards,
                warning,
            } => {
                if repository != self.browser.repository || request != self.browser.request {
                    return;
                }
                for card in cards {
                    self.browser.active_cards.insert(card.issue.number, card);
                }
                if let Some(warning) = warning {
                    self.say(warning, true);
                }
            }
            Event::Metadata {
                repository,
                request,
                metadata,
            } => {
                if repository == self.browser.repository && request == self.browser.request {
                    self.browser.metadata = Some(metadata);
                }
            }
            Event::Loaded {
                repository,
                request,
                views,
                page,
            } => {
                if request != self.browser.request {
                    return;
                }
                if repository == self.browser.repository
                    && views.revision < self.browser.views.revision
                {
                    return;
                }
                self.browser.repository = repository;
                self.browser.active_collapsed = views.active_collapsed;
                self.browser.views = views;
                self.browser.cards.clear();
                self.accept_browser_page(page);
                self.say("Browsing GitHub tickets", false);
            }
            Event::Page {
                repository,
                request,
                page,
            } => {
                if repository != self.browser.repository || request != self.browser.request {
                    return;
                }
                if page.page == 1 {
                    self.browser.cards.clear();
                    self.refresh_active_cards();
                }
                self.accept_browser_page(page);
            }
            Event::Discussion {
                repository,
                request,
                number,
                discussion,
            } => {
                if repository != self.browser.repository
                    || request != self.browser.preview_request
                    || Some(number) != self.browser.selected
                {
                    return;
                }
                self.browser.preview_loading = false;
                self.browser
                    .active_cards
                    .insert(number, discussion.card.clone());
                let more = !discussion.complete && discussion.warning.is_none();
                self.browser.discussion = Some(discussion);
                if more {
                    self.browser.preview_page += 1;
                    self.browser.preview_loading = true;
                    self.browser_command(Command::Discussion {
                        repository: self.browser.repository.clone(),
                        request: self.browser.preview_request,
                        number,
                        page: self.browser.preview_page,
                    });
                }
            }
            Event::Views { repository, views } => {
                if repository != self.browser.repository {
                    return;
                }
                if views.revision < self.browser.views.revision {
                    self.finish_browser_edit();
                    return;
                }
                let changed = self.browser.views.active != views.active
                    || self.browser.filter() != views.selected().filter;
                self.browser.active_collapsed = views.active_collapsed;
                self.browser.pending_editor = None;
                self.browser.views = views;
                if changed {
                    self.browser.reselect_after_query = true;
                    self.browser.cards.clear();
                    self.query_browser();
                }
                self.finish_browser_edit();
            }
        }
    }
    fn accept_browser_page(&mut self, page: browser::Page) {
        self.browser.refreshed_at = Some(Instant::now());
        self.browser.loading = false;
        self.browser.page = page.page;
        self.browser.has_more = page.has_more;
        self.browser.total = page.total;
        self.browser.limited = page.limited;
        self.browser.warning = page.warning.unwrap_or_default();
        for card in page.issues {
            if let Some(old) = self
                .browser
                .cards
                .iter_mut()
                .find(|old| old.issue.number == card.issue.number)
            {
                *old = card;
            } else {
                self.browser.cards.push(card);
            }
        }
        // A background refresh retains a now-filtered-out preview until selection changes.
        if self.browser.reselect_after_query
            && !self
                .browser_numbers()
                .iter()
                .any(|number| Some(*number) == self.browser.selected)
        {
            self.browser.selected = None;
            self.browser.discussion = None;
        }
        self.browser.reselect_after_query = false;
        if self.browser.selected.is_none() {
            self.browser.selected = self.browser_numbers().first().copied();
            self.preview_selected();
        } else if self.browser.discussion.is_none() && !self.browser.preview_loading {
            self.preview_selected();
        }
        self.issues = self
            .browser
            .cards
            .iter()
            .map(|card| card.issue.clone())
            .collect();
        if self.browser.page < self.browser.refresh_pages
            && self.browser.has_more
            && self.browser.warning.is_empty()
        {
            self.browser.loading = true;
            self.browser_command(Command::Page {
                repository: self.browser.repository.clone(),
                request: self.browser.request,
                filter: self.browser.filter(),
                page: self.browser.page + 1,
            });
        }
    }
    pub(super) fn browser_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return false;
        }
        match key.code {
            KeyCode::Char('j' | 'k') | KeyCode::Up | KeyCode::Down
                if self.browser.preview_focus =>
            {
                self.browser.preview_scroll =
                    if matches!(key.code, KeyCode::Char('k') | KeyCode::Up) {
                        self.browser.preview_scroll.saturating_sub(1)
                    } else {
                        self.browser.preview_scroll.saturating_add(1)
                    };
            }
            KeyCode::Char('j' | 'k') | KeyCode::Up | KeyCode::Down => {
                let numbers = self.browser_numbers();
                let current = numbers
                    .iter()
                    .position(|number| Some(*number) == self.browser.selected)
                    .unwrap_or(0);
                let index = step(
                    current,
                    if matches!(key.code, KeyCode::Char('k') | KeyCode::Up) {
                        -1
                    } else {
                        1
                    },
                    numbers.len(),
                );
                self.browser.selected = numbers.get(index).copied();
                self.browser.preview_scroll = 0;
                self.preview_selected();
                if index.saturating_add(5) >= numbers.len()
                    && self.browser.has_more
                    && !self.browser.loading
                {
                    self.browser.loading = true;
                    self.browser_command(Command::Page {
                        repository: self.browser.repository.clone(),
                        request: self.browser.request,
                        filter: self.browser.filter(),
                        page: self.browser.page + 1,
                    });
                }
            }
            KeyCode::Tab
            | KeyCode::BackTab
            | KeyCode::Char('h' | 'l')
            | KeyCode::Left
            | KeyCode::Right => self.browser.preview_focus = !self.browser.preview_focus,
            KeyCode::PageDown => {
                self.browser.preview_scroll = self.browser.preview_scroll.saturating_add(10)
            }
            KeyCode::PageUp => {
                self.browser.preview_scroll = self.browser.preview_scroll.saturating_sub(10)
            }
            KeyCode::Enter => {
                if let Some(number) = self.browser.selected {
                    if let Some(active) = self
                        .active_browser_tickets()
                        .iter()
                        .find(|active| active.ticket.issue.number == number)
                    {
                        self.workflow_command(WorkflowCommand::Reopen {
                            ticket: active.ticket.id(),
                        });
                    } else {
                        self.workflow_command(WorkflowCommand::Open { number });
                    }
                }
            }
            KeyCode::Char('o') => {
                if let (Some(project), Some(number)) = (&self.project, self.browser.selected) {
                    self.enqueue(Job::OpenIssue(project.path.clone(), number));
                }
            }
            KeyCode::Char('a') => {
                self.browser.active_collapsed = !self.browser.active_collapsed;
                self.browser_command(Command::SetActiveCollapsed {
                    repository: self.browser.repository.clone(),
                    collapsed: self.browser.active_collapsed,
                });
            }
            KeyCode::Char('R') => {
                self.query_browser();
                self.preview_selected();
            }
            KeyCode::Char('f') => self.open_browser_filters(),
            KeyCode::Char('c') => self.begin_comment(),
            KeyCode::Char('v') => self.browser.editor = Some(Editor::Views { cursor: 0 }),
            KeyCode::Char('?') => self.modal = Some(Modal::Help),
            _ => return false,
        }
        true
    }
    fn open_browser_filters(&mut self) {
        self.browser_command(Command::Metadata {
            repository: self.browser.repository.clone(),
            request: self.browser.request,
        });
        let filter = self.browser.filter();
        let assignees = match filter.assignees {
            AssigneeFilter::Anyone => "anyone".into(),
            AssigneeFilter::Me => "me".into(),
            AssigneeFilter::Unassigned => "unassigned".into(),
            AssigneeFilter::Users(users) => users.join(","),
        };
        self.browser.editor = Some(Editor::Filters {
            fields: vec![
                format!("{:?}", filter.state).to_lowercase(),
                assignees,
                filter.labels.join(","),
                format!("{:?}", filter.label_mode).to_lowercase(),
                filter.exclude_labels.join(","),
                filter.text,
                if filter.include_comments { "yes" } else { "no" }.into(),
                format!("{:?}", filter.sort).to_lowercase(),
            ],
            cursor: 0,
        });
    }
    pub(super) fn browser_editor_key(&mut self, key: KeyEvent) {
        let Some(mut editor) = self.browser.editor.take() else {
            return;
        };
        if key.code == KeyCode::Esc {
            return;
        }
        let mut keep = true;
        match &mut editor {
            Editor::Filters { fields, cursor } => match key.code {
                KeyCode::Tab | KeyCode::Down => *cursor = (*cursor + 1) % fields.len(),
                KeyCode::BackTab | KeyCode::Up => {
                    *cursor = (*cursor + fields.len() - 1) % fields.len()
                }
                KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    match parse_filters(fields) {
                        Ok(filter) => {
                            let view = self.browser.views.selected();
                            self.browser_command(Command::SaveView {
                                repository: self.browser.repository.clone(),
                                id: view.id.clone(),
                                name: view.name.clone(),
                                filter,
                                expected: Some(view.clone()),
                            });
                            keep = false;
                        }
                        Err(error) => self.say(error, true),
                    }
                }
                KeyCode::Backspace => {
                    fields[*cursor].pop();
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    fields[*cursor].clear()
                }
                KeyCode::Char(value) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    fields[*cursor].push(value)
                }
                _ => {}
            },
            Editor::Views { cursor } => match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    *cursor = step(*cursor, 1, self.browser.views.views.len())
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    *cursor = step(*cursor, -1, self.browser.views.views.len())
                }
                KeyCode::Enter => {
                    if let Some(view) = self.browser.views.views.get(*cursor) {
                        self.browser_command(Command::SelectView {
                            repository: self.browser.repository.clone(),
                            id: view.id.clone(),
                        });
                    }
                    keep = false;
                }
                KeyCode::Char('n' | 'd' | 'r') => {
                    let action = match key.code {
                        KeyCode::Char('n') => "create",
                        KeyCode::Char('d') => "duplicate",
                        _ => "rename",
                    };
                    editor = Editor::Name {
                        action: action.into(),
                        text: String::new(),
                        id: self
                            .browser
                            .views
                            .views
                            .get(*cursor)
                            .map(|view| view.id.clone())
                            .unwrap_or_default(),
                    };
                }
                KeyCode::Char('x') => {
                    if let Some(view) = self.browser.views.views.get(*cursor) {
                        self.browser_command(Command::DeleteView {
                            repository: self.browser.repository.clone(),
                            id: view.id.clone(),
                        });
                    }
                    *cursor = 0;
                }
                _ => {}
            },
            Editor::Name { action, text, id } => match key.code {
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::Enter if !text.trim().is_empty() => {
                    let selected = self
                        .browser
                        .views
                        .views
                        .iter()
                        .find(|view| view.id == *id)
                        .unwrap_or_else(|| self.browser.views.selected());
                    let command = if action == "duplicate" {
                        Command::DuplicateView {
                            repository: self.browser.repository.clone(),
                            id: selected.id.clone(),
                            name: text.trim().into(),
                        }
                    } else {
                        Command::SaveView {
                            repository: self.browser.repository.clone(),
                            id: if action == "create" {
                                String::new()
                            } else {
                                selected.id.clone()
                            },
                            name: text.trim().into(),
                            filter: if action == "create" {
                                Filter::default()
                            } else {
                                selected.filter.clone()
                            },
                            expected: if action == "create" {
                                None
                            } else {
                                Some(selected.clone())
                            },
                        }
                    };
                    self.browser_command(command);
                    keep = false;
                }
                KeyCode::Char(value) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    text.push(value)
                }
                _ => {}
            },
        }
        if keep {
            self.browser.editor = Some(editor);
        } else if !matches!(editor, Editor::Views { .. }) {
            self.browser.pending_editor = Some(editor);
        }
    }
}

fn parse_filters(fields: &[String]) -> Result<Filter, String> {
    let list = |text: &str| {
        text.split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let state = match fields[0].trim().to_lowercase().as_str() {
        "open" => IssueState::Open,
        "closed" => IssueState::Closed,
        "all" => IssueState::All,
        _ => return Err("State must be open, closed or all".into()),
    };
    let assignees = match fields[1].trim().to_lowercase().as_str() {
        "anyone" | "" => AssigneeFilter::Anyone,
        "me" => AssigneeFilter::Me,
        "unassigned" => AssigneeFilter::Unassigned,
        _ => AssigneeFilter::Users(list(&fields[1])),
    };
    let label_mode = match fields[3].trim().to_lowercase().as_str() {
        "all" => LabelMode::All,
        "any" => LabelMode::Any,
        _ => return Err("Label mode must be all or any".into()),
    };
    let labels = list(&fields[2]);
    let exclude_labels = list(&fields[4]);
    if labels.iter().any(|label| exclude_labels.contains(label)) {
        return Err("A label cannot be both required and excluded".into());
    }
    let include_comments = match fields[6].trim().to_lowercase().as_str() {
        "yes" => true,
        "no" => false,
        _ => return Err("Include comments must be yes or no".into()),
    };
    let sort = match fields[7].trim().to_lowercase().as_str() {
        "updated" => Sort::Updated,
        "newest" => Sort::Newest,
        "oldest" => Sort::Oldest,
        _ => return Err("Sort must be updated, newest or oldest".into()),
    };
    Ok(Filter {
        state,
        assignees,
        labels,
        label_mode,
        exclude_labels,
        text: fields[5].clone(),
        include_comments,
        sort,
    })
}
