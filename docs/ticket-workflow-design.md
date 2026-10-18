# Ticket workflow design

Status: design synthesized into the [feature specification](specs/ticket-native-agent-orchestration.md), published as [GitHub issue #8](https://github.com/andikon/treerails/issues/8) with the ready-for-agent label after confirmation of the testing approach. Implementation and live compatibility validation have not started.

## Foundation

The default entry is project → ticket, alongside the existing direct project → worktree → launch workflow. Tickets are already-defined GitHub Issues; tasks are persistent Tree Rail workstreams attached to them, and each task may have multiple agent sessions. Copilot is the first integrated agent.

See [the glossary](../CONTEXT.md), [ADR 0003](adr/0003-default-to-a-github-ticket-centered-workflow.md), [ADR 0004](adr/0004-share-a-ticket-worktree-across-its-tasks.md), [ADR 0005](adr/0005-host-copilot-terminals-in-the-ticket-workflow.md), and [ADR 0006](adr/0006-keep-agent-sessions-in-a-persistent-local-host.md).

## Ticket and workspace behavior

- A ticket's tasks and agent sessions share its worktree and branch, allowing delivery through a pull request.
- The issue defines the shape of the work. Tree Rail imposes no fixed refinement → implementation → review sequence.
- An issue without an explicit task breakdown begins as one task. Acceptance criteria remain requirements rather than automatically becoming separate tasks; an explicit task list can be imported through an inspectable step without automatic refinement.
- Opening a ticket shows its overview without launching an agent. A prominent Continue action targets the current task's recorded session; starting a new session is a separate action.
- The first Start work offers worktree creation with a suggested branch, explicit base and directory, or association with an existing worktree. Subsequent tasks and sessions reuse that association.
- An actual checkout has at most one managed agent owner across tickets, clients and configuration contexts. Existing-worktree association must reject a conflicting active ticket association.

## Hosted sessions and handover

- Copilot's interactive terminal is hosted inside Tree Rail. The ticket workflow is an orchestrator with manual starts and handoffs; automatic cycle execution remains outside this scope.
- One agent session may work in a ticket's checkout at a time. Different tickets may have working sessions in parallel; other recorded conversations remain resumable.
- A session waiting for input retains checkout ownership. Explicit handover stops its work and preserves the conversation before another task's session takes over.
- A new session receives the issue title/body, selected task objective, worktree/base information, and an optional explicit handoff. The user can inspect the starting brief.
- Resuming uses the existing conversation's provider context. A new conversation does not silently receive the entire previous transcript.
- The first release manages conversations started through Tree Rail. Existing worktrees can be adopted with an explicit handoff brief for a new conversation; importing older conversations or controlling externally running ones is deferred.

## Session host and recovery

- A persistent local session host owns live agent terminals, attention observations and ticket-worktree ownership. UI clients can detach or close while agents continue; reopening reconnects to the live terminals while that host remains alive.
- The first version serves the current user on the local machine and has one controlling UI, with explicit takeover. Detachment releases client input control without releasing an agent's checkout ownership.
- Start the host automatically when the ticket workflow needs it. Explicit Stop session and Stop host actions control shutdown; a host with no live sessions may exit. No OS service installation or automatic login startup is included, and read-only listing remains read-only.
- Durable workflow references follow the existing single-TOML policy. Current terminal screens and bounded scrollback remain in host memory; reconnecting to a live terminal is distinct from resuming a conversation after its process ended.
- Persist task records, starting briefs, cached issue snapshots and session associations as workflow state under that same combined-document policy, preserving settings, comments and unrelated external edits. Provider transcripts remain in Copilot's storage.
- After host failure or reboot, preserve ticket/task/conversation references and show affected sessions as Interrupted. Reconcile remaining activity before explicit resume; do not automatically restart agents or resend their previous instructions.

## Attention, health and input modes

- Session attention has three labels: Running, Needs input, and Ready. Ready means the agent finished responding and is available for another instruction, without claiming that the task is complete.
- Session health is separate from attention. Show Interrupted after an interruption and Status unavailable if observation fails, rather than guessing an attention state from terminal output.
- Experimental Copilot event integration is acceptable if an early compatibility test proves it works. Support a tested version range and make observation failures visible. The terminal can remain accessible when status observation fails; handover must still verify that the previous agent has stopped.
- Enter focuses the selected hosted terminal in agent-input mode; a visible, configurable shortcut returns to navigation mode. Ordinary Copilot controls, including Escape and Ctrl+C, reach Copilot while agent-input mode is active.
- Navigation mode and agent-input mode use distinct colors and visible mode labels, similar to Vim mode indicators. Other tickets' attention indicators remain visible without stealing focus.

## Issue changes and progression

- Retain the starting brief used for each session. If the GitHub issue changes, flag Issue updated, show the changes, and let the user explicitly update tasks or send the revised requirements to Copilot. Preserve existing tasks, sessions and progress rather than silently replacing them.
- Ticket progression remains user-managed. Automatic task decomposition and cycle execution belong to future skills and are outside this feature's scope.
- When GitHub closes an issue, show Ticket closed in the ticket overview and against its tasks, retaining their previous progress rather than marking them all successful. Preserve task/session records and let the user explicitly delete the worktree. Use Stale for outdated information, not as a synonym for a closed issue.
- GitHub issue-access failures do not block reattachment to local sessions. Known tickets may use cached briefs, with visible last-sync information and an unavailable/stale warning. Copilot's own connectivity remains separate.

## Cleanup and notifications

- Worktree deletion requires explicitly stopping any owning agent session first, including one waiting for input. Tree Rail's basic deletion path must honor the same managed ownership.
- Preserve the existing refusal to remove dirty worktrees and protection of the original checkout. Retain the branch by default, show unpublished local commits, and keep ticket/task/session records after directory removal.
- Desktop notifications are an additional opt-in setting for Needs input and session failures. In-app attention indicators remain available regardless; ordinary responses, tool calls and transitions to Ready do not generate desktop notifications.

## Implementation-design follow-ups

These refine the agreed product behavior rather than add new workflow scope:

- Define inspectable task-list import syntax and stable task identity during explicit updates.
- Preserve existing configuration overrides and source behavior when selecting/finding a host, with checkout ownership enforced across configuration contexts.
- Specify the private local protocol, atomic ownership/start behavior, attachment snapshots, input takeover, bounded output, orderly stop and recovery reconciliation.
- Set configurable/default keybindings, colors, history bounds and notification delivery details.
- Prove native Windows detach/reattach, hosted Copilot behavior, and supported provider/platform versions before claiming release compatibility. See the [persistent session host assessment](research/ticket-session-host.md).

## Design acceptance scenarios

1. Open an already-defined GitHub issue, inspect its tasks and starting brief, create or associate a worktree, and start Copilot inside Tree Rail.
2. Switch between navigation and agent-input modes with visible label/color changes; Copilot keys reach Copilot only in its input mode.
3. Run sessions for two distinct ticket checkouts while rejecting a second managed owner of the same checkout, even through another ticket/configuration context.
4. Observe Running, Needs input and Ready without equating them to task completion; verify that a waiting session retains ownership until explicit handover.
5. Close the UI while Copilot works or waits; reopen and reattach to the same live terminal and its current screen without duplicating the process.
6. Interrupt the host, retain work references, report Interrupted, reconcile activity and explicitly resume the saved conversation rather than automatically restarting work.
7. Change the GitHub issue during work, inspect the update and explicitly apply/send revised requirements while retaining the original starting brief and session history.
8. Lose GitHub issue access and retain access to known local tickets/sessions, with visible cached-data timestamps and warnings.
9. Close a ticket through GitHub or its PR, see Ticket closed without fabricated task success, stop its owning session and optionally remove a clean linked worktree while retaining the branch and work records.
10. Enable desktop notifications as a setting and receive them for Needs input/failures, without routine Ready/tool-output notifications.
