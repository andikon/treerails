# Tree Rail

Status: approved version-one scope. The initial Rust/Ratatui implementation is in `src/`, with usage and validation notes in [the README](../README.md). Cross-platform CI is configured; local validation is on Windows.

Tree Rail is a terminal project explorer and Git worktree manager for development with coding agents. Its command is `treerail`. It runs on Windows, macOS, and Linux, with Windows as the primary development and validation environment.

## Core workflow

1. Launch Tree Rail with project directories, parent directories, a paths file, or saved project sources.
2. Search for and select a project.
3. Select one of its worktrees, including its original checkout, or create a worktree from a chosen starting point.
4. Launch a new terminal in that worktree with Copilot running. Tree Rail stays open.
5. Inspect changes as the editor or agent works; stage files, commit, fetch, pull, and publish or push.
6. Remove the finished worktree, with branch deletion offered separately.

## Settled decisions

### Platform and dependencies

- Implement in **Rust using Ratatui with the Crossterm backend** for terminal rendering, keyboard events, and Windows support.
- Choose a compatible stable Ratatui/Crossterm pair at implementation time, record the toolchain version, and commit `Cargo.lock` for reproducible application builds. Use Cargo for development, testing, and release builds.
- Distribute a native executable per supported operating system and architecture, including `treerail.exe` on Windows. Users need the executable and its documented operating-system dependencies; they do not need a Rust toolchain.
- Validate the normal Windows MSVC target and the selected macOS/Linux targets on their respective platforms. Document any native linking requirements rather than promising a universally static binary.
- Use Clap for CLI arguments, Serde with TOML for editable configuration, `toml_edit` for comment-preserving saves, and platform directory helpers for configuration and local application state.
- Use Tokio for background process orchestration, refresh timers, and event channels. Keep terminal rendering and application-state updates on one UI event loop.
- Invoke the installed Git executable for Git operations, preserving normal hooks, signing, credentials, and repository configuration except for explicit operation choices such as fast-forward-only pull.
- Use the installed GitHub CLI for GitHub PR integration and the installed Copilot CLI for the default agent launch profile.
- Keep project discovery, worktree management, and local Git features usable independently of GitHub and Copilot availability.
- Framework evidence and packaging caveats are recorded in [the research note](research/tui-frameworks.md).

### Rust application structure

- Separate application state and actions, Ratatui rendering, Git/GitHub process integration, configuration, and platform-specific terminal launching into focused modules.
- Use an event/action loop: keyboard input, resize events, refresh timers, and background results produce actions; state updates drive rendering. Git subprocesses and filesystem scans must not block that loop.
- Run Git queries with bounded concurrency. Coordinate mutating operations for the same repository, and discard stale background results when the user changes project, worktree, or file selection.
- Represent filesystem locations with Rust path types and pass subprocess arguments separately, using `Command` and an explicit working directory. Preserve paths with spaces and platform-specific characters throughout discovery, Git operations, and launching.
- Use Git's machine-readable output for repository/worktree/status data. Keep process execution and output parsing independently testable.
- Restore raw mode, the alternate screen, cursor visibility, and other changed terminal settings after normal exit, initialization errors, application errors, and unwinding panics.
- Treat input modes, focus routing, form editing, multi-key Vim sequences, and visual styling as application behavior. Ratatui supplies rendering primitives and widgets; Tree Rail defines the interaction model.

### Project sources and discovery

- Accept one or several directory arguments. A directory can be a repository itself or the direct parent of repositories.
- Support a file containing a list of paths and a configuration file containing saved sources.
- Combine saved sources with temporary sources supplied for the current launch; remove duplicate project entries.
- Scan immediate children of parent directories by default. Recursive discovery must be explicitly requested.
- Exclude directories whose names contain `.worktrees` from project discovery and do not descend into them.
- Discover a project's worktrees using Git's registered worktree list, including worktrees created by other tools or stored outside Tree Rail's default directory.
- Identify linked worktrees as belonging to their repository rather than treating them as separate projects.

