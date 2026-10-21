# Separate configuration from remembered state

Keep user-declared settings in `config.toml` and remembered paths, project tabs, navigation, drafts and conversation associations in a sibling `state.json`. This reverses ADR 0001: ordinary use must not rewrite user configuration, while both documents remain in the same OS application directory and are loaded before the first terminal frame. Preserve a complete backup when migrating the former combined document; retain historical ticket records in that backup rather than importing a removed domain into the new state model.

ADR 0002's automatic remembering remains, but explicitly supplied paths now belong to state rather than becoming declared configuration. `--temporary` and read-only `--list` remain supported.
