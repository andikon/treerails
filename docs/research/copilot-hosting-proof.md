# Native Copilot hosting compatibility proof

Ticket: [#9](https://github.com/andikon/treerails/issues/9). Tested 17 October 2026.

**Gate: passed for the native Windows/Copilot 1.0.94 proof path described here.** The production host must still validate its own integrated launch/transport/UI path. These observations do not certify every terminal supervisor, Copilot version, operating system, or terminal escape sequence.

## What actually ran

The Windows executable installed through WinGet reported **GitHub Copilot CLI 1.0.94**. A separate detached Rust process owned a real ConPTY and continuously fed its output into `vt100` with 1,000 rows of bounded scrollback. Copilot ran its own native interactive UI in a disposable linked Git worktree. A small raw-mode view rendered the emulated current screen; it did not implement an SDK chat interface.

The observer used the SDK bundled into the native CLI, `joinSession({})`, with `--experimental`. It checked `session.sessionId` against the exact requested UUID before publishing any observation. It registered no permission handlers, tools, questions, or automated replies. A monotonically ordered stream carried attention event names and request IDs, with a separate heartbeat. Terminal quietness was never a status source.

| Check | Actual result |
| --- | --- |
| Native new conversation | `--session-id` accepted UUID `ead8e737-ac4f-48fd-83f4-c33b0ad5f5c2`; bridge confirmed that exact UUID. |
| Exact resume | After explicitly stopping its old process, `--resume ead8e737-ac4f-48fd-83f4-c33b0ad5f5c2` restored the same Alpha answer and transcript in a new process. No prompt was replayed. |
| Interactive kickoff | A second new conversation used `--interactive` and reached a real Alpha/Beta question without typing the kickoff into the terminal. |
| Running | Native `assistant.turn_start` produced Running. Final kickoff trace: sequence 3. |
| Needs input | Native `ask_user` produced **`elicitation.requested`**, not `user_input.requested`, with request ID `986543b2-3263-4ab9-ad06-377103af687d`. Sequence 7 produced Needs input and a real rendered Alpha/Beta prompt. |
| Resolution | Enter selected Alpha; the same request ID appeared in `elicitation.completed` at sequence 316; `session.idle` at 320 produced Ready. |
| Cancellation | Ctrl-C in the earlier resumed native question produced its correlated `elicitation.completed`, followed by `session.idle`; Ready was observed without declaring the task complete. |
| Keyboard and paste | Real terminal Enter and Ctrl-C worked; a bracketed paste supplied a new prompt. The native UI sometimes requires a separate Enter after pasted text. |
| Native resize and rendering | Reattaching the view resized the child from 100×30 to 90×24. The emulated native prompt, cursor, colors and current screen survived reconnect. |
| View closes during activity and input wait | The view was launched while Running, gracefully detached, force-closed, and recreated during the resulting question wait. The separate native Copilot PID **22616** and pending request were retained; subsequent views rendered the current question. |
| Controlled detached output | A separate fixture wrote **5,000 lines / 108,959 bytes** without an attached view, entered alternate screen and bracketed-paste modes, then accepted keyboard input after resize. Its PID remained unchanged across graceful and forced view closure. This is fixture evidence, not Copilot evidence. |
| Production bridge transport | The production observer sent ordered, exact-identity observations over an actual Windows named pipe with big-endian length-prefixed JSON. Fragmented replies passed. Its SDK events were controlled fixtures; this is not a second live-provider claim. |

The launch environment was the Codex desktop command runner on native Windows, not WSL. The launching process returned while the detached owner continued. Closing means terminating/detaching the embedded view process and its ConPTY, not killing its owner. Physical Windows Terminal, WezTerm, IDE supervisors, Windows sign-out and reboot were not tested. A parent Job Object can impose lifetime constraints; production uses its own process-lifecycle checks instead of assuming `DETACHED_PROCESS` overrides every supervisor.

## Reproduce without a provider or network

```powershell
cargo build --example copilot_probe
python scripts/copilot_probe_smoke.py
cargo test --lib copilot
node scripts/copilot_bridge_smoke.mjs
```

The Python check uses real OS processes and ConPTY on Windows. It checks detached output draining, current screen/alternate screen, resize, input, and graceful/forced closure of an actual embedded view. It never starts Copilot. The Node check uses an actual private transport but mocks the SDK session callbacks. Neither check consumes model credits.

## Reproduce the live check

This requires an already authenticated Copilot CLI and may consume plan credits. Do not add auto-approval flags. Use a disposable linked Git worktree and only the generated observer. Do not place the probe observer into a normal working repository or the user's global extension directory.

```powershell
git worktree add --detach target/copilot-sandbox HEAD
New-Item -ItemType Directory -Force target/copilot-sandbox/.github/extensions/treerail-probe
Copy-Item examples/probe/extension.mjs target/copilot-sandbox/.github/extensions/treerail-probe/extension.mjs
$env:TREERAIL_PROBE_BRIEF = 'Ask me Alpha or Beta using ask_user. Do not use any other tools. After I answer say only Done.'
.\target\debug\examples\copilot_probe.exe start "$PWD\target\copilot-endpoint.json" "$PWD\target\copilot-sandbox" > target/copilot-start.json
Remove-Item Env:TREERAIL_PROBE_BRIEF
.\target\debug\examples\copilot_probe.exe view "$PWD\target\copilot-endpoint.json"
```

Accept folder trust for this disposable worktree only. Use Ctrl-] to detach the view, reopen it while the question waits, and press Enter to choose Alpha. `snapshot ENDPOINT` exposes attention, health, bounded recent event metadata, native UUID, PID, current screen and cursor. `view-smoke ENDPOINT` runs a real embedded view, detaches it, starts another view and force-closes it, and verifies the original child is still alive. `resize ENDPOINT ROWS COLS` and `input ENDPOINT TEXT` are bounded probe controls. `stop ENDPOINT` explicitly terminates the owner and its hosted child; detaching does not.

