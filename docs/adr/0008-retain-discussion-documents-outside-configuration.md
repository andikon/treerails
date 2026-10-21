# Retain discussion documents outside configuration

> Superseded by [ADR 0011](0011-return-to-worktrees-with-native-ahp-conversations.md). The original decision is retained below for history.

Retain each approved full-discussion snapshot as a separate immutable Markdown document in Tree Rail's data directory, outside managed worktrees, so long threads do not overwhelm the editable configuration and checkout cleanup does not destroy a conversation's captured context. Settings, saved ticket views, comment drafts and snapshot references remain in the combined configuration, while snapshot bodies are an explicit exception to ADR 0001's single-file choice. This trades a single self-contained state file for retained context artifacts whose identity, completeness and availability must be verified before agent delivery; missing artifacts must not be silently replaced by a newer thread.
