//! Host-owned notification delivery. UI attachment never replays a notification.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, time::Duration};
use tokio::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    NeedsInput,
    Failure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub session: String,
    pub ticket: String,
    pub kind: Kind,
    pub message: String,
}

/// A bounded host-local transition ledger, independent of connected clients.
#[derive(Default)]
pub struct Notifications {
    enabled: bool,
    current: BTreeMap<String, Option<(Kind, String)>>,
}

impl Notifications {
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Observe every authoritative transition, even while delivery is disabled.
    /// `identity` is the provider request/failure identity, never terminal prose.
    pub fn observe(
        &mut self,
        session: &str,
        ticket: &str,
        action: Option<(Kind, &str)>,
        message: &str,
    ) -> Option<Notice> {
        let next = action.map(|(kind, id)| (kind, id.to_owned()));
        if self.current.get(session) == Some(&next) {
            return None;
        }
        self.current.insert(session.to_owned(), next);
        if !self.enabled {
            return None;
        }
        action.map(|(kind, _)| Notice {
            session: session.to_owned(),
            ticket: ticket.to_owned(),
            kind,
            message: message.chars().take(512).collect(),
        })
    }

    /// Call when a session is removed from the live host to bound ledger memory.
    pub fn forget(&mut self, session: &str) {
        self.current.remove(session);
    }
}

/// Delivery failures are diagnostics, not session-health or ownership changes.
pub async fn deliver(notice: &Notice) -> Result<()> {
    let title = match notice.kind {
        Kind::NeedsInput => format!("Tree Rail · {} needs input", notice.ticket),
        Kind::Failure => format!("Tree Rail · {} session error", notice.ticket),
    };
    let mut command = desktop_command(&title, &notice.message)?;
    command.kill_on_drop(true);
    let result = tokio::time::timeout(Duration::from_secs(5), command.output())
        .await
        .context("Desktop notification delivery timed out")?
        .context("Cannot start desktop notification service")?;
    anyhow::ensure!(
        result.status.success(),
        "Desktop notifications unavailable: {}",
        String::from_utf8_lossy(&result.stderr)
            .chars()
            .take(512)
            .collect::<String>()
    );
    Ok(())
}

#[cfg(windows)]
fn desktop_command(title: &str, message: &str) -> Result<Command> {
    // All variable text travels as data. No issue/provider text enters the script.
    let script = r#"$ErrorActionPreference = 'Stop';
[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType=WindowsRuntime] > $null;
[Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType=WindowsRuntime] > $null;
$xml = [Windows.Data.Xml.Dom.XmlDocument]::new();
$xml.LoadXml('<toast><visual><binding template="ToastGeneric"><text/><text/></binding></visual></toast>');
$nodes = $xml.GetElementsByTagName('text');
$nodes.Item(0).AppendChild($xml.CreateTextNode($env:TREERAIL_NOTICE_TITLE)) > $null;
$nodes.Item(1).AppendChild($xml.CreateTextNode($env:TREERAIL_NOTICE_BODY)) > $null;
$app = '{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe';
$notifier = [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($app);
if ($notifier.Setting -ne 0) { throw ('Desktop notifications disabled by Windows: ' + $notifier.Setting) };
$notifier.Show([Windows.UI.Notifications.ToastNotification]::new($xml));"#;
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-WindowStyle",
        "Hidden",
        "-Command",
        script,
    ]);
    command
        .env("TREERAIL_NOTICE_TITLE", title)
        .env("TREERAIL_NOTICE_BODY", message);
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    Ok(command)
}

#[cfg(target_os = "linux")]
fn desktop_command(title: &str, message: &str) -> Result<Command> {
    let mut command = Command::new("notify-send");
    command.args(["--app-name=Tree Rail", "--", title, message]);
    Ok(command)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn desktop_command(_: &str, _: &str) -> Result<Command> {
    anyhow::bail!("Desktop notifications are not supported on this platform")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_transitions_are_opt_in_and_reconnect_does_not_replay() {
        let mut notifications = Notifications::default();
        assert!(
            notifications
                .observe(
                    "s",
                    "#1",
                    Some((Kind::NeedsInput, "permission-1")),
                    "approve?"
                )
                .is_none()
        );
        notifications.set_enabled(true);
        assert!(
            notifications
                .observe(
                    "s",
                    "#1",
                    Some((Kind::NeedsInput, "permission-1")),
                    "approve?"
                )
                .is_none()
        );
        assert!(notifications.observe("s", "#1", None, "Ready").is_none());
        let notice = notifications
            .observe("s", "#1", Some((Kind::NeedsInput, "question-2")), "answer?")
            .unwrap();
        assert_eq!(notice.kind, Kind::NeedsInput);
        assert!(
            notifications
                .observe("s", "#1", Some((Kind::NeedsInput, "question-2")), "answer?")
                .is_none()
        );
        assert!(
            notifications
                .observe("s", "#1", Some((Kind::Failure, "process-1")), "failed")
                .is_some()
        );
        assert!(
            notifications
                .observe("s", "#1", Some((Kind::Failure, "process-1")), "failed")
                .is_none()
        );
        notifications.set_enabled(false);
        assert!(
            notifications
                .observe("other", "#2", Some((Kind::Failure, "process-2")), "failed")
                .is_none()
        );
    }

    #[test]
    fn independent_tickets_keep_their_request_identity() {
        let mut notifications = Notifications::default();
        notifications.set_enabled(true);
        for session in ["one", "two"] {
            assert!(
                notifications
                    .observe(
                        session,
                        session,
                        Some((Kind::NeedsInput, "same-provider-request")),
                        "?"
                    )
                    .is_some()
            );
        }
        assert!(
            notifications
                .observe(
                    "one",
                    "one",
                    Some((Kind::NeedsInput, "same-provider-request")),
                    "?"
                )
                .is_none()
        );
        assert!(
            notifications
                .observe("one", "one", None, "Running")
                .is_none()
        );
        assert!(
            notifications
                .observe("one", "one", Some((Kind::NeedsInput, "next-request")), "?")
                .is_some()
        );
    }
}
