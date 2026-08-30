# Targeted DESIGN review — Q7/Q9 READY-barrier remediation

**Reviewer:** Codex, applying the `nw-solution-architect-reviewer` stance  
**Date:** 2026-08-28  
**Commit:** `563fe26c499ef0d3b2f41283f2fc979410d3e2bf`  
**Compared with:** `d1d8f76fd5fee2725489dd9bb263005bfd182257`  
**Scope:** the Q7/Q9 amendments in `design/wave-decisions.md`,
`feature-delta.md`, ADR-0088, and ADR-0089, checked against the production
seams they claim to reuse.  
**Verdict:** **NEEDS REVISION**

## Decision summary

The core lifecycle correction is sound: making READY mean that guest platform
initialization, including networking, has completed removes the
schedule-dependent post-READY/pre-EXEC `EXIT` classification. A setup failure
before READY naturally loses the beacon race to the existing VMM-exit arm,
never reaches Running, never releases EXEC, and leaves the Beacon Published
Language unchanged. The existing Job natural-exit branch also precedes the
restart branch, so the intended no-restart behavior is feasible.

The amendment is not ready for a fresh DISTILL pass, however. Its metal proof
can currently false-pass, and two claimed reuse paths do not have the behavior
the design attributes to them.

## Findings

### HIGH-1 — the pre-intercept traffic invariant has no falsifiable packet boundary

**Dimension:** Security / acceptance-test observability  
**Locations:** `design/wave-decisions.md:161-168`,
`feature-delta.md:496`, ADR-0088 `:129-134`, ADR-0089 `:77-85`

The amendment prohibits active setup behaviors such as DHCP, DNS, reachability
probes, neighbor warm-up, socket connects, and workload sends, then asks metal
to observe zero guest-originated “workload packet.” It does not define whether
autonomous link traffic is forbidden guest traffic or allowed platform/control
traffic. It also does not pin the capture point, capture-ready edge, or protocol
coverage. The current platform cmdline does not disable IPv6
(`crates/overdrive-core/src/vm/config.rs:724-730`), and guest setup raises the
interface (`crates/overdrive-init/src/main.rs:393-425`), so autonomous ARP or
IPv6 control behavior is an implementation concern the proof must classify,
not leave behind the word “workload.”

The stale S-GTI-02/roadmap observable only rejects a cleartext TCP SYN for the
mesh destination. That can pass while other guest-originated frames cross the
tap before interception, and therefore cannot prove the new boot-through-install
claim.

**Required remediation:** pin one of these explicit contracts in the design:

- zero guest-originated L2 frames of any kind before the intercept is live; or
- a closed, named allowlist of platform/control frames, with every other
  guest-originated frame forbidden.

The metal observable must start a verified capture on the already-provisioned
tap before VMM spawn (and correlate that capture with the host-veth/intercept
observable), retain it through the intercept-live edge, inspect all relevant
EtherTypes/protocols rather than only peer-bound SYNs, and then prove the
operator's first mesh connection is captured. If control traffic is allowed,
the design must name and constrain it so payload-bearing TCP/UDP or an
unexpected destination cannot hide inside the allowance.

### MEDIUM-1 — the existing VMM diagnostic capture cannot contain the init error

**Dimension:** Feasibility / operator diagnostics / reuse accuracy  
**Locations:** `design/wave-decisions.md:134-144`,
`feature-delta.md:188-196,511`, ADR-0088 `:113-123`, ADR-0089 `:97-104`

All four amended artifacts say the existing captured VMM console detail carries
`overdrive-init`'s concrete setup error. The production path separates those
streams:

- Cloud Hypervisor sends guest serial output to the run-directory
  `console.log` via `--serial file=...`, while the hypervisor process's stderr
  is independently piped (`crates/overdrive-host/src/vmm.rs:245-292`).
- `VmmDiagnostics` is populated only by `spawn_stderr_capture` over that
  process stderr, and `VmmExit.stderr_tail` is documented as the hypervisor's
  own stderr (`crates/overdrive-host/src/vmm.rs:486-522,687-704`;
  `crates/overdrive-core/src/traits/vmm.rs:243-275,348-358`).
- The pre-READY VMM-exit arm deletes the run directory before constructing the
  rejection detail and then uses only `VmmExit.stderr_tail`
  (`crates/overdrive-worker/src/vm_driver.rs:930-950,1345-1375`).

Consequently, the final row will normally carry hypervisor stderr or the “no
stderr captured” fallback, not the guest's concrete malformed-token/net-apply
error. The public `VmGuestExitUnreported` cause still classifies the boot phase,
but the promised diagnostic detail and earned-trust claim are not implemented
by the reused path.

