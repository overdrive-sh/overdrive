# Finalize documentation review: reconciler-convergence-guarantees (GH #351)

Verdict: **APPROVED after fixes** (the blocking findings below were fixed in the working tree by this review; re-run of typecheck, lint and build is green).

## Findings

| # | Severity | Location | Finding | Status |
|---|---|---|---|---|
| 1 | BLOCKING | blog post "We replayed the model", and `docs/evolution/...md` section 6 item 5 | The post listed "View writes" as a replay correction. The persist-skip (`persistErr` only where the View changes) was already in the spec at DISTILL (`f66c237c8`, D-17), so it was not a replay correction. The corrections in `288f857c0`/`3c90c1a61`/`17ebe5449` and the spec README are: managed-set coupling, dispatch partiality (orphan held SVID on a failed start, landed backend row, programmed dataplane on a failed result-row write), hydrator need recomputed on a landed publish, backoff window ended by a landed publish. | Fixed in the post (four items, orphan SVID folded under partial dispatch). The evolution doc already matches. |
| 2 | BLOCKING | blog post and `reconcilers.mdx` limits | "4.35 seconds" does not trace. Recorded samples at 1,000 workloads: 4.18 to 4.92 s, mean 4.53 s (`spike-scratch/reconciler-resync-cost/evidence/full-*/results.json`). #357 says about 4.6 s. | Changed to "about 4.5 seconds (4.2 to 4.9 across five samples)" in the post. `reconcilers.mdx` says "about 4.5 seconds". |
| 3 | BLOCKING | blog post, `reconcilers.mdx` | Untracked limitation: "a View-write error is retried by the next pass" cannot succeed on real redb, which poisons the handle after a failed commit (#360, open; its scope covers this). The post's recovery claim also said "each of the three faults ... within one period", but the dataplane fault recovers at the backoff deadline, not by the pass. | Post now states what each test shows, that the View-write test clears an injected fault, and cites #360. Added a #360 bullet to the Current limits section of `reconcilers.mdx`. |
| 4 | MAJOR | blog post | Disputed claim (a): the post never mentioned the multi-tick property suite and the evolution doc says it exists. It exists: `crates/overdrive-control-plane/src/convergence_property/` (modules hydrator, workload, svid, service, vm_reclaim, gateway, executor_idempotency, managed_targets_containment). It runs the production `run_convergence_tick` and `resync_reconciler` over Sim ports from generated worlds, converging within 8 rounds then 3 stable rounds, fixed seeds. `noop-heartbeat` and `workflow-lifecycle` carry no-obligation rulings. | Added a paragraph to the post that says exactly this. The evolution doc claim is accurate. |
| 5 | MAJOR | blog post | Undefined terms relied on before explanation: Quint, TLC, quint-connect, broker, interest router. "The spike" is a process label. "is the planned fix" hints at future work (principle 11). | Defined each inline, replaced "spike" with "measurement", replaced the planned-fix sentence with a descriptive link to #270. |
| 6 | MINOR | blog post | The cost measurement used `SimDriver` workloads (probe `hypotheses.md`), with real runtime, broker and stores. The post says "converged workloads" only. | Not changed. Consider adding "with a simulated driver". |
| 7 | MINOR | evolution doc, section 6 item 5 | Says the Quint evidence is recorded against `70946eedb` (dirty) and that `verify-evidence` was not re-run. Honest, but the spec changed after in `17ebe5449`. I did not re-run it either (metal box not used). | Open, accurately disclosed. |
| 8 | MINOR | website test | `bun run test:blog` fails: docs search top hit for the DST query is `/docs/concepts/workload-identity-lifecycle`, not the DST concept page. I could not run the check on a clean HEAD (no stash allowed). It touches search ranking, not these pages' facts, but I have not proven it pre-existing. | Reported, not fixed. |

## Verified against sources

- Slot and spike numbers (5.41 ms, 107 us, 4.7 us / 6.3 us, 5.84 s, 112 ms, 6.2 ms, 6 of 16, 1,735 / 2,734 / 994, about 17% of a core): match `spike/findings-programmed-service-map-read.md`. Two extra slots are 16,381 and 16,382, inner capacity 16,383.
- Pass cost (337/265, 1,537/1,225, 3,037/2,425; 15.2% vs 15.5%; AMD EPYC 8024P, 16 threads, Linux 7.0.0-38): match feature-delta V-3 and `host.txt`. The pass is not above baseline at any N.
- Model: 14 of 14 checks matched, `ProgressByPass` 9,777.6 s (163 min), hazards 14.2 and 14.3 s, `MAXF = 2`, `MAXD = 2`, ten named runs and ten `#[quint_test]` plus `#[quint_run]` in the driver.
- UDP 8 of 8 refused no reply: ADR-0298 and #310. Guest-flow refusal is the `FlowRefusal::Policy` path.
- Issues, all open and in scope: #356 (period), #357 (deploy latency), #270 (scan cost), #275 (host edge wakes), #310 (guest UDP to VIP), #155 (IPv6, cited in `guest-networking.mdx`), #360 (redb).

## Decision story and style

Problem, four rejected alternatives with reasons, the measured three-way comparison, the cost of the chosen path and what is unmeasured are all present. No decision IDs, step IDs, phase names or people-roles in reader-facing text. "We" voice, no semicolons. "Metal" and "Lima" do not appear. Concept and how-to pages follow C-7 (current state). `what-is-ebpf.mdx`, `comparisons.mdx`, `architecture.mdx`, `guest-networking.mdx` edits match the deleted local-backend path.

## Validation (from `website/`)

- `bun run typecheck`, `bun run lint`, `bun run build`: pass. The new post prerenders at `/blog/every-reconciler-gets-re-evaluated-every-30-seconds`.
- `bun run assert:one-index`: pass (37 pages, 12 blog, reachable from .md export, llms.txt and search).
- `bun run test:blog`: FAIL, see finding 8.
- Not run: `bunx opennextjs-cloudflare build` and wrangler deploy checks, `test:mcp`, `test:mcp:analytics`.
