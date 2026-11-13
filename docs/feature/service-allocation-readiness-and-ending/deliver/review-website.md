# Independent review — website documentation and blog

## Metadata

| Item | Value |
|---|---|
| Feature | `service-allocation-readiness-and-ending` |
| Review scope | Website writer commit `5cd87c7a51cb1402d2fbbc792742e73817ed1238`, including the blog, five changed docs pages, and `deliver/website-verification.md` |
| Iteration | 2 — final |
| Review date | 2026-11-13 |
| Reviewer | Independent documentation reviewer |
| Working tree | Unrelated dirty work was preserved. This review changed only this artifact. |

## Verdict

**APPROVED — iteration 2 (final).** The original two factual-precision findings are closed. The blog now distinguishes an omitted startup section from the explicit empty-list opt-out and states the different cleanup categories measured by the positive and negative captures. The bounded changes match the accepted contract and retained logs.

## Review summary

I reviewed the exact website commit and its verification record against `.claude/rules/finalize.md`, `website/CLAUDE.md`, the repository `technical-writer` skill, the accepted D13–D17 decisions, relevant ADRs, the 05-07 implementation review, native verification, and the four listener and boot spike findings.

The docs distinguish guest boot (`Running`) from a committed `Stable` claim, describe the default guest-listener gate for all declared TCP and UDP listeners, retain the explicit empty-list opt-out, assign readiness and liveness to their documented roles, and identify `ServiceLifecycle` and `WorkloadLifecycle` responsibilities. The deploy guide and CLI reference distinguish intent acceptance from a Service verdict, state the 60-second default startup deadline and derived progress deadline, and label `last attempt` as context rather than the give-up cause. The claims about the single native 1,363 ms result, the six-attempt negative run, the probe/guest evidence, and limitations are otherwise scoped to the cited evidence.

The post explains the motivating polling failure, the listener and channel experiments, the serious alternatives, the chosen row and lifecycle ownership, measured costs, and unmeasured production latency. Its prose uses the team voice and does not expose agent roles or wave names. The revised wording preserves that voice, distinguishes absent from explicitly empty startup configuration, and limits cleanup evidence to the measured surfaces. The internal links and website discovery evidence are recorded below.

## Findings

### F1 — Blocking — omitted startup section is conflated with the explicit opt-out

**Location:** `website/content/blog/when-a-microvm-service-is-ready.mdx:28-30`.

The post says, “We also rejected making a Service with no declared startup probe `Stable` as soon as it reached `Running`.” That sentence treats all Services without a declared startup probe alike. The implemented contract distinguishes an **omitted** `health_check.startup` section, which materializes the guest-listener gate, from an **explicit empty** `health_check.startup = []`, which opts out and reaches `Stable` at `Running`. The next sentence gives the fast-exit rationale for the rejected alternative, but does not restore that omitted-versus-empty distinction.

This contradicts the exact operator behavior pinned in ADR-0270 § Decision and § Where the default is applied, and described on the deploy guide at `website/content/docs/how-to/deploy-a-workload.mdx:61-69`. The article should say we rejected treating an **omitted** startup section as an opt-out, or otherwise name the exact absence case. Keep the explicit empty-list opt-out distinct.

**Evidence:** `docs/product/architecture/adr-0270-a-probe-less-service-is-stable-when-the-guest-reports-every-declared-listener-open.md:41-45,60-62`; native/default behavior is also exercised by `docs/feature/service-allocation-readiness-and-ending/deliver/step-05-07-verification.md:53-57`.

**Iteration 1 disposition:** Open. Returned this wording correction to the original website writer. No change to the accepted behavior was requested.

**Iteration 2 disposition:** **Closed.** Commit `edd592233a3406d96999aab9917219a06de67fb0` says an omitted `health_check.startup` section selects the guest-listener gate and explicitly preserves `health_check.startup = []` as the opt-out. This matches ADR-0270 and the deploy guide. The feature behavior and its wording are now distinct.

### F2 — Blocking — cleanup summary implies broader negative-capture coverage than recorded

**Location:** `website/content/blog/when-a-microvm-service-is-ready.mdx:149-150`.

The post says, “Both captures reported zero owned resource deltas after cleanup.” The positive capture records `vm=0 probe=0 network=0 cgroup=0 run-directory=0 mount=0 loop=0 preparation=0`. The negative capture records `vm=0 network=0 cgroup=0 run-directory=0 mount=0 loop=0 preparation=0 serve=0` and has no probe-ownership delta. The broad sentence does not name the different measured surfaces and can be read as claiming that the negative capture also established a zero probe delta.

Keep the cleanup claim at the evidence's actual scope. For example, name the categories separately for the two captures, or say only that both reported zero deltas for their respective measured cleanup surfaces. Do not imply a negative-capture probe-ownership measurement.

**Evidence:** `docs/feature/service-allocation-readiness-and-ending/deliver/step-05-07-verification.md:45-49`; retained captures `.context/step-05-07-native/rebased-20261112/positive/cleanup-resource-delta.log` and `.context/step-05-07-native/rebased-20261112/negative-retry/cleanup-resource-delta.log` show the category lists directly.

**Iteration 1 disposition:** Open. Returned this precision correction to the original website writer. No cleanup or lifecycle implementation change was requested.

