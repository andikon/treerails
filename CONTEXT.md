# Tree Rail

Tree Rail is a terminal explorer for projects and their Git worktrees. It keeps repository operations and workspace launch profiles close to the user's development workflow.

## Language

**Project**:
A Git repository whose original checkout and registered linked worktrees are managed together.
_Avoid_: Worktree when referring to the repository as a whole.

**Worktree**:
A checked-out workspace belonging to a project, including its original checkout or a linked worktree.
_Avoid_: Project when referring to one checked-out workspace.

**Ticket**:
A GitHub Issue defining a requested outcome and its acceptance criteria. A ticket can contain multiple tasks sharing its worktree.
_Avoid_: Task when referring to the issue as a whole.

**Task**:
A workstream tracked by Tree Rail, such as implementation, review or a local investigation. A task can belong to a GitHub ticket or local work and can have multiple agent sessions.
_Avoid_: Ticket or agent session when referring to the workstream.

**Local work**:
Tasks and their objectives associated with a project and worktree, without requiring a GitHub issue.
_Avoid_: Local ticket or synthetic issue.

**Workspace**:
The shared working view of a checkout, its tasks, changes and agent conversations, reached from Work, Tickets or Worktrees.
_Avoid_: Worktree when referring to the working interface rather than the physical checkout.

**Work view**:
A project's task overview, combining ticket-backed and local tasks with their attention, progress and checkout context.
_Avoid_: Tickets when referring to all ongoing work.

**Ticket filter**:
Criteria determining which GitHub tickets are shown for a project.
_Avoid_: Ticket status when referring to a rule that selects tickets.

**Saved ticket view**:
A named choice of ticket filtering and ordering belonging to a project. A project can have multiple saved ticket views.
_Avoid_: Planning board when referring to a saved selection of GitHub Issues in Tree Rail.

**Active section**:
A compact view of current local and ticket-backed work, including agent attention, available alongside project browsing.
_Avoid_: Filter match when an active ticket is visible independently of the saved view's criteria.

**Ticket preview**:
A view of a ticket's GitHub description and discussion alongside the ticket list.
_Avoid_: Task workspace when referring to reading an issue without working on its local tasks.

**Discussion snapshot**:
The ticket description and complete visible issue discussion captured at a particular time for a new agent conversation.
_Avoid_: Live thread when referring to the fixed content approved for a starting brief.

**Agent session**:
An agent conversation associated with a task.
_Avoid_: Session without qualification when it could mean the running Tree Rail application.

**Session attention**:
Whether an agent session is Running, Needs input, or Ready for another instruction.
_Avoid_: Task completion when describing the agent's current interaction state.

**Session health**:
Whether Tree Rail can reliably observe and control an agent session, including interruption or unavailable status observation.
_Avoid_: Session attention when describing a failure to observe or control the session.

**Starting brief**:
The task objective, checkout context and optional handoff captured for the start of an agent session, including the complete issue discussion when linked to a ticket.
_Avoid_: Conversation transcript when referring to the supplied starting context.

**Workflow state**:
Tree Rail's retained tasks, starting briefs, issue snapshots and agent-session associations for ticket work.
_Avoid_: Configuration for work records or remembered selections for the entire workflow state.

**Session host**:
Tree Rail's background owner of live agent sessions, their terminal state, and their ticket-worktree ownership, independent of an attached UI.
_Avoid_: Agent session when referring to the host itself.

**UI client**:
A Tree Rail interface connected to the session host to view and control ticket work.
_Avoid_: Session host when referring to the attached interface.

**Detach**:
Disconnect a UI client while hosted agent sessions continue running or waiting for input.
_Avoid_: Stop when only the interface is disconnected.

**Reattach**:
Reconnect a UI client to an existing live agent terminal in the session host.
_Avoid_: Resume when returning to a process that is still running.

**Resume**:
Continue a saved agent conversation after its previous live terminal has ended.
_Avoid_: Reattach when a new live terminal is needed to continue the conversation.

**Navigation mode**:
The input mode in which keyboard actions control Tree Rail navigation and workflow actions.
_Avoid_: Agent-input mode when Tree Rail receives the keys.

**Agent-input mode**:
The input mode in which keyboard actions control the selected hosted agent terminal, except for the configured shortcut back to navigation mode.
_Avoid_: Navigation mode when the agent receives the keys.

**Project source**:
A directory supplied for project discovery, either a repository itself or a parent directory containing repositories.
_Avoid_: Project when referring to a parent directory containing several repositories.

**Launch profile**:
A named choice of tool and launch options for opening a terminal in a selected worktree.
_Avoid_: Terminal when referring to the entire tool-launch choice.

**Configuration**:
Persistent user choices for project sources, discovery, worktree defaults, launch profiles, and presentation.
_Avoid_: State when referring to user-managed project sources or preferences.

**Remembered state**:
Session memory such as the last selected project, worktree, and starting point used for a project.
_Avoid_: Configuration when referring specifically to remembered selections.

**Git operation**:
A user-requested Git action on a project or worktree that can contain several steps and has one reported outcome.
_Avoid_: Background work when identifying a specific user-requested Git action.

**Operation details**:
The expanded view of a Git operation's streamed output and reported outcome.
_Avoid_: Notification when referring to the operation's inspectable output.

**Operation history**:
The bounded set of recent Git operations and their inspectable details retained for the current Tree Rail session.
_Avoid_: Remembered state when referring to operation output retained only within a running session.
