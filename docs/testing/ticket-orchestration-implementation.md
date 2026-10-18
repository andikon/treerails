# Ticket orchestration implementation and validation

Implemented on `codex/ticket-orchestration` for specification [#8](https://github.com/andikon/treerails/issues/8). Ticket implementations were developed in separate managed worktrees and integrated in dependency order. No GitHub issues were closed and no release or pull request was published.

| Ticket | Delivered behavior | Principal validation |
| --- | --- | --- |
| [#9](https://github.com/andikon/treerails/issues/9) | Native Copilot/ConPTY compatibility proof, exact conversation launch and event observer | Live Windows Copilot 1.0.94 plus reproducible bridge/terminal harnesses |
| [#10](https://github.com/andikon/treerails/issues/10) | Default GitHub-ticket navigation, persistent tasks, explicit imports and shared checkout preparation | Disposable real repositories, controlled GitHub responses, configuration and TUI tests |
| [#11](https://github.com/andikon/treerails/issues/11) | Detached private local host, native terminal pane, input modes and exclusive execution ownership | Real Windows host/client/PTY processes, concurrent clients, standalone executable test |
| [#12](https://github.com/andikon/treerails/issues/12) | Recorded conversations, exact Continue/resume, explicit selection and verified handover | Production coordinator policy, real Git branch/identity checks and native provider history evidence |
| [#13](https://github.com/andikon/treerails/issues/13) | Stable launch retries, Interrupted state, named execution barriers and explicit reconciliation | Lost replies, native host/client crashes, descendant termination, partial/slow clients, protocol failures |
| [#14](https://github.com/andikon/treerails/issues/14) | Inspectable requirements updates, explicit task/revision application and offline cache | Controlled GitHub failures/edits, stable identities and original briefs, public workflow/TUI behavior |
| [#15](https://github.com/andikon/treerails/issues/15) | Closed-ticket information and optional guarded cleanup retaining branches/history | Actual owning host refusal, verified stop, clean directory removal and original/dirty/locked protection |
| [#16](https://github.com/andikon/treerails/issues/16) | Background attention indicators, opt-in notification settings, detached delivery and diagnostics | Host event pipeline with controlled delivery, deduplication, settings/concurrent-edit and input-focus tests |

## Final checks

- `cargo test --all-targets --locked`: **130 tests passed** on native Windows.
- `cargo clippy --all-targets --locked -- -D warnings`: passed.
- Linux (`x86_64-unknown-linux-gnu`) and macOS (`aarch64-apple-darwin`) all-target strict Clippy cross-checks: passed. These are compilation/lint checks, not native execution claims.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- Four Python release-contract tests: passed.
- `node scripts/copilot_bridge_smoke.mjs`: passed against real Windows named-pipe framing with controlled SDK callbacks.
- Optimized Windows binaries built with `cargo build --release --locked --bins`.
- Windows archive packaged locally and its extracted executable's version/help smoke checks passed.

An independent source audit found that an externally changed branch could invalidate an inspected starting brief. The fix checks registered worktree identity, current branch, Git metadata and the physical ticket claim before new launch or handover. Tests reject branch drift and replaced/foreign checkouts before stopping an existing owner.

The standalone executable includes private host/managed-child roles. A test copies only `treerail.exe` into an isolated folder, starts a real hosted terminal child, controls it and stops it without a companion executable. Slow Git workflow work is dispatched outside the live session loop; a gated real Git hook test proves terminal snapshots and host commands remain responsive.

## Compatibility boundaries

Native hosted execution and Windows Job Object reconciliation are validated on Windows with Copilot **1.0.94**, using its experimental native-session extension API. Linux/macOS retain the basic explorer and ticket preparation; native execution ownership there returns an explicit unsupported error until implemented and validated.

Desktop delivery is optional and depends on OS notification availability/settings. Automated checks use a controlled delivery channel rather than showing real notifications. A delivery error does not change agent permissions, task progress or checkout ownership, and remains inspectable in the UI.

Live Copilot evidence, controlled-child evidence and supported process-lifetime guarantees are separated in [the compatibility proof](../research/copilot-hosting-proof.md), [host validation](session-host-validation.md), and [ticket lifecycle validation](ticket-session-lifecycle.md). Closing the UI preserves live work; host failure, sign-out or reboot require retained-reference recovery and explicit resume, not terminal replay or automatic agent restart.
