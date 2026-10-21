# Store settings and remembered state in one configuration file

> Revisited by [ADR 0010](0010-separate-configuration-from-remembered-state.md). The original decision is retained below for history.

Tree Rail will persist both user configuration and remembered state in a single `config.toml`. The user explicitly chose one editable file over separate configuration and state files so saved project sources and session memory are kept together. These remain distinct concepts even though they share a file; automatic state updates must preserve user settings and comments.

This supersedes the separate `state.json` storage decision in the original [product specification](../product-spec.md#configuration). The trade-off is that automatic session-memory updates now touch the same document users edit, so save behavior must handle concurrent edits and multiple running instances deliberately.

The default combined file will be `~/.config/treerails/config.toml` on Unix, including macOS, and `%LOCALAPPDATA%\treerails\config.toml` on Windows. Create it automatically when needed for normal interactive use. If the new default file is absent, import existing Tree Rail configuration and remembered state once, leaving the old files untouched; if the new file already exists, use it without merging old settings. An explicit configuration-file override selects the combined file, including its remembered state.

Multiple Tree Rail instances and external editors remain supported. Saving must preserve unrelated changes made since a settings draft was opened and surface conflicting settings edits for the user to resolve. Automatic remembered-state saves update only that memory while preserving settings; the most recently saved selection wins. Operation output is retained only in bounded session history, not persisted as configuration or remembered state.

[ADR 0008](0008-retain-discussion-documents-outside-configuration.md) extends this decision for frozen issue-discussion document bodies: retain those as separate context artifacts outside worktrees, with references in the combined file. Settings, saved views, comment drafts and other remembered state retain the single-file behavior above.
