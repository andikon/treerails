# Complete discussion delivery

## Current compatibility status

The retained-document workflow and controlled-provider tests are implemented. **Native complete-context delivery is verified on Windows with Copilot 1.0.94.** The capability gate enables only that platform and exact CLI version. Other platforms/versions remain blocked with an explanation. Existing conversations can Continue, and basic worktree profiles remain available.

The installed native Windows executable reports **GitHub Copilot CLI 1.0.94**. Its help lists `--attachment` as valid only in non-interactive mode, so that flag is not used for hosted interactive sessions. Neither a path reference nor an instruction to read a file is treated as proof of complete initial context.

A paid-provider probe was rejected and never run. The user explicitly requires free, local-only testing. Native verification instead used documented `COPILOT_OFFLINE=true`, an isolated provider home, removed credentials and a fake provider bound to `127.0.0.1`. No cloud model or paid inference was used. Do not run paid or cloud tests for this work.

On 19 October 2026, the actual Windows Copilot 1.0.94 terminal and unmodified production extension admitted an **87,537-byte** UTF-8 document through the bundled SDK. Copilot's persisted `user.message` exactly matched the complete document; a recursive string comparison also verified the entire document in the local provider's received request. The delivery claim recorded the admitted message identity. Evidence: session `910af283-335a-4ded-8cc7-839844860d29`, SHA-256 `cf665b4174521679bb130f51cb43f2ae87e80cdb855acee2e9bb64be31bac9aa`, message `f8e73ce5-6360-4660-bac1-57c401fd8414`. Observations included bridge connection, assistant turn start and session idle. A real terminal was required; redirected pipes did not initialize the interactive extension path.

This verifies native session admission and complete provider-bound text independently of a cloud backend. It does not claim a live GitHub-hosted model test or any model reasoning result.

## Delivery mechanism

1. Capture fetches the current issue and every accessible comment page, independently of the preview cache. Any failed or incomplete capture rejects review.
2. Tree Rail writes the complete Markdown document, including issue/comment sources, authors, timestamps, task, checkout, branch and explicit handoff. Review displays that exact retained text and its identity.
3. Approval binds the document SHA-256 and length to the reviewed task and checkout context. Starting again after a changed objective, issue body, checkout or handoff requires another review. A same-request retry refers to the original artifact.
4. Before releasing a previous conversation during handover, Tree Rail validates the artifact, checkout association and native provider capability. A failed preflight leaves the previous owner intact.
5. When capability is verified, the host launches Copilot with an exact conversation UUID and **no automatic initial CLI prompt**. Only the document path, expected SHA-256 and length are supplied in environment variables. Its body does not enter launch arguments or Tree Rail control messages.
6. The native Tree Rail extension joins the exact foreground session, verifies the complete file bytes again, and supplies the entire UTF-8 text through the bundled SDK's `session.send({ prompt })`. This uses Copilot's own session admission protocol rather than a file-reading tool or a truncated command-line prompt. The SDK method returns the admitted user-message identity.
7. Before submitting, the extension creates an exclusive, flushed delivery claim alongside the retained artifact. On confirmed admission it records the message identity. If admission fails or its response is lost, the claim stays `confirmation unknown`. Extension reloads never replay that context automatically. Inspect the recorded conversation and use Continue or explicitly start a separately reviewed new conversation.
8. Continue supplies neither a document nor a starting prompt. Capturing, browsing or commenting never injects content into a live conversation.

The observer installer upgrades only the exact previous Tree Rail observer source. A modified or unrelated observer file is preserved and requires explicit reconciliation.

## Retention

Authoritative documents live under the per-user Tree Rail local data directory:

```text
<ProjectDirs treerails local-data>/discussion-snapshots/
  <SHA-256 of canonical configuration path>/<document UUID>.md
```

Configuration retains approved artifact references even when provider preflight fails, and records the same reference with a successfully created conversation. References include integrity/completeness metadata and a hash binding the reviewed task and checkout context. It does not contain the full discussion. Canonical configuration paths isolate explicit configuration contexts; Windows identities are case-normalized. The data directory is independent of checkout location, including when an explicit configuration file is inside a checkout. Removing a ticket worktree does not remove retained documents. Missing or changed artifacts cannot be silently recreated from a cache or a newer discussion.

Delivery claims use `<document UUID>.md.<conversation UUID>.delivery`. They are retained with the document to prevent duplicate admission after uncertain failures. They are not permission to replay the starting context.

## Controlled validation

Run without a GitHub mutation or paid agent invocation:

```text
cargo test --locked --lib discussion_snapshot
cargo test --locked --lib hosted::tests
cargo test --locked --test hosted_ui
node scripts/discussion_delivery_smoke.mjs
node scripts/copilot_bridge_smoke.mjs
```

The tests cover documents larger than the old 48 KiB brief limit; exact retained content; source/author/edited metadata; partial capture; missing/changed files; configuration isolation; checkout cleanup; task drift; request identity; same-request retries; Continue without replay; UI review and handoff invalidation; SDK admission/reload/lost-response behavior; and observer framing. Controlled SDK tests prove Tree Rail's behavior, **not Copilot compatibility or model receipt**.

## Repeating native validation locally

Run `node scripts/native_discussion_offline_smoke.cjs` in a real Windows terminal with the supported CLI installed. The probe sets offline mode, removes provider credentials and starts only a loopback fake provider. It does not require GitHub authentication or credits. Keep the full text out of native launch arguments; inspect the persisted user message, complete local provider request and admitted delivery claim. Evidence is retained under `target/native-discussion-proof/` for inspection. This opt-in native test is never automatically run in CI.

Also test extension reload after successful admission, uncertain admission, modified/missing documents, and resume without replay. Stop the isolated provider process after evidence is captured. Enable the capability gate only alongside successful recorded native evidence; a stubbed provider, SDK type declaration, file attachment reference or successful file read alone is insufficient.

This establishes complete captured text admission through the supported provider. It does not promise unlimited provider context, attachment binaries, deleted comment history or any particular model reasoning. Provider admission failures must remain visible rather than falling back to partial text.
