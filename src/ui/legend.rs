//! Action hints for the current input owner; routine navigation stays out of legends.
use crate::{
    app::{App, Form, FormKind, Modal, PickerKind, hosted_ui::Dialog},
    hosted::{self, Health},
    settings::{Mode, Section},
};
use unicode_width::UnicodeWidthStr;

pub fn agent_escape(app: &App) -> String {
    display_key(
        app.config
            .keys
            .iter()
            .find(|(_, action)| action.as_str() == "agent_navigation")
            .map(|(key, _)| key.as_str())
            .unwrap_or("ctrl+]"),
    )
}

fn display_key(key: &str) -> String {
    if key == " " {
        "Space".into()
    } else {
        key.replace("ctrl+", "Ctrl+").replace("alt+", "Alt+")
    }
}

// Only explorer navigation reaches [keys]. Other input owners use fixed keys.
fn explorer(app: &App, actions: &[(&str, &str, &str)]) -> Vec<String> {
    actions
        .iter()
        .filter_map(|(default, action, label)| {
            let matches = |value: &str| value == *action || value == *default;
            let key = if app.config.keys.get(*default).is_none_or(|v| matches(v)) {
                Some(*default)
            } else {
                app.config
                    .keys
                    .iter()
                    .filter(|(_, value)| matches(value))
                    .min_by_key(|(key, _)| key.len())
                    .map(|(key, _)| key.as_str())
            }?;
            Some(format!("{} {label}", display_key(key)))
        })
        .collect()
}

fn fixed(actions: &[&str]) -> Vec<String> {
    actions.iter().map(|s| (*s).into()).collect()
}

pub fn form_hint(form: &Form) -> &'static str {
    if matches!(
        form.kind,
        FormKind::Create { existing: false } | FormKind::Branch | FormKind::TicketCreate(_)
    ) {
        "Ctrl+s submit · Ctrl+b choose starting point"
    } else {
        "Ctrl+s submit"
    }
}

pub fn picker_hint(kind: &PickerKind) -> &'static str {
    match kind {
        PickerKind::Branches => "a new branch · d delete local branch",
        PickerKind::Prs => "a enter PR number or URL",
        PickerKind::Operations => "Recent Git operations · current UI session only",
        PickerKind::TicketImport(_) => "Import one explicit task at a time",
        PickerKind::TicketAdopt(_) => "Associate the selected checkout with this ticket",
        _ => "",
    }
}

