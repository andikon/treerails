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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub additions: u64,
    pub deletions: u64,
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
    let mut snapshot = parse_status(&data)?;
    match run(
        path,
        [
            "diff",
            "--numstat",
            "-z",
            "--no-ext-diff",
            "--no-textconv",
            "HEAD",
            "--",
        ],
    )
    .await
    {
        Ok(stats) => {
            let mut records = stats.split(|b| *b == 0);
            while let Some(record) = records.next() {
                let fields = record.splitn(3, |b| *b == b'\t').collect::<Vec<_>>();
                if fields.len() != 3 {
                    continue;
                }
                snapshot.additions += String::from_utf8_lossy(fields[0])
                    .parse::<u64>()
                    .unwrap_or(0);
                snapshot.deletions += String::from_utf8_lossy(fields[1])
                    .parse::<u64>()
                    .unwrap_or(0);
                if fields[2].is_empty() {
                    records.next();
                    records.next();
                }
            }
            for file in snapshot.files.iter().filter(|f| f.untracked) {
                snapshot.additions += text_lines(&path.join(&file.path)).await?;
            }
        }
        Err(error) => {
            // Before the first commit every extant file is an addition.
            if run(path, ["rev-parse", "--verify", "HEAD"]).await.is_ok() {
                return Err(error);
            }
            for file in &snapshot.files {
                snapshot.additions += text_lines(&path.join(&file.path)).await?;
            }
        }
    }
    Ok(snapshot)
}
async fn text_lines(path: &Path) -> Result<u64> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e.into()),
    };
    if metadata.file_type().is_symlink() {
        return Ok(1);
    }
    if !metadata.is_file() {
        return Ok(0);
    }
    let mut file = tokio::fs::File::open(path).await?;
    let mut bytes = vec![0u8; 65536];
    let (mut lines, mut read, mut last) = (0, 0, None);
    loop {
        let count = file.read(&mut bytes).await?;
        if count == 0 {
            break;
        }
        if read < 8000 && bytes[..count.min(8000 - read)].contains(&0) {
            return Ok(0);
        }
        read += count;
        lines += bytes[..count].iter().filter(|b| **b == b'\n').count() as u64;
        last = bytes[..count].last().copied();
    }
    Ok(lines + u64::from(last.is_some_and(|b| b != b'\n')))
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
        anyhow::ensure!(
            wt.branch != branch,
            "Branch '{branch}' is already checked out at {}",
            wt.path.display()
        );
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
    let _lease = crate::checkout::Lease::acquire(path).await?;
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
    let _lease = crate::checkout::Lease::acquire(path).await?;
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
    let _lease = crate::checkout::Lease::acquire(path).await?;
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
pub async fn pull(path: &Path) -> Result<()> {
    let _lease = crate::checkout::Lease::acquire(path).await?;
    run(path, ["pull", "--progress", "--ff-only"]).await?;
    Ok(())
}
pub async fn choose_remote(path: &Path, preferred: Option<&str>) -> Result<String> {
    let remotes = remotes(path).await?;
    if let Some(remote) = preferred {
        anyhow::ensure!(
            remotes.iter().any(|r| r == remote),
            "Configured remote '{remote}' does not exist"
        );
        return Ok(remote.into());
    }
    if remotes.iter().any(|r| r == "origin") {
        return Ok("origin".into());
    }
    anyhow::ensure!(
        remotes.len() == 1,
        "Choose a remote explicitly (available: {})",
        remotes.join(", ")
    );
    Ok(remotes[0].clone())
}
/// Query the actual remote HEAD after fetching. Never infer main/develop or
/// silently fall back to the selected checkout when the remote is unavailable.
pub async fn fresh_default_base(path: &Path, preferred: Option<&str>) -> Result<String> {
    let remote = choose_remote(path, preferred).await?;
    run(path, ["fetch", "--progress", "--prune", "--", &remote]).await?;
    let advertised = text(path, &["ls-remote", "--symref", "--", &remote, "HEAD"]).await?;
    let branch = advertised
        .lines()
        .find_map(|line| {
            line.strip_prefix("ref: refs/heads/")
                .and_then(|s| s.split_once('\t'))
                .map(|(branch, _)| branch)
        })
        .context("Remote has no default branch; select an explicit starting point")?;
    let reference = format!("refs/remotes/{remote}/{branch}");
    run(
        path,
        [
            "fetch",
            "--progress",
            "--",
            &remote,
            &format!("refs/heads/{branch}:{reference}"),
        ],
    )
    .await?;
    Ok(format!("{remote}/{branch}"))
}
pub async fn push_or_publish(path: &Path, preferred: Option<&str>) -> Result<()> {
    if status(path).await?.upstream.is_some() {
        return push(path).await;
    }
    let branch = text(path, &["symbolic-ref", "--short", "HEAD"])
        .await
        .context("Cannot publish a detached checkout")?;
    let remote = choose_remote(path, preferred).await?;
    publish(path, &remote, &branch).await
}
pub async fn remotes(path: &Path) -> Result<Vec<String>> {
    Ok(text(path, &["remote"])
        .await?
        .lines()
        .map(str::to_owned)
        .collect())
}
pub async fn publish(path: &Path, remote: &str, branch: &str) -> Result<()> {
    let _lease = crate::checkout::Lease::acquire(path).await?;
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
    let _lease = crate::checkout::Lease::acquire(path).await?;
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
