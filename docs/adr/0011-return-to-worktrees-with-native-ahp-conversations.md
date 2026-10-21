# Return to worktrees with native AHP conversations

The live conversation rendering choice is superseded by [ADR 0012](0012-run-the-real-copilot-cli-as-an-ahp-frontend.md). The worktree hierarchy, AHP identity and temporary host lifetime remain in effect.

Make Project → Worktree → Conversation the sole application workflow. Remove the ticket, task, discussion, starting-brief and persistent terminal-host layers from ADRs 0003–0009. The user chose a focused worktree tool and a native chat rendered by Tree Rail over the cost and coupling of issue orchestration and terminal emulation.

Use Microsoft's Agent Host Protocol for conversation state, streaming, approvals and resume; Copilot is the first provider. The official Copilot Rust SDK owns a temporary local runtime and authenticated AHP listener. Its lifetime follows Tree Rail; reopening the application restores views and history without restarting turns. Saved conversations resume explicitly without replaying their original prompt. An explicitly configured external host retains its own lifetime.

Keep project tabs in one header, stable Worktrees + Sessions / Changes columns, and the conversation / diff on the right. A single segmented status line holds checkout context. Creation uses local + controls; secondary commands live in Help. Closing a tab hides active sessions after a warning; quitting stops owned runtime work. One active turn, including input waits, excludes Tree Rail Git writes in a physical checkout; different worktrees can run in parallel. External tools and agents controlled by other clients are observed through AHP but cannot be fenced by a local application alone.

Use the pinned full native CLI bootstrap so its bundled extension SDK is available.
Honor Copilot's native folder trust with an explicit confirmation before loading a
new checkout. Keep the AHP host within its launch workspace; derive that workspace
from startup locations and configured worktree paths, with an explicit provider
override. Adding an outside location requires a restart, without interrupting
active turns to broaden access. The live compatibility probe uses isolated
provider settings and sends no model turn.
