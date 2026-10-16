# Tree Rail usage guide

A keyboard-first project explorer and Git worktree manager, built in Rust with Ratatui. Jump into a project, create a worktree from the right branch, and launch Copilot or your editor in a new terminal while the explorer stays open.

## Run

Git must be on `PATH`. The default agent profile also needs `copilot`; PR features need an authenticated GitHub CLI (`gh`). Local Git features work independently of those optional tools.

From this directory:

```powershell
cargo run --release -- 'C:\Code'
cargo run --release -- 'C:\Code\my-project' 'D:\OtherProjects'
```

Or build once and launch the executable:

```powershell
cargo build --release --locked
.\target\release\treerail.exe 'C:\Code'
```

On Linux/macOS, the executable is `target/release/treerail`. Building needs a stable Rust toolchain (tested with Rust 1.95.0). Windows builds use the MSVC toolchain and Visual C++ build tools. End users do not need Rust installed.

```text
treerail [OPTIONS] [PATHS]...

--paths-file projects.txt    One source folder per line; # comments allowed
--recursive                 Search beyond immediate children
--temporary                 Do not remember explicitly supplied sources
--config config.toml        Choose the combined settings and remembered-state file
--list                      Print discovered projects as JSON without a TUI
--help                      Show all options
```

Paths in a paths file are relative to that file. Saved roots are relative to their configuration file; CLI directory arguments are relative to the current directory. Explicit paths and path-list entries are remembered automatically after interactive startup. Use `--temporary` for a one-off set of additional sources. Without any sources, Tree Rail uses the current directory for that session and does not remember it. Saved and temporary sources are combined, normalized to absolute paths, and deduplicated. `--list` never writes sources or creates configuration files.

Parent directories are scanned one level deep by default. Folders containing `.worktrees` in their name are excluded from discovery. Registered linked worktrees still appear under their owning project, including those made by other tools.

## Everyday workflow

1. Select a project with `j/k` and `Enter`.
2. Press `a` for a new branch/worktree, or `b` to select a local or remote branch. The default directory is a sibling such as `my-project.worktrees/feature-login`.
3. Set the starting point, such as `origin/develop`. `Ctrl+b` opens its picker. Check the destination, optionally choose a fetch, and press `Ctrl+s` to create.
4. Press `o` to open a new terminal running Copilot in the selected worktree. `O` selects another profile, including plain terminal and Neovim.
5. Inspect live file changes, mark files with `Space`, and stage/unstage using `s/u`. Press `t` to switch staged/unstaged diffs.
6. Press `c` to enter a commit subject/body; `Ctrl+s` commits staged changes. Hooks and signing use your normal Git configuration.
7. Press `p` to push. With no upstream, the publish form suggests `origin` and the current branch name and establishes its upstream. `P` pulls using fast-forward only.
8. Once finished, focus the worktree list and press `d`. Removal keeps the branch and refuses dirty, locked, missing, or original worktrees. Branch deletion is a separate action in `b`, with Git's normal merge protections.

A branch already checked out in a worktree selects that worktree. New feature branches deliberately do not inherit the base branch as their upstream: they publish under their own name.

Every Git action keeps a live status above the explorer: its action, original target, current phase and elapsed time. Transfer percentages and rates appear when Git reports them; a spinner continues during silent commands. These percentages describe the reported transfer phase, rather than an invented overall total.

Press `i` to inspect streamed Git and hook output while an operation runs, or its result afterward. Details follow new output by default; `j/k` and `Ctrl+u/d` scroll, `g`/`gg` goes to the beginning, and `G` resumes following the end. `Esc`, `q` or `i` closes details while Git continues. Output retains the latest 64 KiB and shows a notice when older output was discarded. Navigation, searching and refresh notices keep the operation attached to its original target. Only one Git operation runs at a time; cancellation is outside this version.

Press `H` for recent Git operations from this session. The list shows each action, original project/worktree and outcome; use `j/k`, `gg/G` or `/` to find a result, then `Enter` for its details. An older result stays selected while a newer operation's live status remains visible above it. `H` returns to history from details. Up to 20 operations are retained, including the current one; an inspected result is kept while the oldest other results are discarded. The list reports how many were discarded. History and output are never saved to `config.toml` and start empty next launch.

## Keyboard controls