### Navigation and presentation

- Navigate from projects into worktrees. The original checkout appears as an ordinary selectable worktree with an identifying label.
- Display adjacent worktree, changed-file, and diff panes when terminal space permits. Adapt to narrower terminal sizes.
- Display each worktree's branch, path, and summary of its current changes.
- Display added files with a green `+`, deleted files with a red `-`, and modified files with a yellow indicator. Labels and symbols communicate meaning alongside color.
- Color added and removed diff lines green and red respectively. Allow inspecting staged and unstaged changes.
- Support whole-file staging and unstaging, including selecting several files. Editing files and partial-file staging are outside version one.
- Refresh changes made by editors and agents automatically, keep input responsive, and preserve the current selection during refresh.
- Refresh the selected worktree promptly and other visible worktrees with bounded background work. Load projects and diffs on demand, and avoid continuously scanning every project or rebuilding every diff.
- Provide searchable lists, visible action hints, and contextual help with `?`.
- Make Vim navigation a first-class interaction model, while also supporting arrow keys.
- Define a consistent Tree Rail theme: restrained borders, clear pane focus, aligned columns, readable spacing, and semantic status colors. Provide usable standard-font symbols without requiring a patched font.
- Handle navigation, search entry, forms, and confirmation dialogs as distinct input contexts. Verify text-input controls for branch names and multiline commit messages against the selected Ratatui version.

### Branches and worktree creation

- Offer creating a new branch and worktree, or using an existing local or remote branch.
- Allow creating a branch independently when a new worktree is not needed.
- Always show the branch name, starting point, and resulting worktree directory in the creation form.
- Provide a searchable starting-point picker for local and remote branches, with support for an explicit commit or tag.
- Remember the last selected starting point per project. On first use, suggest the remote's default branch when available; make the actual base explicit before creation.
- Support an optional fetch before creation; do not silently assume remote-tracking references are current.
- When a chosen branch is already checked out in a registered worktree, select that worktree instead of forcing another checkout of the branch.
- Default to a sibling directory named `<project>.worktrees`, matching Copilot's layout. Flatten branch-name slashes into dashes for the default folder name.
- Allow global and per-project directory overrides.

Example:

```text
C:\Code\my-project
C:\Code\my-project.worktrees\feature-login
C:\Code\my-project.worktrees\review-pr-123
```

### GitHub pull requests

- List open PRs or accept a PR number or URL for the selected project.
- Prepare a local review branch and worktree at the PR head.
- Show the PR's changes relative to its base using GitHub-compatible merge-base comparison, separately from the worktree's uncommitted changes.
- Offer opening the PR in a browser.
- Keep submitting review comments, approvals, and change requests outside version one.
- Keep ticket integration manual: the user supplies a branch name and starting point.

### Everyday Git operations

- Inspect status and diffs, stage/unstage files, commit staged changes, fetch, pull, and push.
- Provide a commit form with a subject and optional multiline body.
- Fetch on request or as an explicit choice before worktree creation.
- Pull using fast-forward only. Explain divergence when that operation cannot proceed.
- Push to the configured upstream when one exists.
- With no upstream, offer **Publish branch**. Default to `origin` and the same name as the local branch, allow changing the destination, and establish the upstream on successful publication.
- For example, publishing `feature/test-123` suggests `origin/feature/test-123`.
- If `origin` does not exist, require choosing an available destination rather than inventing one.
- Present operation progress and useful Git errors without blocking navigation unnecessarily.
- Leave interactive rebase, conflict resolution, history editing, and force-push outside version one.

### Worktree cleanup

- Include worktree removal in the UI.
- Confirm removal of a clean linked worktree, keeping its branch by default.
- Refuse removal when there are uncommitted changes.
- Offer local branch deletion separately, preserving Git's normal unmerged-branch protections.
- Treat the original checkout as the project's root checkout, not as a removable linked worktree.

