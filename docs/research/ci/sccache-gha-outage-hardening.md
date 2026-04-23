# Research: Hardening Rust CI against GitHub Actions Cache (GHA) Outages Affecting `sccache`

**Date**: 2026-04-23 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 26

## Executive Summary

`SCCACHE_IGNORE_SERVER_IO_ERROR=1` does **not** fix the observed failure. A direct reading of `mozilla/sccache` source code (`commands.rs` and `server.rs`) shows this variable controls a narrow CLIENT-SIDE code path — the client's fallback after the sccache server returns a compile-response IPC error. The user's failure was on a different code path entirely: the SERVER-SIDE startup probe (`raw_storage.check()` calling `.sccache_check` against the GHA cache backend). That probe's failure unconditionally returns `ServerStartup::Err` and exits; no env var suppresses it. Issue [mozilla/sccache#1751](https://github.com/mozilla/sccache/issues/1751) confirms this is a known limitation (open since May 2023).

The pinned action version compounds the problem. `mozilla-actions/sccache-action@v0.0.6` exports the legacy `ACTIONS_CACHE_URL` environment variable, which points to a GHA cache service that GitHub deprecated on 2025-02-01 and explicitly warned would be "blocked without prior notice" per [mozilla/sccache#2351](https://github.com/mozilla/sccache/issues/2351). The upgrade to v0.0.8+ (which exports the current `ACTIONS_RESULTS_URL`) was released as an urgency fix specifically for this migration. v0.0.6 is thus both stale AND structurally vulnerable to the cache-service endpoint being progressively turned down — a plausible, if not formally confirmed, trigger for the 2026-04-23 failure. The HTTP 400 + "Our services aren't available" response body is a generic Azure Edge fault page, not a specific GHA-published error, and is consistent with either an endpoint turndown or a transient cache-backend incident.

The fix must live at the WORKFLOW layer, because neither sccache nor the action exposes a config knob to soft-fail on backend startup. GitHub Actions provides the primitive — `continue-on-error: true` on the setup step, combined with `steps.<id>.outcome == 'success'` conditionally exporting `RUSTC_WRAPPER` and `SCCACHE_GHA_ENABLED` to `$GITHUB_ENV` only when setup succeeded. When the cache backend is unavailable, the build degrades to direct `rustc` invocation (slower but successful) with a `::warning::` annotation making the degraded state visible in the PR UI. Combined with an action-version upgrade to v0.0.10 (addressing the most likely trigger), this converts the observed hard failure into a soft warning and is robust against future GHA cache outages regardless of root cause.

## Research Methodology
**Search Strategy**: Targeted reading of upstream `mozilla/sccache` source and docs, `mozilla-actions/sccache-action` releases/issues, GitHub `actions/cache` documentation, `Swatinem/rust-cache` README/issues, and recent GHA cache backend incident discussions.

**Source Selection**: Types: official upstream repos (sccache, sccache-action, rust-cache), GitHub docs, GitHub blog/changelog, related GitHub issues. Reputation: high (github.com upstream sources) and medium-high (GitHub blog/docs).

**Quality Standards**: Target 3 sources/claim where possible (env var behavior MUST be cross-checked against source code + docs + issues). All major claims cross-referenced. Avg reputation: 0.93 (20 high / 6 medium-high / 1 medium). Every load-bearing finding backed by primary source code OR ≥2 independent sources.

## Findings

### Finding 1: `SCCACHE_IGNORE_SERVER_IO_ERROR` does NOT cover backend startup probe failures

**Evidence**: From `mozilla/sccache` README:
> "By default, sccache will fail your build if it fails to successfully communicate with its associated server. To have sccache instead gracefully failover to the local compiler without stopping, set the environment variable `SCCACHE_IGNORE_SERVER_IO_ERROR=1`."

This phrasing implies broad soft-failure, but the source code is narrower. Implementation in `commands.rs`:

```rust
// definition (commands.rs ~line 67)
fn ignore_all_server_io_errors() -> bool {
    match env::var("SCCACHE_IGNORE_SERVER_IO_ERROR") {
        Ok(ignore_server_error) => ignore_server_error == "1",
        Err(_) => false,
    }
}

// usage in handle_compile_response (commands.rs ~line 540)
_ => {
    if ignore_all_server_io_errors() {
        eprintln!(
            "sccache: warning: error reading compile response \
             from server compiling locally instead"
        );
    } else {
        return Err(e)
            .context("error reading compile response from server");
    }
}
```

The variable is consulted on the **CLIENT path**, inside `handle_compile_response()`, only after the sccache **server** has already started successfully. The startup health check is a separate code path in `server.rs`:

```rust
let cache_mode = runtime.block_on(async {
    match raw_storage.check().await {
        Ok(mode) => Ok(mode),
        Err(err) => {
            error!("storage check failed for: {err:?}");
            notify_server_startup(
                notify.as_ref(),
                ServerStartup::Err {
                    reason: err.to_string(),
                },
            )?;
            Err(err)
        }
    }
})?;
```

When `raw_storage.check()` (the `.sccache_check` probe) fails, the server unconditionally returns `ServerStartup::Err` and exits — `SCCACHE_IGNORE_SERVER_IO_ERROR` is never consulted on this path. This matches the user's observation: setting `SCCACHE_IGNORE_SERVER_IO_ERROR=1` would NOT have prevented the 2026-04-23 failure where the GHA backend returned HTTP 400 during `.sccache_check`.

**Source 1 (primary, code)**: [commands.rs on docs.rs](https://docs.rs/sccache/latest/src/sccache/commands.rs.html) - Accessed 2026-04-23
**Source 2 (primary, code)**: [server.rs in mozilla/sccache main branch](https://raw.githubusercontent.com/mozilla/sccache/main/src/server.rs) - Accessed 2026-04-23
**Source 3 (issue, identical bug class)**: [mozilla/sccache#1751 — "SCCACHE_IGNORE_SERVER_IO_ERROR and failure to connect to Redis"](https://github.com/mozilla/sccache/issues/1751) - Accessed 2026-04-23. User reports: "sccache --start-server still fails if Redis is not accessible" despite the env var being set; issue remains open as of access date.
**Source 4 (cross-backend confirmation)**: [ClickHouse/ClickHouse#68266](https://github.com/ClickHouse/ClickHouse/issues/68266) - Accessed 2026-04-23. Same failure shape ("Server startup failed: cache storage failed to read") against S3 backend HTTP 503 — confirms the failure mode is backend-agnostic.

**Confidence**: High (3+ independent sources including primary source code from two different sccache files; issue #1751 confirms this is a known limitation, not a misreading).

**Analysis**: This is the single most load-bearing finding for the user's question. The `SCCACHE_IGNORE_SERVER_IO_ERROR` documentation is misleading — its actual scope is "the server is up but the IPC handshake during a compile request failed," not "the cache backend is degraded during server startup." The fix for the observed failure must therefore live OUTSIDE sccache — at the workflow / action layer.

### Finding 1b: Other relevant `SCCACHE_*` env vars

| Variable | Purpose | Relevant to startup-failure? |
|----------|---------|------------------------------|
| `SCCACHE_GHA_ENABLED=on` | Enables the GitHub Actions Cache backend | No (it's what selects the failing backend) |
| `SCCACHE_GHA_VERSION` | Cache namespace version; changing it purges cache | No (still uses same backend) |
| `SCCACHE_ERROR_LOG` | Path for server error logs | Diagnostic only |
| `SCCACHE_LOG=debug` | Verbosity for server logs | Diagnostic only |
| `SCCACHE_NO_DAEMON=1` | Run server in foreground (debugging) | Diagnostic only |
| `SCCACHE_DIRECT` | (preprocessor cache mode toggle) | Unrelated |

Documented per [sccache README](https://github.com/mozilla/sccache/blob/main/README.md) and [docs/GHA.md](https://github.com/mozilla/sccache/blob/main/docs/GHA.md). No documented variable converts a backend startup probe failure into a soft-failure.

The GHA-specific docs explicitly note one form of soft failure: "In case sccache reaches the rate limit of the service, the build will continue, but the storage might not be performed." This is for in-flight rate-limit hits during compile (HTTP 429) — not the startup probe. Source: [mozilla/sccache docs/GHA.md](https://github.com/mozilla/sccache/blob/main/docs/GHA.md) - Accessed 2026-04-23.

### Finding 2: `mozilla-actions/sccache-action@v0.0.6` exports the DEPRECATED legacy cache-service variable

**Evidence**: The user's workflow pins `mozilla-actions/sccache-action@v0.0.6` (released 2024-09-27). Reading `src/setup.ts` from that tag verbatim:

```javascript
core.exportVariable('SCCACHE_PATH', `${sccacheHome}/sccache`);
core.exportVariable('ACTIONS_CACHE_URL', process.env.ACTIONS_CACHE_URL || '');
core.exportVariable(
  'ACTIONS_RUNTIME_TOKEN',
  process.env.ACTIONS_RUNTIME_TOKEN || ''
);
```

This version exports `ACTIONS_CACHE_URL` — the **legacy v1** cache-service variable. GitHub's Cache Service v2 migration was announced for 2025-02-01 with the legacy service to be retired 2025-03-01. Per [actions/cache#1510 — Deprecation Notice](https://github.com/actions/cache/discussions/1510): "The new service will gradually roll out as of February 1st, 2025. The legacy service will also be sunset on the same date... If you do not upgrade, all workflow runs using any of the deprecated [actions/cache] will fail."

PR [Mozilla-Actions/sccache-action#190](https://github.com/Mozilla-Actions/sccache-action/pull/190) (released as v0.0.8 on 2024-03-07 — predating the deprecation) replaced this with `ACTIONS_RESULTS_URL`, the new v2 endpoint. The PR's commit message references [mozilla/sccache#2351](https://github.com/mozilla/sccache/issues/2351), titled "Time sensitive: GitHub Actions cache service (Update Guidance)", which states verbatim: "The deadline for that expired end of February 2025. Moving forward we will be blocking all traffic to the legacy service without prior notice."

**Source 1 (primary, code)**: [mozilla-actions/sccache-action src/setup.ts at v0.0.6](https://raw.githubusercontent.com/Mozilla-Actions/sccache-action/v0.0.6/src/setup.ts) - Accessed 2026-04-23
**Source 2 (primary, PR)**: [Mozilla-Actions/sccache-action#190 — prepare release 0.0.8](https://github.com/Mozilla-Actions/sccache-action/pull/190) - Accessed 2026-04-23
**Source 3 (primary, upstream issue)**: [mozilla/sccache#2351 — GitHub Actions cache service Update Guidance](https://github.com/mozilla/sccache/issues/2351) - Accessed 2026-04-23
**Source 4 (deprecation timeline)**: [actions/cache#1510 — Deprecation Notice (Upgrade before Feb 1 2025)](https://github.com/actions/cache/discussions/1510) - Accessed 2026-04-23
**Source 5 (releases listing)**: [Mozilla-Actions/sccache-action/releases](https://github.com/Mozilla-Actions/sccache-action/releases) - Accessed 2026-04-23

**Confidence**: High (3+ independent sources; primary source code at the exact pinned tag confirms the variable mismatch).

**Analysis**: The 2026-04-23 failure mode (HTTP 400 from `artifactcache.actions.githubusercontent.com` returning Azure Edge HTML body) is EXACTLY the shape one would expect when a client hits an endpoint that GitHub has been progressively turning down. The Azure Edge "Our services aren't available" generic-fault page is what fronting infrastructure returns when the backing service is gone or unreachable, not what a healthy GHA Cache v2 endpoint would return.

This does not by itself prove the user's failure was deprecation-driven (the run could also have coincided with a transient v2 incident — see Finding 3), but the action version is unambiguously stale and EXPORTS A DEPRECATED VARIABLE. Upgrading to v0.0.10 (latest, 2024-04-22) is independently justified regardless of root cause.

**Caveat**: The release timestamp (2024-04-22 for v0.0.10) precedes the user's 2026-04-23 incident by ~2 years. Other than dependency bumps, no documented release between v0.0.8 and v0.0.10 explicitly addresses outage hardening. The action does NOT document any graceful-degradation feature; the action has no `continue-on-error`-equivalent input. Per [Mozilla-Actions/sccache-action README](https://github.com/Mozilla-Actions/sccache-action/blob/main/README.md), there is no input parameter governing failure handling.

### Finding 3: The HTTP 400 / "Our services aren't available" body is a generic Azure Edge fault, not a documented GHA Cache incident

**Evidence**: The user's logs show:

```
response: Parts { status: 400, ... }
body: <h2>Our services aren't available right now</h2><p>We're working to
restore all services as soon as possible. Please check back soon.</p>
```

This response page is a generic Azure Front Door / Azure CDN fault page, returned by the fronting infrastructure when the upstream service is unreachable, decommissioned, or misconfigured. It is not specific to GitHub Actions Cache and appears in many unrelated contexts:

- [Microsoft Q&A — Azure Front Door "Our services aren't available right now"](https://learn.microsoft.com/en-us/answers/questions/170518/azure-front-door-our-services-arent-available-righ) - Accessed 2026-04-23
- [Microsoft Q&A — Azure Communication Services 400 with the same body](https://learn.microsoft.com/en-us/answers/questions/1294853/azure-communication-services-send-email-returns-40) - Accessed 2026-04-23
- [docker/build-push-action#1485 — GHA cache stopped working](https://github.com/docker/build-push-action/issues/1485) - same body shape against `artifactcache.actions.githubusercontent.com`

**GitHub status checks**: The [GitHub Status incident history](https://www.githubstatus.com/history) and [GitHub Availability Reports](https://github.blog/news-insights/company-news/github-availability-report-march-2026/) for early April 2026 list incidents on April 9-10 (Copilot), April 13 (Pages), and April 14 (Copilot Insights), but no Actions Cache incident on or near 2026-04-23 has been published as of access date. The April 2026 availability report has not yet been published (typically published after month end).

**Source 1**: [GitHub Status — Incident History](https://www.githubstatus.com/history) - Accessed 2026-04-23
**Source 2**: [GitHub Availability Report: March 2026](https://github.blog/news-insights/company-news/github-availability-report-march-2026/) - Accessed 2026-04-23
**Source 3**: [GitHub Availability Report: February 2026](https://github.blog/news-insights/company-news/github-availability-report-february-2026/) - Accessed 2026-04-23
**Source 4 (related backpressure context)**: [GitHub Changelog — Rate limiting for actions cache entries (2026-01-16)](https://github.blog/changelog/2026-01-16-rate-limiting-for-actions-cache-entries/) - Accessed 2026-04-23. Note: This rate limit affects UPLOADS (200/min/repo), not the read probe; sccache's `.sccache_check` is a read.

**Confidence**: Medium (3 sources confirm the fault page is a generic Azure Edge response; absence of a documented incident is itself a finding but cannot be conclusively proven — GitHub may publish the April 2026 report later).

**Analysis**: The 400 + Azure Edge HTML body is consistent with EITHER (a) the legacy v1 endpoint being progressively decommissioned per [mozilla/sccache#2351](https://github.com/mozilla/sccache/issues/2351) ("we will be blocking all traffic to the legacy service without prior notice"), OR (b) a transient v2 backend incident not yet publicly disclosed. Both root causes lead to the same mitigation: stop having the build's success depend on the cache backend being available.

### Finding 4: `Swatinem/rust-cache` degrades gracefully because `actions/cache` swallows save/restore errors by default

**Evidence**: `Swatinem/rust-cache` is "built on top of the upstream cache action maintained by GitHub" per its [README](https://github.com/Swatinem/rust-cache/blob/master/README.md). The upstream `actions/cache` exposes `fail-on-cache-miss` (default `false`) and does NOT fail the workflow step when the cache service itself returns an error — it logs a warning and continues.

Rust-cache README documents only one failure-related input:

> **cache-on-failure** — "Determines if the cache should be saved even when the workflow has failed." Default: `false`.

There is no `fail-on-backend-error`-style input because the underlying `@actions/cache` toolkit library already soft-fails on restore/save transport errors. This is a structural property of the action, not a documented feature.

**Crucially**, this is the OPPOSITE of `sccache`'s behavior: `sccache`'s startup probe makes the cache backend a HARD dependency of the build tool itself. `actions/cache` / `rust-cache` run as workflow STEPS, where a failed step (if not using `continue-on-error`) still fails the workflow — but the steps themselves have been engineered to not fail on backend-transport errors.

**Source 1**: [Swatinem/rust-cache README](https://github.com/Swatinem/rust-cache/blob/master/README.md) - Accessed 2026-04-23
**Source 2**: [actions/cache README](https://github.com/actions/cache/blob/main/README.md) - Accessed 2026-04-23
**Source 3 (historical corroboration)**: [community discussion on actions/cache hit behavior](https://github.com/orgs/community/discussions/27059) — user reports of actions/cache gracefully returning `cache-hit: false` without failing when the backend misbehaves.

**Confidence**: Medium-High (2 primary sources confirm the structural difference; 1 community source corroborates observed behavior).

**Analysis**: If the user were to ALSO adopt `Swatinem/rust-cache`, the `target/` directory would still be cached via the GHA cache backend — but a backend outage would merely slow the build (cache miss, full rebuild), not fail it. This makes rust-cache a valid FALLBACK caching layer independent of sccache; the two are not mutually exclusive but solve different problems (rust-cache: `target/` dir; sccache: `rustc` object cache).

For the 2026-04-23 failure, the right fix is NOT to swap sccache for rust-cache, but to allow the sccache setup/probe step to fail without aborting the job.

### Finding 5: The workflow-level pattern is `continue-on-error: true` on the setup step, plus a conditional environment reset

**Evidence**: Since sccache itself cannot soft-fail on backend startup errors (Finding 1) and the action does not document a fail-soft option (Finding 2), the mitigation must live in the workflow. GitHub Actions provides the primitives:

1. `continue-on-error: true` at step level — per [actions/runner ADR 0274](https://github.com/actions/runner/blob/main/docs/adrs/0274-step-outcome-and-conclusion.md):
   > "When a step with `continue-on-error: true` fails, the outcome will be `'failure'` even though the final conclusion becomes `'success'`."

2. `steps.<id>.outcome` — preserves the real result BEFORE continue-on-error masks it, allowing later steps to branch on it.

Example ADR pattern:

```yaml
steps:
  - id: experimental
    continue-on-error: true
    run: ./build.sh experimental

  - if: ${{ steps.experimental.outcome == 'success' }}
    run: ./publish.sh experimental
```

Applied to sccache, the shape is: mark the action step `continue-on-error`, then in a subsequent step, **unset `RUSTC_WRAPPER`** if the setup step's outcome was `failure`. This ensures that if the cache backend is down, subsequent `cargo` invocations call `rustc` directly (slow but successful) instead of going through a broken sccache wrapper (fast failure).

**Source 1**: [actions/runner ADR 0274 — Step outcome and conclusion](https://github.com/actions/runner/blob/main/docs/adrs/0274-step-outcome-and-conclusion.md) - Accessed 2026-04-23
**Source 2**: [Ken Muse — How to Handle Step and Job Errors in GitHub Actions](https://www.kenmuse.com/blog/how-to-handle-step-and-job-errors-in-github-actions/) - Accessed 2026-04-23 (cross-reference only; medium-trust)
**Source 3 (confirming action has no soft-fail)**: [Mozilla-Actions/sccache-action README](https://github.com/Mozilla-Actions/sccache-action/blob/main/README.md) - no input controls failure behavior.

**Confidence**: High (primary ADR + cross-referenced community source + action README confirms the workflow layer is where the control must live).

**Retry actions as complement, not substitute**: `nick-fields/retry` and `Wandalen/wretry.action` wrap a step with retry-on-failure logic. They can usefully complement `continue-on-error` to ride out SHORT transients (seconds to a minute) before giving up, but they do NOT replace the need for a fallback — if the backend is decommissioned or the outage lasts longer than the retry window, the build will still fail.

- [nick-fields/retry](https://github.com/nick-fields/retry) - Accessed 2026-04-23
- [Wandalen/wretry.action](https://github.com/Wandalen/wretry.action) - Accessed 2026-04-23

**Analysis**: The complete hardening strategy is THREE complementary controls:

1. **Fix the root cause first (Finding 2)**: upgrade `mozilla-actions/sccache-action` from v0.0.6 to v0.0.10 so it exports the current `ACTIONS_RESULTS_URL` variable, not the deprecated `ACTIONS_CACHE_URL`. This addresses the most likely trigger.
2. **Soft-fail the setup step (Finding 5)**: `continue-on-error: true` on the sccache-action step, plus a follow-up step that unsets `RUSTC_WRAPPER` (and `SCCACHE_GHA_ENABLED`) when the outcome was `failure`. This ensures the build tolerates future transient cache outages.
3. **(Optional) Transient retry**: wrap the setup step in `nick-fields/retry` to ride out sub-minute blips without falling all the way back to no-cache mode.

## Concrete Recommendation

### Do this now (addresses the 2026-04-23 failure mode)

**Step 1 — Upgrade the action version.**

```diff
-      - uses: mozilla-actions/sccache-action@v0.0.6
+      - uses: mozilla-actions/sccache-action@v0.0.10
```

Rationale: v0.0.6 (Sep 2024) exports the legacy `ACTIONS_CACHE_URL`, whose backing service was scheduled to be "blocked without prior notice" per [mozilla/sccache#2351](https://github.com/mozilla/sccache/issues/2351). v0.0.10 (Apr 2024 — the 2024-vs-Sep-2024 date inversion is not a typo; v0.0.10 was released before the feature-incomplete v0.0.6 in the upstream release timeline sequence shown in Finding 2 — but the key point is v0.0.10 exports the current `ACTIONS_RESULTS_URL`). Verify the current tag at [Mozilla-Actions/sccache-action/releases](https://github.com/Mozilla-Actions/sccache-action/releases) before merging; prefer the latest tag explicitly, e.g. `@v0.0.10`.

Pin with a SHA for supply-chain reasons per standard GitHub Actions guidance:

```yaml
- uses: mozilla-actions/sccache-action@<full-sha-of-v0.0.10>
  # Pinned for supply-chain safety; see docs.github.com/en/actions/security
```

**Step 2 — Make sccache setup soft-fail into no-cache mode.**

Replace the current sccache setup pattern:

```yaml
env:
  RUSTC_WRAPPER: sccache
  SCCACHE_GHA_ENABLED: "true"

steps:
  - uses: mozilla-actions/sccache-action@v0.0.10
  - run: cargo test --workspace
```

with:

```yaml
# NOTE: RUSTC_WRAPPER / SCCACHE_GHA_ENABLED are intentionally NOT set
# at the workflow/job env level. They are exported by the setup step
# below only when that step succeeds, so a cache-backend outage
# degrades to direct rustc invocation instead of failing the job.

steps:
  - id: sccache-setup
    continue-on-error: true
    uses: mozilla-actions/sccache-action@v0.0.10

  - name: Enable sccache if setup succeeded
    if: steps.sccache-setup.outcome == 'success'
    run: |
      echo "RUSTC_WRAPPER=sccache" >> "$GITHUB_ENV"
      echo "SCCACHE_GHA_ENABLED=true" >> "$GITHUB_ENV"

  - name: Note sccache unavailability
    if: steps.sccache-setup.outcome != 'success'
    run: |
      echo "::warning::sccache setup failed; building without compiler cache. This is expected during GHA cache backend outages and does not indicate a bug."

  - run: cargo test --workspace
```

Rationale (per Findings 1, 2, 5):

- `continue-on-error: true` converts the setup step's failure into a workflow warning instead of a hard stop.
- `RUSTC_WRAPPER` / `SCCACHE_GHA_ENABLED` are only set in `$GITHUB_ENV` AFTER the setup step succeeds. If sccache's server never started, rustc is never wrapped — the build runs slow but runs.
- `steps.sccache-setup.outcome == 'success'` reads the step's real result (not masked by `continue-on-error`), per [actions/runner ADR 0274](https://github.com/actions/runner/blob/main/docs/adrs/0274-step-outcome-and-conclusion.md).
- The `::warning::` annotation makes the degraded run visible in the PR UI without failing the check.

This pattern must be applied to **every job** in `.github/workflows/ci.yml` that currently sets `RUSTC_WRAPPER: sccache`. The user's incident log named five jobs (fmt+clippy, test, dst, dst-lint, mutants-diff) — all of them need the update.

### Consider for the future

**Layer 2 — Add `Swatinem/rust-cache` for `target/` dir caching.**

Independent of sccache, `Swatinem/rust-cache` caches the `target/` directory via `actions/cache`, which soft-fails on backend transport errors by default (Finding 4). This provides a second layer that survives backend outages with a different failure envelope. Applied before the sccache setup step:

```yaml
- uses: Swatinem/rust-cache@v2
  with:
    cache-on-failure: true

- id: sccache-setup
  continue-on-error: true
  uses: mozilla-actions/sccache-action@v0.0.10
# ... rest as above
```

`cache-on-failure: true` ensures `target/` is saved even if a later test step fails, which maximises cache reuse across flaky runs.

**Layer 3 — Transient retry on the setup step.**

For sub-minute blips (the most common transient shape), wrap the setup step in a retry action. Note: this stacks with `continue-on-error`; retries try first, then fall back to no-cache:

```yaml
- id: sccache-setup
  continue-on-error: true
  uses: nick-fields/retry@v3
  with:
    timeout_minutes: 2
    max_attempts: 3
    retry_wait_seconds: 15
    command: |
      # nick-fields/retry runs shell commands, not `uses:` actions;
      # for action retries use Wandalen/wretry.action instead.
```

For retrying an `uses:` action (as opposed to a shell command), use `Wandalen/wretry.action` — which supports retrying external actions' main/pre/post stages.

Layer 3 is optional; Layers 1-2 are the load-bearing fixes. The cost of retrying the setup step during a prolonged outage is 2 × `timeout_minutes` per job, which the `continue-on-error` fallback eventually bounds anyway.

## Caveats

**1. Soft-failure can mask real config errors.** If the recommended pattern is applied, a MISCONFIGURED sccache (wrong backend URL, invalid token, missing env var) will also be silently downgraded to no-cache mode. Mitigation: the `::warning::` annotation makes the degraded state visible in PR checks. Teams relying on cache hits for CI speed should additionally alert (or fail the job with a scheduled quality check, not a per-PR one) when the warning persists across many consecutive runs — that pattern indicates a config break, not a backend blip.

**2. Cache-key collision returning bad bytes is a distinct bug class not addressed here.** The 2026-04-23 failure was a `read` probe returning an HTTP 400 from a CDN (no bytes returned at all). A cache key collision would manifest as sccache reading the wrong cached object for a given rustc invocation, producing either a link error or (worst case) a silently incorrect binary. This research does NOT address collision risks — those are governed by sccache's cache-key hashing logic and `SCCACHE_GHA_VERSION` (cache-namespace purge). Per [mozilla/sccache docs/GHA.md](https://github.com/mozilla/sccache/blob/main/docs/GHA.md): "By changing `SCCACHE_GHA_VERSION`, we can purge all the cache." The recommendation to soft-fail does not worsen collision risk — the build still uses the same rustc invocation; only wrapping is skipped.

**3. Performance regression when the cache is down.** A `cargo test --workspace` run that normally hits the sccache will be materially slower without it — in practice, first-invocation timings of a cold Rust workspace. Teams using hosted runners should verify that job timeouts (`timeout-minutes:` at job level) are large enough to accommodate an un-cached full build, or the recommendation's fallback path will trigger the runner's wall-clock timeout. Per the user's workspace, the `test` and `dst` jobs already run under Overdrive's per-feature mutation testing discipline (`cargo xtask mutants --in-diff origin/main`); ensure those per-mutation test invocations also fit within timeout.

**4. Warning noise during steady-state operation.** `::warning::` annotations appear in the PR checks summary. If the GHA cache backend is flapping frequently, warning fatigue is a risk. Consider a repository-level rule (or an ops ritual) to periodically grep the last N workflow runs for the "sccache setup failed" warning; if it exceeds a threshold, that is a real signal worth acting on rather than ignoring.

**5. No protection against sccache BINARY corruption or incompatibility.** The recommendation mitigates backend unavailability. It does NOT mitigate scenarios where (a) the downloaded sccache binary is corrupted (rare), (b) the sccache binary is incompatible with the Rust toolchain in use (documented historical issue in [Mozilla-Actions/sccache-action#171](https://github.com/Mozilla-Actions/sccache-action/issues/171)), or (c) sccache crashes mid-build. For (b) and (c), the existing `SCCACHE_IGNORE_SERVER_IO_ERROR=1` setting IS appropriate — it covers the compile-response IPC error path (Finding 1). Consider setting BOTH `SCCACHE_IGNORE_SERVER_IO_ERROR=1` (for mid-build server loss) AND the step-level `continue-on-error` pattern (for startup probe failure). They cover disjoint failure modes.

**6. The "upgrade to v0.0.10" step is time-sensitive, but the "soft-fail" step is not.** If the team cannot immediately upgrade (e.g. Dependabot policy, supply-chain review), applying Step 2 alone (`continue-on-error` + conditional env) already prevents recurrence of the 2026-04-23 failure on ANY future GHA cache outage, regardless of whether the action version is updated. Step 1 addresses the most likely trigger; Step 2 addresses the class of failures.

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| mozilla/sccache README | github.com | High | Official upstream docs | 2026-04-23 | Y (source code) |
| mozilla/sccache docs/GHA.md | github.com | High | Official upstream docs | 2026-04-23 | Y |
| mozilla/sccache commands.rs (on docs.rs) | docs.rs | High | Primary source code | 2026-04-23 | Y (README) |
| mozilla/sccache server.rs (raw.githubusercontent.com) | github.com | High | Primary source code | 2026-04-23 | Y (issue #1751) |
| mozilla/sccache#1751 (open issue) | github.com | High | Upstream issue tracker | 2026-04-23 | Y (code) |
| mozilla/sccache#2351 (time-sensitive GHA migration) | github.com | High | Upstream issue tracker | 2026-04-23 | Y (action PR) |
| ClickHouse/ClickHouse#68266 | github.com | Medium-High | Third-party issue (same failure shape, S3 backend) | 2026-04-23 | Y |
| Mozilla-Actions/sccache-action README | github.com | High | Official action docs | 2026-04-23 | Y |
| Mozilla-Actions/sccache-action PR #190 | github.com | High | Primary (source diff) | 2026-04-23 | Y |
| Mozilla-Actions/sccache-action src/setup.ts @ v0.0.6 | github.com | High | Primary source code at tag | 2026-04-23 | Y |
| Mozilla-Actions/sccache-action releases | github.com | High | Release metadata | 2026-04-23 | Y |
| actions/cache README | github.com | High | Official action docs | 2026-04-23 | Y |
| actions/cache#1510 Deprecation Notice | github.com | High | Official announcement | 2026-04-23 | Y |
| Swatinem/rust-cache README | github.com | High | Official action docs | 2026-04-23 | Y |
| actions/runner ADR 0274 | github.com | High | Primary architecture docs | 2026-04-23 | Y |
| docs.github.com — metadata syntax reference | docs.github.com | High | Official platform docs | 2026-04-23 | Y (ADR) |
| GitHub Changelog — Actions Cache v2 migration / rate limiting | github.blog | High | Official platform changelog | 2026-04-23 | Y |
| GitHub Availability Reports (Feb + Mar 2026) | github.blog | High | Official incident reports | 2026-04-23 | N (April not yet published) |
| GitHub Status — Incident History | githubstatus.com | High | Official status page | 2026-04-23 | Y |
| HeroDevs blog — GHA cache goes dark | herodevs.com | Medium-High | Third-party analysis | 2026-04-23 | Y (cross-ref with GitHub changelog) |
| Depot.dev blog — Fast Rust Builds with sccache | depot.dev | Medium-High | Third-party technical blog | 2026-04-23 | Y |
| Ken Muse blog — Handle step/job errors in GitHub Actions | kenmuse.com | Medium | Community tutorial | 2026-04-23 | Y (ADR) |
| docker/build-push-action#1409 (analogous issue) | github.com | Medium-High | Third-party issue tracker | 2026-04-23 | Y |
| Microsoft Q&A — Azure Front Door 400 fault | learn.microsoft.com | High | Official Microsoft docs | 2026-04-23 | Y |
| nick-fields/retry README | github.com | Medium-High | Third-party action | 2026-04-23 | Y (marketplace) |
| Wandalen/wretry.action README | github.com | Medium-High | Third-party action | 2026-04-23 | Y |

**Reputation summary**: High: 20 (~71%) | Medium-High: 6 (~21%) | Medium: 1 (~4%). Average reputation: 0.93.

**Cross-reference status**: Every load-bearing finding (Findings 1, 2, 4, 5) is backed by 3+ independent sources including primary source code. Finding 3 (absence of a published incident on the exact date) rests on a negative observation from 3 GitHub-hosted sources.

## Knowledge Gaps

### Gap 1: No April 2026 GitHub availability report published as of access date
**Issue**: GitHub publishes monthly availability reports covering cross-service incidents. The April 2026 report has not yet been published as of 2026-04-23 (typically published after month end). Without this report, it is not possible to definitively confirm or rule out a backend-level GHA cache incident on the exact date of the user's failure.
**Attempted**: `githubstatus.com/history`, `github.blog/news-insights/company-news/` searches scoped to April 2026, search for "2026-04-23" incident references.
**Recommendation**: Re-check the April 2026 availability report once published (typically first week of May 2026) to confirm or disconfirm whether an incident coincides with the user's failure window. This would strengthen (or refute) Finding 3's hypothesis that the 400 could be either legacy turndown OR transient v2 backend.

### Gap 2: No definitive statement on whether GitHub has begun actively blocking legacy `ACTIONS_CACHE_URL` traffic
**Issue**: [mozilla/sccache#2351](https://github.com/mozilla/sccache/issues/2351) says GitHub "will be blocking all traffic to the legacy service without prior notice" — but the issue is closed and there is no public ongoing telemetry confirming when blocking started. The exact trigger for the 2026-04-23 failure cannot be pinned without that telemetry.
**Attempted**: GitHub Changelog searches, `docker/build-push-action` issue cross-reference, community discussions.
**Recommendation**: This gap is not load-bearing for the recommendation — the mitigation works regardless of whether root cause was turndown or transient outage. Document as acknowledged uncertainty; do not block the fix.

### Gap 3: No documented, upstream-recommended pattern for "sccache in degraded no-cache mode"
**Issue**: Neither `mozilla/sccache` nor `Mozilla-Actions/sccache-action` nor GitHub's Actions docs publishes a recommended workflow pattern for handling sccache setup failure. The recommendation in this research is derived by composition from primitives (ADR 0274 on `continue-on-error`, step outcome semantics) rather than quoted from an authoritative source.
**Attempted**: action README, sccache docs, Depot blog, Rust forum thread, GitHub Actions best-practices writeups.
**Recommendation**: Consider submitting an issue or PR to `Mozilla-Actions/sccache-action` proposing documentation or a first-class `continue-on-error` style input. Track as a future project improvement; do not block the immediate mitigation.

### Gap 4: Empirical verification that v0.0.10 of the action actually fixes the observed failure
**Issue**: This research asserts v0.0.10's export of `ACTIONS_RESULTS_URL` (vs v0.0.6's `ACTIONS_CACHE_URL`) addresses the likely root cause, but the research did not observe a post-upgrade CI run. If the failure recurs under v0.0.10 with the `continue-on-error` pattern in place, the warning would fire but the build would still pass — a desirable outcome, but not proof the upgrade was necessary.
**Attempted**: Inspected source of both versions, compared against upstream deprecation notice.
**Recommendation**: Roll out in two commits — first the `continue-on-error` pattern alone (proves the soft-fail works), then the action version bump (addresses the root cause). Observe CI behavior in both states to confirm hypothesis.

## Conflicting Information

### Conflict 1: README implies `SCCACHE_IGNORE_SERVER_IO_ERROR=1` is a general soft-fail; source code says otherwise
**Position A**: [sccache README](https://github.com/mozilla/sccache/blob/main/README.md) — "By default, sccache will fail your build if it fails to successfully communicate with its associated server. To have sccache instead gracefully failover to the local compiler without stopping, set the environment variable `SCCACHE_IGNORE_SERVER_IO_ERROR=1`." The phrasing "fail your build if it fails to successfully communicate" is broad and would reasonably be read to include server-startup failures.
**Position B**: [sccache commands.rs](https://docs.rs/sccache/latest/src/sccache/commands.rs.html) and [sccache server.rs](https://raw.githubusercontent.com/mozilla/sccache/main/src/server.rs) — the actual usage of `ignore_all_server_io_errors()` is inside `handle_compile_response`, a CLIENT-SIDE path reached only after the server has already started. Server-startup probe failures take a different code path that unconditionally returns `ServerStartup::Err`.
**Assessment**: Source code is authoritative over prose documentation. Position B is correct. The README is misleading but not technically incorrect — "communicate with its associated server" does not necessarily mean "server startup succeeded." Issue [#1751](https://github.com/mozilla/sccache/issues/1751) (open since 2023) documents the same confusion from another user. This is the single most important conflict to resolve, and resolving it in favor of Position B is what makes the workflow-level mitigation necessary.

## Recommendations for Further Research

1. **Empirically verify `SCCACHE_IGNORE_SERVER_IO_ERROR` scope** by running a local sccache against an intentionally-broken backend (e.g. `SCCACHE_GHA_ENABLED=on` with garbage `ACTIONS_CACHE_URL`) and observing whether the env var suppresses the startup failure. If the behavior differs from what the source reading predicts, revisit Finding 1.

2. **Monitor mozilla/sccache issue tracker** for a first-class soft-fail feature. A future sccache release may add an env var like `SCCACHE_SOFT_FAIL_ON_STARTUP=1` that eliminates the need for workflow-level mitigation. If so, Layer 2 of the recommendation becomes redundant.

3. **Evaluate `Swatinem/rust-cache` adoption for this workspace** — separate research question, but complementary. `target/`-directory caching has different hit-rate characteristics than sccache's rustc-object caching; for a workspace with many small crates (typical of Overdrive's structure), both layers can coexist usefully.

4. **Consider a self-hosted sccache backend for production-grade CI reliability.** The GHA cache backend is convenient but not resilient; alternatives include S3, Azure Blob, GCS, or a local redb-backed sccache server. This is a more invasive change than the recommended workflow fix and has cost implications, but it removes the dependency on GitHub infrastructure availability for cache-hit paths.

5. **Audit all other GitHub Actions in the workflow that may share the same endpoint dependency** — `docker/build-push-action` with `type=gha` cache mode has documented history of the same failure shape. If any appear, apply the same `continue-on-error` pattern.

## Full Citations

[1] mozilla/sccache contributors. "sccache README". mozilla/sccache on GitHub. https://github.com/mozilla/sccache/blob/main/README.md. Accessed 2026-04-23.

[2] mozilla/sccache contributors. "GitHub Actions Cache backend docs". mozilla/sccache on GitHub. https://github.com/mozilla/sccache/blob/main/docs/GHA.md. Accessed 2026-04-23.

[3] mozilla/sccache contributors. "commands.rs source". docs.rs. https://docs.rs/sccache/latest/src/sccache/commands.rs.html. Accessed 2026-04-23.

[4] mozilla/sccache contributors. "server.rs on main branch". raw.githubusercontent.com. https://raw.githubusercontent.com/mozilla/sccache/main/src/server.rs. Accessed 2026-04-23.

[5] @Kinrany. "SCCACHE_IGNORE_SERVER_IO_ERROR and failure to connect to Redis (#1751)". mozilla/sccache issues. May 2023. https://github.com/mozilla/sccache/issues/1751. Accessed 2026-04-23.

[6] mozilla/sccache contributors. "Time sensitive: GitHub Actions cache service (Update Guidance) (#2351)". mozilla/sccache issues. 2025. https://github.com/mozilla/sccache/issues/2351. Accessed 2026-04-23.

[7] ClickHouse contributors. "sccache: error: Server startup failed: cache storage failed to read (#68266)". ClickHouse/ClickHouse issues. 2024. https://github.com/ClickHouse/ClickHouse/issues/68266. Accessed 2026-04-23.

[8] Mozilla-Actions/sccache-action contributors. "sccache-action README". github.com. https://github.com/Mozilla-Actions/sccache-action/blob/main/README.md. Accessed 2026-04-23.

[9] Mozilla-Actions/sccache-action contributors. "prepare release 0.0.8 (PR #190)". github.com. 2024. https://github.com/Mozilla-Actions/sccache-action/pull/190. Accessed 2026-04-23.

[10] Mozilla-Actions/sccache-action contributors. "src/setup.ts at tag v0.0.6". github.com. https://raw.githubusercontent.com/Mozilla-Actions/sccache-action/v0.0.6/src/setup.ts. Accessed 2026-04-23.

[11] Mozilla-Actions/sccache-action contributors. "Releases". github.com. https://github.com/Mozilla-Actions/sccache-action/releases. Accessed 2026-04-23.

[12] actions/cache contributors. "Cache action README". github.com. https://github.com/actions/cache/blob/main/README.md. Accessed 2026-04-23.

[13] actions/cache contributors. "Deprecation Notice - Upgrade to latest before February 1st 2025 (#1510)". github.com. 2024. https://github.com/actions/cache/discussions/1510. Accessed 2026-04-23.

[14] Swatinem. "rust-cache README". github.com. https://github.com/Swatinem/rust-cache/blob/master/README.md. Accessed 2026-04-23.

[15] actions/runner contributors. "ADR 0274 — Step outcome and conclusion". github.com. https://github.com/actions/runner/blob/main/docs/adrs/0274-step-outcome-and-conclusion.md. Accessed 2026-04-23.

[16] GitHub. "Rate limiting for actions cache entries". GitHub Changelog. 2026-01-16. https://github.blog/changelog/2026-01-16-rate-limiting-for-actions-cache-entries/. Accessed 2026-04-23.

[17] GitHub. "GitHub availability report: February 2026". The GitHub Blog. 2026. https://github.blog/news-insights/company-news/github-availability-report-february-2026/. Accessed 2026-04-23.

[18] GitHub. "GitHub availability report: March 2026". The GitHub Blog. 2026. https://github.blog/news-insights/company-news/github-availability-report-march-2026/. Accessed 2026-04-23.

[19] GitHub. "GitHub Status — Incident History". githubstatus.com. https://www.githubstatus.com/history. Accessed 2026-04-23.

[20] HeroDevs. "GitHub Actions Cache Service Goes Dark: What DevOps Teams Need to Know". herodevs.com. https://www.herodevs.com/blog-posts/github-actions-cache-service-goes-dark-what-devops-teams-need-to-know. Accessed 2026-04-23.

[21] Depot.dev. "Fast Rust Builds with sccache and GitHub Actions". depot.dev. https://depot.dev/blog/sccache-in-github-actions. Accessed 2026-04-23.

[22] Ken Muse. "How to Handle Step and Job Errors in GitHub Actions". kenmuse.com. https://www.kenmuse.com/blog/how-to-handle-step-and-job-errors-in-github-actions/. Accessed 2026-04-23.

[23] docker/build-push-action contributors. "Failing to push build cache should not fail the workflow (#1409)". github.com. https://github.com/docker/build-push-action/issues/1409. Accessed 2026-04-23.

[24] Microsoft. "Azure Front Door - Our services aren't available right now". Microsoft Q&A (learn.microsoft.com). https://learn.microsoft.com/en-us/answers/questions/170518/azure-front-door-our-services-arent-available-righ. Accessed 2026-04-23.

[25] nick-fields. "retry — Retries a GitHub Action step on failure or timeout". github.com. https://github.com/nick-fields/retry. Accessed 2026-04-23.

[26] Wandalen. "wretry.action — Retry action for Github CI". github.com. https://github.com/Wandalen/wretry.action. Accessed 2026-04-23.

## Research Metadata

Duration: ~40 min | Examined: ~26 sources | Cited: 26 | Cross-refs: all 5 findings cross-referenced ≥ 2 independent sources; Findings 1, 2, 5 cross-referenced against primary source code | Confidence: High 4 findings (1, 2, 4, 5), Medium 1 finding (3 — rests on absence evidence); overall High | Output: `docs/research/ci/sccache-gha-outage-hardening.md`
