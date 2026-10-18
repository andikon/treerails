//! Opt-in compatibility smoke; routine CI remains entirely offline and controlled.
use std::path::Path;
use treerail::{
    ticket_browser::{Filter, LabelMode},
    workflow::{Github, IssueSource},
};

#[tokio::test]
#[ignore = "requires authenticated gh and read-only network access to the checkout's repository"]
async fn real_github_returns_labeled_issue_pages_and_discussion_metadata() {
    let project = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repository = Github.repository_identity(project).await.unwrap();
    let login = Github.current_login(project).await.unwrap();
    assert!(!login.is_empty());
    let page = Github
        .browser_page(project, &repository, &Filter::default(), 1)
        .await
        .unwrap();
    assert!(
        page.issues
            .iter()
            .all(|card| card.issue.state.eq_ignore_ascii_case("open"))
    );
    if let Some(card) = page.issues.first() {
        let discussion = Github
            .discussion_page(project, card.issue.number, 1)
            .await
            .unwrap();
        assert_eq!(discussion.card.issue.number, card.issue.number);
        assert!(!discussion.card.author.is_empty());
        assert_eq!(discussion.card.labels, card.labels);
    }
    let filter = Filter {
        assignees: treerail::ticket_browser::AssigneeFilter::Users(vec![login, "octocat".into()]),
        labels: vec!["ready-for-agent".into(), "needs-info".into()],
        label_mode: LabelMode::Any,
        ..Default::default()
    };
    // Acceptance of this request detects legacy search syntax rejecting comma-assignees.
    Github
        .browser_page(project, &repository, &filter, 1)
        .await
        .unwrap();
}