fn actions(app: &App) -> Vec<String> {
    if let Some(dialog) = &app.hosted.dialog {
        return match dialog {
            Dialog::Start { editing: true, .. } => {
                fixed(&["Tab review brief", "Ctrl+u clear handoff"])
            }
            Dialog::Start { .. } => fixed(&["y confirm launch", "Tab edit handoff"]),
            Dialog::Sessions { .. } => fixed(&["c continue conversation", "Enter select only"]),
            Dialog::RetryStart { .. } => fixed(&["y retry same request", "f forget reminder"]),
            Dialog::Diagnostics { .. } => vec![],
            Dialog::Reconcile(_) => fixed(&["y reconcile execution"]),
            Dialog::Stop(_) => fixed(&["y stop session"]),
            Dialog::StopHost => fixed(&["y stop host"]),
            Dialog::Takeover => fixed(&["y take input control"]),
        };
    }
    if app.hosted.input {
        return vec![format!("{} Tree Rail navigation", agent_escape(app))];
    }
    if let Some(modal) = &app.modal {
        return match modal {
            Modal::Form(form) => fixed(&[form_hint(form)]),
            Modal::Picker(picker) => fixed(&[picker_hint(&picker.kind)]),
            Modal::Requirements(_) => fixed(&["m merge task additions", "s send revision"]),
            Modal::Cleanup(preview) if preview.can_remove() => fixed(&["y remove checkout"]),
            Modal::Cleanup(preview) if preview.owner.is_some() => fixed(&["x stop owner"]),
            Modal::Cleanup(_) => vec![],
            Modal::Confirm(..) => fixed(&["y confirm"]),
            Modal::OperationDetails => fixed(&["G follow output", "H operation history"]),
            Modal::Help => vec![],
        };
    }
    if app.settings_open {
        let Some(draft) = &app.settings else {
            return vec![];
        };
        return match draft.mode {
            Mode::Conflict(_) => fixed(&["m keep mine", "e keep external"]),
            Mode::Discard { .. } => fixed(&["y discard draft"]),
            Mode::Edit if draft.input.is_some() => fixed(&[
                "Ctrl+s apply",
                "Ctrl+u clear before cursor",
                "Esc explorer (keeps draft)",
            ]),
            Mode::Edit => {
                let mut items = match draft.section {
                    Section::Sources => fixed(&["a add source", "d remove source", "r recursive"]),
                    Section::Worktrees => fixed(&["a project defaults"]),
                    Section::Profiles => fixed(&["a add profile"]),
                    Section::Notifications => vec![],
                };
                items.push("Ctrl+s apply".into());
                if draft.dirty() {
                    items.push("D discard draft".into());
                }
                items.push("? actions".into());
                items
            }
        };
    }
    if app.searching {
        return vec![];
    }
    if app.project.is_none() {
        return explorer(
            app,
            &[
                ("w", "basic_worktrees", "basic worktrees"),
                (",", "settings", "settings"),
                ("R", "refresh", "refresh"),
                ("?", "help", "actions"),
            ],
        );
    }
    if app.ticket_mode {
        let Some(ticket) = &app.ticket else {
            return fixed(&[
                "w basic worktrees",
                "R refresh tickets",
                ", settings",
                "? actions",
            ]);
        };
        let mut items = vec![];
        if app.hosted.pending_launch.is_some() {
            items.push("u retry launch".into());
        }
        if ticket.issue_updated() {
            items.push("v issue changes".into());
        }
        if ticket.is_closed() && ticket.worktree.is_some() {
            items.push("D cleanup checkout".into());
        }
        if let Some(checkout) = &ticket.worktree {
            let selected = ticket.tasks.get(app.task_cursor).and_then(|task| {
                hosted::selected_session(&app.state.workflow, &ticket.id(), &task.id)
            });
            let owner = crate::workflow::deletion_owner(&app.state.workflow, &checkout.path);
            if owner.is_some_and(|owner| selected.is_none_or(|s| s.id != owner.id)) {
                items.push("b handover checkout".into());
            } else if selected
                .is_some_and(|s| matches!(s.health, Health::Interrupted | Health::Failed))
            {
                items.push("r reconcile session".into());
            }
            items.extend(fixed(&[
                "s continue",
                "n new conversation",
                "g conversations",
                "p task progress",
            ]));
            if app.hosted.terminal.is_some() {
                items.push("d detach".into());
            }
        } else {
            items.extend(fixed(&[
                "c create checkout",
                "a adopt worktree",
                "i import tasks",
            ]));
        }
        items.push("? actions".into());
        return items;
    }
    let mut items = match app.focus {
        0 => explorer(
            app,
            &[
                ("a", "create_worktree", "worktree"),
                ("b", "branches", "branches"),
                ("o", "launch", "launch tool"),
                ("O", "profiles", "profiles"),
            ],
        ),
        1 => {
            let mut items = explorer(app, &[(" ", " ", "mark files")]);
            items.extend(explorer(
                app,
                &[
                    ("s", "stage", "stage"),
                    ("u", "unstage", "unstage"),
                    ("c", "commit", "commit"),
                    ("t", "toggle_staged", "staged diff"),
                ],
            ));
            items
        }
        _ => explorer(
            app,
            &[
                ("t", "toggle_staged", "staged diff"),
                ("v", "review_diff", "PR comparison"),
                ("r", "prs", "review PR"),
                ("p", "push", "push"),
            ],
        ),
    };
    items.extend(explorer(
        app,
        &[("T", "tickets", "tickets"), ("?", "help", "actions")],
    ));
    items
}

/// Fit whole action hints, reserving the action-guide shortcut on small terminals.
pub fn footer(app: &App, width: u16) -> String {
    if app.hosted.input && app.hosted.dialog.is_none() {
        let key = agent_escape(app);
        return [
            format!("{key} Tree Rail navigation"),
            format!("{key} navigation"),
            key,
        ]
        .into_iter()
        .find(|hint| hint.width() <= usize::from(width))
        .unwrap_or_default();
    }
    let mut items = actions(app);
    let guide = items.last().filter(|s| s.ends_with(" actions")).cloned();
    if guide.is_some() {
        items.pop();
    }
    let mut text = String::new();
    for item in items
        .iter()
        .flat_map(|s| s.split(" · "))
        .filter(|s| !s.is_empty())
    {
        let candidate = if text.is_empty() {
            item.to_owned()
        } else {
            format!("{text} · {item}")
        };
        let reserved = guide.as_ref().map_or(0, |g| g.width() + 3);
        if candidate.width() + reserved > usize::from(width) {
            break;
        }
        text = candidate;
    }
    if let Some(mut guide) = guide {
        if guide.width() > usize::from(width) {
            guide = guide.trim_end_matches(" actions").to_owned();
        }
        let candidate = if text.is_empty() {
            guide
        } else {
            format!("{text} · {guide}")
        };
        if candidate.width() <= usize::from(width) {
            text = candidate;
        }
    }
    text
}

