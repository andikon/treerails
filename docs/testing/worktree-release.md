# Worktree-first implementation validation

Local Windows validation for the 0.3.0 release, 2026-10-21.

Checks: 48 Rust tests and four Python packaging tests passed. One ignored Rust
function is a child-process fixture launched explicitly by the PTY tests. Formatting and
strict Clippy checks passed. The optimized Windows executable built successfully;
its version, help and read-only discovery passed. Listing with a nonexistent
configuration path created no directory. The local Windows archive passed the
release packager's extraction/version smoke test.

Local outputs: `target/release/treerail.exe`,
`dist/treerail-windows-x86_64.exe`, and `dist/treerail-windows-x86_64.zip`.

The `docs/media/copilot-cli.svg` capture shows the actual CLI attached to a
disposable linked worktree, rendered inside the production Tree Rail UI. The
Ratatui captures in `docs/media/worktree-*.svg` come from production
rendering with illustrative data. Sessions, Changes and session creation were
visually inspected. The fixed pane geometry and single status line are also
covered by product tests.

The Rust suite exercises disposable Git repositories, pure storage migrations,
TUI input and a deterministic loopback AHP server. It checks fresh remote-default
creation, explicit HEAD creation, publication/upstream, staging, removal guards,
checkout leases, streaming reducers, cancellation and resume without replay.
Storage checks include concurrent edits, backups, comments and read-only listing.
Native folder trust defaults to Cancel and requires an explicit affirmative action.
PTY integration tests cover a real child process, ANSI styles and wide characters,
terminal queries, keyboard/paste/mouse routing, resizing, view switching and
closing noisy hidden terminals. The AHP test also verifies checkout exclusion
after observing a turn started by a separate client, and release after cancellation.
Product checks cover direct session creation from the plus button and keyboard,
without a setup form. Git line-count checks cover net staged/unstaged edits,
untracked text, binary files, renames, deletion and repositories without a commit.

The live `ahp_probe` check uses SDK 1.0.22, its full CLI bootstrap (1.0.98),
an actual CLI frontend reporting 1.0.101, isolated
temporary Copilot settings and disposable Git repositories. It verified:

- AHP version negotiation, provider catalog and configuration schema.
- GitHub authentication using a connection-bound HPKE sealed envelope.
- Native folder trust through the production adapter, with no changes to the
  user's real Copilot trust settings.
- Idle conversation creation on a real linked worktree outside its main project,
  when the host launches in a containing workspace.
- Discovery of that exact conversation by VS Code 1.142.0's `agent ps` client.
- The real CLI attaching to that same session, rendering its interface, accepting
  typed input, resizing and closing. The probe types `/help` without submitting
  it, clears the editor and checks that the chat still contains zero turns.
- Explicit idle-session disposal and shutdown of the owned runtime.

No live model prompt was sent. The protocol test's streaming responses are
fixtures, not claimed as evidence of a real Copilot model turn.

Reproduce the idle terminal check and optional production UI capture with:

```powershell
cargo run --example ahp_probe -- 'C:\Path\To\code-tunnel.exe' --terminal --capture=dist/copilot-cli.svg
```

The CLI AHP client is experimental. The adapter enables its AHP_CLIENT feature
and explicit GitHub authentication sharing over loopback or TLS. Compatibility
with other CLI versions remains unverified. Native CLI menus are independent
of Tree Rail's worktree association, and advanced graphics/clipboard escape
extensions are not implemented. External AHP turn guards begin when observed;
they cannot atomically serialize independent clients against Tree Rail Git writes.

The native host restricts access to its launch workspace even after folder trust
is granted. The app derives that directory from startup sources and worktree
locations; `agent.host_directory` provides an explicit override. Adding an
outside project requires a restart. Separate Windows drives cannot share one
containing host directory. This is a current provider limitation.

Remaining manual checks are a real authenticated conversation, tool approval/denial,
model selection, restart/resume and interactive VS Code handoff. The CLI AHP
integration is experimental. Published downloads cover Windows only;
cross-platform builds still need their CI runners.
