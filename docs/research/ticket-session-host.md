# A persistent local session host for Tree Rail

Status: the persistent local host direction was accepted in [ADR 0006](../adr/0006-keep-agent-sessions-in-a-persistent-local-host.md); this assessment retains implementation proposals and unverified risks. Researched 17 October 2026 during the ticket-workflow design interview. No server, PTY prototype or live Copilot integration has been built or tested.

## Recommendation

The chosen direction is a small local server/client architecture to keep agents alive after closing the UI. The host owns the ticket workflow's live sessions, worktree ownership, provider observations and terminal state; TUI clients own navigation, display and input modes. This gives a useful foundation for future skills without implementing automatic workflow execution now.

Limit the first guarantee to surviving a TUI detach, client crash or ordinary client-window closure while the host and operating-system user session remain alive. Host crashes, sign-out and reboot require a separate recovery policy. Conversation resume after a host failure is different from reattaching to the same running terminal.

## Why a separate process changes the behavior

[tmux documents a background server](https://github.com/tmux/tmux/wiki/Getting-Started#the-tmux-server-and-clients) that owns programs and output, with disposable clients. Its [detach behavior](https://github.com/tmux/tmux/wiki/Getting-Started#attaching-and-detaching) preserves those programs. Tree Rail can use that separation without adopting tmux's general window/pane system or requiring tmux as a dependency.

The distinction matters on Windows: [closing a pseudoconsole terminates its attached programs](https://learn.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session#ending-the-pseudoconsole-session). [WezTerm's mux server](https://github.com/wezterm/wezterm/blob/main/wezterm-mux-server/src/main.rs) demonstrates independent server startup with `DETACHED_PROCESS`. However, [Windows Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects) can govern child termination, so merely detaching from the parent's console does not prove survival under every terminal or IDE process supervisor. The actual supported launch environments need lifecycle tests.

## Proposed responsibilities

| Persistent session host | TUI client |
| --- | --- |
| Authoritative managed ticket/task/session associations | Project/ticket navigation and selected views |
| Start, attach, stop and handover decisions | Explicit requests for those actions |
| Exclusive managed-session ownership of a ticket checkout | Display of ownership and handover prompts |
| PTYs, agent child processes and provider event observation | Keyboard/paste forwarding while agent-input mode is active |
| Continuously parsed terminal state, cursor/modes and bounded scrollback | Rendering the selected terminal and retaining local UI selections |
| Current Running / Needs input / Ready observation and its health | Attention badges and mode-specific colors plus text labels |
| Durable workflow reference saves using the accepted TOML policy | Editable settings drafts using existing conflict behavior |

In the ticket workflow, actions that create/remove its worktree or change session ownership must pass through the host. The current basic explorer does not need to be rewritten wholesale to host agents, but it must respect managed-worktree ownership when invoked against the same checkout. External tools remain outside Tree Rail's enforcement.

The input owner and checkout owner are separate: detaching a client relinquishes its keyboard connection, not the running agent's checkout. A session waiting for input still owns that checkout. A second UI must never create another agent simply because it cannot see the first client's terminal.

## Work already required by embedded terminal hosting

These are shared costs in either the in-process or server design:

- PTY/ConPTY process creation, resizing, input encoding and orderly shutdown.
- Terminal screen emulation, cursor/alternate-screen state, paste modes and bounded scrollback.
- Explicit Tree Rail navigation versus Copilot input focus, including mode colors and labels.
- Copilot session identity, kickoff/resume, attention-event mapping and handover.
- Association with tickets/tasks/worktrees and durable references.

These are not currently implemented. `src/launch.rs` opens an external terminal and discards its eventual result. `src/main.rs` owns the UI/event loop; there is no PTY owner or protocol. `src/git.rs`, `src/discover.rs` and the config helpers can already run independently of terminal rendering, providing reuse rather than requiring a full rewrite.

## Additional work for the persistent host

| Work package | Why it is necessary | Relative added work |
| --- | --- | --- |
| Host startup and discovery | Start once despite simultaneous clients; find the right host; handle stale endpoints and incompatible versions | Moderate |
| Private local transport | Framed commands/events, protocol handshake and user-scoped access; no network listener needed | Moderate |
| Independent terminal lifetime | Keep readers, parsers and provider observation running with zero clients; slow clients cannot block agents | Substantial |
| Attach and resynchronization | Restore screen, cursor, dimensions, attention and pending-input state, then ordered live updates without a gap | Substantial |
| Ownership and reconnect correctness | Explicit input takeover, atomic checkout ownership, duplicate-start prevention after lost replies | Moderate to substantial |
| Lifecycle and upgrade behavior | Distinguish detach, stop session and stop host; reconnect after client failure; never restart a busy host silently for an upgrade | Substantial |
| Failure-focused validation | Heavy detached output, prompt waits, reconnection/resize, client death, host death and mismatched versions | Substantial |

Sizing is engineering judgment, not a measurement. A demonstration is plausibly a few focused engineering days. A dependable Windows-first local server/client layer should be budgeted as **roughly 2–4 additional engineer-weeks beyond embedded terminal hosting**, with low confidence until a lifecycle/compatibility spike. This is not an estimate for the whole ticket workflow or a promise about agent-assisted elapsed time. Remote access, simultaneous interactive clients, durable terminal replay and full cross-platform validation would broaden it.

The terminal and Copilot integration remain substantial work even without a daemon. Choosing a host early avoids making the UI authoritative for live sessions and later extracting that same responsibility.

## Keep the initial host narrow

The local-only host, single controlling UI and detach/reattach direction are accepted. The following are proposed operating details where not already fixed by ADR 0006:

- Local machine and current user only; no remote transport, sharing accounts or OS service installation.
- One interactive controlling UI initially; explicit takeover rather than competing keyboards and resize policies. Noninteractive status requests can coexist.
- Reconnect to the existing live terminal. Preserve its last nonzero dimensions while detached and resize when a controlling client attaches.
- Auto-start the host on the first ticket-session action; a read-only `--list` must remain read-only. Host identity/configuration scoping still needs a decision.
- Closing the UI detaches. Stop session and Stop host are separate explicit actions. A host with no live sessions may shut down; no speculative scheduled restart of agents.
- Current screen and bounded history stay in host memory. A slow client can be required to resnapshot rather than retaining unbounded event queues.
- After host failure, surface interrupted/uncertain sessions and reconcile before offering provider conversation resume. Do not silently recreate processes from old metadata.

This excludes a general layout engine, scripting language, remote TLS/SSH server, multi-user sharing, background auto-approval and automatic refinement/implementation/review execution.

## Reattach requires real state

The host must continue draining terminal output and tracking screen state while detached. Microsoft's [ConPTY guidance](https://learn.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session) explicitly warns about blocked I/O and teardown deadlocks. Saving the agent PID and reconnecting only to future output cannot restore an already displayed permission dialog or full-screen application.

Proposed protocol behavior: attach returns an authoritative snapshot at a sequence boundary, followed by later updates. Snapshot capture and live subscription must be coordinated to avoid missing or duplicating changes. This is an architecture inference supported by [WezTerm's sequence-aware pane protocol](https://github.com/wezterm/wezterm/blob/main/wezterm-mux-server-impl/src/sessionhandler.rs), not a commitment to reuse that protocol.

Rust local-transport building blocks exist: [interprocess](https://docs.rs/interprocess/latest/interprocess/local_socket/index.html) uses Windows named pipes and Unix-domain sockets. It does not provide Tree Rail's message framing or application protocol. [Windows pipe security](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights) must be set deliberately; a default local pipe is not automatically restricted to only its owner.

## Attention remains a provider integration

Hosted terminal output/process liveness does not itself identify model activity or a real request for input. [Copilot SDK session events](https://docs.github.com/en/copilot/how-tos/copilot-sdk/features/streaming-events) document turn/idle events and paired permission or user-input requests. Some attention events are ephemeral, so observation must remain active in the host even with no client; losing observations needs an explicit unhealthy/stale indication rather than inventing one of the three attention states.

GitHub demonstrates [extensions joining native CLI sessions and subscribing to events](https://docs.github.com/en/copilot/tutorials/create-an-extension), but marks this extension mechanism experimental. Basic hooks provide lifecycle and attention notifications with less complete request-resolution information. A compatibility spike must establish reliable observation inside the actual hosted Copilot UI, including Windows input/resize, cancellation, disabled extensions/hooks and missed startup events. This uncertainty exists with or without a persistent host.

## Fit with accepted storage decisions

[ADR 0001](../adr/0001-store-settings-and-remembered-state-in-one-config-file.md) does not forbid a background host. A compatible first design keeps configuration and durable ticket/task/session references in the same comment-preserving TOML, keeps live terminals and bounded scrollback in host memory, and leaves provider conversation history in Copilot's own storage. The host would serialize its workflow updates while preserving unrelated UI/editor changes through existing locking/patching behavior.

A separate database for durable workflow records or disk-persisted terminal/event logs would revisit ADR 0001 and need a new decision. Neither is required merely to survive a UI detach. Git-operation history retains its existing semantics unless separately redesigned.

[ADR 0002](../adr/0002-remember-explicit-project-sources-by-default.md) still governs saved sources, temporary sources and read-only listing. A host must preserve those contracts; startup cannot use headless reconnection as a reason to rewrite source settings unexpectedly.

## Evidence needed before a dependable estimate

Use disposable worktrees and controlled child programs to prove:

1. Close the UI window during active output; reopen and reach the same process without duplicate launch.
2. Detach during a permission/question prompt; reconnect to the current screen and correct attention state.
3. Keep bounded memory and responsive output draining with no client or a stalled client.
4. Reattach at different dimensions, preserving terminal/input modes and color-coded Tree Rail focus.
5. Lose the reply to a successful Start; retry/reconcile without spawning another agent.
6. Reject duplicate checkout ownership and make input takeover explicit.
7. Distinguish client failure, host failure and process exit; do not claim task completion or rollback from them.
8. Reject incompatible protocol versions without killing ongoing sessions, and test stop/upgrade behavior.

The initial high-risk checks are native Windows lifecycle survival and the full hosted Copilot/event bridge. Passing a generic PTY echo example alone would not establish either.