### Terminal and tool launching

- Keep Tree Rail running after launching another tool.
- Launch a new terminal instance with the selected worktree as its working directory.
- The default launch profile starts interactive Copilot in that existing worktree; it does not ask Copilot to create another worktree.
- Provide terminal-only and editor profiles, plus user-defined profiles and an optional combined layout.
- Make terminal executable, arguments, tool command, and profiles configurable, globally and per project.
- Express launch profiles as a program, an argument list, a working-directory template, and an optional environment map. Substitute placeholders such as `{worktree}` and `{branch}` as argument values; invoke a shell only when the profile explicitly selects one.
- Use Windows Terminal by default on Windows. Provide native platform adapters and configurable terminal choices on macOS and Linux.
- Hand off interactive tools to the new terminal and return control to the explorer immediately. Interpret a successful terminal launch separately from the eventual Copilot/editor exit status.
- Report unavailable tools and allow choosing another profile, such as terminal-only.

### Configuration

- Begin with a readable **`config.toml`** rather than a settings screen. TOML is a product choice for comments, tables, argument arrays, and literal Windows path strings; Ratatui does not require a particular configuration format.
- Store saved sources, discovery options, configured per-project starting points and worktree directories, terminal choices, launch profiles, theme options, and keybindings in that file.
- Store remembered selections and last-used starting points separately in application-owned **`state.json`**, serialized with Serde. JSON also remains appropriate for structured GitHub CLI output.
- Resolve configuration and state directories using operating-system conventions. Where a distinct state directory is unavailable, use the application's local data directory. Offer an explicit `--config` path override.
- Give explicit CLI options precedence over configured values, and per-project settings precedence over global defaults. Project-source arguments still extend saved sources as described above.
- Allow temporary CLI arguments without automatically rewriting saved settings.
- Preserve configuration comments when saving explicitly edited settings; use comment-preserving TOML editing and replace saved files atomically. Report malformed configuration with its location and useful context.

## Operational defaults

These details make the approved requirements concrete. The configuration example is illustrative; the full documented example is in [examples/config.toml](../examples/config.toml).

### CLI examples

```text
treerail "C:\Code\my-project"
treerail "C:\Code" "D:\OtherProjects"
treerail --paths-file projects.txt
treerail --recursive "C:\Code"
treerail --config "C:\Configs\treerail.toml" "C:\Code"
treerail
```

The paths file contains one directory per line. A launch without arguments restores saved sources; when there are none, it uses the current directory as a repository or parent directory. Explicit sources are temporary unless saved intentionally.

### Vim controls

| Context | Keys | Behavior |
| --- | --- | --- |
| Lists and trees | `j` / `k`, arrows | Move selection |
| Pane focus | `h` / `l`, left/right arrows | Previous/next pane |
| Navigation | `gg` / `G` | First/last item |
| Lists and diff | `Ctrl+u` / `Ctrl+d` | Scroll half a page |
| Pane focus | `Ctrl+w`, then `h/j/k/l` | Focus a pane in that direction |
| Pane focus | `Tab` / `Shift+Tab` | Next/previous pane |
| Search | `/`, then `n` / `N` | Search and move between matches |
| Files | `Space` | Toggle file selection |
| Navigation | `Enter` | Open the selected project/worktree or focus its details |
| Worktrees | `a` | Create a worktree |
| Worktrees | `b` | Open branch selection and branch actions |
| Worktrees | `o` / `O` | Launch default profile / choose profile |
| Worktrees | `d` | Open the worktree removal confirmation |
| Files | `s` / `u` | Stage/unstage selected files, or the current file if none are marked |
| Worktree actions | `c`, `f`, `p`, `P` | Commit, fetch, push/publish, pull |
| Worktree actions | `r` | Open PR selection |
| Navigation | `R` | Refresh explicitly |
| Navigation | `Esc` | Close the current dialog or return to the previous view |
| Navigation | `q` | Back; quit from the project list |
| Any navigation view | `?` | Show contextual shortcuts |

