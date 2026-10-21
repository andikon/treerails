# Default to a GitHub ticket-centered workflow

> Superseded by [ADR 0011](0011-return-to-worktrees-with-native-ahp-conversations.md). The original decision is retained below for history.

The ticket-only task scope is superseded by [ADR 0009](0009-unify-ticket-and-local-work-through-task-workspaces.md): local tasks and ticket tasks now share the workspace workflow. Tickets remain GitHub issues, and Tickets remains the initial view for a new project.

Tree Rail's default workflow will begin with project → ticket, where a ticket is always a GitHub Issue and contains tasks with their agent sessions; Copilot is the first integrated agent. The GitHub Issue remains the ticket's source of truth, avoiding a separate local ticket system. This expands the [product specification's manual ticket integration scope](../product-spec.md#github-pull-requests) while retaining the existing project → worktree → launch workflow as a direct path.

Tickets are expected to be defined already, and their progression remains user-managed. Tasks are Tree Rail workstreams attached to an issue rather than GitHub checklist items or sub-issues. Automatic refinement → implementation → review execution is left to future skills and is outside this feature's scope.
