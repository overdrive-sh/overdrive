# Website documentation and blog verification

Date: 2026-11-13

Feature: `service-allocation-readiness-and-ending`

Scope: website documentation, one blog post, and this record

## Content and evidence

The current-state documentation covers the default guest-listener gate for every
declared TCP and UDP listener, the explicit empty-startup opt-out, the distinction
between `Running`, committed `Stable`, readiness, and liveness, and the sole
restart authority. It also describes the 60-second default startup deadline,
the derived Service deploy progress deadline, the optional last-attempt context,
and the current limitations of event-based listener reporting. The CLI reference
distinguishes intent acceptance from a Service health verdict.

The blog's decision account draws on the feature delta, ADR-0255 through
ADR-0273 as applicable, the four retained listener and boot spikes, and the
native default-feature E08 positive and negative captures in
`step-05-07-verification.md`. It labels the 1,363 ms `Stable` result as one
native run, describes the six-attempt negative run and 90-second stream timeout,
and separates both from spike distributions. No production deployment or
end-to-end startup SLO is claimed. The post links to pinned source evidence.

## Website checks

Commands ran from `website/` unless noted. All commands used the repository's
existing scripts. The runtime scripts call `bunx`, but this machine's
`~/.proto/shims/bunx` points to an unavailable alternate binary. For those
scripts, a temporary external wrapper at `/tmp/overdrive-bunx-shim/bunx`
invoked `/Users/marcus/.bun/bin/bun x` and was placed first on `PATH`. No
repository script or website build configuration changed.

| Check | Result |
|---|---|
| `bun run typecheck` | Passed. Fumadocs MDX generation and `tsc --noEmit` succeeded. |
| `bun run lint` | Passed. ESLint returned zero. |
| `bun run build` | Passed. Next built 40 static pages, including the new post. The build's one-index assertion counted 31 published pages, including eight blog posts, with one draft excluded. |
| `bun run assert:one-index` | Passed after the final prose edit. The build and explicit verifier found all 31 published pages in the Markdown export, `llms.txt`, and search index. |
| OpenNext Cloudflare build | Passed as the first phase of `test:blog`, `test:mcp`, and `test:mcp:analytics`. No deploy or upload ran. |
| `PATH=/tmp/overdrive-bunx-shim:$PATH bun run test:blog` | Its blog list, published post, search/MCP join, Markdown export identity, and draft exclusion checks passed. The later existing docs top-hit assertion failed as described below. |
| `PATH=/tmp/overdrive-bunx-shim:$PATH bun run test:mcp` | Its tool list and schemas passed. The same docs top-hit assertion failed. |
| `PATH=/tmp/overdrive-bunx-shim:$PATH bun run test:mcp:analytics` | Local D1 migrations and tool schemas passed. The same docs top-hit assertion failed before analytics assertions. |
| Direct local workerd checks on port 8993 | `/blog`, the new post, its `.md` export, `/llms.txt`, and all five changed docs pages returned HTTP 200. `/blog` and `llms.txt` included the new post. Search for `unguarded` returned the new post. All 45 internal Markdown links across the changed pages resolved, including their anchors. |
| `git diff --check` | Passed for the content changes. |

The common acceptance failure is the fixed expectation that a search for
`deterministic simulation` ranks
`/docs/concepts/deterministic-simulation-testing` first. All three scripts
instead receive `/docs/concepts/workload-identity-lifecycle` first. This is
pre-existing. I built an untouched `HEAD` website from a separate `git archive`
copy, ran its `bun run typecheck` and its `test:blog` through the same local
workerd path, and reproduced the **same assertion and same first result**.
That baseline had 30 published pages and seven blog posts. The temporary
baseline copy was deleted after the comparison. The changed content does not
cause the ranking failure. The tests do not reach their later assertions after
that common failure, so those later assertions are not recorded as passing.

## Review status

The content and evidence are ready for independent documentation review. The
review verdict is pending. Final mutation and integration evidence are separate
feature gates and are not claimed here. The website has not been deployed or
published.