Commands apply only in the appropriate navigation context. Text fields receive ordinary typed characters, including `j`, `k`, and `q`. Multi-key sequences must not interfere with typing branch names, searches, or commit messages. Keybindings remain configurable.

Crossterm key events must distinguish presses, repeats, and releases. Execute one-shot commands such as commit, creation, deletion, and launching only once per key activation; allow deliberate repetition for navigation and text editing. Validate this on Windows as well as Unix terminals. Support multiline paste in commit forms when the terminal reports bracketed paste.

### Configuration shape

An illustrative editable configuration:

```toml
project_roots = ['C:\Code', 'D:\OtherProjects']
recursive = false
default_profile = 'copilot'

[worktrees]
path_template = '{project_parent}/{project_name}.worktrees/{branch_slug}'

[profiles.copilot]
program = 'copilot'
args = []
cwd = '{worktree}'
terminal = 'auto'

[profiles.editor]
program = 'nvim'
args = ['.']
cwd = '{worktree}'
terminal = 'auto'
```

The terminal adapter opens a new terminal instance for a profile. `terminal = 'auto'` selects Windows Terminal on Windows and a detected/configured terminal on other platforms. Form paths using platform path APIs; the template is a logical path, not a shell command. The built-in terminal-only profile opens the user's shell without a tool command.

The exact configuration schema must be documented and validated during implementation. User-defined terminal adapters must support an executable and arguments in addition to built-in choices. Theme and keybinding overrides remain optional, with documented application defaults.

### Other defaults

- Disambiguate worktrees with both branch and path; never silently reuse a different directory because two branch names flatten to the same folder name.
- Show a proposed destination and require choosing another name when a worktree path is occupied.
- For PRs, suggest a local branch such as `review/pr-123` and its corresponding flattened worktree folder.
- If a repository lacks a remote default branch, show the current checked-out branch/commit as an explicit initial base suggestion.
- Treat counts in worktree summaries as counts of changed files; distinguish these from added/removed line counts in diffs.

## First implementation validation

- Discover direct repositories and parent folders, combine sources without duplicates, and exclude `.worktrees` directories while still listing registered worktrees.
- Create a worktree from a non-HEAD base such as `origin/develop`, and select an existing worktree when its branch is chosen again.
- Show and refresh staged, unstaged, and untracked changes; stage/unstage files and commit with a message.
- Publish a branch to the same name on a remote and establish its upstream; refuse a diverged fast-forward-only pull.
- Prepare a PR review worktree and keep its branch comparison separate from uncommitted changes.
- Validate default and custom launch profiles with spaces in paths, and keep Tree Rail open while Copilot runs elsewhere.
- Verify Vim navigation, text entry, pane focus, resize behavior, and live refresh in Windows Terminal.
- Use Ratatui's `TestBackend` for representative layouts and terminal sizes, alongside action/state tests for context-sensitive Vim keys, forms, and background-result handling. Rendering assertions alone do not verify interaction behavior.
- Use temporary real Git repositories for integration tests of discovery, explicit worktree bases, index changes, commit messages, publication, and cleanup. Test launch argument construction independently of opening a real terminal.
- Exercise terminal restoration on exit and error, Windows key press/repeat/release handling, multiline paste, and custom themes in real terminals.
- Validate TOML parsing, precedence, platform directory fallbacks, comment-preserving explicit saves, and separation of configuration from generated state.
- Confirm safe cleanup of a clean linked worktree and refusal to remove one with uncommitted changes.
- Build release binaries with Cargo and run the packaged Windows executable without a Rust toolchain on its runtime path. Validate macOS/Linux release artifacts on their respective platforms, including documented native dependencies.
- Measure startup, navigation during Git operations, refresh cost across many worktrees, and memory use with large diffs. Use these application measurements to guide optimization.