**Iteration 2 disposition:** **Closed.** Commit `edd592233a3406d96999aab9917219a06de67fb0` lists the positive capture's zero deltas for VM, probe ownership, network, cgroup, run-directory, mount, loop, and preparation. It separately lists the negative capture's zero deltas for VM, network, cgroup, run-directory, mount, loop, preparation, and serve, and explicitly says probe ownership was not measured. The wording matches both retained cleanup logs.

## Evidence and verification review

| Area | Result | Evidence and limit |
|---|---|---|
| Implemented lifecycle and probe behavior | **PASS** | ADR-0270, ADR-0268/0269, ADR-0255/0256/0266, ADR-0261/0262/0263/0264/0265/0267, the accepted feature delta, and the approved 05-07 implementation review support the claims about all declared listeners, `Running`/`Stable`, readiness/liveness, stop/restart ownership, the progress deadline, recorded verdict reads, and labelled `last attempt` context. |
| Spike and decision narrative | **PASS** | The post's cited measurements match the retained findings: 13 TCP and 13 UDP hook cases, 100,000 ordered beacon lines and 138.8 μs p99 round trip, 10 corruptions among 200,000 lines with unguarded writers, 185/300 late-listen cases without a final drain, 1,000/1,000 workload reports with no init-originated report, +0.88 ms p50 for same-object hooks and placement, +20.4 ms p50 for another object load, and +0.36 ms p50 for listener additions against a matching baseline. These are spike measurements on their named substrates, not production SLOs. |
| Native operator proof | **PASS** | The retained default-feature captures show one positive `Stable` result in 1,363 ms and the negative path's 90-second progress timeout, six allocation rows, missing `tcp:18998`, and later recorded `StartupProbeFailed` on identical redeploy. The revised cleanup claim names each capture's measured surface and the negative capture's probe-ownership limit. |
| Product documentation | **PASS** | The five changed documentation pages describe current behavior and operator use. They state listener/report limits, the explicit opt-out, readiness and liveness roles, and timeout semantics. The guide and CLI reference do not claim that progress timeout stops restart reconciliation. |
| Technical-writer and voice rules | **PASS** | The blog leads with the service readiness problem, owns decisions with “we,” explains mechanisms before relying on them, names tradeoffs, separates spike and native measurements, and states current limits. It does not mention “the user,” agent roles, or wave names in its prose. The revised opt-out wording preserves these requirements. |
| Website build and authored-content discovery | **PASS as recorded** | `website-verification.md:38-47` records successful typecheck, lint, build, one-index check, direct Workerd checks, blog/MCP discovery checks, all 45 internal Markdown links, and `git diff --check`. The remediation section at `:61-75` records focused MDX typecheck, lint, and diff checks. This review did not rerun the website toolchain. |
| Existing website acceptance assertion | **Pre-existing failure accurately recorded** | The record at `website-verification.md:49-59` says the three scripts fail on the fixed `deterministic simulation` top-hit assertion and that untouched-HEAD `test:blog` reproduced the same assertion and first result. The checked-in `test-blog.ts:138-254` performs the changed post's blog-list, render, search/MCP, Markdown identity, and draft-exclusion checks before its later fixed docs-search assertion at `:272-279`. The scripts' stopped later assertions are explicitly not claimed as passing. The failure is recorded as pre-existing rather than hidden or waived. |
| Publication | **Not performed** | The verification record states no deploy or upload ran. This review does not authorize publishing. |

## Iteration record and remediation disposition

### Iteration 1 — initial review

**Verdict: NEEDS_REVISION.** Findings F1 and F2 are open and returned to the original website writer for bounded wording corrections. The reviewer made no website-content changes and ran no tests or mutation checks. After the writer's revision, re-review this same website scope and update this artifact with the full second-iteration result before the feature is finalized.

### Iteration 2 — wording remediation

**Remediation commit:** `edd592233a3406d96999aab9917219a06de67fb0` (`docs(service-readiness): clarify startup opt-out and cleanup evidence`). The commit changes only the blog wording and this feature's website-verification record. It changes no docs page, route, link, metadata, product code, or test.

| Check | Result | Evidence and limit |
|---|---|---|
| F1 — omitted versus explicitly empty startup configuration | **PASS** | Blog lines 28–30 name omitted `health_check.startup` as the guest-listener default and `health_check.startup = []` as the opt-out. This matches ADR-0270 and the deploy guide. |
| F2 — per-capture cleanup scope | **PASS** | Blog lines 151–154 name separate positive and negative category lists and explicitly state that the negative capture did not measure probe ownership. Both lists match the retained cleanup logs. |
| Remediation scope | **PASS** | The commit stat and diff show only `when-a-microvm-service-is-ready.mdx` and `website-verification.md` changed. The prose change is bounded to F1 and F2. |
| Focused validation | **PASS as recorded** | `website-verification.md:71-73` records `git diff --check`, a focused source audit, `bun run typecheck`, and `bun run lint`. I independently ran `git diff --check` over the writer commit range and it returned no diagnostics. Typecheck and lint were not rerun because the change is wording-only and the committed focused result is recorded. |
| Existing search assertion | **Remains accurately recorded** | The prior baseline reproduction and the fact that later assertions did not run remain unchanged at `website-verification.md:49-59`. No new claim says the failed assertion passed. |
| New findings | **None** | The limited remediation introduced no additional factual or scope issue. |

**Iteration 2 verdict: APPROVED.** F1 and F2 are closed. Website finalization may use the existing full-check evidence plus the focused remediation checks recorded in `website-verification.md`. Publication remains unauthorized by this review.