Save the exact `session_id` from a snapshot. After stopping, run `start ENDPOINT WORKTREE EXACT_UUID` to resume it; never pass the brief again. The endpoint file contains an ephemeral capability secret: keep it private and do not commit it. The proof's loopback TCP capability transport is deliberately **not** production IPC. The production bridge uses the private host pipe and does not record terminal output or transcripts on disk.

On Windows, use regular-file output for a detached launcher rather than collecting its stdout/stderr using anonymous capture pipes. In this command-runner environment inherited capture handles kept a collecting parent waiting after the launcher exited. The reproducible fixture runner avoids that issue. The production host has its own startup/lifetime implementation.

## Integration contract and limitations

`src/copilot.rs` is the reusable production provider seam: exact UUID generation/validation, distinct New/Resume argument construction, a conservative attention reducer, and the bundled observer source. `install_observer` places the inert user-scoped observer outside ticket checkouts, reuses identical content and refuses to overwrite differing user files. Native project-level loading was exercised live; user-level discovery follows the provider's documented discovery contract and must also be checked in the integrated production smoke.

The host authenticates each observation, validates its recorded provider UUID and consecutive sequence number, and applies `Attention::observe`. It owns heartbeat deadlines, interruption and process health. Missing events, sequence gaps, missing bridges or stopped extensions must become unavailable health; they must not be repaired by guessing from output. The view must continue showing the real terminal during startup folder trust, which occurs before the extension connects. Initial attention remains unknown until a real event boundary is observed. A completed request changes Needs input to Running; only `session.idle` establishes Ready. Multiple outstanding requests are correlated by family and request ID. Subagent-only events are excluded from the main attention stream.

Extensions are experimental and can be disabled through Copilot's persisted extension mode even when `--experimental` is present. The startup heartbeat cannot recover an input request emitted before attachment. Permission/user-input/elicitation request families are supported by the reducer, but the live question was elicitation; a real shell/file permission prompt was not exercised. No hook file scraping, transcript parsing or SDK-owned replacement conversation is used. The proof does not establish Linux/macOS lifetime support, durable screen replay, remote control or general terminal emulator completeness.

## Primary sources

- [Native extension discovery and modes](https://docs.github.com/en/copilot/concepts/agents/copilot-cli/about-cli-extensions).
- [Creating native extensions with the bundled SDK](https://docs.github.com/en/copilot/tutorials/create-an-extension).
- [Native SDK event envelopes, ephemeral events, request correlation and idle boundary](https://docs.github.com/en/copilot/how-tos/copilot-sdk/features/streaming-events).
- [Windows pseudoconsole lifetime and I/O constraints](https://learn.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session).

These sources describe the integration contract. The tested version and empirical results above came from the installed executable and real processes, not from documentation alone.
