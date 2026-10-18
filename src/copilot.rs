//! Native Copilot integration: launch identity and provider-derived attention.
//! Terminal output, process lifetime and task progress are deliberately not attention sources.
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub const EXTENSION_SOURCE: &str = include_str!("copilot/extension.mjs");
const PREVIOUS_EXTENSION_SOURCE: &str = include_str!("copilot/extension-v1.mjs");
pub const TESTED_COPILOT_VERSION: &str = "1.0.94";
// Flip only alongside recorded native compatibility evidence, never from fake-provider tests.
pub const DOCUMENT_DELIVERY_VERIFIED: bool = false;

/// Extension prompt delivery is version-bound; unfamiliar CLIs require validation first.
pub async fn verify_document_provider() -> Result<()> {
    if !DOCUMENT_DELIVERY_VERIFIED {
        bail!(
            "Complete discussion delivery awaits native Copilot compatibility verification. The approved document is retained; existing conversations can Continue. No agent has been started with partial context."
        );
    }
    let output = tokio::process::Command::new("copilot")
        .args(["--no-auto-update", "--version"])
        .output()
        .await?;
    let version = String::from_utf8_lossy(&output.stdout);
    if !output.status.success()
        || !version
            .lines()
            .any(|line| line.trim() == format!("GitHub Copilot CLI {TESTED_COPILOT_VERSION}."))
    {
        bail!(
            "Complete discussion delivery is validated for Copilot {TESTED_COPILOT_VERSION}; install that supported version or validate this provider before starting. Existing conversations can still Continue."
        );
    }
    Ok(())
}

