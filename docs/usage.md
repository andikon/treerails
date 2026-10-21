# Using Tree Rail

Open a repository or a parent directory: `treerail /path/to/projects`. Known projects and remembered locations load before the terminal renders; discovery then refreshes them. `Ctrl+P` opens the project explorer, where **+ Remember a path** adds a repository or discovery folder to state. Opening a project creates or selects its top tab.

## Worktrees and conversations

Select a worktree on the left. Sessions shows its conversations in the middle and the selected conversation on the right. Changes replaces only the middle and right content with files and a diff. Project tabs and the one-row status line remain fixed. Use `v` or the header controls to switch views.

The Worktrees **+** creates a checkout without starting an agent. The Sessions **+**, or `n`, immediately starts a new idle conversation in the selected worktree and opens the real Copilot CLI. Enter your first prompt in Copilot itself; no Tree Rail setup form appears. First-use folder trust still requires explicit confirmation. Model and permission settings can be managed in Copilot.

Help → **New session with options / new worktree** retains the optional setup form. **Create in a new worktree** is initially unchecked. Toggle it to reveal branch, starting reference, directory, remote and Fetch. A blank starting reference means the remote's freshly fetched actual default branch, such as `origin/develop`. `HEAD` explicitly means the selected checkout's current commit; another branch/tag/commit overrides it. Disabling Fetch requires an explicit cached reference. Dirty files are never copied into a new checkout. Fetch failures report an error; there is no silent fallback to stale references.

Use the creation form's Provider options to inspect the host's advertised models, permission settings and agent mode. After starting, the conversation opens immediately. The native Copilot runtime loads its own repository instructions and configured customizations. Sign in with GitHub CLI (`gh auth login`) or provide `COPILOT_GITHUB_TOKEN`; an existing login in another Copilot application does not necessarily expose a credential to an AHP client.

The first prompt is optional: leave it blank to open an idle conversation directly in the real Copilot CLI. A supplied first prompt is sent once through AHP before the CLI attaches. `i` or clicking the right pane focuses the CLI; **Ctrl+] returns to Tree Rail navigation**. Copilot receives all other keys, including Esc, Tab, Ctrl+P, Ctrl+Q and Ctrl+C, and owns its menus, model selection, attachments and approval interface. Use its native commands and controls. Tree Rail does not reinterpret live messages or inject a second copy of your prompt.

The full bundled Copilot CLI runs in a child terminal attached to that conversation's AHP identity. `[agent].cli` optionally selects another compatible CLI executable; `[agent].runtime` selects the SDK backend separately. AHP attachment is an experimental Copilot feature; Tree Rail enables its client flag and explicit GitHub authentication sharing for loopback or TLS hosts. The private endpoint is passed in the child environment, never command arguments or state. Ordinary terminal colors, cursor movement, keyboard, mouse and bracketed paste are supported; graphics and clipboard escape extensions are not bridged.

Switching Tree Rail sessions or views preserves open terminals. The small × in the conversation header closes only that terminal frontend, without cancelling its server turn. Resume reopens it against the same session without replaying the first prompt. Use Help → Stop active turn for explicit cancellation outside the CLI. Copilot's own session chooser is independent of Tree Rail's association; use Tree Rail's session and worktree rails to change checkout context.

## Git

Each worktree row shows green `+added` and red `-removed` line totals for its current checkout against HEAD. Staged and unstaged edits are combined as a net diff, not added twice. Untracked text files contribute added lines; binary files remain in the changed-file count without invented line totals. Before the first commit, existing text files count as additions. All worktrees in the active project refresh in the background.

Use Help for Stage, Unstage, Commit, Fetch, Pull, Push and removal. Space marks files; operations use marked files or the selected file. `t` toggles staged/unstaged diff content. Commits use normal Git hooks and signing. Pull is fast-forward only. The first Push chooses the configured publication remote, otherwise origin, otherwise the sole remote, and sets upstream. If several remotes remain ambiguous, a chooser asks which to use.

An observed active turn, including an approval wait, blocks Tree Rail Git writes in that worktree. Read operations and Fetch remain available. OS checkout leases coordinate Tree Rail's own managed turns and writes across processes. CLI and other AHP client turns acquire a lease when observed; this cannot atomically prevent a separate client starting a turn during a Git operation. Tree Rail blocks its terminal input while a known conflicting turn or Git mutation is active. Different worktrees can run independently; external tools remain responsible for their own writes. Removal refuses original, dirty, locked, missing or busy checkouts and keeps the local branch.

## Saved work and process lifetime

The first conversation in an untrusted checkout asks **Trust folder for Copilot?**. Confirmation adds that exact folder to Copilot's native trusted-folder list; repository instructions, hooks and tools can then load. Cancel leaves the initial prompt unsent. This uses Copilot's own trust API and does not edit its settings by hand.

The native AHP host restricts filesystem access to its launch directory. Tree Rail chooses a shared containing workspace from the locations loaded at startup, including configured sibling worktree paths. Set `[agent].host_directory` explicitly for unusual layouts. New locations outside an already-running host's scope require restarting Tree Rail; existing turns are never stopped to widen access. A single host directory cannot span separate Windows drives; use a compatible external host for a broader filesystem scope.

Multiple conversations belong directly to a worktree. Ready conversations stay in Sessions; Archive hides one locally and Help can show archived conversations again. Missing worktrees keep readable saved history but cannot receive prompts. Resume explicitly attaches to the saved protocol identity without replaying the first message.

Closing a tab with running sessions asks before hiding it. Help → Agent activity reopens active projects. Quitting asks before stopping owned work, closes all child terminals and ends the owned runtime. Conversations and cached history remain saved. Copilot owns live input drafts; legacy Tree Rail drafts stay preserved but are never automatically injected into the CLI. Restart restores project context but starts no turn or terminal. An explicitly configured external host retains its own lifetime.

## Keyboard navigation

| Context | Keys |
| --- | --- |
| Projects / preferences / quit | Ctrl+P / Ctrl+, / Ctrl+Q |
| Project tabs | Ctrl+Tab next; Alt+W close selected |
| Panes | Tab / Shift+Tab; h / l |
| Lists | j / k; arrows |
| Sessions / Changes | v |
| Create worktree / session | a / n |
| Stage / unstage / commit | s / u / c |
| Fetch / pull / push | f / P / p |
| Remove worktree | d |
| Resume / archive | r / e |
| Refresh projects, Git and agents | R |
| Focus Copilot CLI / stop turn from navigation | i / Ctrl+C |
| Leave Copilot CLI focus | Ctrl+] |
| Scroll detail | Ctrl+u / Ctrl+d |
| Form fields / submit / cancel | Tab / Ctrl+Enter / Esc |
| Help | ? |

These are Tree Rail navigation keys. While the CLI is focused, only Ctrl+] is reserved by Tree Rail; every other key belongs to Copilot. Letters typed into a form are text. Mouse support covers project tabs, selections, creation buttons, forms, command choices and the embedded terminal. Very small terminals ask for at least 60 columns × 16 rows while keeping background work alive.

## Files

`config.toml` and `state.json` live together in the OS application directory. `--config /other/config.toml` uses that file and a sibling `state.json`. Configured sources are separate from paths remembered during ordinary use. State saves never patch configuration. Settings and paths load before terminal initialization; Git, discovery and AHP calls run asynchronously afterwards.

Migration of a combined document writes state before removing its old state table and preserves the entire original as `config.pre-worktree-refactor.toml`. Existing separate state is authoritative. Malformed files report an error instead of being overwritten. Historical ticket/task records remain recoverable from the backup; the new app has no ticket or task layer.
