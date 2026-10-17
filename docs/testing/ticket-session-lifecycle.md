# Ticket conversation lifecycle and recovery validation

Implemented for #12 and #13. Validated on native Windows, 17 October 2026.

## Authoritative command surface

Ticket commands run inside the production session host through `HostCommand::Ticket`. Durable records stay in the existing combined TOML: ticket/task/provider UUID, immutable brief, selected conversation, checkout, branch, host generation, pending operation and execution barrier. Terminal screens and provider transcripts are not copied into it.

- `StartRequest` identifies one inspected new-session intent. Retry uses its original UUID and payload. A changed task, handoff, dimensions or approved brief is rejected. Legacy `Start` creates a fresh intent for direct callers.
- New-session and handover requests first verify actual project registration, common Git metadata, current branch and the physical ticket association claim. A replaced directory or external branch switch is rejected before an existing owner is stopped.
- Approved briefs are compared against the latest issue/task/worktree inside the persistence transaction. Handover checks before stopping the old owner and again before launching the new one.
- `Continue` reattaches without a second process, or verifies an ended group and explicitly resumes the exact saved provider UUID without replaying a brief. Missing provider history requires an explicit new conversation.
- `SelectSession` starts nothing. `Handover` must stop and verify the previous managed execution before starting another task in the shared checkout. Failed stops retain ownership.
- `Snapshot` marks records from another host instance Interrupted and can discover accepted pending starts through read-only `FindStart`. It never submits Start.
- `Reconcile` never launches work. It discovers an accepted operation or verifies that the original named Windows Job Object ended and the checkout lease is free. Explicit Continue follows separately. Missing execution identities remain blocked rather than trusting an old PID or free lock.

Before native Start/resume, the stable request and deterministic execution-group name are persisted. A lost native reply or final save cannot cause a second writer. Notification settings are applied inside the host before Start acknowledgement; setup failure preserves the live session with a warning, and reconnect retries setup.

## Checks

```text
cargo test --lib hosted --locked
cargo test --test hosted_recovery --locked
cargo test --test host --locked
cargo clippy --all-targets --locked -- -D warnings
```

The platform-independent coordinator acceptance tests exercise production command policy and combined-document persistence with controlled native runtime responses. They cover implementation, handover, review, exact resume, selection/history, ended terminals, failed stops, missing provider history, identity/context mismatch, stale reviewed briefs, lost successful replies, host generation changes, execution barriers, explicit recovery, changed retry payloads and notification failures.

`hosted_recovery` uses actual Windows hosts, ConPTY children and named Job Objects. Its child is cmd.exe, not Copilot. Another host cannot reconcile or stop an active owner. A real host kill leaves records Interrupted until explicit group reconciliation; a missing provider conversation cannot silently create a replacement. The replacement host has no sessions before or after failed explicit resume.

The underlying host process suite covers client crashes, stale takeover packets, lost replies, heavy output and slow clients, coherent screen/resize snapshots, protocol/version failures, partial frames, managed descendants and host-crash termination. Native tests are Windows-gated; pure protocol and coordinator tests remain portable. Polling returns authoritative sequence-bound snapshots instead of accumulating an unbounded client event queue.

## Limits

Live Copilot UI and event evidence is in [the compatibility proof](../research/copilot-hosting-proof.md). Production user-level observer loading and private-pipe delivery also ran against Copilot 1.0.94: an interactive prompt returned the expected native response and healthy Ready with the exact provider UUID.

Host failure cannot preserve the original terminal screen. Sign-out/reboot recovery means retained references, execution reconciliation and explicit provider resume, rather than durable terminal replay. Named-job barriers and native tests are Windows-specific; basic explorer support elsewhere does not establish native hosting support.

Exact resume supports the tested bounded Copilot session.start JSON and workspace identity metadata. Unknown formats, missing records, changed branches or unavailable contexts fail rather than choosing a globally latest session. Observation failure never completes a task or releases its checkout.