**Required remediation:** explicitly sanction an internal guest-console
snapshot before cleanup (or another concrete observable that actually receives
PID 1's error), define its boundedness and fallback/precedence relative to VMM
stderr, and add the affected driver/host diagnostic seam to the component and
reuse analysis. This can preserve the design's no-new-beacon-message and
no-new-describe-field constraints.

### MEDIUM-2 — the Job finalization classifier is an omitted EXTEND, not existing behavior

**Dimension:** Component completeness / effect isolation / lifecycle correctness  
**Locations:** `design/wave-decisions.md:134-141`,
`feature-delta.md:188-196,420-466`, ADR-0088 `:117-123`, ADR-0089 `:100-103`

The Job natural-exit branch does correctly run before restart handling and emits
`FinalizeFailed` without incrementing the private restart budget. Its classifier
does not, however, preserve `VmGuestExitUnreported.vmm_exit_code` today.
`classify_natural_exit_terminal` recognizes only Process-stopped and
`WorkloadCrashedImmediately`; all other reasons fall back to
`Failed { exit_code: Some(0) }`
(`crates/overdrive-reconcilers/src/workload_lifecycle.rs:1444-1480`).

The amendment states the required future behavior, but its component/reuse
analysis omits `WorkloadLifecycle` and describes the result as existing. A
crafter following that ownership map can move setup before READY yet still
publish the fabricated default terminal code.

**Required remediation:** mark
`WorkloadLifecycle::classify_natural_exit_terminal` as **EXTEND** and pin its
pure mapping from every `VmGuestExitUnreported { vmm_exit_code, .. }` to
`TerminalCondition::Failed { exit_code: vmm_exit_code }`. The downstream pure
property must carry the repository-required exact rustdoc declaration
`/// CONTRACT_SHAPE: pure-function.`. Keep a reconciler-level example proving
no `RestartAllocation`, unchanged private restart state, and unchanged durable
`restart_count`.

## Downstream DISTILL and roadmap remediation

The entries below are intentionally treated as downstream-stale handoff work,
not additional DESIGN defects.

| Artifact | Required change before DELIVER resumes |
|---|---|
| `distill/test-scenarios.md` — Q7 row and C2 state machine | Delete the post-READY/pre-EXEC `EXIT` arm. Model init/token/net-apply failure as poweroff before READY, followed by the existing `VmmExited` start rejection. After READY, `EXIT` is exclusively an operator result. |
| `distill/test-scenarios.md` — S-GTI-08 | Drive a real pre-READY setup failure and assert: no READY/Running/EXEC/operator command; reason `VmGuestExitUnreported`; concrete guest setup detail preserved through the chosen diagnostic seam; terminal `Failed` carries the exact `vmm_exit_code`; no `RestartAllocation`; private and durable restart counts unchanged. Add a pure classifier property over `Option<i32>` plus the metal lifecycle example. |
| `distill/test-scenarios.md` — S-GTI-02 and completeness audit | Expand the capture interval from “first mesh SYN” to verified capture-ready-before-NIC-up through intercept-live. Adopt the design's closed packet/control taxonomy, inspect all non-allowlisted guest-originated traffic, then assert the first operator mesh dial is intercepted. Refresh C2, C6, Q7/Q9 pins, reconciliation, and completeness counts. |
| `distill/red-classification.md` | Replace S-GTI-08's “Q7 EXIT-before-EXEC host arm unbuilt” reason with the missing pre-READY initialization order, guest-console diagnostic snapshot, and `VmGuestExitUnreported` finalization mapping. Update S-GTI-02's RED reason to include the boot-through-install capture contract. |
| `feature-delta.md` — embedded `Wave: DISTILL` snapshot | Refresh the stale Q7 inherited-commitment row (`:555`), Q7 shape-pin text (`:647-656`), state-machine claims, and “0 contradictions” statement as part of the fresh DISTILL pass. These lines currently describe the superseded arm. |
| `deliver/roadmap.json` step `02-02` | Strengthen the S-GTI-02 criteria, acceptance criteria, implementation notes, and blocker clause from “zero mesh cleartext SYN” to the closed boot-through-install traffic/capture contract. Include `overdrive-init`/guest boot policy in implementation scope if suppression is required. |
| `deliver/roadmap.json` step `02-03` | Replace every Q7 EXIT-before-EXEC sentence in the name, description, criteria, acceptance criteria, implementation notes, and `BLOCKER-IF`. Add `overdrive-reconcilers/src/workload_lifecycle.rs` and the selected guest-console diagnostic seam (`vm_driver.rs` plus the relevant host/core diagnostic file if changed) to scope. Require the S-GTI-08 and pure-classifier observables listed above; retain the no-new-PL/no-new-describe-field constraint. |
| `deliver/roadmap.json` validation | Re-run the roadmap review after the edits and move `validation.status` from `pending` to approved before execution. |

## Checks that passed

- READY is now a deterministic post-network-initialization barrier; the former
  host-flush race is removed rather than timed around.
- Init, malformed-token, and net-apply failure all share the same pre-READY
  fail-closed path and cannot execute the operator command.
- The existing pre-READY VMM-exit reason supplies a typed boot-phase cause; no
  Beacon Published Language addition is required.
- The Job natural-exit branch precedes restart handling, so the desired
  no-restart policy is compatible with the current reconciler structure.
- Q9's existing deferred EXEC reply remains compatible with the stronger READY
  meaning and the two D6 install sites.
- Commit scope is documentation-only and limited to the four declared design
  artifacts. `git diff --check 563fe26c^ 563fe26c` passed.

## Defect counts

| Severity | Count |
|---|---:|
| Blocker | 0 |
| Critical | 0 |
| High | 1 |
| Medium | 2 |
| Low | 0 |

The approval rule requires zero Blocker, Critical, High, and Medium findings.
This review therefore returns **NEEDS REVISION**.

---

## Iteration 2 — 2026-08-29

**Reviewed commit:** `5590af6731ecd97a0148e41aae83f53aae082562`  
**Compared with:** `563fe26c499ef0d3b2f41283f2fc979410d3e2bf`  
**Scope:** disposition of Iteration-1 HIGH-1, MEDIUM-1, and MEDIUM-2;
regression scan across the four changed artifacts, production seams, and the
effective architecture SSOT.  
**Verdict:** **NEEDS REVISION**

### Iteration-1 finding dispositions

| Finding | Disposition | Evidence |
|---|---|---|
| HIGH-1 — non-falsifiable pre-intercept traffic boundary | **CLOSED** | The design now chooses a closed zero-guest-L2-frame contract, explicitly disables per-interface IPv6 and pins/read-backs `arp_notify=0` before NIC-up, arms tap and host-veth witnesses before real VMM spawn, correlates allocation/netns/names/ifindices/MAC/address, covers tagged and unknown EtherTypes plus every named L3/L4 class/destination/payload shape, fails on drops/truncation/unknown direction or time/ambiguous identity, defines intercept-live by successful install plus observed exact rule, and follows the first operator five-tuple through rule increment, leg-F, and TLS with no cleartext copy (`design/wave-decisions.md:183-226`; `feature-delta.md:232-271,581,615`; ADR-0088 `:146-180`; ADR-0089 `:77-104`). |
| MEDIUM-1 — VMM stderr cannot contain PID 1's error | **CLOSED** | The amendment now identifies `VmRunDir::console_log()` as the actual CH guest-serial source and keeps `VmmExit.stderr_tail` as hypervisor stderr only. `VmDriver` snapshots before destructive cleanup, reads at most the final 8 KiB/five line fragments, retains an unterminated final fragment, renders lossy UTF-8, gives nonempty guest serial precedence, defines absent/empty/unreadable and neither-source fallbacks, and forbids snapshot failure from masking cleanup/rejection. It adds no Beacon, `VmmExit`, observation, or describe field (`design/wave-decisions.md:156-174`; `feature-delta.md:213-229,508,514-515,597,613`; ADR-0088 `:129-144`; ADR-0089 `:116-126`). |
| MEDIUM-2 — omitted Job classifier extension | **CLOSED** | `WorkloadLifecycle::classify_natural_exit_terminal` is now explicitly **EXTEND** with the exact `Option<i32>` mapping from `VmGuestExitUnreported`; the property ranges over every `Option<i32>` and arbitrary signal and requires the exact `/// CONTRACT_SHAPE: pure-function.` line. Reconciler/action-shim evidence seeds restart state and proves `FinalizeFailed` only, unchanged View/private counts, and unchanged durable `restart_count` (`design/wave-decisions.md:140-154`; `feature-delta.md:202-212,516,598,613`; ADR-0088 `:117-128`; ADR-0089 `:127-137`). |

The Linux feasibility premise also checks out against the primary kernel
documentation: per-interface `disable_ipv6=1` disables IPv6 operation and
removes addresses/routes, while `arp_notify=0` is the documented do-nothing
mode and `1` is what generates gratuitous ARP on device-up/MAC change. The
design's write-and-read-back requirement avoids depending on defaults. See the
[Linux IP sysctl documentation](https://docs.kernel.org/networking/ip-sysctl.html).

### New finding

#### HIGH-2 — the authoritative architecture brief still mandates the superseded Q7/Q9 design

**Dimension:** Cross-artifact consistency / security handoff  
**Locations:** `docs/product/architecture/brief.md:10085-10155`, compared with
`feature-delta.md:1-7,202-271,581,613-615` and ADR-0088 `:86-180`

The four changed artifacts now agree, but the effective architecture does not.
`CLAUDE.md:86-87` identifies `brief.md` as the scope SSOT, and the feature delta
itself points to this exact brief section as its SSOT. That normative section
still says:

- guest setup is apply-or-`EXIT` (`brief.md:10117-10120`), the racy Q7 shape
  this amendment rejects;
- first-connect safety is only `install-success ≺ EXEC-release` and a
  peer-bound cleartext-SYN assertion (`:10146-10150`), omitting the closed
  capture-ready-to-intercept-live zero-frame contract;
- the “ONLY new production code” has four items (`:10095-10131`), omitting
  pre-cleanup guest-console selection and the Job classifier extension; and
- the hard-gate tally remains 7 REUSE / 7 EXTEND / 1 CREATE (`:10154-10155`),
  while the amended design is 9 / 9 / 1.

A DISTILL or DELIVER reader following the repository's SSOT can therefore
reintroduce the exact schedule-dependent EXIT arm and weak metal oracle that
the amended ADRs reject. This is not downstream-stale DISTILL/roadmap content;
it is an authoritative DESIGN artifact.

**Required remediation:** update the normative GH #222 section in
`brief.md` to the accepted Q7/Q9 amendment: silent pre-READY setup with NIC-down
verification, per-interface IPv6 disable and verified `arp_notify=0`; poweroff
before READY without guest `EXIT`; bounded pre-cleanup `console.log` selection;
the exact `VmGuestExitUnreported` Job mapping/no-restart facts; the full
capture-ready-to-first-connect witness; and the 9/9/1 reuse tally. Add a dated
changelog amendment rather than rewriting historical rows.

As a tightly related consistency cleanup, correct the high-signal summaries
that still say “one-site” despite D6's two install sites
(`design/wave-decisions.md:20-30`; ADR-0089 `:296-302`) and make the wave
summary's “ONLY new production code” include the newly sanctioned diagnostic
and lifecycle extensions. The detailed decisions are correct; these summary
lines must stop contradicting them.

### Downstream obligations after the SSOT correction

The existing DISTILL and roadmap are intentionally stale and are not counted
as additional design defects. A fresh handoff must make these exact changes:

| Artifact | Required amendment |
|---|---|
| `distill/test-scenarios.md` Q7/C2/S-GTI-08 | Remove the post-READY/pre-EXEC `EXIT` state entirely. Drive init, malformed-token, suppression, and net-apply failure before READY; assert no READY/Running/EXEC/operator command, typed `VmGuestExitUnreported`, primary bounded PID 1 serial detail, exact terminal `Option<i32>`, `FinalizeFailed` only, and unchanged private/durable restart counts. Add the exact-contract-shape pure classifier property. |
| `distill/test-scenarios.md` Q9/S-GTI-02/S-GTI-01 | Replace the peer-SYN-only oracle with capture-ready before real VMM spawn; exact allocation/netns/tap/host-veth identity; all-EtherType zero-frame interval; conservative failure on drops, malformed/truncated/unknown records or ordering; observed exact rule-live; capture across EXEC; first operator five-tuple → rule increment → leg-F → TLS and no cleartext copy. Add suppression/read-back failure examples and refresh reconciliation/completeness. |
| `distill/red-classification.md` | Replace S-GTI-08's missing EXIT-before-EXEC arm with missing pre-READY ordering, suppression, console-tail selection, and exact lifecycle mapping. Expand S-GTI-02's reason to the closed witness/decorator contract. |
| `feature-delta.md` embedded `Wave: DISTILL` section | Refresh the stale Q7 inherited-commitment row, shape pins, reconciliation claim, scenario/audit text, adapter coverage, carry-forwards, and registered born-captured outcome. The current embedded DISTILL record still names EXIT-before-EXEC and the weaker invariant. |
| Roadmap step `01-03` | Replace apply-before-exec / nonzero guest `EXIT` with NIC-down verification → IPv6 disable/read-back → `arp_notify=0` write/read-back → static IPv4/resolver → connect/READY. Every init/token/suppression/apply error powers off before READY. Add bounded unit tests for the suppression/config sequencing and named failure cases; Beacon PL remains unchanged. |
| Roadmap step `02-02` | Preserve the already-approved production EXEC deferral and D6 install site. Replace its metal criteria/notes/AC with the observation-only real-VMM decorator, exact identity tuple, all-frame closed interval, conservative unknown/drop behavior, exact observed rule-live, and first-five-tuple proof. Do not reinterpret this as a production ordering rewrite. |
| Roadmap step `02-03` | Delete every Q7 host EXIT-phase arm and its `BLOCKER-IF`. Add the bounded pre-cleanup serial snapshot behavior and boundary cases (>8 KiB, >5 lines, unterminated final fragment, invalid UTF-8, empty/missing/unreadable console, stderr precedence/fallback, neither source), real S-GTI-08 detail, exact pure classifier property, no-restart example, and unchanged counts. Add `overdrive-reconcilers/src/workload_lifecycle.rs`; retain no-new-Beacon/`VmmExit`/describe/observation-field constraints. |
| `deliver/roadmap.json` validation | Re-run roadmap review after all amendments and move `validation.status` from `pending` to approved before DELIVER resumes. |

### Regression and compatibility checks

- **Networking before READY:** coherent. The guest can mount `/proc`, verify
  NIC-down, apply suppression, configure IPv4/resolver, then connect/send READY;
  vsock does not depend on guest IP.
- **Beacon Published Language:** coherent and byte-for-byte unchanged. Setup
  failure powers off before connection/READY and no longer overloads `EXIT`.
- **Typed terminal classification:** coherent. The action shim already writes
  `Failed` with `VmGuestExitUnreported`; the Job-first branch precedes restart;
  the specified pure extension closes the current `Some(0)` fallback.
- **Restart accounting:** coherent. Returning the input View preserves private
  counts; FinalizeFailed's terminal-to-terminal row construction forwards the
  prior durable `restart_count`.
- **Approved 02-01/02-02 compatibility:** coherent. Both VM install gates and
  deferred EXEC release remain unchanged; the added packet observer is a
  metal-only wrapper that arms before real `Vmm::create`, and the stronger READY
  meaning simply lengthens the existing pre-READY boot phase.
- **Guest diagnostic feasibility:** coherent. CH already writes serial to the
  per-allocation console file; the driver owns both the resolved VMM-exit arm
  and the cleanup point, so the bounded snapshot fits without a new public
  seam.
- **Mechanical review:** the commit changes only the four declared design
  artifacts, and `git diff --check 563fe26c 5590af67` passes.

### Iteration-2 defect counts

| Severity | Count |
|---|---:|
| Blocker | 0 |
| Critical | 0 |
| High | 1 |
| Medium | 0 |
| Low | 0 |

All three Iteration-1 findings are closed. Approval still requires zero High
findings, so Iteration 2 returns **NEEDS REVISION** until the effective
architecture SSOT is reconciled.

---

## Iteration 3 — 2026-08-29

**Reviewed commit:** `29ab0bf71ef8c178a04035010d0ad72084b9ce7b`  
**Compared with:** `5590af6731ecd97a0148e41aae83f53aae082562`  
**Scope:** disposition of Iteration-2 HIGH-2; cross-artifact regression scan
over the architecture brief, ADR index, normative GH #222 section, changelog,
the four effective feature-design artifacts, and the intentionally stale
downstream DISTILL/roadmap handoff.  
**Verdict:** **APPROVED**

### Iteration-2 finding disposition

| Finding | Disposition | Evidence |
|---|---|---|
| HIGH-2 — authoritative `brief.md` retained the superseded Q7/Q9 contract | **CLOSED** | The normative GH #222 section now has six complete production changes, not four: tap provision, CH attach, silent pre-READY guest setup, both D6 install gates, bounded pre-cleanup guest-console selection, and exact Job terminal classification (`brief.md:10095-10164`). It restores READY as the initialization barrier, requires IPv6-disable and `arp_notify=0` read-back, powers off every setup failure before READY with no guest `EXIT`, keeps post-READY `EXIT` operator-only, and leaves Beacon PL unchanged (`:10112-10129`). The metal contract is capture-ready before real CH spawn, exact-allocation correlated, zero guest L2 frames of every shape through observed exact-rule `intercept-live`, conservative on capture uncertainty, and follows the first operator five-tuple through rule increment, leg-F, TLS, and no cleartext copy (`:10181-10203`). The hard-gate tally is 9 REUSE-AS-IS / 9 EXTEND / 1 CREATE-NEW (`:10206-10208`). |

The high-signal summaries are also reconciled. ADR index rows 0088 and 0089
carry the pre-READY lifecycle, bounded diagnostic/classifier ownership, exact
packet witness, and both install sites (`brief.md:3497-3498`). The DESIGN wave
summary names the two-site gate plus the diagnostic and lifecycle extensions
(`design/wave-decisions.md:20-33`), while ADR-0089's Consequences now calls the
gate a two-site production change and names the partial-application fail-open
hazard (ADR-0089 `:297-304`). The four detailed design artifacts continue to
agree on:

- lifecycle order: `capture-ready ≺ VMM-spawn ≺ network-ready ≺ READY ≺
  intercept-live ≺ EXEC-release ≺ operator-first-connect`;
- a zero-guest-originated-L2-frame interval with no control allowlist, exact
  tap/host-veth identity, capture uncertainty as failure, and a post-release
  five-tuple/rule/leg-F/TLS/no-cleartext witness;
- bounded `VmRunDir::console_log()` selection before cleanup, with guest
  console primary, separately bounded VMM stderr fallback, and a stable
  neither-source fallback, without a public-field addition;
- exact `VmGuestExitUnreported.vmm_exit_code` mapping for Job, the exact
  `/// CONTRACT_SHAPE: pure-function.` declaration, `FinalizeFailed` only,
  unchanged private View, and unchanged durable `restart_count`;
- D6 at fresh-start and restart install sites, ungated teardown, and the
  9/9/1 reuse tally.

### Historical and downstream-stale statements

The old architecture changelog rows are retained honestly as history. The new
2026-08-29 amendment explicitly says the 2026-08-27/28 apply-or-`EXIT`, weaker
ordering, older summary, and 7/7/1 statements are superseded
(`brief.md:10216-10218`). They no longer compete with the normative section or
ADR index as current design.

The remaining `EXIT-before-EXEC` and `install-success ≺ EXEC-release` text in
`feature-delta.md` is confined to its embedded **Wave: DISTILL** snapshot
(`:635-792`), not the current DESIGN model. The current Q7/Q9 rows explicitly
mark the former lifecycle superseded and instruct downstream DISTILL/roadmap
to replace the weaker assertion (`:616-618`); `design/wave-decisions.md:247-253`
does the same. These are tracked handoff obligations, not live authoritative
DESIGN alternatives.

### Required downstream DISTILL and roadmap obligations

Approval of the design does not approve the current downstream artifacts.
Before DELIVER resumes, the fresh handoff must complete every item below:

| Artifact | Exact obligation |
|---|---|
| `distill/test-scenarios.md` Q7/C2/S-GTI-08 | Delete the post-READY/pre-EXEC `EXIT` state. Cover init, malformed-token, IPv6/`arp_notify` suppression/read-back, and net-apply failures before READY. Assert no READY, Running, EXEC, or operator command; typed `VmGuestExitUnreported`; primary bounded guest-console detail with the defined fallbacks; exact terminal `Option<i32>`; `FinalizeFailed` only; and unchanged private and durable restart counts. Add the source-local pure classifier property with the exact contract-shape rustdoc. |
| `distill/test-scenarios.md` Q9/S-GTI-02/S-GTI-01 | Replace the peer-SYN-only oracle with the complete observer contract: capture-ready before real VMM spawn; exact allocation/slot/netns/tap/host-veth/MAC/address identity; all-EtherType zero-frame interval through observed exact-rule `intercept-live`; fail on drops, overflow, malformed/truncated records, unknown direction/time/order, or ambiguous correlation; continue across EXEC; then prove first operator five-tuple → rule increment → leg-F → TLS with no cleartext copy. Refresh reconciliation and completeness. |
| `distill/red-classification.md` | Replace the missing EXIT-phase implementation rationale with missing pre-READY sequencing/suppression, console-tail selection, and exact lifecycle classification. Expand S-GTI-02's RED reason to the complete observer/witness contract. |
| `feature-delta.md` embedded `Wave: DISTILL` snapshot | Refresh inherited commitments, Q7/Q9 shape pins, state-machine and reconciliation claims, scenario/audit text, adapter coverage, carry-forwards, and `OUT-GTI-BORNCAPTURED`; remove the stale EXIT-before-EXEC and weaker install-success wording. |
| Roadmap step `01-03` | Replace apply-before-exec/nonzero guest `EXIT` with NIC-down verification → IPv6 disable/read-back → `arp_notify=0` write/read-back → static IPv4/resolver → connect/READY. Every init/token/suppression/apply error powers off before READY. Add bounded sequencing and named-failure tests; Beacon PL remains unchanged. |
| Roadmap step `02-02` | Preserve the already-approved production EXEC deferral and both D6 install sites. Replace the metal criteria/notes/AC with the observation-only real-VMM decorator, exact identity tuple, all-frame closed interval, conservative unknown/drop behavior, observed exact rule-live, and first-five-tuple proof. Do not turn the test observer into production networking or a new install order. |
| Roadmap step `02-03` | Delete every Q7 host EXIT-phase arm and `BLOCKER-IF`. Add pre-cleanup console bounds and edge cases (>8 KiB, >5 line fragments, unterminated final fragment, invalid UTF-8, empty/missing/unreadable console, stderr fallback, neither source); real S-GTI-08 detail; exact classifier property; no-restart example; and unchanged counts. Include `overdrive-reconcilers/src/workload_lifecycle.rs`; retain the no-new-Beacon/`VmmExit`/describe/observation-field constraints. |
| `deliver/roadmap.json` validation | Re-run roadmap review after the amendments and move `validation.status` from `pending` to approved before DELIVER resumes. |

### Preserved closed findings and compatibility checks

- Iteration-1 HIGH-1 remains closed: the packet boundary is exhaustive and
  falsifiable, including suppression/read-back and fail-conservative capture.
- Iteration-1 MEDIUM-1 remains closed: guest serial and hypervisor stderr are
  correctly distinguished, bounded, ordered, and read before cleanup.
- Iteration-1 MEDIUM-2 remains closed: classifier ownership, exact mapping,
  property declaration, no-restart behavior, and both restart-count stores are
  explicit.
- Approved step 02-01/02-02 mechanics remain compatible: the production
  deferred EXEC mechanism and both install sites are unchanged; the stronger
  proof is an observation-only decorator around the real VMM.
- Commit scope is documentation-only and limited to the four design artifacts
  plus the architecture SSOT. `git diff --check 5590af67 29ab0bf7` passes.

### Iteration-3 defect counts

| Severity | Count |
|---|---:|
| Blocker | 0 |
| Critical | 0 |
| High | 0 |
| Medium | 0 |
| Low | 0 |

HIGH-2 is closed, every prior finding remains closed, and no new defect was
found. Iteration 3 therefore returns **APPROVED**. This verdict approves the
DESIGN only; the enumerated DISTILL/roadmap remediation remains mandatory
before DELIVER execution.

---

## Iteration 4 — 2026-08-29

**Reviewed commit:** `dfe8eb2427676328c668116415a586ddc737f502`  
**Requested comparison base:** `29ab0bf71ef8c178a04035010d0ad72084b9ce7b`  
**Direct target parent:** `7255c68e`  
**Scope:** the D7 nft exact-rule-hit amendment, its closure of DISTILL platform
P3, the effective Q7/Q9 design and architecture SSOT, and regression checks for
all findings closed in Iterations 1–3. The requested base range also contains
the intervening DISTILL reconciliation commit; the D7 commit itself changes
the five authoritative DESIGN/architecture documents.  
**Verdict:** **NEEDS REVISION**

### Decision summary

D7 removes the original P3 contradiction in mechanism and classification. An
anonymous `counter` expression after the existing `iifname` and TCP matches is
technically valid and non-terminal; the existing raw encoder can express it as
an empty `NFTA_EXPR_DATA` nest, and the kernel initializes absent packet/byte
seed attributes to zero. `GETRULE` returns the expression tree with packet and
byte values as big-endian `u64`, so the proposed additive
`RuleInfo.counter: Option<RuleCounterSnapshot>` projection is implementable.
The counter's placement also preserves the existing match, TPROXY, mark,
verdict, userdata, and teardown behavior.

The first-five-tuple proof is no longer circular: complete host-veth capture
identifies the only eligible directional tuple, the counter is a separate
kernel rule-hit observation, and leg-F/original-destination plus TLS/no-
cleartext evidence prove the downstream path. The authoritative documents
also agree on 8 REUSE-AS-IS / 10 EXTEND / 1 CREATE-NEW, keep all public
protocol/persistence/observation schemas unchanged, and preserve the sole
production install/adopt/delete owner.

The amendment is not yet safe to hand back to DISTILL, however. It asserts that
reset, replacement, and concurrent dump mutation fail, but the pinned
same-userdata+handle identity and positive-upper-bound arithmetic cannot
establish those claims. One central false-pass defect remains.

### Preserved prior finding dispositions

| Prior finding | Disposition | Iteration-4 regression evidence |
|---|---|---|
| HIGH-1 — non-falsifiable pre-intercept packet boundary | **REMAINS CLOSED** | D7 leaves the exhaustive all-EtherType zero-frame interval, exact interface/allocation correlation, capture-before-spawn edge, suppression read-backs, and fail-conservative capture behavior unchanged. |
| MEDIUM-1 — wrong guest diagnostic source | **REMAINS CLOSED** | Bounded pre-cleanup `console.log` selection, guest-console precedence, VMM-stderr fallback, and the neither-source fallback remain intact and outside D7. |
| MEDIUM-2 — omitted Job classifier extension | **REMAINS CLOSED** | The exact `VmGuestExitUnreported` mapping, `FinalizeFailed`-only behavior, restart-count preservation, and exact `/// CONTRACT_SHAPE: pure-function.` declaration remain intact. |
| HIGH-2 — stale architecture brief | **REMAINS CLOSED** | The normative GH #222 section, ADR index, changelog supersession, feature delta, wave decisions, and both ADRs all carry D7 and the corrected 8/10/1 tally. Historical 9/9/1 and REUSE-AS-IS text is explicitly superseded rather than live. |

### HIGH-3 — handle+userdata and bounded-positive deltas cannot detect in-place reset or replacement

**Dimension:** Security proof / concurrency / kernel-observable identity  
**Locations:** `design/wave-decisions.md:265-282`,
`feature-delta.md:321-361`, ADR-0088 `:206-217`, ADR-0089 `:108-128`,
`brief.md:10183-10196`

The design calls `(userdata, handle)` an immutable rule identity and says
checked positive deltas bounded above by captured counts/bytes make reset and
replacement fail. Neither statement holds for the nft kernel API.

1. `NLM_F_REPLACE` accepts `NFTA_RULE_HANDLE`, looks up the old rule, constructs
   a new rule with `rule->handle = handle`, and accepts caller-provided userdata.
   A transactional replacement can therefore preserve both values while
   changing the expression program and installing a fresh counter. No
   disappearance, reappearance, handle change, tag change, or unstable quiet
   pair is required. See the kernel's
   [`nf_tables_newrule`](https://github.com/torvalds/linux/blob/master/net/netfilter/nf_tables_api.c)
   implementation and the documented
   [`replace rule ... handle ...`](https://netfilter.org/projects/nftables/manpage.html#RULES)
   operation.
2. The UAPI exposes `NFT_MSG_GETRULE_RESET`, whose purpose is to dump rules and
   reset their stateful expressions. The counter implementation fetches and
   resets the per-CPU totals without changing rule handle or userdata. For a
   concrete passing counterexample, let a same-tag adoption baseline be
   `packets=1, bytes=60`; reset it in place, then capture four eligible frames
   totalling 300 nft-counted bytes. The after value `4/300` yields checked
   deltas `3/240`: both are positive and no greater than the captured `4/300`,
   so the specified oracle passes even though the reset the design says must
   fail occurred. The same arithmetic lets a handle-preserving replacement's
   fresh counter overtake a small adopted baseline. See the
   [`NFT_MSG_GETRULE_RESET` UAPI](https://github.com/torvalds/linux/blob/master/include/uapi/linux/netfilter/nf_tables.h)
   and [`nft_counter_fetch_and_reset`](https://github.com/torvalds/linux/blob/master/net/netfilter/nft_counter.c).
3. The decoder contract is strict only for the selected counter subtree. The
   current `parse_rules` intentionally returns the prefix decoded before a
   truncated nlmsg/attribute, `list_rules` accepts EOF without `NLMSG_DONE`, and
   it does not reject `NLM_F_DUMP_INTR` (`overdrive-netlink/src/nft.rs:632-683,
   920-952`). A malformed or concurrently interrupted trailing portion can hide
   a duplicate target while the returned prefix still appears unique. Two
   identical partial reads do not make either dump complete or coherent.

This is a false-pass in the exact security witness introduced to close P3, not
merely a missing edge-case test. Full capture and leg-F/TLS evidence do not
repair it: they prove the expected flow existed and reached the proxy, but not
that the same unchanged production rule/counter spanned the declared cuts.

**Required remediation:** pin a mutation-aware, complete snapshot contract in
all authoritative D7 summaries before the next review:

- bracket the complete witness with a kernel ruleset-generation or equivalent
  nfnetlink change witness, and fail on any generation change, replacement,
  deletion/reinsert, notification loss, or generation wrap; additionally bind
  a normalized full expression-program identity to the expected production
  encoder so same-handle/same-userdata is not treated as proof of rule shape;
- replace the one-sided upper bounds with an equality-capable reset proof:
  packet delta must equal the complete capture's count of packets matching the
  counter's preceding predicates, and byte delta must equal the same packets'
  nft byte domain (the kernel increments by `skb->len`, so generic captured L2
  byte length is not an exact substitute). If the chosen capture cannot make
  that equivalence exact, choose a reset-epoch/change witness instead of
  claiming resets are detectable. Continue to use checked subtraction and
  reject wrap;
- make `list_rules` a strict, bounded multipart operation: enforce socket/read
  timeout, sequence/sender integrity, valid length/alignment for every nlmsg and
  attribute, exactly one successful `NLMSG_DONE`, nonzero `NLMSG_ERROR`, and
  `NLM_F_DUMP_INTR`; reject malformed/trailing/partial dumps before uniqueness
  is evaluated. Valid counter-free sibling rules may still project `None`;
- retain the observer's read-only constraint, exact allocation tag, sibling
  non-mutation, same-tag adoption, by-handle teardown, and boot-sweep ownership.

The exact implementation may use a conservatively global ruleset-generation
guard—unrelated concurrent nft changes may fail the metal witness—but it may
not convert mutation ambiguity into a pass. After this DESIGN correction, the
fresh DISTILL pass must replace its generic “rule increment” wording with the
ratified complete/coherent snapshot and reset/replacement oracle.

### Checks that passed

- **Expression feasibility:** the existing `expr(name, data)` encoder already
  produces the nested expression framing needed for `expr("counter", &[])`;
  kernel counter initialization accepts absent seed attributes, evaluates one
  packet plus `skb->len` per hit, and dumps packet/byte totals as big-endian
  `u64` attributes.
- **Counter placement:** every live D7 artifact places the counter after both
  unchanged predicates and before the unchanged TPROXY/mark/accept tail;
  shared, inbound, and output-divert rules remain counter-free.
- **Non-circular correlation:** capture supplies tuple/completeness evidence,
  the rule counter supplies an independent kernel observation, and leg-F/TLS
  supplies downstream-path evidence; the observer supplies no functional
  networking or nft mutation.
- **Lifecycle and siblings:** `install_outbound_tproxy` remains sole production
  owner; userdata bytes, same-tag adoption, exact-handle stop, boot sweep, and
  no cross-restart comparison remain coherent. The read projection does not
  serialize or alter sibling rules/counters.
- **Surface compatibility:** no Beacon Published Language, REST/OpenAPI,
  persistence, describe, or observation schema changes. `RuleInfo` is an
  additive workspace-internal Rust projection.
- **Reuse hard gate:** all current summaries and the 19-row table agree on
  8 REUSE-AS-IS / 10 EXTEND / 1 CREATE-NEW. The old 9/9/1 changelog row is
  explicitly superseded.
- **Mechanical scope:** `git diff --check 29ab0bf7 dfe8eb24` passes. The
  requested range changes seven documentation files because it includes the
  intervening DISTILL commit; the direct D7 commit changes the five declared
  authoritative DESIGN/architecture files only.

### Iteration-4 defect counts

| Severity | Count |
|---|---:|
| Blocker | 0 |
| Critical | 0 |
| High | 1 |
| Medium | 0 |
| Low | 0 |

Approval requires zero Blocker, Critical, High, and Medium findings. D7 is a
feasible direction and closes the original “no rule-hit mechanism” half of P3,
but HIGH-3 leaves its reset/replacement/concurrency claims non-falsifiable.
Iteration 4 therefore returns **NEEDS REVISION**.

---

## Iteration 5 — 2026-08-29

**Reviewed commit:** `cd12725159a6b2a92619f17aa4dc5f0ff621b842`  
**Compared with:** `dfe8eb2427676328c668116415a586ddc737f502`  
**Scope:** disposition of Iteration-4 HIGH-3; adversarial re-review of the
generation/change witness, reset/replacement/delete/reinsert/wrap/loss cases,
normalized rule-program identity, exact packet/byte accounting, strict
multipart completion, lifecycle ownership, Q7/Q9 consistency, public schemas,
and the 8/10/1 reuse gate across the five authoritative DESIGN/architecture
artifacts and the production kernel/netlink seams they constrain.  
**Verdict:** **APPROVED**

### Iteration-4 finding disposition

| Finding | Disposition | Evidence |
|---|---|---|
| HIGH-3 — handle+userdata and bounded-positive deltas could not detect reset/replacement, while partial dumps could hide a duplicate | **CLOSED** | D7 now requires the exact tag, handle, and normalized complete production expression program; brackets every complete rule dump with the same full `NFTA_GEN_ID`; guards the entire before-to-after witness with loss-reporting `NFNLGRP_NFTABLES` observation; rejects every notification, generation change/wrap, `ENOBUFS`, overrun, partial/interrupted dump, and ambiguity; and replaces upper bounds with checked equality against the complete capture's packet count and IPv4 `tot_len`/nft-`skb->len` total (`design/wave-decisions.md:266-333`; `feature-delta.md:285-413`; ADR-0088 `:196-269`; ADR-0089 `:122-173`; `brief.md:10170-10244`). |

The former counterexamples no longer pass. A same-handle/same-userdata
`NLM_F_REPLACE` commit either changes the normalized expression program or is
caught by the ruleset generation and notification guard; delete/reinsert and
rule reordering have the same generation evidence. The kernel increments its
nonzero 32-bit `base_seq` for each committed nft transaction, marks an
inconsistent dump with `NLM_F_DUMP_INTR`, and emits the generation/change
notifications the subscribed socket treats as failure. A full-value wrap would
also require intervening transactions and therefore cannot be accepted through
the loss-detecting notification guard merely because the final numeric value
equals `G`.

`NFT_MSG_GETRULE_RESET` is deliberately not treated as a generation event: the
kernel resets the stateful counter while serving the reset read. Instead, the
measured `before`→`after` interval is protected by exact arithmetic. For any
reset after at least one interval packet, the post-reset total loses a prefix,
so it cannot satisfy both `after - before == captured` and
`before + captured == after`; regression and `u64` wrap also fail the checked
operations. A zero-valued reset before the first increment changes no measured
state, while same-tag adoption legitimately establishes a fresh baseline from
whatever accumulated value is present before the measured cut. This is the
correct observable boundary for proving the operator flow's rule hit.

### Kernel-observable and dump-integrity review

- **Full program identity is feasible and sufficient.** The production egress
  program is a finite ordered sequence of `meta`, `cmp`, `immediate`, `tproxy`,
  and verdict expressions plus the new anonymous counter. The amended
  normalizer preserves every expression kind and operand and ignores only the
  counter's live packet/byte values; unknown, extra, absent, or reordered target
  expressions fail. A replacement cannot hide behind an unchanged tag and
  handle.
- **The generation witness is complete for ruleset mutation.** The observer
  subscribes after the production install and before the initial full
  `NFTA_GEN_ID`, each snapshot is `GETGEN(G) -> complete GETRULE -> GETGEN(G)`,
  and the final notification drain plus generation read must still equal `G`.
  Notification allocation/receive loss is surfaced as `ENOBUFS`/overrun rather
  than accepted, while unrelated nft changes conservatively fail.
- **Multipart success cannot be inferred from a prefix.** The new contract uses
  a dedicated socket and absolute whole-operation deadline, validates kernel
  sender and request sequence on every message, checks every message and nested
  attribute length/alignment, accepts only the expected nft rule reply
  family/type, requires exactly one zero-status `NLMSG_DONE`, and rejects
  `NLMSG_ERROR`, `NLM_F_DUMP_INTR`, overrun, timeout/EOF, extra DONE, and all
  malformed/trailing/partial bytes before uniqueness. Strict single-reply
  `GETGEN` independently rejects extra or incomplete replies and consumes the
  full nonzero `NFTA_GEN_ID`, not the 16-bit `nfgenmsg.res_id` projection.
- **Packet and byte equality are in the kernel's domain.** The capture is bound
  to the exact root host-veth ingress and retains direction, ifindex, and
  protocol; `recvmsg(MSG_TRUNC)`, the full IPv4-sized L3 buffer, zero closing
  `PACKET_STATISTICS` drops, and explicit rejection of fragment/offload or
  capture-equivalence ambiguity make loss a failure. For a valid unfragmented
  IPv4/TCP skb, IPv4 validation trims to `tot_len` before the priority -150
  prerouting counter, while `nft_counter_eval` adds exactly one packet and
  `skb->len`. Thus the required checked packet equality and validated
  `tot_len` sum compare like with like; L2 length, snap length, and TCP payload
  length are correctly forbidden substitutes.

These conclusions match the primary kernel implementations in
[`nf_tables_api.c`](https://github.com/torvalds/linux/blob/master/net/netfilter/nf_tables_api.c),
[`nft_counter.c`](https://github.com/torvalds/linux/blob/master/net/netfilter/nft_counter.c),
[`ip_input.c`](https://github.com/torvalds/linux/blob/master/net/ipv4/ip_input.c),
[`af_packet.c`](https://github.com/torvalds/linux/blob/master/net/packet/af_packet.c),
and the [`nf_tables`](https://github.com/torvalds/linux/blob/master/include/uapi/linux/netfilter/nf_tables.h)
and [`netlink`](https://github.com/torvalds/linux/blob/master/include/uapi/linux/netlink.h)
UAPI headers.

### Preserved design and compatibility checks

- All Iterations 1–3 closures remain intact: READY is still the deterministic
  post-network-initialization barrier; the exhaustive zero-guest-L2-frame
  interval and pre-spawn capture remain; bounded pre-cleanup guest-console
  selection still precedes VMM-stderr fallback; and the exact
  `VmGuestExitUnreported` Job mapping, `FinalizeFailed`-only behavior, unchanged
  restart counts, and exact `/// CONTRACT_SHAPE: pure-function.` declaration
  remain authoritative.
- `install_outbound_tproxy` remains the sole install/adopt/delete owner. The
  counter is after the unchanged `iifname` and TCP predicates and before the
  byte-identical TPROXY/mark/accept tail. Same-tag adoption keeps counts and
  takes a new baseline; normal stop deletes the exact handle; boot recovery
  sweeps before reinstall; no comparison crosses restart; and the observer is
  read-only.
- Target uniqueness is evaluated only after a complete dump. Counter-free
  siblings remain valid `None`, exact target identity excludes sibling
  counters, and the quiescent-sibling teardown assertion remains unchanged.
- No REST/OpenAPI, Beacon Published Language, persistence, observation,
  describe, or rkyv schema changes are introduced. `RuleCounterSnapshot`, the
  normalized expression identity, generation decoding, and notification
  handling remain workspace-internal read projections.
- Every current summary and the 19-row reuse table agree on **8 REUSE-AS-IS / 10
  EXTEND / 1 CREATE-NEW**. The older 9/9/1 historical row remains explicitly
  superseded.
- The current DESIGN and architecture SSOT agree on Q7 and the mutation-aware
  Q9 oracle. The embedded DISTILL snapshot and roadmap still use the older
  generic “rule increment” wording, but every current design artifact marks
  that downstream state stale and requires a fresh DISTILL/roadmap handoff
  before DELIVER resumes; as in Iteration 3, that tracked handoff is not a live
  competing DESIGN decision.
- Mechanical scope is clean: `cd127251^` is exactly `dfe8eb24`; the commit
  changes only the five declared authoritative documentation files; and
  `git diff --check dfe8eb24 cd127251` passes. No source test run is warranted
  for this documentation-only design review.

### Iteration-5 defect counts

| Severity | Count |
|---|---:|
| Blocker | 0 |
| Critical | 0 |
| High | 0 |
| Medium | 0 |
| Low | 0 |

HIGH-3 is closed, all earlier findings remain closed, and no new defect was
found. Iteration 5 therefore returns **APPROVED**. This verdict approves the
DESIGN only; the explicitly required fresh DISTILL/roadmap remediation remains
mandatory before DELIVER execution.

---

## Iteration 6 — 2026-08-29

**Reviewed commit:** `85550e4a267cbd53ac266fa54f4d8cda164910af`
**Requested comparison base:** `cd12725159a6b2a92619f17aa4dc5f0ff621b842`
**Direct parent:** `a5bd158edb76308a44ccf99597d349950898f0f7`
**Scope:** adversarial review of the corrected D6 allocation-identity route and
native-metal execution contract; preservation of both install gates, Q7, Q9,
D7, public-schema boundaries, teardown ownership, sibling preservation, and the
8/10/1 reuse gate; plus a scan of the intervening DISTILL remediation,
verification-expectation stubs, active downstream handoffs, supersession labels,
and production seams that make the amended route executable.
**Verdict:** **APPROVED**

The requested range is not the target commit's direct diff. It contains the
intervening `a5bd158e` DISTILL remediation followed by the direct six-file
DESIGN/testing correction in `85550e4a`. This iteration reviewed both the
requested 15-file range and the six-file direct target change; it does not
attribute the intervening DISTILL edits to the target commit itself.

### D6 route and two-gate review

| Lifecycle case | Allocation identity and install site | Review result |
|---|---|---|
| Initial VM deploy | fresh `AllocationId`; fresh-start `Running` gate | **Correct.** D6 still requires `DriverType::Exec | DriverType::Vm` at the fresh-start site. |
| Unclean control-plane restart while durable VM Job intent stands | boot-epoch `VmReclamation` observes the non-terminal intended allocation without a live supervision claim, authors `StoppedBy::PlatformReclaimed`, and `WorkloadLifecycle` emits `RestartAllocation` with the same `AllocationId`; restart `Running` gate | **Correct and production-reachable.** This is the sole same-id VM Job route assigned to S-GTI-06a/06b. |
| Natural VM Job result or crash | run-once terminal finalization; no re-drive | **Correct.** It is explicitly forbidden as an S-GTI-06 substitute. |
| `overdrive workload restart` | generation replacement ends the old instance and mints a fresh `AllocationId`; fresh-start gate | **Correct.** It is explicitly forbidden as an S-GTI-06 substitute. |

The route is consistent in D6 and its rationale
(`design/wave-decisions.md:18,68-83,260-279`), the feature delta
(`feature-delta.md:557-607,823-831,1005-1007`), ADR-0089 (`:42-89,383-389`),
the normative brief (`brief.md:10135-10149,10306-10309`), DISTILL
(`distill/test-scenarios.md:73-79,244-260`), and E09
(`verification/expectations/E09-vm-guest-reclamation-and-stop-preserve-rules/README.md:7-12,39-50`).
Natural Job finalization remains earlier than the restartable branch in
`workload_lifecycle.rs:823-859`; `is_natural_exit` excludes only Platform
Reclamation (`:1444-1465`); and the latter branch emits
`RestartAllocation { alloc_id: failed.alloc_id.clone(), spec.alloc:
failed.alloc_id.clone() }` (`:993-1026`). The boot drive uses the same desired
join, pure reclamation diff, and executor as steady state
(`vm_reclamation_boot.rs:78-147`); the executor writes the Platform Reclamation
ending and re-enqueues `WorkloadLifecycle` (`action_shim/reclamation.rs:135-224`).
The generation-replacement/fresh-schedule branch derives a new id through
`mint_alloc_id` (`workload_lifecycle.rs:1078-1183`).

Both security gates remain mandatory and distinct. The target source contains
the VM-inclusive fresh-start gate at `action_shim/mod.rs:1727-1749` and the
VM-inclusive restart gate at `:2023-2056`; both run before the corresponding
exit/EXEC release. Teardown remains intentionally driver-kind agnostic. No
amended artifact collapses the restart proof into a fresh deploy or removes
either gate.

### Native-metal execution boundary

The amended execution contract is consistent across the active #222 DESIGN,
architecture, DISTILL, and E07-E09 verification surfaces:

- runtime evidence requires a native, non-virtualized x86_64 host with a
  hardware-backed, usable `/dev/kvm`; nested or otherwise virtualized hosts and
  Lima are compile-only/non-signal;
- `kvm-tests` remains the Cargo feature name, not a substrate claim;
- the canonical transport remains `cargo xtask metal run --` with the
  user-provided `OVERDRIVE_METAL_TARGET`/gitignored `.env` target;
- preflight fails closed on architecture, device/open/KVM API/create-VM, CPU
  extension, or virtualization-detection uncertainty, and the host-wide lease
  spans command execution and cleanup.

The global rule states these constraints at
`.claude/rules/testing.md:1447-1482`; the feature DESIGN repeats them at
`wave-decisions.md:76-83,281-287`, `feature-delta.md:653-657,1024-1027`,
ADR-0088 `:179-186`, ADR-0089 `:66-80`, and `brief.md:10262-10275`.
DISTILL pins the same fail-closed qualification and lease at
`test-scenarios.md:141-164`, and E07-E09 plus the expectation index preserve it.
The three new pending runner stubs pass `bash -n` and truthfully report that no
runtime command or evidence exists.

Historical #222 changelog rows still contain the old same-id and “nested KVM”
wording, but `brief.md:10328` explicitly supersedes those exact clauses and
leaves them as history rather than live guidance. No active #222 DESIGN summary
uses a virtualized or nested runtime as evidence.

### Prior finding and compatibility regression scan

- **HIGH-1 remains closed:** capture is armed before real VMM spawn and the
  exhaustive zero-guest-L2-frame interval remains fail-conservative.
- **MEDIUM-1 remains closed:** bounded pre-cleanup guest-console selection,
  VMM-stderr fallback, and neither-source totality remain unchanged.
- **MEDIUM-2 remains closed:** the exact `VmGuestExitUnreported` Job mapping,
  `FinalizeFailed`-only behavior, restart-state preservation, and exact
  `/// CONTRACT_SHAPE: pure-function.` requirement remain authoritative.
- **HIGH-2 remains closed:** the normative brief, ADR index, both ADRs, feature
  delta, and wave decisions all carry the current Q7/Q9/D7 contract.
- **HIGH-3 remains closed:** normalized full-program identity, strict complete
  `GETRULE`/single-reply `GETGEN`, the full nonzero `NFTA_GEN_ID`, loss-detecting
  `NFNLGRP_NFTABLES` guard, exact packet/IPv4-`tot_len`/nft-`skb->len` equality,
  checked arithmetic, and conservative reset/replacement/wrap/loss handling are
  unchanged.
- The observer remains read-only; `install_outbound_tproxy` remains the sole
  install/adopt/delete owner; same-tag adoption, by-handle teardown, boot sweep,
  exact sibling nonmutation, and the no-cross-restart comparison rule remain.
- No REST/OpenAPI, Beacon Published Language, persistence, observation,
  describe, rkyv, crate, port, daemon, or dependency surface is introduced.
  Every current summary and the nineteen-row table retain **8 REUSE-AS-IS / 10
  EXTEND / 1 CREATE-NEW**.

### LOW-1 — downstream-status prose still calls the remediated DISTILL stale

`design/wave-decisions.md:405-410` and `brief.md:10315-10320` still say the
current DISTILL and roadmap carry generic Q7/Q9 wording. In the requested range,
`a5bd158e` has already corrected DISTILL: it now contains the sole boot-
reclamation same-id route, native non-virtualized substrate, Q7 lifecycle, and
the complete D7 oracle. Only the pending roadmap remains stale.

This is status/handoff drift, not a competing behavioral contract: the corrected
DISTILL text is explicit and the roadmap has `validation.status = pending`, so
repository policy already forbids executing it. On the next authoritative docs
touch, narrow those two sentences to the roadmap and completed DISTILL handoff.

The target roadmap still names invalid same-id substitutes and “nested KVM.”
That known downstream artifact remains a mandatory remediation condition before
DELIVER can resume; its pending validation and explicit DISTILL carry-forward
make it non-authoritative for this DESIGN verdict. It must not be approved or
executed in that state.

### Mechanical verification

- `git diff --check cd127251..85550e4a` — pass.
- `git diff --check a5bd158e..85550e4a` — pass.
- Requested range — 15 files, 1,077 insertions, 677 deletions; documentation
  plus three pending expectation-stub pairs and the expectation index.
- Direct target change — six files, 169 insertions, 63 deletions; testing rule
  plus authoritative DESIGN/ADR/brief documentation only.
- E07/E08/E09 pending runner syntax — pass under `bash -n`.
- Target roadmap validation — `pending`; no approval or execution inferred.
- No source test run is warranted for the documentation-only target correction;
  production source was inspected only to validate route feasibility and both
  gate locations.

### Iteration-6 defect counts

| Severity | Count |
|---|---:|
| Blocker | 0 |
| Critical | 0 |
| High | 0 |
| Medium | 0 |
| Low | 1 |

All blocking severities are zero. The corrected same-id lifecycle, two install
gates, native-metal trust boundary, Q7/Q9/D7 oracle, compatibility boundary, and
reuse tally are coherent. Iteration 6 therefore returns **APPROVED**. LOW-1 is a
non-blocking handoff-status cleanup; the still-pending roadmap must be remediated
and reviewed before DELIVER execution.
