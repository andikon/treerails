# Tree Rail v0.3.1 — Starting-point worktree creation

- Start from the selected worktree's branch or commit, or fuzzy-search local and remote branches.
- Choose an existing branch for review, or name a new branch from that starting point.
- Fetch automatically and use the configured checkout location; remote, directory and Fetch controls are removed from the form.
- Review checkouts establish upstream tracking and fast-forward only; checked-out branches, diverged commits and conflicting upstreams are protected.

Windows downloads include the standalone executable and a ZIP with the usage guide
and example configuration. SHA-256 checksums are included. Git must be on PATH;
no Rust or separate Node.js installation is required.

Validation: 52 Rust tests, four packaging tests, formatting and strict Clippy passed.
The Copilot CLI AHP integration remains experimental; real model responses, tool
approvals and interactive VS Code handoff remain manual checks. Linux/macOS
downloads are not included in this release.

---

# Tree Rail v0.3.0 — Worktrees and the real Copilot CLI

Tree Rail now follows Project → Worktree → Conversation. The ticket, task,
discussion and persistent terminal-host layers have been removed.

- Project tabs, fixed worktree and session/file columns, Copilot CLI/diff on the
  right, small creation controls and one segmented status line.
- The actual Copilot CLI runs in the right pane, attached to the same AHP
  conversation. Copilot owns menus, input and approvals. Ctrl+] returns focus
  to Tree Rail; switching views preserves terminals. The experimental AHP
  frontend and compatible SDK backend follow Tree Rail's lifetime.
- Independent worktree creation and session creation, with an optional new
  worktree. New checkouts start at the freshly fetched remote default branch
  unless an explicit starting reference is supplied.
- Sessions + opens a new idle Copilot CLI conversation directly, without a
  setup form. Optional session/worktree settings remain in Help. Worktree rows
  show added/removed line totals, including untracked text files.
- Stage, unstage, commit, fetch, fast-forward pull and push/publish through Help.
  Checkout leases exclude managed turns and Git writes in the same worktree;
  turns started by other clients acquire guards when observed.
- Pure `config.toml` settings and separate sibling `state.json`, both loaded
  before the first frame. Combined legacy files migrate with a complete backup.
- Explicit confirmation for active-tab closure and quitting; closing a tab hides
  its conversations, while quitting stops the owned runtime. Restart never
  sends a prompt automatically.

Windows downloads are built and tested locally. Download the standalone
`treerail-windows-x86_64.exe`, or extract `treerail-windows-x86_64.zip` for the
executable, usage guide and example configuration. Git must be on PATH;
no Rust or separate Node.js installation is required. Sign in with GitHub CLI
or provide `COPILOT_GITHUB_TOKEN`. Binaries are not code-signed.

Validation: 48 Rust tests, four packaging tests, formatting and strict Clippy
passed. The real CLI accepted typing and resizing on an idle AHP conversation,
and VS Code discovered that same session. No live model prompt was submitted.
The CLI's AHP client is experimental; real model responses, tool approvals
and interactive VS Code handoff still need verification. Linux/macOS downloads
are not included in this release. See the
[validation record](https://github.com/andikon/treerails/blob/v0.3.0/docs/testing/worktree-release.md).

---

# Tree Rail v0.2.0 — One workspace for tickets, worktrees and agents

Work, Tickets and Worktrees now feed the same task workspace. Find a GitHub issue,
resume work needing attention, or open an existing checkout, then use the same
Git and hosted-agent workflow throughout.

## What's new

- **Shared task workspace:** Overview, Changes, Agent, Discussion and Activity keep
  the task, branch and checkout together. Projects remember their last entry view.
- **Local or ticket-backed work:** start a local task from a worktree without a
  GitHub issue, then explicitly link it later while retaining objectives, task
  identities, conversations and branches. Ticket tasks share their ticket checkout.
- **A useful ticket browser:** labeled cards, Active work, multiple named saved
  views, guided filters, automatic pagination and a side-by-side discussion preview.
  Cached tickets and discussions remain readable offline.
- **Discussion and comments:** formatted issue bodies and chronological comments,
  persistent drafts, explicit review before posting, and recovery when a send's
  confirmation is unknown. Open the issue in GitHub for further details.
- **Hosted Copilot sessions:** a persistent local host keeps sessions alive after
  the UI closes. Detach, reattach, resume the same conversation, hand over a
  checkout, and reconcile interrupted work. Running, Needs input and Ready are
  separate from manual task progress and GitHub issue state.
- **Reviewed starting context:** new conversations receive a complete retained
  discussion document, or a local task objective and handoff. Approved context is
  frozen and survives checkout cleanup; Continue does not replay it.
- **Clearer controls and safeguards:** contextual legends, distinct navigation and
  agent-input modes, protection against stale input or delayed replies, and one
  managed owner per physical checkout across all entry points.

## Getting started

Use `W` for Work, `T` for Tickets and `w` for Worktrees. In a workspace, use
`1` Overview, `2` Changes, `3` Agent, `4` Discussion and `5` Activity.
`S` opens agent review; a bare checkout first asks for a local task objective.
`s` continues/starts from Overview or Agent. `?` shows contextual actions.

## Downloads

This release's binaries were built and checked locally on Windows. Linux/macOS
binaries are not included; source archives remain available for building there.
No cloud builds or paid agent tests were run for this release.

| Machine | Download |
| --- | --- |
| Windows, Intel/AMD 64-bit | `treerail-windows-x86_64.exe` (standalone) or `treerail-windows-x86_64.zip` (with documentation) |

For the standalone download, rename it to `treerail.exe`, place it in a folder on
`PATH`, and run `treerail 'C:\Code'` in a terminal. Alternatively, extract the ZIP
and run the executable inside. No Rust installation is needed for these downloads.
The archive includes the usage guide and example configuration; `SHA256SUMS`
contains checksums for the two downloads. Binaries are not code-signed.

Git must be on `PATH`. Ticket/discussion features need authenticated GitHub CLI
(`gh`). Native hosted starts are currently verified for **Windows with Copilot CLI
1.0.94**; other platform/version combinations remain gated. External tool profiles
and ordinary Git/worktree operations remain available independently.

Existing settings are retained without destructive migration. Local task records
and retained documents remain on disk when downgrading, but older releases cannot
manage the new local tasks. See the [usage guide](https://github.com/andikon/treerails/blob/v0.2.0/docs/usage.md)
and [native delivery compatibility](https://github.com/andikon/treerails/blob/v0.2.0/docs/native-discussion-delivery.md).
