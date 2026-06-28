# Adversarial review — step 02-02 (Walking skeleton — `getent <job>.svc.overdrive.local` → stable F → live backend → inter-agent mTLS)

- **Artifact:** roadmap step `02-02` — *the dial-by-name walking-skeleton vertical slice (S-DBN-WS + S-DBN-SINGLE-SRC GREEN; S-DBN-WS-STABLE + S-DBN-CHURN deferred `#[ignore]` to #249), plus the two production surfaces the slice revealed: the REV-5 OUTPUT-hook leg-B interception datapath (`mtls_intercept.rs`) and the `workload_addr` forward-carry fix (`action_shim/mod.rs`).*
- **Commit range:** `b413736e..HEAD` (`784e6c4c` scaffolds → `82202670` REV-5 datapath → `4fd34548` plaintext-client test-model correction → `99325953` re-size → `b8b7a7ad` REV-6 reconcile).
- **Reviewer:** `nw-software-crafter-reviewer` (Opus, adversarial "different-fox" posture). Every blocking/significant finding was independently re-traced against source/tests by a second verifier subagent and then re-confirmed firsthand by the lead reviewer (the C1 leak by reading `install_inbound_tproxy`, D1 by `grep`'ing the control-plane fixtures, D2 by enumerating the `// mutants: skip` sites and `.cargo/mutants.toml`). Subagent claims were not trusted unverified.
- **Verdict:** **NEEDS_REVISION** — 2 blocking (D1 the `workload_addr` forward-carry branch — the step's central control-plane behavior change — has no default-lane test, so its mutants survive; D2 the new REV-5 nft-shim `// mutants: skip` comments suppress nothing and have no `exclude_re` backing, so the deferred mutation gate will report them MISSED), 4 non-blocking (N1 dual-chain partial-install rule leak, N2 post-hoc test-only→production reconciliation, N3 S-DBN-SINGLE-SRC overlaps S-DBN-WS, N4 spike-findings capture provenance).

> **The core dial-by-name loop is genuinely proven end-to-end through `serve` + `deploy` on the real datapath — this is a real vertical slice, not a dead mechanism, and the hard-won test-model correction is exactly right.** S-DBN-WS and S-DBN-SINGLE-SRC drive name → stable F (∈ 10.98.0.0/16, asserted first and separately) → re-keyed `MtlsResolve` translation → live backend → inter-agent leg-B↔leg-C mTLS, all through the production composition root with **no hand-installed production effect** (boot is `dataplane_override: None` + real `EbpfDataplane`; the dial uses the production egress rule; nothing test-side binds :53, writes a resolv.conf, allocates F, programs a map, or installs the capture). The corrected egress test model — the dialer speaks **plaintext** and the mTLS proof is the `lo:SERVICE_PORT` 0x17-record oracle on the inter-agent wire, **not** a client TLS handshake — is precisely the model CLAUDE.md now codifies, and it was reached honestly via a population-diff RCA. The REV-5 datapath was **design-first**: spike-proven WORKS on a real kernel (falsification-tested against a `type filter` decoy), user-PROMOTE'd, architect-pinned to an exact contract in feature-delta REV-5 / ADR-0072 REV-5, and the code adds **zero surface beyond the two sanctioned consts** — the *opposite* of the ADR-0065 invent-surface failure mode. The two deferred ATs carry full real bodies and `#[ignore]` reasons that name the genuine blocker (the missing restart-after-stop verb), tracked in real OPEN issues #249/#248 the user authored; GREEN is honestly logged `EXECUTED FAIL (2/4)`.
>
> The blocking issues are **not** about the loop being wrong — they are the *same "green suite over an unmet mutation gate" class that blocked 02-00 (D3) and 02-01 (D1), now recurring a third time*. 02-02 landed real mutable control-plane logic (the `is_stable` forward-carry branch) and real new nft-shim I/O, recorded the mutation gate "ENVIRONMENTALLY BLOCKED → MERGE-GATE", and that deferral **masks two gaps that do not need the flaky Lima environment to close**: a forward-carry branch with no in-lane test (D1) and skip comments that suppress nothing (D2). Both are cheap and in-scope; close them before the merge-gate run, or it cannot honestly pass.

---

## Per-criterion verdicts

| # | Criterion | Verdict | Evidence / litmus |
|---|---|---|---|
| 0 | **S-DBN-WS** (name → stable F → translate → live backend → inter-agent mTLS) | **MET (GREEN)** | `deployed_workload_resolves_peer_stable_frontend_and_hop_is_mtls` (`dns_responder_walking_skeleton.rs:1315`). Resolves F via `getaddrinfo` from the client's production netns, asserts F∈10.98.0.0/16 **first and separately** (`:1379-1388`), then plaintext dial + byte-exact round-trip (`:1408-1422`), then `assert_inter_agent_hop_is_mtls` separately (`:1428`). Boot is real `EbpfDataplane`, no hand-installed effects. **Praise P1/P2.** |
| 1 | **S-DBN-WS-STABLE** (F byte-stable across alloc cycle) | **DEFERRED — honest** | `#[ignore = "…#249…"]` (`:1685`), full real body (asserts `alloc_b1 != alloc_b2`, `f1_again == f1`, post-cycle mTLS, F never ∈10.99/16). Blocker (sticky operator-stop; no replace/restart verb) is accurate and tracked in OPEN #249. Sanctioned `#[ignore]` form (testing.md). |
| 2 | **S-DBN-SINGLE-SRC** (answered F is the addr `MtlsResolve` translates) | **MET (GREEN), with overlap** | `answered_frontend_is_the_addr_mtls_resolve_translates_to_a_mesh_backend` (`:1549`). Adds `frontend != server_backend` byte-distinctness (`:1597`). Indirect oracle (resolve not called directly — live allocator not on `ServerHandle`), honestly documented. See **N3** (overlaps criterion 0's production path). |
| 3 | **S-DBN-CHURN** (in-flight churn fails fast, no `sock_destroy`) | **DEFERRED — honest** | `#[ignore = "…#249…"]` (`:1855`), full real body measuring fail-fast elapsed vs `CHURN_BOUND`. Same #249 blocker (the "next connect lands B2" half needs the replace verb). |
| 4 | **Vertical-slice litmus** (no test installs a production effect) | **MET — verified** | Boot fixture (`Skeleton::boot`, `:561`) composes `run_server_with_obs_and_driver` with `dataplane_override: None`; `dial_frontend_in_netns` (`:361`) docstring + body confirm "No test rule is installed" — capture is the production egress rule. Confirmed firsthand against the keystone/transparent-mtls anti-pattern. **Praise P3.** |
| 5 | **getent-not-dig** (K2) | **MET** | `resolve_frontend_in_netns`/`parse_getent_v4` (`:294-334`) drive `getent`/`getaddrinfo` from inside the netns; the K2 failure message names both culprits (source-pin OR healthy-gate) (`:1371-1376`). No `dig`-only assertion. |
| 6 | **Merge gate** (pinned-6.18 Tier-3 matrix, ADR-0068) | **OPEN → DEVOPS** | Correctly a DEVOPS/Tier-3 obligation, not closeable at this step. GREEN tests record `uname -r` (dev-Lima 7.0.0-22-generic); the criterion itself states dev-Lima is necessary-but-not-sufficient. No action for the crafter; flag carried to DEVOPS. |
| 7 | **Pinned surface** (RECONCILED — REV-5 datapath landed) | **MET — design-first verified** | Spike `findings-output-hook-legb.md` (WORKS, falsification-tested), user PROMOTE (`wave-decisions.md`, 2026-06-27), feature-delta REV-5 + ADR-0072 REV-5 pin the exact contract (`IP_FREEBIND=15`, `NFT_OUTPUT_CHAIN`, `type route hook output`, divert shape, dual-chain teardown, `Vec<(&'static str,u64)>` guard). Diff adds **zero** surface beyond it: no new public fn, **no new error variant**, exactly the two sanctioned consts. **Praise P4.** |
| 8 | **RED convention** | **MET** | `#[ignore]` reasons name #249 (the sanctioned external-dependency form); GREEN honestly `EXECUTED FAIL (2/4)` in execution-log. No green-on-RED, no fake stubs (deferred bodies are real). |
| 9 | **Mutation gate ≥80%** (RECONCILED — env-blocked) | **NOT-MET** | New mutable logic shipped with the gate unrun. The deferral is honest for the pure parsers (they *are* covered in-lane), but **masks D1 + D2** below — gaps that do not need the Lima environment to close. |

---

## Blocking issues

### D1 — `issue (blocking)`: the `workload_addr` forward-carry branch — the step's central control-plane behavior change — has no default-lane test; its mutants survive

02-02 made `workload_addr` a **required** parameter of `build_alloc_status_row` and forward-carries it on the `FinalizeFailed { Stable }` arm (`crates/overdrive-control-plane/src/action_shim/mod.rs`):

```rust
let prior_workload_addr = prior_row.workload_addr;          // ~:1020
…
// is_stable = matches!(terminal, Some(TerminalCondition::Stable { .. }))
if is_stable { prior_workload_addr } else { None },          // ~:1083
```

This is the **load-bearing fix** of the whole step — the diff's own comment says it closes "the walking-skeleton backend-drop the GAP-9 guard only HALF-closed", and #248 documents that the drop made the bridge fall back to `host_ipv4` and the dial-by-name egress translation target an unreachable addr. Yet **no default-lane test pins the branch**:

- Repo-wide, `crates/overdrive-control-plane/{src,tests}` contains **zero** `workload_addr: Some(...)` fixtures. `seed_running_row` hardcodes `workload_addr: None` (`tests/integration/alloc_netns_lifecycle.rs:567`; second seed at `:161`).
- The only tests dispatching `FinalizeFailed` through the real `action_shim` — `finalize_failed_stable_does_not_tear_down_live_running_alloc` and `finalize_failed_genuine_failure_still_tears_down_alloc` (`alloc_netns_lifecycle.rs`) — assert **`row.state`** only (`:408`, `:633`, `:709`); **none asserts `row.workload_addr`**.

Because every prior row is `None`, **both arms of `if is_stable { prior_workload_addr } else { None }` evaluate to `None`** — the swap-arms mutant, the always-`None` mutant, the always-`prior` mutant, and the `==`→`!=` mutant in the `matches!` discriminant **all survive**. This is exactly the bug class the change exists to prevent, shipped without a regression test pinning it closed.

Note the behavior *is* defended **indirectly at Tier-3**: the healthy server in S-DBN-WS passes its probes → `FinalizeFailed { Stable }` keeps it Running, and `deploy_and_wait_stable_backend` polls specifically for a per-instance addr ∈ 10.99.0.0/16 (`:1457-1470`), so a broken forward-carry → `host_ipv4` fallback → S-DBN-WS RED. But that test is `is_root()`-gated, Lima-only, and `integration-tests`-gated — i.e. it lives in the **same flaky environment the mutation gate is deferred for**, so it is precisely *not* a reliable killer for the action_shim mutants.

**Required:** add a default-lane (no root, no Lima) unit-shaped test that seeds a `workload_addr: Some(addr)` Running prior row, dispatches `FinalizeFailed { Stable }`, and asserts the successor row keeps `Some(addr)`; and dispatches a genuine `FinalizeFailed { Failed }`, asserting it drops to `None`. That kills the `is_stable` branch mutants independent of the Lima environment, and pins the exact invariant #248 is the defense-in-depth for.

### D2 — `issue (blocking)`: the new REV-5 nft-shim `// mutants: skip` comments suppress nothing and have no `exclude_re` backing — the deferred mutation gate will report them MISSED

Per `.claude/rules/testing.md`, a bare `// mutants: skip` **comment suppresses nothing** — only the `#[mutants::skip]` attribute or a `.cargo/mutants.toml` `exclude_re` entry actually excludes. In `crates/overdrive-worker/src/mtls_intercept.rs` every skip is the **bare-comment form** (zero attributes):

| Line | Function | Pre-existing? | `exclude_re` entry? |
|---|---|---|---|
| 546 | `sweep_per_workload_tproxy_rules` | yes | **yes** (backed) |
| 759 | `find_egress_rule_handle` | yes | **yes** (backed) |
| **578** | `sweep_one_chain` (NEW) | no | **NO** |
| **796** | `chain_has_leg_s_exemption` (NEW signature) | no | **NO** |
| **956** | `find_output_divert_rule_handle` (NEW) | no | **NO** |
| — | `list_named_chain` (NEW, `:839`) | no | no skip + **NO** entry |

The author copied the *style* of the pre-existing siblings (bare comment) but not the mechanism that makes them work (the matching `exclude_re` line). `grep` of `.cargo/mutants.toml` for the four new names returns nothing. These three shims *are* genuinely untestable in the default lane (each delegates to a pure, mutation-covered `*_in_dump` parser then shells out to real `nft`), so they *should* be excluded — but their whole-function-body mutants (`Ok(0)`, `Ok(false)`, `Ok(<handle>)`) will surface as **MISSED** on the per-PR `--diff` run, denting the kill-rate exactly as the testing.md rule warns. This is an in-scope fix (a code-quality/gate completeness finding, not a deferral) per CLAUDE.md.

**Required:** add justified `exclude_re` entries (mirroring the sibling pattern, each carrying its per-exclusion rationale) — or `#[mutants::skip]` attributes — for `sweep_one_chain`, `chain_has_leg_s_exemption`, `find_output_divert_rule_handle`, and `list_named_chain`, before the merge-gate mutation run. (The pure parsers `output_divert_handle_in_dump` and the widened `per_workload_rule_handles_in_dump` are correctly *not* skipped — they have co-located killing tests; see Praise P5.)

> D1 + D2 are the same root: the mutation gate is deferred "environmentally blocked", but the deferral is doing more work than env-flakiness justifies — D1 is an in-lane gap that needs no Lima at all, and D2 is an undeclared suppression. The gate cannot be honestly closed at merge until both land.

---

## Non-blocking

### N1 — `suggestion (non-blocking)`: REV-5 widens the dual-chain partial-install window — an appended rule can leak with no guard

`install_inbound_tproxy` (`mtls_intercept.rs:293-373`) now performs **two** fallible `run_nft` appends (prerouting tproxy `:303`, output divert `:339`) and **two** handle recoveries (`:368-369`) *before* the `TproxyInterceptGuard` is constructed (`:370`). Every `?` before line 370 returns early with the rule(s) already committed to the kernel and **no guard to remove them**:

- output-divert append (`:339`) fails after the prerouting append (`:303`) succeeded → the prerouting rule is **leaked**;
- either handle recovery (`:368-369`) fails → **both** rules leaked.

The pre-existing single-rule path had the same shape for handle-recovery failure; REV-5 **adds** the second-append window. The leak is bounded — the §5 boot-recovery sweep (`sweep_per_workload_tproxy_rules` → both chains via `sweep_one_chain`) reaps orphans on the *next* control-plane restart — and `nft` rarely fails mid-sequence (EPERM / lock / missing binary), which is why this is non-blocking. But within a single boot a failed install leaves a stale divert rule for that virt's `(daddr,dport)` until restart. Either add an error-path cleanup of the appended rules (RAII scope-guard over the partial install), or add a one-line comment at `:362` stating the §5 sweep is the accepted reaper (consistent with the codebase's converge-on-boot posture) so the next reader does not read the gap as an oversight.

### N2 — `thought (non-blocking)`: a test-only step grew real production scope, reconciled a day later

The step was framed "adds NO new production type" at `b413736e`; it landed a real datapath + a control-plane behavior change. The reconciliation (REV-6, `b8b7a7ad`, 2026-06-28) is **honest and complete** — it states plainly that "02-02 **diverged** from its original test-only spec" — but it is *post-hoc*. The `execution-log.json` `02-02 / RED_UNIT` SKIP rationale ("test-only step adds no new below-port mutable production logic", `2026-06-27T03:15:47Z`) is now contradicted by the landed mutable logic, and was never corrected in the log. Per `feedback_behavior_change_must_mark_stale_adjacent_docs`, the honest move is to surface the resize *at the point of divergence*, not a day later — the design-first work itself was clean (spike → PROMOTE → architect-pin all preceded the code), so this is purely a step-framing/log-honesty note, not a divergence of substance.

### N3 — `nitpick (non-blocking)`: S-DBN-SINGLE-SRC re-exercises S-DBN-WS's exact production path

`answered_frontend_is_the_addr_mtls_resolve_translates_to_a_mesh_backend` (`:1549`) drives the identical production sequence as S-DBN-WS (deploy server+client → resolve → plaintext dial → byte-exact round-trip + `assert_inter_agent_hop_is_mtls`); its only distinct claim is `assert_ne!(frontend, server_backend)` (`:1597`). The indirect oracle is justified (the live `FrontendAddrAllocator` isn't exposed on `ServerHandle`), and the byte-distinctness assertion is genuinely additional — but the two tests pay two full boot+deploy Tier-3 fixtures for one extra `assert_ne!`. A candidate for the `nw-test-optimizer`'s eye: fold the `frontend != backend` distinctness assertion into S-DBN-WS and drop the second fixture, or keep both and accept the cost as documentation. Non-blocking either way.

### N4 — `question (non-blocking)`: spike-findings capture provenance

`spike/findings-output-hook-legb.md` discloses that the spike crafter's file-write was guard-blocked and the orchestrator re-emitted the program stdout "verbatim". The findings do paste real output and a falsification control (the `type filter` decoy counter-test), which satisfies the spirit of spike.md's "paste real output, never narrate" — but the capture is one hop removed from a crafter-authored artifact. Worth confirming the re-emitted output matches what the probe actually produced (a re-run on the dev-Lima kernel would settle it), since the entire REV-5 datapath rests on that WORKS verdict. Not blocking — the verdict is corroborated by the GREEN S-DBN-WS round-trip on the real datapath.

---

## Praise

- **P1 — the corrected egress test model is exactly right, and hard-won.** `TestPkiHandle::dial` (`:838`) is genuinely plaintext (`TcpStream` + `write_all(REQUEST)` + read `RESPONSE`, no rustls), and the mTLS proof is the inter-agent `lo:SERVICE_PORT` 0x17-record oracle (`WireCapture` `:929`, `assert_inter_agent_hop_is_mtls` `:1160`, both-directions + zero-cleartext). This is precisely the model CLAUDE.md now codifies (§ "East-west mTLS tests — the egress DIALER speaks PLAINTEXT"), reached via a population-diff RCA — the *correct* response to a multi-day stall, not a contorted green.
- **P2 — resolve asserted first and separately from mTLS.** The K2 two-culprits honesty (prior reviewer suggestion #1) is implemented faithfully: F∈10.98/16 and `!=`10.99/16 are pinned before the dial, with a failure message naming both source-pin and healthy-gate (`:1370-1388`).
- **P3 — vertical-slice integrity holds under adversarial reading.** No test binds :53, writes a resolv.conf, allocates F, programs a map, or installs the egress capture — the boot fixture composes the real production path and the dial rides the production rule. This is the transparent-mtls (#236) anti-pattern *avoided*, verified line-by-line.
- **P4 — design-first on new production surface, the inverse of the ADR-0065 trap.** The REV-5 datapath was spike-proven (real kernel, falsification-tested), user-PROMOTE'd, architect-pinned to an exact contract, and implemented with **zero invented surface** (no new public fn, no new error variant, exactly the two sanctioned consts, the pinned guard shape). The roadmap's "no new typed-error variant / no other new public type" claims verify true against the diff.
- **P5 — the new pure parsers carry their own killing tests.** `output_divert_handle_in_dump` and the widened `per_workload_rule_handles_in_dump` have co-located default-lane tests (`:1714`, `:1744`, `:1768`) that kill the dangerous mutants — including the dropped-`meta mark set` discriminator that would otherwise recover the shared leg-S exemption's handle and tear down shared infra. The mutation deferral genuinely does *not* leave these unprotected.

---

## Required before merge (close-out checklist)

1. **D1** — default-lane unit test pinning the `FinalizeFailed { Stable }` `workload_addr` forward-carry (seed a `Some` prior row; Stable keeps it, genuine-Failed drops to `None`).
2. **D2** — `exclude_re` entries (or `#[mutants::skip]` attributes) for `sweep_one_chain`, `chain_has_leg_s_exemption`, `find_output_divert_rule_handle`, `list_named_chain`, each with a one-line justification.
3. **Mutation gate** — once D1/D2 land, run the deferred per-PR diff-scoped gate over `mtls_intercept.rs` + `action_shim/mod.rs` (the merge-gate the roadmap names) and record kill-rate ≥80%; sweep leaked Lima cgroups/XDP/nft first (project memory) to get past the flaky baseline.
4. **N1** (advisory) — decide the partial-install posture (cleanup-on-error vs document §5 as the reaper).
5. **Criterion 6** — carry the pinned-6.18 Tier-3 re-confirmation to DEVOPS.

The core loop is sound and the design discipline is exemplary; this is a close-out-the-gate revision, not a rework.

---

## Resolution

Close-out of the NEEDS_REVISION checklist by `@nw-software-crafter` (review-resolution dispatch; no execution-log writes, 02-02 GREEN/COMMIT already logged). Verdict is NOT self-stamped — a different-fox re-verification is the orchestrator's call; this records the resolution facts only.

### D1 (BLOCKING) — RESOLVED. Default-lane PBT pins the `workload_addr` forward-carry branch.

Added `crates/overdrive-control-plane/tests/acceptance/finalize_failed_forward_carries_workload_addr.rs` (wired into `tests/acceptance.rs` beside the sibling ungated action-shim dispatch test `release_service_vip_dispatch`). Default lane — no root, no Lima-only gate, **no `integration-tests` feature** (runs under bare `cargo nextest`). Drives the production `action_shim::dispatch` driving port against a seeded `Running` prior `AllocStatusRow` carrying `workload_addr: Some(addr)`, asserting on the driven-port boundary (the successor row written to `SimObservationStore`). Two property tests (proptest, the codebase's PBT tool):

- `finalize_failed_stable_keeps_the_running_alloc_workload_addr` — for any IPv4 `addr`, a `FinalizeFailed { Stable }` keeps the row `Running` AND keeps `Some(addr)`. Kills always-`None`, swap-arms, and the `matches!` `==`→`!=` discriminant mutant.
- `finalize_failed_genuine_terminal_drops_workload_addr` — for any IPv4 `addr` and any genuine terminal (`Failed` / `Completed` / `BackoffExhausted`), the row lands `Failed` AND drops to `None`. Kills always-`prior` and swap-arms.

**Falsifiability litmus (proves not Testing Theater):** temporarily mutating the production branch to `if is_stable { None } else { None }` (always-`None`) turned `finalize_failed_stable_keeps…` RED (`left: None, right: Some(0.0.0.0)`); reverted to the correct `if is_stable { prior_workload_addr } else { None }` and both tests GREEN. No public API surface invented (CLAUDE.md) — the existing `dispatch` signature + `mtls_worker: None` / fresh `NetSlotAllocator` (the genuine-terminal teardown is a clean no-op when the worker is absent) sufficed.

### D2 (BLOCKING) — RESOLVED. Four `exclude_re` entries back the new nft-shim skips.

Added to `.cargo/mutants.toml` (mirroring the `sweep_per_workload_tproxy_rules` / `find_egress_rule_handle` sibling entries, each carrying a per-exclusion rationale), for the four NEW REV-5 nft-I/O shims in `crates/overdrive-worker/src/mtls_intercept.rs`:

- `replace sweep_one_chain -> Result<usize> with Ok` (pure decision `per_workload_rule_handles_in_dump`, mutated + killed)
- `replace chain_has_leg_s_exemption -> Result<bool> with Ok` (pure decision `dump_has_leg_s_exemption`, mutated)
- `replace find_output_divert_rule_handle -> Result<u64> with Ok` (pure decision `output_divert_handle_in_dump`, killed — Praise P5)
- `replace list_named_chain -> Result<String> with Ok` (pure decision `stderr_reports_absent_chain`, mutated)

The bare `// mutants: skip` comments are retained on all four functions as human-facing documentation (now each names the `.cargo/mutants.toml` entry as the actual mechanism per testing.md); `list_named_chain`, which had none, gained one. The pure parsers `output_divert_handle_in_dump` / `per_workload_rule_handles_in_dump` are deliberately NOT skipped (co-located killing tests).

### N1 (ADVISORY) — RESOLVED via the posture comment (codebase converge-on-boot consistency).

Added a comment at the REV-5 dual-append partial-install site in `install_inbound_tproxy` (`mtls_intercept.rs`, before step (4)) stating the §5 boot-recovery sweep (`sweep_per_workload_tproxy_rules` → `sweep_one_chain` over both chains) is the accepted reaper of any orphaned rule from a `?` between the two appends — consistent with the codebase's converge-on-boot posture (#234), so the next reader does not read the bounded leak as an oversight. No RAII unwind added (the comment matches the posture and is sufficient for a non-blocking finding).

### Mutation gate (merge-gate criterion 9) — worker PASS; action_shim 2/3 with a surfaced pre-existing residual.

Run per-PR diff-scoped (`--diff origin/main --features integration-tests`) over both production files, after a comprehensive Lima sweep (cgroups / netns / **XDP / veth** / nft / procs).

**`overdrive-worker` / `mtls_intercept.rs` — PASS, 100% kill rate (17/17 caught).** The first run surfaced 5 MISSED mutants (76.2%), none of them D2 functions; resolved as:
- **2 pure-parser kill gaps closed (the substantive fix):** the REV-5 dual-conjunct branch in `per_workload_rule_handles_in_dump` (`… && line.contains("meta mark set ") && line.contains("tcp dport ")`) had two surviving `&&`→`||` mutants. Added `classifier_output_divert_branch_requires_all_three_conjuncts` (a fixture of partial-conjunct lines that must ALL be excluded); litmus-verified it turns RED under each `&&`→`||` flip, GREEN on the correct triple-`&&`. These parsers stay mutated and are now killed.
- **3 real-`nft`-I/O mutants justifiably excluded** (same class as the existing nft-shim entries — unobservable default-lane, killable only by the real-kernel Tier-3 ATs): `sweep_per_workload_tproxy_rules`'s REV-5 `+`→`-`/`+`→`*` count-sum (both operands from real-`nft` shell-outs; `0+0 == 0-0 == 0*0` in-lane), and `ensure_shared_routing_infra`'s `delete !` on the REV-5 OUTPUT-chain head-exemption idempotence guard (flips a real-`nft` insert's firing condition).

**`overdrive-control-plane` / `action_shim/mod.rs` — 2/3 caught (66.7%); the step's own change is CAUGHT.** Three in-diff mutants:
- `dispatch_single -> Ok(())` — **CAUGHT** by the new D1 acceptance test (the whole-body replacement skips the `FinalizeFailed` write → no successor row → D1's `expect("a successor … must exist")` fires). This is 02-02's actual forward-carry change.
- `fail_closed_on_netns_provision -> Ok(())` — **CAUGHT** by the existing Tier-3 `provision_failure_drives_alloc_to_failed_row_not_pending_retry`.
- `fail_closed_on_mtls_install -> Ok(())` — **MISSED.** **Pre-existing, out of 02-02 scope (surfaced for user decision).** This helper is from transparent-mtls-host-socket (step 06-03), NOT dial-by-name 02-02. Its only diff-touch is the required-parameter wiring `None,` added to its `build_alloc_status_row` call (a Failed row carries no addr — correct); cargo-mutants' `--in-diff` therefore includes the whole-body mutant from the changed-region window. It has **no killer test** predating 02-02 (no integration test forces an `MtlsInterceptInstallError` on a Running alloc and asserts the fail-closed Failed row). Closing it requires mtls-intercept-install fault-injection infrastructure the review did not name in D1 (forward-carry) or D2 (nft shims). **Surfaced as a blocker for the user/orchestrator to decide** (fix in a follow-up vs. expand scope) — not faked, not unilaterally issue-tracked.

#### Bonus fix — pre-existing test-isolation defect that env-blocked the action_shim baseline.

The first two `action_shim` full-feature runs failed the **unmutated baseline** with `IfaceXdpSlotBusy { iface: "ovd-veth-cli" }` — independently reproduced as a parallel race between the two `dns_responder_walking_skeleton` WS tests (both boot a real `EbpfDataplane` on the FIXED `ovd-veth-cli` / `ovd-veth-bk` ifaces + the shared root cgroup, but were MISSING from the `host-kernel-shared` `max-threads = 1` nextest group). 02-02 introduced the real-dataplane WS tests without registering them in the single-writer group. Fixed in `.config/nextest.toml` by adding `test(dns_responder_walking_skeleton)` (by-MODULE, rename-proof) to the control-plane root-attacher block — verified via `cargo nextest show-config test-groups --profile mutants` that both WS tests now resolve into `host-kernel-shared`, and that the two tests pass together after the fix (the baseline now passes under the mutation harness, lifting the env-block the review's gate deferral anticipated).

### Other findings (no code action, as scoped).

- **N2 / N3 / N4** — acknowledged, no action (N2 already reconciled in REV-6, log is append-only + Bash-locked; N3 is a test-optimizer candidate, not a defect; N4's WORKS verdict is corroborated by the GREEN S-DBN-WS round-trip).
- **Criterion 6** — carried to DEVOPS (pinned-6.18 Tier-3 re-confirmation).

### Regression check.

594 default-lane tests (`overdrive-worker` lib + `overdrive-control-plane` acceptance) pass after all edits; `clippy -D warnings` clean on both crates (default + `integration-tests`).

---

## Independent second-opinion pass (different-fox re-verification, `nw-software-crafter-reviewer` Opus + lead re-trace)

**Posture:** the Resolution section above correctly declines to self-stamp its verdict and defers to a different-fox re-verification. This is that pass. An independent `nw-software-crafter-reviewer` re-traced every Resolution claim to current source (HEAD `b9365879`); the lead reviewer then re-confirmed the verdict-critical evidence firsthand. Nothing was trusted from the Resolution text.

### Verdict: **APPROVED** — D1, D2, N1 are genuinely closed in the working tree (verified, not trusted); the residual `fail_closed_on_mtls_install` MISSED mutant is correctly scoped pre-existing and tracked compliantly. This supersedes the NEEDS_REVISION verdict above; the close-out has landed.

### Resolution claims — independently verified

- **D1 — RESOLVED (firsthand-confirmed, not theater).** `crates/overdrive-control-plane/tests/acceptance/finalize_failed_forward_carries_workload_addr.rs` (wired at `tests/acceptance.rs:185`) drives the production `dispatch` driving port with `Action::FinalizeFailed` (`:199-220`) and asserts on the `ObservationStore` driven-port boundary (`:222-228`) — never internal state. Two default-lane PBTs pin **both** arms: `finalize_failed_stable_keeps_the_running_alloc_workload_addr` (`:247`) asserts `Some(addr)` survives a `Stable` terminal; `finalize_failed_genuine_terminal_drops_workload_addr` (`:276`) asserts `None` over a `prop_oneof!` of Failed/Completed/BackoffExhausted. This kills all four mutants D1 named (always-`None`, always-`prior`, swap-arms, `matches!` `==`→`!=`). Confirmed CAUGHT by the diff-scoped mutation run (`dispatch_single -> Ok(())` killed by this test).
- **D2 — RESOLVED (firsthand-confirmed).** `.cargo/mutants.toml` now carries real `exclude_re` entries for all four new shims — `sweep_one_chain` (`:576`), `chain_has_leg_s_exemption` (`:580`), `find_output_divert_rule_handle` (`:584`), `list_named_chain` (`:590`) — plus the REV-5 dual-chain arithmetic + head-exemption `delete !` mutants. The source `// mutants: skip` comments now read "DOCUMENTATION ONLY — the actual suppression is the `exclude_re` entry", the testing.md-compliant form. The bare-comment-suppresses-nothing gap is closed.
- **N1 — RESOLVED (advisory).** The partial-install window is real, but the install fn now documents the §5 boot-recovery sweep (`sweep_per_workload_tproxy_rules` → `sweep_one_chain` over both chains; #234) as the accepted reaper — the sanctioned move for a non-blocking finding, consistent with the codebase's converge-on-boot posture.
- **Mutation gate / #250 governance — VERIFIED COMPLIANT.** The one MISSED mutant (`fail_closed_on_mtls_install`) is correctly diagnosed pre-existing (defined at `action_shim/mod.rs:413`, step 06-03; 02-02's only diff-touch is the required-param `None,` wiring) and was surfaced to the user rather than faked. Issue **#250** is OPEN, user-authored (marcus-sa), scope-matching ("fault-injection test infra to kill the fail_closed_on_mtls_install mutant"), and created `2026-06-27T19:27:55Z` — **before** the citing commit `b9365879` (`2026-06-28T02:31`). CLAUDE.md deferral discipline (issue + user approval before citation) is satisfied.

### MET criteria re-verified
Vertical-slice integrity (`TestPkiHandle` holds nothing; `dial` is plaintext `TcpStream`, zero client rustls), the corrected plaintext-egress test model (mTLS proven only on the inter-agent `lo:SERVICE_PORT` 0x17 oracle, asserted separately from resolve + round-trip), the honest `#[ignore]→#249` deferrals with full real bodies, and the design-first pinned surface (zero invented surface) all hold under independent line-by-line reading.

### Residual (non-blocking, out of 02-02 scope)
- **nitpick:** `mtls_intercept.rs:178-182` carries a statement-level `// mutants: skip` (the `if fd < 0` guard) with no backing `exclude_re` — by the same testing.md rule applied to D2 it suppresses nothing. Pre-existing (not REV-5, not in 02-02's named scope), so not a 02-02 defect; worth a sweep when the file is next touched.

### Bottom line
**APPROVED.** The NEEDS_REVISION findings were accurate; the resolution dispatch closed them with substantive, non-theater fixes (a real two-arm PBT, real `exclude_re` backing, a documented reaper, and a bonus nextest single-writer-group fix that lifted the env-block); the one residual is honestly pre-existing and compliantly tracked in #250. Step 02-02 is clear to merge once the criterion-6 pinned-6.18 Tier-3 re-confirmation is carried by DEVOPS.
