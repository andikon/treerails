//! Unified projections and checkout exclusion use only local Git and fake state.
use std::path::{Path, PathBuf};
use treerail::{
    copilot::{Attention, AttentionState},
    git,
    host::{CheckoutLease, runtime::SessionSummary},
    hosted::{Health, SessionRecord},
    work,
    workflow::WorkflowState,
};

async fn repository(root: &Path) -> PathBuf {
    let path = root.join("project");
    std::fs::create_dir(&path).unwrap();
    for args in [
        vec!["init", "--initial-branch=main"],
        vec!["config", "user.name", "Unified Test"],
        vec!["config", "user.email", "test@example.invalid"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["commit", "--allow-empty", "-m", "initial"],
    ] {
        git::run(&path, args).await.unwrap();
    }
    path
}

fn state(project: &Path) -> WorkflowState {
    serde_json::from_value(serde_json::json!({
        "tickets": {
            "owner/repo#1": {
                "repository":"owner/repo", "project":project,
                "issue":{"number":1,"title":"Ticket","body":"Objective","state":"CLOSED","url":"https://github.com/owner/repo/issues/1"},
                "fetched_at":1,"selected_task":"implement",
                "tasks":[{"id":"implement","title":"Implement","progress":"in_progress"}],
                "worktree":{"path":project.join("ticket"),"branch":"ticket","original":false}
            }
        },
        "local_work": {
            "local:1": {
                "id":"local:1", "project":project,"title":"Local investigation","created_at":2,
                "selected_task":"inspect",
                "tasks":[{"id":"inspect","title":"Inspect","progress":"pending"},{"id":"history","title":"Old work","progress":"done"}],
                "worktree":{"path":project,"branch":"main","original":true}
            },
            "local:elsewhere": {
                "id":"local:elsewhere", "project":project.join("different-repository"),"title":"Other project","created_at":1,
                "selected_task":"other", "tasks":[{"id":"other","title":"Other","progress":"in_progress"}],
                "worktree":{"path":project.join("different-repository"),"branch":"main","original":true}
            }
        }
    })).unwrap()
}

fn session(project: &Path, workspace: &str, task: &str, health: Health) -> SessionRecord {
    serde_json::from_value(serde_json::json!({
        "id":format!("{workspace}/{task}"),"ticket":workspace,"task":task,
        "checkout":project,"provider_session_id":"fixture","starting_brief":"Approved local objective",
        "created_at":2,"host_session":"terminal","health":health,"current":true,"error":null
    })).unwrap()
}

#[tokio::test]
async fn work_rows_share_canonical_tasks_and_keep_ready_local_agents_active() {
    let temp = tempfile::tempdir().unwrap();
    let project = repository(temp.path()).await;
    let mut state = state(&project);
    let live = session(&project, "local:1", "inspect", Health::Live);
    state.sessions.insert(live.id.clone(), live.clone());
    let mut old = session(&project, "local:1", "history", Health::Stopped);
    old.current = false;
    state.sessions.insert(old.id.clone(), old);
    let summary = SessionSummary {
        session: "terminal".into(),
        provider_session_id: None,
        attention: Attention {
            value: Some(AttentionState::Ready),
            healthy: true,
            ..Default::default()
        },
        exited: false,
        notification_error: None,
    };
    let rows = work::rows_with_attention(&state, &project, &[summary]);
    assert_eq!(rows.len(), 3, "Other repositories must not leak into Work");
    assert_eq!(rows.iter().filter(|row| row.active).count(), 2);
    let local = rows.iter().find(|row| row.task == "inspect").unwrap();
    assert_eq!(local.workspace, live.ticket);
    assert_eq!(local.session.as_ref().unwrap().id, live.id);
    assert_eq!(local.attention, Some(AttentionState::Ready));
    assert!(local.status.contains("owns checkout"));
    assert!(local.issue.is_none());
    assert!(rows.iter().any(|row| {
        row.issue
            .as_ref()
            .is_some_and(|issue| issue.state == "CLOSED")
            && row.active
    }));
    assert!(
        !rows
            .iter()
            .find(|row| row.task == "history")
            .unwrap()
            .active
    );
    assert_eq!(
        state.local_work["local:1"].tasks.len(),
        2,
        "Projection must not create extra tasks"
    );
}

#[tokio::test]
async fn recovery_and_needs_input_precede_running_and_historical_sessions() {
    let temp = tempfile::tempdir().unwrap();
    let project = repository(temp.path()).await;
    let mut state = state(&project);
    let mut recovery = session(&project, "local:1", "inspect", Health::Interrupted);
    recovery.current = false;
    state.sessions.insert(recovery.id.clone(), recovery.clone());
    let mut newer_stopped = recovery.clone();
    newer_stopped.id = "newer-stopped".into();
    newer_stopped.created_at = 10;
    newer_stopped.current = false;
    newer_stopped.health = Health::Stopped;
    state
        .sessions
        .insert(newer_stopped.id.clone(), newer_stopped);
    let rows = work::rows(&state, &project);
    assert_eq!(rows[0].workspace, "local:1");
    assert_eq!(rows[0].session.as_ref().unwrap().id, recovery.id);
    assert_eq!(rows[0].status, "Recovery needed");
    assert!(work::mutation_guard(&state, &project).is_err());
    state.sessions.get_mut(&recovery.id).unwrap().health = Health::Live;
    let summary = SessionSummary {
        session: "terminal".into(),
        provider_session_id: None,
        attention: Attention {
            value: Some(AttentionState::NeedsInput),
            healthy: true,
            ..Default::default()
        },
        exited: false,
        notification_error: None,
    };
    let rows = work::rows_with_attention(&state, &project, &[summary]);
    assert_eq!(rows[0].workspace, "local:1");
    assert_eq!(rows[0].status, "Needs input");
    state.sessions.get_mut(&recovery.id).unwrap().health = Health::Stopped;
    work::mutation_guard(&state, &project).unwrap();
}

#[tokio::test]
async fn managed_checkout_lease_blocks_pull_and_removal_from_basic_git_entrypoints() {
    let temp = tempfile::tempdir().unwrap();
    let project = repository(temp.path()).await;
    let linked = temp.path().join("linked");
    git::create_worktree(&project, "feature", "main", &linked, false, false)
        .await
        .unwrap();
    let lease = CheckoutLease::acquire(&linked).await.unwrap();
    let removal = git::remove_worktree(&project, &linked).await.unwrap_err();
    assert!(format!("{removal:#}").contains("owned by another managed agent"));
    let pull = git::pull(&linked).await.unwrap_err();
    assert!(format!("{pull:#}").contains("owned by another managed agent"));
    // Read-only Git stays available while the agent owns this checkout.
    assert_eq!(git::status(&linked).await.unwrap().branch, "feature");
    assert_eq!(git::worktrees(&project).await.unwrap().len(), 2);
    drop(lease);
    git::remove_worktree(&project, &linked).await.unwrap();
    assert!(!linked.exists());
}

#[tokio::test]
async fn local_tasks_require_a_registered_checkout_and_preserve_cross_context_claims() {
    let temp = tempfile::tempdir().unwrap();
    let project = repository(temp.path()).await;
    let first = treerail::workflow::Workflow::new(temp.path().join("first.toml"));
    let nested = project.join("nested");
    std::fs::create_dir(&nested).unwrap();
    let invalid = first
        .execute(
            &project,
            treerail::workflow::Command::CreateLocal {
                path: nested,
                objective: "Do not adopt an arbitrary subdirectory".into(),
            },
        )
        .await
        .unwrap_err();
    assert!(invalid.to_string().contains("registered project checkout"));
    assert!(first.snapshot().unwrap().local_work.is_empty());

    let event = first
        .execute(
            &project,
            treerail::workflow::Command::CreateLocal {
                path: project.clone(),
                objective: "Investigate with a local task".into(),
            },
        )
        .await
        .unwrap();
    let treerail::workflow::Event::Workspace(workspace) = event else {
        panic!("Expected the canonical workspace");
    };
    let claim = treerail::workflow::claim_path(&project).await.unwrap();
    assert_eq!(std::fs::read_to_string(&claim).unwrap(), workspace.id);
    let before = first.snapshot().unwrap();
    let second = treerail::workflow::Workflow::new(temp.path().join("second.toml"));
    let conflict = second
        .execute(
            &project,
            treerail::workflow::Command::CreateLocal {
                path: project.clone(),
                objective: "Attempt another configuration context".into(),
            },
        )
        .await
        .unwrap_err();
    assert!(
        conflict
            .to_string()
            .contains("another workspace or configuration")
    );
    assert!(second.snapshot().unwrap().local_work.is_empty());
    assert_eq!(first.snapshot().unwrap(), before);
    assert_eq!(std::fs::read_to_string(claim).unwrap(), workspace.id);

    git::run(&project, ["switch", "-c", "external-drift"])
        .await
        .unwrap();
    let drift = first
        .execute(
            &project,
            treerail::workflow::Command::CreateLocal {
                path: project.clone(),
                objective: "Must not silently adopt changed branch".into(),
            },
        )
        .await
        .unwrap_err();
    assert!(drift.to_string().contains("branch"));
    assert_eq!(first.snapshot().unwrap(), before);
}
