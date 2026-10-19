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