| Keys | Action |
| --- | --- |
| `j/k`, arrows | Move selection; scroll in the diff pane |
| `gg`, `G` | First/last item |
| `Ctrl+u/d` | Move half a page, adapting to terminal height |
| `h/l`, `Tab`, `Shift+Tab` | Focus adjacent panes |
| `Ctrl+w`, then `h/j/k/l` | Focus adjacent panes using a Vim-style prefix |
| `/` | Filter the current list or search the diff |
| `n/N` | Next/previous filtered result or diff match |
| `Enter` | Explore a project or focus the next pane |
| `a`, `b` | Create worktree; open branch picker |
| `o`, `O` | Launch default profile; choose profile |
| `Space`, `s/u` | Mark files; stage/unstage files |
| `t` | Toggle staged/unstaged diff |
| `c`, `f`, `p`, `P` | Commit, fetch, push/publish, fast-forward pull |
| `i` | Inspect live operation output or the latest result |
| `H` | Open this session's recent Git operation history |
| `r`, `v`, `w` | PR picker, PR comparison, open PR in browser |
| `d` | Confirm worktree removal, while focused on worktrees |
| `,`, `S` | Open settings (also available with an empty explorer) |
| `R` | Refresh |
| `Esc`, `q` | Close/back; quit from projects |
| `?` | Keyboard help |

Forms accept ordinary text: `j`, `k`, and `q` never act as navigation while typing. Use `Tab`/`Shift+Tab` to change fields, arrows/Home/End to edit, and `Ctrl+s` to submit. Commit bodies support newlines and bracketed paste. `Enter` advances a single-line field and submits the last one.

Pickers support `j/k`, `gg/G`, arrows, and `Ctrl+n/p`. Press `/` to filter, type the query, and `Enter` to select. `Esc` leaves filter entry, then closes the picker on another press. In navigation mode, `a` creates a branch in the branch picker, and `d` offers safe local branch deletion. In the PR picker, `a` lets you enter a PR number or URL directly.

## Pull requests

`r` lists up to 100 open PRs for the selected project. Select one or enter its number/URL. Tree Rail fetches the GitHub pull ref, including fork PRs, into its own local refs and prepares `review/pr-123` in `my-project.worktrees/review-pr-123`. It never force-resets an existing review branch.

`v` shows the review branch's merge-base diff against the fetched PR base; `t` returns to local staged/unstaged changes. `w` opens the PR in a browser. Existing review worktrees are selected as-is; updating them is an explicit Git operation. Posting reviews is outside this version.

## Configuration

Interactive startup automatically creates one combined `config.toml`. On Windows it lives at `%LOCALAPPDATA%\treerails\config.toml`; on Linux and macOS it lives at `~/.config/treerails/config.toml`. The Unix location uses your home directory even when `XDG_CONFIG_HOME` is set. `?` displays the active path. Use `--config` to choose another file, including that file's remembered selections and starting points. You can also start from [examples/config.toml](../examples/config.toml).

Settings and remembered selections share this document. Session memory is stored under `[state]` and `[state.bases]`. Automatic memory saves preserve comments and unrelated settings, including changes saved by other instances or your editor. The most recently saved selection wins; starting points changed in separate projects are preserved. Writes are atomic and Tree Rail instances coordinate through a small `.lock` sidecar containing no settings or history. `--list` only reads files: it never creates a configuration, imports data to disk, or saves selections. Invalid settings produce an error without replacing the document.

When the new **default** file is missing, Tree Rail imports the former configuration and `state.json` before creating it. Relative saved paths retain their original meaning. The old files remain untouched. Windows legacy settings used `%APPDATA%\treerail\config\config.toml` and memory used `%LOCALAPPDATA%\treerail\data\state.json`; Linux used `$XDG_CONFIG_HOME/treerail/config.toml` (or `~/.config/treerail/config.toml`) and `$XDG_STATE_HOME/treerail/state.json` (or `~/.local/state/treerail/state.json`); macOS used `~/Library/Application Support/treerail/`. An existing new file is authoritative and never reimports old values. If import fails, the original data is preserved and the error names the failed component. Correct it before retrying, or choose a separate file with `--config`. Explicit overrides do not import the default legacy files. The old `--state` option has been removed because settings and memory now belong to the same file.

### Settings in the TUI

Press `,` from project or worktree navigation to open settings. The page shows the active configuration path and three sections: **Sources**, **Worktrees**, and **Profiles**. Switch with `1/2/3`, `h/l`, or `Tab`; use `j/k`, arrows, `gg/G`, or `Ctrl+u/d` to navigate within a section. In Sources, `a` adds a source, `Enter` edits one, `d` removes one from the draft, and `r` toggles recursive discovery. Relative source paths resolve beside the configuration file. New sources must be available directories. Existing sources that become unavailable remain saved and receive a warning from the background scan.

In Worktrees, edit global path/default-profile choices or the selected project's starting point, path and profile overrides. Configured projects are also listed, and `a` adds defaults for another directory. Blank project overrides inherit the existing precedence: an explicit project base wins over its remembered starting point and Git's default, while project paths/profiles override global choices.

