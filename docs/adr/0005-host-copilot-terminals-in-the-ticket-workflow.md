# Host Copilot terminals in the ticket workflow

[ADR 0009](0009-unify-ticket-and-local-work-through-task-workspaces.md) extends this hosting decision to all task workspaces, including local tasks entered through Worktrees. External tool profiles remain distinct from managed agents.

The ticket workflow will host Copilot's interactive terminal inside Tree Rail so the user can operate agents and see their status in the same application. This replaces the external-terminal handoff for integrated ticket sessions while preserving existing launch profiles in the basic workflow. Hosting makes Tree Rail responsible for terminal input, rendering and session lifecycle; reliable attention states also require Copilot events rather than inference from quiet terminal output.
