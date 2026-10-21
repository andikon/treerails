# Freeze discussion context for new agent conversations

> Superseded by [ADR 0011](0011-return-to-worktrees-with-native-ahp-conversations.md). The original decision is retained below for history.

New agent conversations receive the issue description and full visible discussion captured before starting-brief review, so decisions recorded in comments are included alongside the task, checkout and explicit handoff. Freeze the discussion snapshot the user approves: later edits or comments do not silently change that conversation's starting context, and continuing a recorded conversation does not replay it. Supply the complete captured discussion as a retained Markdown document with a short starting prompt, rather than silently truncating long threads to fit launch arguments; block unsupported delivery, and specify artifact storage separately in the ticket browser design.
