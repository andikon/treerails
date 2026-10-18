# Share a ticket worktree across its tasks

A ticket's tasks and agent sessions share its worktree and branch, rather than creating a worktree for every task or conversation. This keeps implementation and follow-up review on the same changes and lets the ticket be delivered through one pull request, with review waiting for implementation when necessary. The trade-off is that task sessions share working files, so their execution must be coordinated instead of relying on separate checkouts for isolation.

Only one agent session may work in a ticket's checkout at a time; sessions for other tickets may work in parallel. Other conversations remain recorded and resumable, and another task must wait for the current session to finish or explicitly hand over the workspace. This governs agent sessions; it does not change the existing Git-operation serialization policy.

A session waiting for input retains checkout ownership. Handover must stop its work and preserve its conversation before another session takes ownership; a waiting prompt cannot remain able to restart edits in the previous session after handover.

Checkout ownership applies to the actual worktree across tickets, clients and configuration contexts. Associating an existing worktree must reject a conflicting active ticket association, and two managed sessions cannot acquire the same checkout through different ticket records. Unmanaged external tools are outside Tree Rail's ownership enforcement.