pub fn help(app: &App, width: u16) -> String {
    let body = if app.settings_open {
        "SETTINGS\n1 Sources · 2 Worktrees · 3 Profiles · 4 Notifications\na add source, project defaults or profile\nSources: d remove source · r recursive discovery\nCtrl+s validate, save and apply · D discard draft\nEsc returns to explorer and keeps unfinished edits\n\nEDITING\nCtrl+u clears text before the cursor\nFinal-field acceptance updates the draft, not the file\nConflicts: m keep mine · e keep external values"
            .into()
    } else if app.project.is_some() && app.ticket_mode {
        format!(
            "TICKET WORKFLOW\nc create shared checkout · a adopt worktree\ni import explicit tasks · p cycle task progress\nR refresh GitHub · v inspect issue changes\nRequirements review: m merge tasks · s send revision\n\nAGENT CONVERSATIONS\ns continue selected conversation · n new conversation\ng recorded conversations: select only, or c continue\nb handover checkout (stops its current owner first)\nd detach view · x stop selected task's session\n{} returns from agent input to Tree Rail\n\nRECOVERY & CLEANUP\nr reconcile execution · u retry unresolved launch\n! take input control · X stop host (no live agents)\nE diagnostics · D preview optional checkout cleanup\n\nw basic worktrees · , settings\nReady means finished responding; task progress is manual.",
            agent_escape(app)
        )
    } else {
        let mut rows = vec![if app.project.is_some() {
            "BASIC WORKTREES".into()
        } else {
            "PROJECTS".into()
        }];
        if app.project.is_some() {
            rows.extend(explorer(
                app,
                &[
                    ("a", "create_worktree", "new worktree"),
                    (
                        "b",
                        "branches",
                        "branch picker: a new branch, d delete local",
                    ),
                    (
                        "d",
                        "remove_worktree",
                        "remove clean linked worktree; branch retained",
                    ),
                    ("o", "launch", "launch default external tool"),
                    ("O", "profiles", "choose external tool profile"),
                ],
            ));
            rows.push("\nFILES & GIT".into());
            rows.extend(explorer(app, &[(" ", " ", "mark files (Files pane)")]));
            rows.extend(explorer(
                app,
                &[
                    ("s", "stage", "stage files (Files pane)"),
                    ("u", "unstage", "unstage files (Files pane)"),
                    ("t", "toggle_staged", "switch staged/unstaged diff"),
                    ("c", "commit", "commit: Ctrl+s submits"),
                    ("f", "fetch", "fetch"),
                    ("p", "push", "push / publish branch"),
                    ("P", "pull", "fast-forward pull"),
                    ("r", "prs", "review PR checkout"),
                    ("v", "review_diff", "PR comparison"),
                    ("w", "open_pr", "open PR in browser"),
                    ("T", "tickets", "ticket workflow"),
                ],
            ));
        } else {
            rows.extend(explorer(
                app,
                &[(
                    "w",
                    "basic_worktrees",
                    "open basic worktrees instead of tickets",
                )],
            ));
        }
        rows.push("\nTOOLS & OPERATIONS".into());
        rows.extend(explorer(
            app,
            &[
                (",", "settings", "settings"),
                ("R", "refresh", "refresh"),
                ("i", "operation_details", "latest Git output"),
                ("H", "operation_history", "recent Git operations"),
            ],
        ));
        let mut lines = Vec::<String>::new();
        for (index, row) in rows.into_iter().enumerate() {
            if index == 0 || row.starts_with('\n') {
                lines.push(row);
            } else if let Some(last) = lines.last_mut() {
                let combined = format!("{last} · {row}");
                if index > 1 && !last.contains('\n') && combined.width() <= usize::from(width) {
                    *last = combined;
                } else {
                    lines.push(row);
                }
            }
        }
        lines.join("\n")
    };
    format!("{body}\n\nConfig: {}", app.config_path.display())
}
