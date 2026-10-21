# Run the real Copilot CLI as an AHP frontend

Replace Tree Rail's live chat renderer with the actual Copilot CLI in a child terminal: ConPTY on Windows and a PTY on Unix. Attach it to the same AHP session owned or observed by Tree Rail, so Copilot owns its interface, menus, input and approvals. This revisits ADR 0011 because the user prefers the original CLI experience to a separately maintained chat renderer.

Keep terminal processes in the Tree Rail process; switching views preserves them, closing a terminal disconnects its frontend, and quitting closes terminals before stopping the owned SDK host. Reserve Ctrl+] to leave terminal focus and forward other input to Copilot. Persist conversation identity and cached history, never terminal connections or automatic prompt replay.

Use the full bundled CLI by default, with a separate compatible frontend override. Its experimental AHP client requires a feature flag and explicit GitHub authentication sharing; permit loopback or TLS connections and pass connection secrets only through the child environment. AHP reducers continue to provide Tree Rail's activity and cached history; leases for externally initiated turns begin when observed and cannot atomically fence independent clients.
