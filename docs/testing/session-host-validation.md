# Native session host validation

Validated on Windows on 17 October 2026, with GitHub Copilot CLI 1.0.94.

## Production native smoke

The production `treerail-session-host` binary handled `StartCopilot` in a disposable Git worktree using its installed user-level observer, a native ConPTY and the private framed named-pipe protocol. Copilot received an interactive starting brief asking for `treerail-host-smoke` without tools and produced that response in its actual native terminal.

- Recorded provider UUID: `79e56460-2502-41eb-9f19-94de78989ee6`.
- Host session: `session-19655438-acf9-4ebe-b86a-cd807c26ecda`.
- Managed launcher process: `18088`.
- Snapshot after native completion: `Ready`, healthy observation, no outstanding requests, exact provider UUID retained.
- Explicit client detach followed by a fresh snapshot retained process `18088`, the current native screen, and the same healthy Ready observation.
- Explicit Stop and Stop host both returned Done. The host and managed launcher exited; the verified clean disposable worktree was removed. The unchanged observer installed by this smoke was removed so subsequent source revisions do not conflict with that test-created copy.

This smoke proves production user-level observer discovery and end-to-end private event transport. The separate native compatibility proof covers actual human-input questions/resolution, cancellation, exact resume and ordinary terminal-window closure. This production smoke did not repeat model calls for each of those provider-level scenarios.

## Repeatable automated coverage

`cargo test --test host` exercises the public production command interface with actual host, UI fixture and PTY child processes, independent of a model subscription:

- A deliberately lost Start reply reconciles to the original process. A retry after Stop returns Ended.
- Aliases of an owned checkout cannot start another managed agent.
- Detach retains the process and screen; explicit takeover revokes stale input tokens.
- A real UI process killed while attached relinquishes input authority without stopping its agent.
- A detached agent writes 12,000 lines and a completion marker with no controlling UI; the snapshot remains a bounded current screen.
- A partial local-client frame is isolated from other bounded connections and expires on a deadline; the same host instance remains authoritative.
- Twelve concurrent clients complete 600 status/snapshot requests while another connection deliberately withholds its frame.
- Named execution groups provide a queryable barrier, including a deterministic name derived from the host and Start UUIDs so it can be recorded before launch.
- Windows clients verify the server process belongs to their account before sending any data; pipe access grants the actual user SID.
- Simultaneous clients coordinate one configuration-scoped host. An incompatible Stop host request is refused without replacing it.
- Explicit Stop terminates a managed process and its already-running descendant before releasing the checkout lease.
- Abrupt host termination closes its kill-on-close Windows Job, terminates descendants and releases the kernel-backed lease.

`node scripts/copilot_bridge_smoke.mjs` separately tests native pipe framing, fragmented replies, exact conversation correlation, ordered controlled SDK observations and workspace-context events. These are fixtures rather than live Copilot claims.

Strict compilation/lint validation: `cargo clippy --all-targets -- -D warnings`.

## Scope

The native execution-group implementation is Windows-first. Unix execution intentionally returns an unsupported validation error; portable types or a Unix socket implementation are not evidence of an equivalent native lifecycle guarantee. A host crash ends its live terminals; durable workflow reconciliation and explicit conversation resume are handled above this runtime, without automatically replaying instructions.
