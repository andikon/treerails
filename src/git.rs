use crate::feedback::{self, Update};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::Stdio,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
};

#[derive(Debug, Clone, Serialize)]
pub struct Project {
    pub path: PathBuf,
    pub name: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: String,
    pub head: String,
    pub original: bool,
    pub locked: bool,
    pub prunable: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct ChangedFile {
    pub path: PathBuf,
    pub original: Option<PathBuf>,
    pub index: char,
    pub work: char,
    pub untracked: bool,
}
impl ChangedFile {
    pub fn staged(&self) -> bool {
        !matches!(self.index, ' ' | '?')
    }
    pub fn unstaged(&self) -> bool {
        self.untracked || self.work != ' '
    }
    pub fn code(&self) -> char {
        if self.untracked || self.index == 'A' {
            '+'
        } else if self.index == 'D' || self.work == 'D' {
            '-'
        } else {
            '~'
        }
    }
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct Snapshot {
    pub files: Vec<ChangedFile>,
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
}
impl Snapshot {
    pub fn counts(&self) -> (usize, usize, usize) {
        let (mut a, mut m, mut d) = (0, 0, 0);
        for file in &self.files {
            match file.code() {
                '+' => a += 1,
                '-' => d += 1,
                _ => m += 1,
            }
        }
        (a, m, d)
    }
}
#[derive(Debug, Clone)]
pub struct Branch {
    pub name: String,
    pub remote: bool,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub head_ref_name: String,
    pub base_ref_name: String,
    pub url: String,
}

fn command(program: &str, cwd: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.current_dir(cwd).stdin(Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    cmd
}
pub async fn run<I, S>(cwd: &Path, args: I) -> Result<Vec<u8>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args: Vec<OsString> = args.into_iter().map(|s| s.as_ref().to_owned()).collect();
    let mut cmd = command("git", cwd);
    cmd.arg("--literal-pathspecs")
        .args(&args)
        .env("GIT_OPTIONAL_LOCKS", "0");
    let output = if feedback::active() {
        let words: Vec<_> = args.iter().map(|s| s.to_string_lossy()).collect();
        let first = words.first().map(|s| s.as_ref()).unwrap_or("Git");
        let phase = match first {
            "fetch" if words.iter().any(|s| s.contains("refs/pull/")) => "Fetching PR head",
            "fetch" if words.iter().any(|s| s.contains("/base")) => "Fetching PR base",
            "fetch" => "Fetching remotes",
            "pull" => "Pulling (fast-forward only)",
            "push" => "Pushing to remote",
            "worktree" if words.get(1).is_some_and(|s| s == "add") => "Creating worktree",
            "worktree" if words.get(1).is_some_and(|s| s == "remove") => "Removing worktree",
            "branch" if words.iter().any(|s| s == "-d") => "Deleting branch",
            "branch" => "Creating branch",
            "add" => "Staging files",
            "restore" | "rm" => "Unstaging files",
            "check-ref-format" => "Checking branch name",
            "status" => "Checking worktree status",
            _ => "Checking repository",
        };
        feedback::report(Update::Phase(phase.into())).await;
        let visible = matches!(
            first,
            "fetch" | "pull" | "push" | "branch" | "add" | "restore" | "rm"
        ) || first == "worktree"
            && words.get(1).is_some_and(|s| s == "add" || s == "remove");
        capture(&mut cmd, None, visible).await?
    } else {
        cmd.output()
            .await
            .context("Cannot start Git; install Git and add it to PATH")?
    };
    if !output.status.success() {
        bail!("{}", failure_message(&output));
    }
    Ok(output.stdout)
}
fn failure_message(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stderr.trim().is_empty() {
        stderr.trim().into()
    } else if !stdout.trim().is_empty() {
        stdout.trim().into()
    } else {
        format!("Command failed ({}) without output", output.status)
    }
}

// Drain both pipes together: hooks, remote helpers and signing programs can write
// to either stream. A slow consumer applies backpressure instead of growing a queue.
async fn capture(
    cmd: &mut Command,
    input: Option<Vec<u8>>,
    visible: bool,
) -> Result<std::process::Output> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    if input.is_some() {
        cmd.stdin(Stdio::piped());
    }
    let mut child = cmd
        .spawn()
        .context("Cannot start command; check Git/GitHub CLI availability")?;
    let stdout = child.stdout.take().context("Command stdout unavailable")?;
    let stderr = child.stderr.take().context("Command stderr unavailable")?;
    let stdin = child.stdin.take();
    let write = async move {
        if let (Some(mut stdin), Some(input)) = (stdin, input) {
            stdin.write_all(&input).await?;
        }
        Ok::<_, anyhow::Error>(())
    };
    let (stdout, stderr, written) = tokio::join!(
        read_pipe(stdout, false, visible, visible),
        read_pipe(stderr, true, true, true),
        write
    );
    let status = child.wait().await?;
    // An early Git failure can close stdin before a long commit message is
    // delivered. Preserve Git's actual failure output instead of masking it
    // with a secondary broken-pipe error.
    if status.success() {
        written?;
    }
    Ok(std::process::Output {
        status,
        stdout: stdout?,
        stderr: stderr?,
    })
}
async fn read_pipe(
    mut pipe: impl AsyncRead + Unpin,
    stderr: bool,
    visible: bool,
    bounded: bool,
) -> Result<Vec<u8>> {
    const CAPTURE_LIMIT: usize = 1024 * 1024;
    let mut retained = Vec::new();
    let mut pending = Vec::new();
    let mut bytes = [0; 2048];
    loop {
        let count = pipe.read(&mut bytes).await?;
        if count == 0 {
            break;
        }
        retained.extend_from_slice(&bytes[..count]);
        if bounded && retained.len() > CAPTURE_LIMIT {
            retained.drain(..retained.len() - CAPTURE_LIMIT);
        }
        if visible {
            pending.extend_from_slice(&bytes[..count]);
            // Keep an incomplete UTF-8 character until the next read. Invalid
            // bytes are rendered with replacement characters, never escape codes.
            let length = match std::str::from_utf8(&pending) {
                Ok(_) => pending.len(),
                Err(error) if error.error_len().is_none() => error.valid_up_to(),
                Err(_) => pending.len(),
            };
            if length > 0 {
                feedback::report(Update::Output {
                    stderr,
                    text: String::from_utf8_lossy(&pending[..length]).into_owned(),
                })
                .await;
                pending.drain(..length);
            }
        }
    }
    if !pending.is_empty() {
        feedback::report(Update::Output {
            stderr,
            text: String::from_utf8_lossy(&pending).into_owned(),
        })
        .await;
    }
    Ok(retained)
}
pub async fn text(cwd: &Path, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8_lossy(&run(cwd, args).await?)
        .trim()
        .to_owned())
}
pub async fn project(path: &Path) -> Result<Project> {
    let common = text(
        path,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .await?;
    let common = PathBuf::from(common);
    // The standard common directory is <original checkout>/.git, even when called in a linked worktree.
    let root = if common.file_name() == Some(OsStr::new(".git")) {
        common.parent().unwrap().to_owned()
    } else {
        PathBuf::from(text(path, &["rev-parse", "--show-toplevel"]).await?)
    };
    Ok(Project {
        name: root
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        path: root,
    })
}
pub async fn worktrees(path: &Path) -> Result<Vec<Worktree>> {
    let data = run(path, ["worktree", "list", "--porcelain", "-z"]).await?;
    let mut result = Vec::new();
    let mut current: Option<Worktree> = None;
    for record in data.split(|b| *b == 0) {
        if let Some(bytes) = record.strip_prefix(b"worktree ") {
            if let Some(wt) = current.take() {
                result.push(wt);
            }
            current = Some(Worktree {
                path: bytes_path(bytes),
                branch: "(detached)".into(),
                head: String::new(),
                original: result.is_empty(),
                locked: false,
                prunable: false,
            });
        } else if let Some(wt) = current.as_mut() {
            if let Some(v) = record.strip_prefix(b"branch refs/heads/") {
                wt.branch = String::from_utf8_lossy(v).into_owned();
            } else if let Some(v) = record.strip_prefix(b"HEAD ") {
                wt.head = String::from_utf8_lossy(v).into_owned();
            } else if record.starts_with(b"locked") {
                wt.locked = true;
            } else if record.starts_with(b"prunable") {
                wt.prunable = true;
            }
        }
    }
    if let Some(wt) = current {
        result.push(wt);
    }
    Ok(result)
}
pub async fn status(path: &Path) -> Result<Snapshot> {
    let data = run(
        path,
        [
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
        ],
    )
    .await?;
    parse_status(&data)
}
pub fn parse_status(data: &[u8]) -> Result<Snapshot> {
    let mut snapshot = Snapshot::default();
    let mut records = data.split(|b| *b == 0);
    while let Some(record) = records.next() {
        if let Some(v) = record.strip_prefix(b"# branch.head ") {
            snapshot.branch = String::from_utf8_lossy(v).into_owned();
        } else if let Some(v) = record.strip_prefix(b"# branch.upstream ") {
            snapshot.upstream = Some(String::from_utf8_lossy(v).into_owned());
        } else if let Some(v) = record.strip_prefix(b"# branch.ab ") {
            let s = String::from_utf8_lossy(v);
            let mut parts = s.split_whitespace();
            snapshot.ahead = parts
                .next()
                .unwrap_or("+0")
                .trim_start_matches('+')
                .parse()
                .unwrap_or(0);
            snapshot.behind = parts
                .next()
                .unwrap_or("-0")
                .trim_start_matches('-')
                .parse()
                .unwrap_or(0);
        } else if record.starts_with(b"? ") {
            snapshot.files.push(ChangedFile {
                path: bytes_path(&record[2..]),
                original: None,
                index: '?',
                work: '?',
                untracked: true,
            });
        } else if matches!(record.first(), Some(b'1' | b'2' | b'u')) {
            let fields: Vec<&[u8]> = record
                .splitn(
                    if record[0] == b'1' {
                        9
                    } else if record[0] == b'2' {
                        10
                    } else {
                        11
                    },
                    |b| *b == b' ',
                )
                .collect();
            anyhow::ensure!(
                fields.len() >= 9 && fields[1].len() == 2,
                "Malformed Git status record"
            );
            let xy = fields[1];
            let original = if record[0] == b'2' {
                Some(bytes_path(records.next().context("Missing rename source")?))
            } else {
                None
            };
            snapshot.files.push(ChangedFile {
                path: bytes_path(fields.last().unwrap()),
                original,
                index: if xy[0] == b'.' { ' ' } else { xy[0] as char },
                work: if xy[1] == b'.' { ' ' } else { xy[1] as char },
                untracked: false,
            });
        }
    }
    snapshot.files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(snapshot)
}
fn bytes_path(bytes: &[u8]) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        PathBuf::from(OsStr::from_bytes(bytes))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(bytes).as_ref())
    }
}
pub async fn diff(path: &Path, file: &ChangedFile, staged: bool) -> Result<String> {
    if file.untracked {
        let bytes = tokio::fs::read(path.join(&file.path)).await?;
        if bytes.contains(&0) {
            return Ok("Binary untracked file".into());
        }
        return Ok(String::from_utf8_lossy(&bytes)
            .lines()
            .take(20_000)
            .map(|s| format!("+{s}\n"))
            .collect());
    }
    let mut args: Vec<OsString> = vec![
        "--no-pager".into(),
        "diff".into(),
        "--no-ext-diff".into(),
        "--no-textconv".into(),
        "--no-color".into(),
    ];
    if staged {
        args.push("--cached".into());
    }
    args.push("--".into());
    args.push(file.path.clone().into_os_string());
    if let Some(original) = &file.original {
        args.push(original.clone().into_os_string());
    }
    let bytes = run(path, args).await?;
    Ok(String::from_utf8_lossy(&bytes)
        .lines()
        .take(20_000)
        .collect::<Vec<_>>()
        .join("\n"))
}
pub async fn branches(path: &Path) -> Result<Vec<Branch>> {
    let data = text(
        path,
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/heads",
            "refs/remotes",
        ],
    )
    .await?;
    Ok(data
        .lines()
        .filter(|s| !s.ends_with("/HEAD"))
        .filter_map(|s| {
            s.strip_prefix("refs/heads/")
                .map(|n| Branch {
                    name: n.into(),
                    remote: false,
                })
                .or_else(|| {
                    s.strip_prefix("refs/remotes/").map(|n| Branch {
                        name: n.into(),
                        remote: true,
                    })
                })
        })
        .collect())
}
pub async fn default_base(path: &Path) -> String {
    let mut names = remotes(path).await.unwrap_or_default();
    names.sort_by_key(|name| (name != "origin", name.clone()));
    for remote in names {
        if let Ok(v) = text(
            path,
            &[
                "symbolic-ref",
                "--short",
                &format!("refs/remotes/{remote}/HEAD"),
            ],
        )
        .await
        {
            return v;
        }
    }
    if let Ok(v) = text(path, &["symbolic-ref", "--short", "HEAD"]).await {
        return v;
    }
    "HEAD".into()
}
pub async fn create_worktree(
    project: &Path,
    branch: &str,
    base: &str,
    destination: &Path,
    existing: bool,
    fetch: bool,
) -> Result<PathBuf> {
    let destination = if destination.is_absolute() {
        destination.to_owned()
    } else {
        project.join(destination)
    };
    run(project, ["check-ref-format", "--branch", branch]).await?;
    for wt in worktrees(project).await? {
        if wt.branch == branch {
            return Ok(wt.path);
        }
    }
    anyhow::ensure!(
        !destination.exists(),
        "Destination already exists: {}",
        destination.display()
    );
    if fetch {
        run(project, ["fetch", "--progress", "--all", "--prune"]).await?;
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut args: Vec<OsString> = vec!["worktree".into(), "add".into()];
    if !existing {
        args.extend(["--no-track".into(), "-b".into(), branch.into()]);
    }
    args.push("--".into());
    args.push(destination.as_os_str().to_owned());
    args.push(if existing { branch.into() } else { base.into() });
    run(project, args).await?;
    Ok(destination)
}
pub async fn stage(path: &Path, files: &[ChangedFile], unstage: bool) -> Result<()> {
    let mut args: Vec<OsString> = if unstage {
        if run(path, ["rev-parse", "--verify", "--quiet", "HEAD"])
            .await
            .is_ok()
        {
            vec!["restore".into(), "--staged".into()]
        } else {
            vec!["rm".into(), "--cached".into(), "--ignore-unmatch".into()]
        }
    } else {
        vec!["add".into(), "-A".into()]
    };
    args.push("--".into());
    for file in files {
        args.push(file.path.as_os_str().to_owned());
        if let Some(original) = &file.original {
            args.push(original.as_os_str().to_owned());
        }
    }
    if !files.is_empty() {
        run(path, args).await?;
    }
    Ok(())
}
pub async fn commit(path: &Path, subject: &str, body: &str) -> Result<String> {
    anyhow::ensure!(!subject.trim().is_empty(), "A commit subject is required");
    feedback::report(Update::Phase(
        "Committing (hooks and signing enabled)".into(),
    ))
    .await;
    let mut cmd = command("git", path);
    cmd.args(["commit", "--file=-"]);
    let message = format!("{}\n\n{}\n", subject.trim(), body);
    let output = capture(&mut cmd, Some(message.into_bytes()), true).await?;
    if !output.status.success() {
        bail!("{}", failure_message(&output));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().into())
}
pub async fn remove_worktree(project: &Path, path: &Path) -> Result<()> {
    let trees = worktrees(project).await?;
    let wt = trees
        .iter()
        .find(|w| crate::config::same_path(&w.path, path))
        .context("Worktree is no longer registered")?;
    anyhow::ensure!(!wt.original, "The original checkout cannot be removed");
    anyhow::ensure!(
        !wt.locked && !wt.prunable,
        "Locked or missing worktrees must be repaired with Git first"
    );
    anyhow::ensure!(
        status(path).await?.files.is_empty(),
        "Worktree has uncommitted changes; removal refused"
    );
    run(
        project,
        [
            OsStr::new("worktree"),
            OsStr::new("remove"),
            OsStr::new("--"),
            path.as_os_str(),
        ],
    )
    .await?;
    Ok(())
}
pub async fn remotes(path: &Path) -> Result<Vec<String>> {
    Ok(text(path, &["remote"])
        .await?
        .lines()
        .map(str::to_owned)
        .collect())
}
pub async fn publish(path: &Path, remote: &str, branch: &str) -> Result<()> {
    anyhow::ensure!(
        remotes(path).await?.iter().any(|r| r == remote),
        "Unknown remote: {remote}"
    );
    run(path, ["check-ref-format", "--branch", branch]).await?;
    run(
        path,
        [
            "push",
            "--progress",
            "--set-upstream",
            "--",
            remote,
            &format!("HEAD:refs/heads/{branch}"),
        ],
    )
    .await?;
    Ok(())
}
pub async fn push(path: &Path) -> Result<()> {
    let branch = text(path, &["symbolic-ref", "--short", "HEAD"])
        .await
        .context("Cannot push a detached checkout; create a branch first")?;
    let remote = text(
        path,
        &["config", "--get", &format!("branch.{branch}.remote")],
    )
    .await
    .context("No upstream remote; publish this branch first")?;
    let merge = text(
        path,
        &["config", "--get", &format!("branch.{branch}.merge")],
    )
    .await
    .context("No upstream branch; publish this branch first")?;
    anyhow::ensure!(merge.starts_with("refs/heads/"), "Upstream is not a branch");
    feedback::report(Update::Destination(format!(
        "{remote}/{}",
        merge.trim_start_matches("refs/heads/")
    )))
    .await;
    run(
        path,
        [
            "push",
            "--progress",
            "--",
            &remote,
            &format!("HEAD:{merge}"),
        ],
    )
    .await?;
    Ok(())
}
async fn gh(path: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let mut cmd = command("gh", path);
    cmd.args(args);
    let output = if feedback::active() {
        feedback::report(Update::Phase("Reading pull request information".into())).await;
        capture(&mut cmd, None, false).await?
    } else {
        cmd.output()
            .await
            .context("GitHub CLI unavailable; install gh and authenticate with gh auth login")?
    };
    anyhow::ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(output.stdout)
}
pub async fn prs(path: &Path) -> Result<Vec<PullRequest>> {
    Ok(serde_json::from_slice(
        &gh(
            path,
            &[
                "pr",
                "list",
                "--limit",
                "100",
                "--json",
                "number,title,headRefName,baseRefName,url",
            ],
        )
        .await?,
    )?)
}
pub async fn review(project: &Path, number: &str, destination: &Path) -> Result<PathBuf> {
    let pr: PullRequest = serde_json::from_slice(
        &gh(
            project,
            &[
                "pr",
                "view",
                number,
                "--json",
                "number,title,headRefName,baseRefName,url",
            ],
        )
        .await?,
    )?;
    let repo: serde_json::Value =
        serde_json::from_slice(&gh(project, &["repo", "view", "--json", "url"]).await?)?;
    let url = repo["url"]
        .as_str()
        .context("GitHub repository URL missing")?;
    anyhow::ensure!(
        pr.url
            .starts_with(&format!("{}/pull/", url.trim_end_matches('/'))),
        "The PR belongs to a different repository than the selected project"
    );
    let mut transport = url.to_owned();
    for remote in remotes(project).await? {
        if let Ok(remote_url) = text(project, &["remote", "get-url", &remote]).await
            && repository_identity(&remote_url) == repository_identity(url)
        {
            transport = remote;
            break;
        }
    }
    let branch = format!("review/pr-{}", pr.number);
    for wt in worktrees(project).await? {
        if wt.branch == branch {
            return Ok(wt.path);
        }
    }
    anyhow::ensure!(
        !destination.exists(),
        "Review destination already exists: {}",
        destination.display()
    );
    // Fetch GitHub's pull ref through the configured base-repository remote, including fork PRs.
    let reference = format!("refs/treerail/pr/{}/head", pr.number);
    let fetch_head = format!("+refs/pull/{}/head:{reference}", pr.number);
    run(
        project,
        ["fetch", "--progress", "--", &transport, &fetch_head],
    )
    .await?;
    let base_ref = format!("refs/treerail/pr/{}/base", pr.number);
    let fetch_base = format!("+refs/heads/{}:{base_ref}", pr.base_ref_name);
    run(
        project,
        ["fetch", "--progress", "--", &transport, &fetch_base],
    )
    .await?;
    if run(
        project,
        [
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .await
    .is_ok()
    {
        let existing_head = text(project, &["rev-parse", &format!("refs/heads/{branch}")]).await?;
        let pr_head = text(project, &["rev-parse", &reference]).await?;
        anyhow::ensure!(
            existing_head == pr_head,
            "Review branch {branch} already exists at a different commit; choose another branch or update it explicitly"
        );
        create_worktree(project, &branch, &reference, destination, true, false).await
    } else {
        create_worktree(project, &branch, &reference, destination, false, false).await
    }
}
pub async fn review_diff(path: &Path, branch: &str) -> Result<String> {
    let number = branch
        .strip_prefix("review/pr-")
        .context("This is not a Tree Rail PR review branch")?;
    let base = format!("refs/treerail/pr/{number}/base...HEAD");
    text(
        path,
        &[
            "--no-pager",
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            &base,
        ],
    )
    .await
}
pub async fn open_pr(path: &Path, branch: &str) -> Result<()> {
    let number = branch.strip_prefix("review/pr-").unwrap_or(branch);
    gh(path, &["pr", "view", number, "--web"]).await?;
    Ok(())
}
fn repository_identity(url: &str) -> Option<String> {
    let (authority, path) = if let Some((_, rest)) = url.split_once("://") {
        rest.split_once('/')?
    } else {
        url.split_once(':')?
    };
    let host = authority.rsplit('@').next()?;
    Some(format!("{host}/{}", path.trim_matches('/').trim_end_matches(".git")).to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::repository_identity;
    #[test]
    fn github_transport_identity_preserves_ssh_remote_selection() {
        let expected = repository_identity("https://github.com/Owner/Repo");
        assert_eq!(
            expected,
            repository_identity("git@github.com:Owner/Repo.git")
        );
        assert_eq!(
            expected,
            repository_identity("ssh://git@github.com/Owner/Repo.git")
        );
        assert_ne!(
            expected,
            repository_identity("https://github.com/other/repo")
        );
    }
}