/// Install only the observer's own entrypoint outside managed Git checkouts.
/// Other Copilot sessions discover it too, but it exits immediately without host context.
/// Refuse collisions and user modifications; an upgrade must reconcile them explicitly.
pub fn install_observer(copilot_home: &Path) -> Result<PathBuf> {
    let directory = copilot_home
        .join("extensions")
        .join("treerail-session-host");
    if let Ok(metadata) = fs::symlink_metadata(&directory)
        && metadata.file_type().is_symlink()
    {
        bail!("Observer directory is a symlink; refusing installation");
    }
    fs::create_dir_all(&directory)?;
    let entry = directory.join("extension.mjs");
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&entry)
    {
        Ok(mut file) => {
            file.write_all(EXTENSION_SOURCE.as_bytes())?;
            file.sync_all()?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(&entry)?;
            if metadata.file_type().is_file()
                && fs::read(&entry)? == PREVIOUS_EXTENSION_SOURCE.as_bytes()
            {
                crate::config::atomic_write(&entry, EXTENSION_SOURCE.as_bytes())?;
                return Ok(entry);
            }
            if !metadata.file_type().is_file()
                || metadata.len() != EXTENSION_SOURCE.len() as u64
                || fs::read(&entry)? != EXTENSION_SOURCE.as_bytes()
            {
                bail!(
                    "Copilot observer already exists with different contents: {}. Preserve or move that file before retrying",
                    entry.display()
                );
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(entry)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttentionState {
    Running,
    NeedsInput,
    Ready,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Observation {
    pub session_id: String,
    pub sequence: u64,
    pub kind: String,
    pub request_id: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attention {
    pub value: Option<AttentionState>,
    pub healthy: bool,
    pub reason: String,
    pub pending: BTreeSet<String>,
}
impl Default for Attention {
    fn default() -> Self {
        Self {
            value: None,
            healthy: false,
            reason: "Waiting for native Copilot observation".into(),
            pending: BTreeSet::new(),
        }
    }
}
impl Attention {
    pub fn unavailable(&mut self, reason: impl Into<String>) {
        self.healthy = false;
        self.reason = reason.into();
    }
    /// Apply only ordered, authenticated observations for the recorded provider UUID.
    /// The transport owns sequence validation and heartbeat timeouts.
    pub fn observe(&mut self, kind: &str, request_id: Option<&str>) {
        match kind {
            "bridge.connected" => {
                self.pending.clear();
                self.unavailable("Connected; waiting for a native attention boundary");
            }
            "bridge.lost" | "session.error" => self.unavailable(kind),
            "assistant.turn_start" => self.set(if self.pending.is_empty() {
                AttentionState::Running
            } else {
                AttentionState::NeedsInput
            }),
            "permission.requested" | "user_input.requested" | "elicitation.requested" => {
                let Some(id) = valid_request_id(request_id) else {
                    self.unavailable("Request lacked a valid correlation identity");
                    return;
                };
                if self.pending.len() >= 256 {
                    self.unavailable("Too many pending provider requests");
                    return;
                }
                self.pending.insert(format!(
                    "{}:{id}",
                    kind.split('.').next().unwrap_or_default()
                ));
                self.set(AttentionState::NeedsInput);
            }
            "permission.completed" | "user_input.completed" | "elicitation.completed" => {
                let Some(id) = valid_request_id(request_id) else {
                    self.unavailable("Completion lacked a valid correlation identity");
                    return;
                };
                if !self.pending.remove(&format!(
                    "{}:{id}",
                    kind.split('.').next().unwrap_or_default()
                )) {
                    self.unavailable("Completion without observed request");
                } else if self.pending.is_empty() {
                    self.value = Some(AttentionState::Running);
                    // A completion cannot repair an already broken observation stream.
                }
            }
            "session.idle" if self.pending.is_empty() => self.set(AttentionState::Ready),
            _ => {}
        }
    }
    fn set(&mut self, value: AttentionState) {
        self.value = Some(value);
        self.healthy = true;
        self.reason.clear();
    }
}
fn valid_request_id(id: Option<&str>) -> Option<&str> {
    id.filter(|id| !id.is_empty() && id.len() <= 256)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchMode {
    New,
    Resume,
}

/// Exact UUID arguments. Never use `--continue`, names, prefixes or a prompt on resume.
/// The caller must verify a recorded conversation exists before choosing Resume.
pub fn launch_arguments(
    session_id: &str,
    mode: LaunchMode,
    starting_brief: Option<&str>,
) -> Result<Vec<String>> {
    if !is_uuid(session_id) {
        bail!("Copilot requires an exact conversation UUID");
    }
    if mode == LaunchMode::Resume && starting_brief.is_some() {
        bail!("Resume cannot replay a starting brief");
    }
    let mut args = vec![
        "--no-auto-update".into(),
        "--experimental".into(),
        match mode {
            LaunchMode::New => "--session-id".into(),
            LaunchMode::Resume => "--resume".into(),
        },
        session_id.into(),
    ];
    if let Some(brief) = starting_brief {
        args.push("--interactive".into());
        args.push(brief.into());
    }
    Ok(args)
}
pub fn new_session_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn is_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, b)| {
            if [8, 13, 18, 23].contains(&index) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multiple_native_requests_require_matching_resolution_then_idle() {
        let mut state = Attention::default();
        state.observe("assistant.turn_start", None);
        state.observe("permission.requested", Some("same-id"));
        state.observe("elicitation.requested", Some("same-id"));
        state.observe("permission.completed", Some("same-id"));
        state.observe("session.idle", None);
        assert_eq!(state.value, Some(AttentionState::NeedsInput));
        state.observe("elicitation.completed", Some("same-id"));
        assert_eq!(state.value, Some(AttentionState::Running));
        state.observe("assistant.turn_end", None);
        assert_eq!(state.value, Some(AttentionState::Running));
        state.observe("session.idle", None);
        assert_eq!(state.value, Some(AttentionState::Ready));
    }
    #[test]
    fn startup_silence_heartbeat_and_unknown_completion_do_not_claim_ready() {
        let mut state = Attention::default();
        state.observe("bridge.connected", None);
        state.observe("bridge.heartbeat", None);
        assert!(!state.healthy);
        assert!(state.value.is_none());
        state.observe("user_input.completed", Some("missed"));
        assert!(!state.healthy);
        state.observe("permission.requested", None);
        assert!(!state.healthy);
        state.observe("assistant.turn_start", None);
        state.observe("bridge.lost", None);
        assert_eq!(state.value, Some(AttentionState::Running));
        assert!(!state.healthy);
    }
    #[test]
    fn launch_requires_exact_identity_and_does_not_replay_resume() {
        let id = "ead8e737-ac4f-48fd-83f4-c33b0ad5f5c2";
        let args = launch_arguments(id, LaunchMode::New, Some("Issue brief")).unwrap();
        assert_eq!(
            &args[2..],
            &["--session-id", id, "--interactive", "Issue brief"]
        );
        let args = launch_arguments(id, LaunchMode::Resume, None).unwrap();
        assert_eq!(&args[2..], &["--resume", id]);
        assert!(launch_arguments("ead8e737", LaunchMode::Resume, None).is_err());
        assert!(launch_arguments(id, LaunchMode::Resume, Some("replay")).is_err());
    }
    #[test]
    fn attention_snapshot_round_trips_for_protocol() {
        let mut state = Attention::default();
        state.observe("elicitation.requested", Some("question"));
        let encoded = serde_json::to_string(&state).unwrap();
        assert_eq!(state, serde_json::from_str(&encoded).unwrap());
    }
    #[test]
    fn install_reuses_identical_and_preserves_existing_user_files() {
        let home = tempfile::tempdir().unwrap();
        let entry = install_observer(home.path()).unwrap();
        assert_eq!(install_observer(home.path()).unwrap(), entry);
        fs::write(&entry, "user edits").unwrap();
        assert!(install_observer(home.path()).is_err());
        assert_eq!(fs::read_to_string(entry).unwrap(), "user edits");
    }
}