In Profiles, `a` creates a named profile and `Enter` edits an existing one. Fields cover program, arguments, working-directory template, environment and terminal adapter. Use `Tab`/`Shift+Tab` between fields; the editor scrolls to keep the active field visible. `Ctrl+u` clears the text before the cursor. Arguments use a TOML string array such as `['a file.txt', '--flag']`; environment uses a TOML map such as `{ MODE = 'dev', BRANCH = '{branch}' }`. Ordinary letters and digits always enter text while editing. A blank program opens a plain terminal. Choose `auto` or the name of an existing custom terminal adapter. Keybindings, colors and adapter definitions remain editable in the file.

`Ctrl+s` validates, saves and applies the complete draft. Invalid arrays, environment maps, profile references or adapter choices remain editable and leave active settings untouched. Worktree/profile changes affect the next creation form or launch immediately; only source/discovery changes trigger a project rescan, retaining valid project/worktree selections. Sources can be repositories or parent directories. Removing a source only changes discovery; it never deletes directories, repositories or worktrees. `Esc` returns to the explorer while retaining your draft, including unfinished input. Reopen settings to continue. `D` asks before discarding a draft, and quitting the application also asks before dropping unsaved settings.

If another instance or your editor changes an unrelated setting, Save & Apply keeps that change. Project overrides merge by project and individual field, so edits to another project are preserved. If the same field changed in both places, settings presents the conflict without writing anything: `m` applies your changes to conflicting fields, `e` keeps their external values while saving your nonconflicting edits, and `Esc` leaves the draft editable. If a project/profile was removed externally, keeping your draft restores its full settings. Comments, advanced settings and remembered state are preserved. `S` remains an alias for opening settings; it no longer saves temporary sources implicitly.

```toml
project_roots = ['C:\Code', 'D:\Projects']
default_profile = 'copilot'

[profiles.editor]
program = 'nvim'
args = ['.']
cwd = '{worktree}'
terminal = 'auto'

[[projects]]
path = 'C:\Code\my-project'
base = 'origin/develop'
default_profile = 'editor'
```

Built-in `copilot`, `editor`, and `terminal` profiles are always available unless overridden. A profile describes a tool, argument array, working directory, optional environment map, and terminal adapter. Relative profile working directories resolve inside the worktree.

The automatic adapter uses Windows Terminal in a new window on Windows, Terminal.app on macOS, and `$TERMINAL` or a detected terminal on Linux. Custom adapters can target another emulator or create a combined layout. Each `{command}` entry expands to the profile's program and its individual arguments; other arguments remain separate.

```toml
[terminals.my-terminal]
program = 'my-terminal'
args = ['--working-directory', '{cwd}', '--', '{command}']

[profiles.custom]
program = 'my-agent'
args = ['--project', '{worktree}']
cwd = '{worktree}'
terminal = 'my-terminal'
env = { WORK_BRANCH = '{branch}' }
```

`{worktree}` and `{branch}` refer to the selection; terminal adapters also accept `{cwd}` for the profile's resolved working directory. Select a shell explicitly as the profile program when you want shell syntax. Tool availability is checked before launch. A successful terminal spawn is reported separately from whatever the tool does afterward.

Worktree paths support `{project_parent}`, `{project_name}`, and `{branch_slug}`. Relative templates resolve against the project. Slashes in a branch name become dashes in its folder name; occupied destinations are refused instead of overwritten.

```toml
[worktrees]
path_template = '{project_parent}/{project_name}.worktrees/{branch_slug}'

[theme]
accent = '#7dd3fc'
added = '#86efac'
deleted = '#fda4af'
modified = '#fde68a'

[keys]
'alt+o' = 'profiles'
'ctrl+n' = 'down'
```

Key overrides apply in navigation contexts. Action names include `down`, `up`, `first`, `last`, `focus_left`, `focus_right`, `search`, `help`, `back`, `refresh`, `settings`, `save_roots` (settings alias), `create_worktree`, `branches`, `launch`, `profiles`, `stage`, `unstage`, `commit`, `fetch`, `pull`, `push`, `remove_worktree`, `prs`, `review_diff`, `open_pr`, and `toggle_staged`.

## Development and validation

```text
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked
```

Integration tests use disposable real Git repositories and local bare remotes; they do not push to your projects. They cover non-HEAD worktree bases, publication/upstream setup, staging, rename handling, commits, fast-forward divergence, discovery, cleanup, configuration, input contexts, and rendering at multiple sizes.

The CI workflow runs checks and builds native artifacts on Windows, Linux, and macOS. Local validation has been performed on Windows; live GitHub/Copilot authentication and other OS terminal emulators require testing in their respective environments. Tree Rail serializes mutations while allowing navigation, and reads status/diffs in background workers. File diffs display up to 20,000 lines.

Product scope: [docs/product-spec.md](product-spec.md). Framework research: [docs/research/tui-frameworks.md](research/tui-frameworks.md).
