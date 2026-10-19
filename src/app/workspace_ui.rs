use super::*;

pub struct BrowserPreview {
    project: Option<PathBuf>,
    repository: String,
    selected: Option<u64>,
    discussion: Option<crate::ticket_browser::Discussion>,
    scroll: u16,
    page: u32,
    warning: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]

pub enum WorkspaceTab {
    #[default]
    Overview,
    Changes,
    Agent,
    Discussion,
    Activity,
}

impl App {
    fn remember_browser_preview(&mut self) {
        if self.browser_return.is_none() {
            self.browser_return = Some(BrowserPreview {
                project: self.project.as_ref().map(|project| project.path.clone()),
                repository: self.browser.repository.clone(),
                selected: self.browser.selected,
                discussion: self.browser.discussion.clone(),
                scroll: self.browser.preview_scroll,
                page: self.browser.preview_page,
                warning: self.browser.preview_warning.clone(),
            });
        }
    }

    pub fn restore_browser_preview(&mut self) {
        let Some(saved) = self.browser_return.take() else {
            return;
        };
        if saved
            .project
            .as_ref()
            .zip(self.project.as_ref())
            .is_none_or(|(path, project)| !same_path(path, &project.path))
        {
            return;
        }
        self.browser.repository = saved.repository;
        self.browser.selected = saved.selected;
        self.browser.discussion = saved.discussion;
        self.browser.preview_scroll = saved.scroll;
        self.browser.preview_page = saved.page;
        self.browser.preview_warning = saved.warning;
        self.browser.preview_loading = false;
        self.browser.preview_request += 1;
    }
    pub fn current_workspace(&self) -> Option<crate::workspace::Workspace> {
        self.workspace
            .as_ref()
            .and_then(|w| crate::workspace::resolve(&self.state.workflow, &w.id).ok())
            .or_else(|| self.workspace.clone())
            .or_else(|| {
                self.ticket
                    .as_ref()
                    .and_then(|t| crate::workspace::resolve(&self.state.workflow, &t.id()).ok())
            })
    }

    pub fn workspace_session_matches(&self, record: &crate::hosted::SessionRecord) -> bool {
        self.current_workspace().is_some_and(|workspace| {
            workspace.id == record.ticket
                && workspace
                    .tasks
                    .get(self.task_cursor)
                    .is_some_and(|task| task.id == record.task)
        })
    }
    pub fn workspace_changes_ready(&self) -> bool {
        match self.current_workspace() {
            Some(workspace) => workspace.worktree.as_ref().is_some_and(|checkout| {
                checkout.path.is_dir()
                    && self
                        .selected_tree()
                        .is_some_and(|tree| same_path(&tree.path, &checkout.path))
            }),
            None => self.selected_tree().is_some_and(|tree| tree.path.is_dir()),
        }
    }
    pub fn sync_workspace_checkout(&mut self) {
        if self
            .hosted
            .record
            .as_ref()
            .is_some_and(|record| !self.workspace_session_matches(record))
        {
            self.detach_agent();
            self.hosted.record = None;
        }
        if self.workspace_tab == WorkspaceTab::Changes {
            self.focus = self.focus.clamp(1, 2);
        }
        if let Some(checkout) = self.current_workspace().and_then(|w| w.worktree) {
            self.filter.clear();

            if let Some(index) = self
                .trees
                .iter()
                .position(|tree| same_path(&tree.path, &checkout.path))
            {
                self.tree = index;

                self.selection_changed();
            } else {
                self.desired_tree = Some(checkout.path);
            }
        }
    }

    pub fn switch_entry(&mut self, entry: &str) {
        self.workspace_start_pending = false;
        self.restore_browser_preview();
        self.workspace_open = false;
        self.workspace = None;
        self.ticket = None;
        self.hosted.input = false;

        self.work_mode = entry == "work";

        self.ticket_mode = entry == "tickets";

        self.filter.clear();
        self.focus = 0;

        if let Some(project) = self.project.as_ref() {
            self.state
                .workspace_views
                .insert(project.path.to_string_lossy().to_string(), entry.into());
        }

        if self.ticket_mode {
            self.load_browser();
        }

        self.say("W Work · T Tickets · w Worktrees", false);
    }

    pub fn open_worktree_workspace(&mut self) {
        let Some(path) = self.selected_tree().map(|tree| tree.path.clone()) else {
            return;
        };

        self.workspace = crate::workspace::for_checkout(&self.state.workflow, &path);
        self.task_cursor = self
            .workspace
            .as_ref()
            .and_then(|workspace| {
                workspace
                    .tasks
                    .iter()
                    .position(|task| Some(&task.id) == workspace.selected_task.as_ref())
            })
            .unwrap_or(0);

        self.ticket = self
            .workspace
            .as_ref()
            .and_then(|w| self.state.workflow.tickets.get(&w.id))
            .cloned();

        self.workspace_open = true;

        self.workspace_tab = WorkspaceTab::Changes;

        self.focus = 1;

        self.selection_changed();
    }

