//! Ephemeral ticket-browser state. Browsing never creates durable ticket work.
use crate::ticket_browser::{Discussion, IssueCard, RepositoryViews};
use crate::{
    host::runtime::SessionSummary,
    hosted::Health,
    workflow::{Progress, TicketRecord, WorkflowState},
};

#[derive(Default)]
pub struct Browser {
    pub repository: String,
    pub request: u64,
    pub preview_request: u64,
    pub preview_page: u32,
    pub views: RepositoryViews,
    pub cards: Vec<IssueCard>,
    pub discussion: Option<Discussion>,
    pub selected: Option<u64>,
    pub page: u32,
    pub has_more: bool,
    pub total: Option<u64>,
    pub limited: bool,
    pub loading: bool,
    pub warning: String,
    pub preview_loading: bool,
    pub preview_warning: String,
    pub preview_scroll: u16,
    pub preview_focus: bool,
    pub active_collapsed: bool,
    pub editor: Option<Editor>,
    pub pending_editor: Option<Editor>,
    pub metadata: Option<crate::ticket_browser::FilterMetadata>,
    pub refreshed_at: Option<std::time::Instant>,
    pub reselect_after_query: bool,
    pub refresh_pages: u32,
    pub work_refreshed_at: Option<std::time::Instant>,
    pub active_cards: std::collections::BTreeMap<u64, IssueCard>,
    pub active_numbers: Vec<u64>,
    pub editing: bool,
    pub edit_queue: std::collections::VecDeque<crate::ticket_browser::Command>,
}

#[derive(Clone)]
pub enum Editor {
    Filters {
        fields: Vec<String>,
        cursor: usize,
    },
    Views {
        cursor: usize,
    },
    Name {
        action: String,
        text: String,
        id: String,
    },
}

impl Browser {
    pub fn filter(&self) -> crate::ticket_browser::Filter {
        self.views
            .views
            .iter()
            .find(|view| view.id == self.views.active)
            .map(|view| view.filter.clone())
            .unwrap_or_default()
    }
    pub fn view_name(&self) -> &str {
        self.views
            .views
            .iter()
            .find(|view| view.id == self.views.active)
            .map(|view| view.name.as_str())
            .unwrap_or("Open tickets")
    }
    pub fn selected_card(&self) -> Option<&IssueCard> {
        self.cards
            .iter()
            .find(|card| Some(card.issue.number) == self.selected)
            .or_else(|| {
                self.discussion
                    .as_ref()
                    .filter(|discussion| Some(discussion.card.issue.number) == self.selected)
                    .map(|discussion| &discussion.card)
            })
    }
}

#[derive(Clone, Debug)]
pub struct ActiveTicket {
    pub ticket: TicketRecord,
    pub attention: String,
    pub rank: u8,
}

pub fn active_tickets(
    state: &WorkflowState,
    repository: &str,
    summaries: &[SessionSummary],
) -> Vec<ActiveTicket> {
    let mut result = Vec::new();
    for ticket in state.tickets.values().filter(|ticket| {
        crate::ticket_browser::normalize_repository(&ticket.repository)
            == crate::ticket_browser::normalize_repository(repository)
    }) {
        let mut active = ticket
            .tasks
            .iter()
            .any(|task| task.progress == Progress::InProgress);
        let mut attention = String::new();
        let mut rank = 3;
        for session in state
            .sessions
            .values()
            .filter(|session| session.ticket == ticket.id())
        {
            let unresolved = session.pending_start.is_some()
                || (session.current
                    && matches!(
                        session.health,
                        Health::Starting | Health::Interrupted | Health::Failed
                    ));
            let live = matches!(session.health, Health::Live);
            if !(unresolved || live) {
                continue;
            }
            active = true;
            if unresolved {
                attention = "Recovery needed".into();
                rank = 0;
            } else if let Some(summary) = summaries
                .iter()
                .find(|summary| session.host_session.as_ref() == Some(&summary.session))
            {
                use crate::copilot::AttentionState;
                let (value, priority) = if summary.exited {
                    ("Execution ended · refresh recovery", 0)
                } else if !summary.attention.healthy {
                    ("Observation unavailable", 0)
                } else {
                    match summary.attention.value {
                        Some(AttentionState::NeedsInput) => ("Needs input", 0),
                        Some(AttentionState::Running) => ("Running", 1),
                        Some(AttentionState::Ready) => ("Ready · owns checkout", 2),
                        None => ("Observation unavailable", 0),
                    }
                };
                if priority <= rank {
                    attention = value.into();
                    rank = priority;
                }
            } else if rank > 1 {
                attention = "Live · observation pending".into();
                rank = 1;
            }
        }
        if active {
            if attention.is_empty() {
                attention = "Task in progress".into();
            }
            result.push(ActiveTicket {
                ticket: ticket.clone(),
                attention,
                rank,
            });
        }
    }
    result.sort_by(|a, b| {
        a.rank
            .cmp(&b.rank)
            .then(a.ticket.issue.number.cmp(&b.ticket.issue.number))
    });
    result
}
