# Tree Rail

Tree Rail is a terminal explorer for projects and their Git worktrees. It keeps repository operations and workspace launch profiles close to the user's development workflow.

## Language

**Project**:
A Git repository whose original checkout and registered linked worktrees are managed together.
_Avoid_: Worktree when referring to the repository as a whole.

**Worktree**:
A checked-out workspace belonging to a project, including its original checkout or a linked worktree.
_Avoid_: Project when referring to one checked-out workspace.

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
