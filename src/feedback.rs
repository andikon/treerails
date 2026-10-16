//! Session-only feedback from a user-requested operation. Background Git queries
//! run outside the scoped reporter and never create operation records.
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use unicode_width::UnicodeWidthChar;

pub const OUTPUT_LIMIT: usize = 64 * 1024;
pub const HISTORY_LIMIT: usize = 20;

fn readable_output(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
        .map(|c| if c == '\r' { '\n' } else { c })
        .collect()
}

#[derive(Debug, Clone)]
pub enum Update {
    Phase(String),
    Destination(String),
    Output { stderr: bool, text: String },
}
#[derive(Debug, Clone)]
pub struct Event {
    pub id: u64,
    pub update: Update,
}
#[derive(Clone)]
pub struct Reporter {
    pub id: u64,
    pub sender: mpsc::Sender<Event>,
}
tokio::task_local! { static REPORTER: Reporter; }

pub async fn scope<T>(reporter: Reporter, future: impl Future<Output = T>) -> T {
    REPORTER.scope(reporter, future).await
}
pub async fn report(update: Update) {
    let reporter = REPORTER.try_with(Clone::clone).ok();
    if let Some(reporter) = reporter {
        let _ = reporter
            .sender
            .send(Event {
                id: reporter.id,
                update,
            })
            .await;
    }
}
pub fn active() -> bool {
    REPORTER.try_with(|_| ()).is_ok()
}

#[derive(Debug, Clone)]
pub struct Record {
    pub id: u64,
    pub action: String,
    pub project: PathBuf,
    pub target: PathBuf,
    pub phase: String,
    pub started: Instant,
    pub elapsed: Option<Duration>,
    pub result: Option<Result<String, String>>,
    pub output: String,
    pub truncated: bool,
    pub progress: String,
    pub destination: Option<String>,
    pending_stdout: String,
    pending_stderr: String,
}
impl Record {
    pub fn new(id: u64, action: String, project: PathBuf, target: PathBuf) -> Self {
        Self {
            id,
            action,
            project,
            target,
            phase: "Queued".into(),
            started: Instant::now(),
            elapsed: None,
            result: None,
            output: String::new(),
            truncated: false,
            progress: String::new(),
            destination: None,
            pending_stdout: String::new(),
            pending_stderr: String::new(),
        }
    }
    pub fn token(&self) -> String {
        format!("operation:{}", self.id)
    }
    pub fn duration(&self) -> Duration {
        self.elapsed.unwrap_or_else(|| self.started.elapsed())
    }
    pub fn running(&self) -> bool {
        self.result.is_none()
    }
    pub fn display_lines(&self, width: usize) -> Vec<String> {
        Self::wrap_lines(&self.output, width)
    }
    pub fn details_lines(&self, width: usize) -> Vec<String> {
        let outcome = match &self.result {
            Some(Ok(message)) => format!("Succeeded: {message}"),
            Some(Err(message)) => format!("Failed: {message}"),
            None => "Running".into(),
        };
        Self::wrap_lines(
            &format!(
                "Project: {}\nTarget: {}\n{}\n\n{}",
                self.project.display(),
                self.target.display(),
                outcome,
                self.output
            ),
            width,
        )
    }
    fn wrap_lines(text: &str, width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        if width == 0 {
            return lines;
        }
        for line in text.lines() {
            let mut row = String::new();
            let mut columns = 0;
            for character in line.chars() {
                let size = character.width().unwrap_or(0);
                if columns + size > width && !row.is_empty() {
                    lines.push(std::mem::take(&mut row));
                    columns = 0;
                }
                row.push(character);
                columns += size;
            }
            lines.push(row);
        }
        lines
    }
    pub fn apply(&mut self, update: Update) {
        match update {
            Update::Destination(destination) => self.destination = Some(destination),
            Update::Phase(phase) => {
                self.phase = phase;
                self.progress.clear();
                self.pending_stdout.clear();
                self.pending_stderr.clear();
                self.append(&format!("\n› {}\n", self.phase));
            }
            Update::Output { stderr, text } => {
                // Carriage returns are Git's normal progress line delimiters.
                // Retain those updates as readable lines, including partial output.
                let text = readable_output(&text);
                self.append(&text);
                let pending = if stderr {
                    &mut self.pending_stderr
                } else {
                    &mut self.pending_stdout
                };
                pending.push_str(&text);
                for line in pending.lines() {
                    let line = line.trim().trim_start_matches("remote: ");
                    if [
                        "Enumerating objects:",
                        "Counting objects:",
                        "Compressing objects:",
                        "Writing objects:",
                        "Receiving objects:",
                        "Resolving deltas:",
                        "Updating files:",
                        "Checking out files:",
                    ]
                    .iter()
                    .any(|prefix| line.starts_with(prefix))
                    {
                        self.progress = line.chars().take(240).collect();
                    }
                }
                if let Some(end) = pending.rfind('\n') {
                    pending.drain(..=end);
                }
                if pending.len() > 8192 {
                    pending.clear();
                }
            }
        }
    }
    fn append(&mut self, text: &str) {
        self.output.push_str(text);
        if self.output.len() > OUTPUT_LIMIT {
            let mut cut = self.output.len() - OUTPUT_LIMIT;
            while !self.output.is_char_boundary(cut) {
                cut += 1;
            }
            self.output.drain(..cut);
            self.truncated = true;
        }
    }
    pub fn finish(&mut self, mut result: Result<String, String>) {
        if result.as_deref() == Ok("Completed") {
            if self
                .output
                .lines()
                .any(|line| line.trim() == "Everything up-to-date")
            {
                result = Ok("Everything up-to-date".into());
            } else if self
                .output
                .lines()
                .any(|line| line.trim() == "Already up to date.")
            {
                result = Ok("Already up to date.".into());
            }
        }
        let message = match &mut result {
            Ok(message) | Err(message) => message,
        };
        *message = readable_output(message);
        if message.len() > 8192 {
            let mut start = message.len() - 8192;
            while !message.is_char_boundary(start) {
                start += 1;
            }
            *message = format!("[Earlier result text discarded]\n{}", &message[start..]);
        }
        self.elapsed = Some(self.started.elapsed());
        self.phase = if result.is_ok() {
            "Succeeded"
        } else {
            "Failed"
        }
        .into();
        self.progress.clear();
        self.append(&format!(
            "\n{}: {}\n",
            self.phase,
            result.as_ref().unwrap_or_else(|error| error)
        ));
        self.result = Some(result);
    }
    pub fn headline(&self) -> String {
        let indicator = if self.running() {
            ["◐", "◓", "◑", "◒"][(self.started.elapsed().as_millis() / 150 % 4) as usize]
        } else if self.result.as_ref().is_some_and(Result::is_ok) {
            "✓"
        } else {
            "!"
        };
        format!(
            "{indicator} {} · {} · {}s · i details",
            self.action_label(),
            self.phase,
            self.duration().as_secs()
        )
    }
    pub fn action_label(&self) -> String {
        let destination = self
            .destination
            .as_ref()
            .map(|d| format!(" → {d}"))
            .unwrap_or_default();
        format!("{}{destination}", self.action)
    }
    pub fn summary(&self) -> String {
        format!("{}\n{}", self.headline(), self.target.display())
    }
}