    fn local_task_form(&mut self) {
        if self
            .current_workspace()
            .is_some_and(|workspace| workspace.issue.is_some())
        {
            self.say(
                "Select an existing ticket task in Overview; use i to import explicit tasks",
                false,
            );
            return;
        }
        let Some(path) = self.selected_tree().map(|tree| tree.path.clone()) else {
            self.say("Select a checkout first", true);
            return;
        };

        self.modal = Some(Modal::Form(Form {
            title: "New local task · enter the objective".into(),
            kind: FormKind::LocalTask(path),
            fields: vec![Field::new("Objective", String::new(), true)],
            active: 0,
            path_custom: false,
        }));
    }

    pub fn workspace_key(&mut self, key: KeyEvent) -> bool {
        if self.project.is_none() || self.settings_open || self.searching {
            return false;
        }

        if key.modifiers != KeyModifiers::NONE {
            return false;
        }
        if key.kind == KeyEventKind::Repeat
            && !matches!(
                key.code,
                KeyCode::Char('j' | 'k') | KeyCode::Up | KeyCode::Down
            )
        {
            return self.workspace_open
                || self.work_mode
                || matches!(key.code, KeyCode::Char('W' | 'T'));
        }

        if matches!(key.code, KeyCode::Char('W' | 'T' | 'w')) {
            match key.code {
                KeyCode::Char('W') => self.switch_entry("work"),
                KeyCode::Char('T') => {
                    self.ticket = None;
                    self.switch_entry("tickets");
                }
                _ => self.switch_entry("worktrees"),
            };
            return true;
        }

        if self.work_mode && !self.workspace_open {
            let rows = self
                .project
                .as_ref()
                .map(|p| {
                    crate::work::rows_with_attention(
                        &self.state.workflow,
                        &p.path,
                        &self.hosted.summaries,
                    )
                })
                .unwrap_or_default();

            match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    self.work_cursor = step(self.work_cursor, 1, rows.len())
                }

                KeyCode::Char('k') | KeyCode::Up => {
                    self.work_cursor = step(self.work_cursor, -1, rows.len())
                }

                KeyCode::Enter => {
                    if let Some(row) = rows.get(self.work_cursor) {
                        self.workspace =
                            crate::workspace::resolve(&self.state.workflow, &row.workspace).ok();

                        self.ticket = self.state.workflow.tickets.get(&row.workspace).cloned();

                        self.task_cursor = self
                            .workspace
                            .as_ref()
                            .and_then(|w| w.tasks.iter().position(|t| t.id == row.task))
                            .unwrap_or(0);

                        self.workspace_open = true;
                        self.workspace_tab = WorkspaceTab::Overview;
                        self.sync_workspace_checkout();
                    }
                }

                KeyCode::Esc | KeyCode::Char('q') => {
                    self.project = None;
                    self.epoch += 1;
                }

                KeyCode::Char('?') | KeyCode::Char(',') => return false,

                _ => {}
            }
            return true;
        }

        if !self.workspace_open {
            if !self.ticket_mode && key.code == KeyCode::Enter && self.focus == 0 {
                self.open_worktree_workspace();
                return true;
            }

            return false;
        }

        match key.code {
            KeyCode::Char('1') => self.workspace_tab = WorkspaceTab::Overview,

            KeyCode::Char('2') => {
                self.workspace_tab = WorkspaceTab::Changes;
                self.sync_workspace_checkout();
                self.focus = 1;
            }

            KeyCode::Char('3') => self.workspace_tab = WorkspaceTab::Agent,

            KeyCode::Char('4') => {
                self.remember_browser_preview();
                self.workspace_tab = WorkspaceTab::Discussion;
                if let Some(ticket) = &self.ticket {
                    self.browser.selected = Some(ticket.issue.number);
                    self.browser.repository =
                        crate::ticket_browser::normalize_repository(&ticket.repository);
                    self.preview_selected();
                }
            }
            KeyCode::Char('S') => {
                self.workspace_tab = WorkspaceTab::Agent;
                if self.current_workspace().is_none() {
                    self.local_task_form();
                } else {
                    self.begin_agent(false);
                }
            }
            KeyCode::Char('E') if self.workspace_tab == WorkspaceTab::Activity => {
                self.action("operation_details")
            }
            KeyCode::Char('H') if self.workspace_tab == WorkspaceTab::Activity => {
                self.action("operation_history")
            }
            KeyCode::Char('E') | KeyCode::Char('H') | KeyCode::Char('?') | KeyCode::Char(',') => {
                return false;
            }
            KeyCode::Char('A') => self.local_task_form(),
            KeyCode::Char('L') => {
                if let Some(workspace) = self.current_workspace()
                    && workspace.issue.is_none()
                {
                    let entries = self
                        .state
                        .workflow
                        .tickets
                        .values()
                        .filter(|ticket| same_path(&ticket.project, &workspace.project))
                        .map(|ticket| {
                            (
                                format!("#{} {}", ticket.issue.number, ticket.issue.title),
                                ticket.id(),
                            )
                        })
                        .collect();
                    self.picker(
                        "Link local work to an opened GitHub ticket",
                        PickerKind::LinkWork(workspace.id),
                        entries,
                    );
                }
            }

            KeyCode::Char('5') => self.workspace_tab = WorkspaceTab::Activity,

            KeyCode::Esc | KeyCode::Char('q') => {
                self.restore_browser_preview();
                self.workspace_open = false;
                self.ticket = None;
                self.hosted.input = false;
            }

            _ => {
                if self.workspace_tab == WorkspaceTab::Changes {
                    if self.workspace_changes_ready() {
                        return false;
                    }
                    self.say("Prepare the workspace checkout and wait until it is loaded before Git actions",true);
                    return true;
                }

                if self.workspace_tab == WorkspaceTab::Discussion {
                    match key.code {
                        KeyCode::Char('o') => {
                            if let (Some(p), Some(t)) = (&self.project, &self.ticket) {
                                self.enqueue(Job::OpenIssue(p.path.clone(), t.issue.number));
                            }
                        }
                        KeyCode::Char('c') => {
                            self.begin_comment();
                        }
                        KeyCode::Char('j') | KeyCode::Down => {
                            self.browser.preview_scroll =
                                self.browser.preview_scroll.saturating_add(1)
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            self.browser.preview_scroll =
                                self.browser.preview_scroll.saturating_sub(1)
                        }
                        _ => {}
                    }

                    return true;
                }

                if self.current_workspace().is_none() {
                    if matches!(key.code, KeyCode::Char('s' | 'n')) {
                        self.local_task_form();
                    } else if matches!(key.code, KeyCode::Char('?') | KeyCode::Char(',')) {
                        return false;
                    }

                    return true;
                }

                match key.code {
                    KeyCode::Char('s') => {
                        self.workspace_tab = WorkspaceTab::Agent;
                        self.begin_agent(false);
                    }

                    KeyCode::Char('n') => {
                        self.workspace_tab = WorkspaceTab::Agent;
                        self.begin_agent(true);
                    }

                    KeyCode::Char('j') | KeyCode::Down => {
                        if let Some(w) = self.current_workspace() {
                            self.task_cursor = step(self.task_cursor, 1, w.tasks.len());
                            self.sync_workspace_checkout();
                        }
                    }

                    KeyCode::Char('k') | KeyCode::Up => {
                        if let Some(w) = self.current_workspace() {
                            self.task_cursor = step(self.task_cursor, -1, w.tasks.len());
                            self.sync_workspace_checkout();
                        }
                    }

                    KeyCode::Enter
                        if self.workspace_tab == WorkspaceTab::Agent
                            && self.hosted.token.is_some()
                            && self
                                .hosted
                                .record
                                .as_ref()
                                .is_some_and(|record| self.workspace_session_matches(record)) =>
                    {
                        self.hosted.input = true;
                    }

                    KeyCode::Char('p') => {
                        if let Some(w) = self.current_workspace()
                            && let Some(t) = w.tasks.get(self.task_cursor)
                        {
                            let progress = match t.progress {
                                workflow::Progress::Pending => workflow::Progress::InProgress,
                                workflow::Progress::InProgress => workflow::Progress::Done,
                                workflow::Progress::Done => workflow::Progress::Pending,
                            };
                            self.workflow_command(WorkflowCommand::SetProgress {
                                ticket: w.id,
                                task: t.id.clone(),
                                progress,
                            });
                        }
                    }

                    _ if self.ticket.is_some() => return self.ticket_key(key),

                    KeyCode::Char('g') => self.show_sessions(),
                    KeyCode::Char('d') => self.detach_agent(),
                    KeyCode::Char('x') => self.stop_agent_dialog(),
                    KeyCode::Char('r') => self.reconcile_agent_dialog(),
                    KeyCode::Char('b') => self.begin_handover(),
                    KeyCode::Char('u') => self.retry_agent_dialog(),
                    KeyCode::Char('?') | KeyCode::Char(',') => return false,

                    _ => {}
                }
            }
        }

        true
    }
}
