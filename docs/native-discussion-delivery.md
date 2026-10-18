# Complete discussion delivery

## Current compatibility status

The retained-document workflow and controlled-provider tests are implemented. **Native complete-context delivery is not yet verified.** Production document starts are deliberately blocked by `DOCUMENT_DELIVERY_VERIFIED = false`; the error explains that verification is pending. Capture, inspection and retention remain available. Existing conversations can Continue, and basic worktree profiles remain available.

The installed native Windows executable reports **GitHub Copilot CLI 1.0.94**. Its help lists `--attachment` as valid only in non-interactive mode, so that flag is not used for hosted interactive sessions. Neither a path reference nor an instruction to read a file is treated as proof of complete initial context.

A native compatibility probe was prepared with an isolated provider home and an 87,537-byte synthetic document. The executable rejected a zero-credit budget and required a minimum budget of 30 AI credits. Automatic approval review rejected the paid probe because spending had not been explicitly authorized. **No successful native delivery result is claimed.** Obtain explicit approval before performing that probe; a budget limit is not a prediction of the actual charge.

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

## Native validation before enabling

Use an explicitly approved, isolated native probe with the supported version, synthetic Markdown larger than both the old brief limit and Windows command-line limit, and no repository work. Keep the full text out of the native launch arguments. Exercise the production extension's exact `session.send` path, then inspect Copilot's persisted first `user.message` event and the admitted delivery claim. Compare every UTF-8 byte to the approved file, including its final marker, and confirm there was no earlier partial starting prompt. Record the platform/version, document length/hash, invocation configuration and comparison result here.

Also test extension reload after successful admission, uncertain admission, modified/missing documents, and resume without replay. Stop the isolated provider process after evidence is captured. Enable the capability gate only alongside successful recorded native evidence; a stubbed provider, SDK type declaration, file attachment reference or successful file read alone is insufficient.

This establishes complete captured text admission through the supported provider. It does not promise unlimited provider context, attachment binaries, deleted comment history or any particular model reasoning. Provider admission failures must remain visible rather than falling back to partial text.
