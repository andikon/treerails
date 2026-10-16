# Remember explicit project sources by default

Tree Rail will remember directories explicitly supplied on the command line as well as sources added through the TUI. The user chose automatic persistence so opening a new project source does not require a separate save action to make it available on later launches. Offer an explicit temporary-source option for one-off CLI sources.

This reverses the original [product specification's](../product-spec.md#project-sources-and-discovery) default of keeping CLI sources temporary. Settings-page edits remain a draft until validated and saved with Save & Apply; source/discovery changes then trigger a rescan while preserving valid selections. Unsaved drafts survive navigation and require a choice before they are discarded.

Noninteractive `--list` remains read-only, and a current-directory source inferred because no sources were supplied remains temporary. Temporarily unavailable saved sources remain configured with a warning instead of being removed automatically.
