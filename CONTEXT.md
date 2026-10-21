# Tree Rail

Tree Rail is a terminal application for projects, Git worktrees and agent conversations. The worktree is the place where changes and conversations belong.

## Language

**Project**:
A Git repository whose original checkout and linked worktrees are explored together.
_Avoid_: Worktree when referring to the whole repository.

**Worktree**:
A checked-out workspace belonging to a project, including the original checkout or a linked checkout.
_Avoid_: Task or ticket as a worktree's owner.

**Project tab**:
An open view of a project's selected worktree, conversation and changes.
_Avoid_: Session when referring to an application view.

**Conversation**:
An agent conversation associated directly with a worktree, with retained messages and an explicit identity for continuation.
_Avoid_: Task, ticket or terminal when referring to an agent conversation.

**Agent session**:
The provider execution context behind a conversation.
_Avoid_: Project tab or running Tree Rail process.

**Terminal view**:
An interactive agent interface opened for a conversation, with a lifetime separate from the agent session.
_Avoid_: Conversation when referring only to the visible terminal frontend.

**Turn**:
One user instruction and the agent's response, including tool execution and requests for input.
_Avoid_: Conversation when referring to one response cycle.

**Session attention**:
Whether an agent is Running, Needs input, Ready, Interrupted or Unavailable.
_Avoid_: Task completion when describing an agent's interaction state.

**Resume**:
Explicitly reconnect to a saved conversation without replaying its initial instruction.
_Avoid_: Restart when no new turn has been requested.

**Archive**:
Hide a conversation locally while keeping its history and identity available for restoration.
_Avoid_: Delete when the provider's conversation is retained.

**Project source**:
A repository directory or a parent folder used for project discovery.
_Avoid_: Project when referring to a parent containing several repositories.

**Configuration**:
User-declared discovery sources, worktree defaults, provider choices and preferences.
_Avoid_: Remembered state for settings deliberately managed by the user.

**Remembered state**:
Locations remembered during use, project tabs, navigation, drafts and conversation associations.
_Avoid_: Configuration for ordinary runtime memory.

**Git operation**:
A requested action on a project or worktree, possibly composed of multiple Git steps.
_Avoid_: Agent turn when referring to a Git command initiated through Tree Rail.
