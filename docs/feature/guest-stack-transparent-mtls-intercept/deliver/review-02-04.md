# Adversarial review — step 02-04

- **Feature:** `guest-stack-transparent-mtls-intercept`
- **Step:** `02-04` — Rust D7 accounting and the sole E07 journey
- **Reviewer:** `nw-software-crafter-reviewer` (fresh isolated reviewer)
- **Review ID:** `code_rev_20260830_02_04_iteration_1`
- **Iteration:** 1
- **Reviewed commit:** `45cab7eaead0e69c80238f251777a984af0d4a72`
- **Parent:** `9be3680b82ed216fa44cb1a231ad62fb6a79a41b`
- **Subject:** `feat(guest-stack-mtls): prove exact D7 and E07 journey`
- **Trailer:** `Step-Id: 02-04`
- **Verdict:** **NEEDS_REVISION**

## Executive summary

Step 02-04 cannot advance. The required native S-GTI-01 execution fails on the
reviewed commit: the fixture deliberately keeps the successful guest process
alive for 45 seconds, the host then publicly stops that Job, and the terminal
projection correctly contains no exit code. The test nevertheless requires
`Some(0)` and fails at `guest_stack_mtls_egress.rs:1299`. The DES GREEN claim is
therefore empirically false for one of the five explicitly required native
scenarios.

Four independent D7 defects can also allow false green results. The tap witness
checks the required pre-intercept all-EtherType zero-frame interval, but the
host-veth witness silently discards every non-IPv4/TCP frame. The peer-wire
confidentiality scan concatenates packet payloads in capture order without TCP
sequence reassembly or loopback-copy de-duplication, so ordinary segmentation,
retransmission, reordering, or the two loopback directions can break a
plaintext marker before it is searched. The purported strict multipart
`GETRULE` decoder accepts data replies without `NLM_F_MULTI`, and its positive
property fixture encodes exactly that non-multipart form. Finally, the same-tag
adoption test adopts a zero-hit rule immediately after installation, so it
cannot distinguish genuine adoption from delete/reinsert or counter reset.

The sole E07 expectation itself is appropriately narrow and valid. Its
captured native-metal run uses the built default-feature product, deploys one
`[service]+[exec]` callee and one `[job]+[vm]` caller, observes the exact reply
and successful exit-code projection, and uses only public stop/cleanup
surfaces. There is no E08/E09 expectation, no Rust test launches the product
binary or an expectation, and the async `release_for_exit_emission` lifecycle
remains awaited at both call sites. Those sound boundaries do not compensate
for the failed native scenario and incomplete D7 oracles.

## Review scope and evidence integrity

The review covered the complete parent-to-target diff, all fourteen roadmap
mappings owned by 02-04, the final DESIGN/DISTILL D7 and expectation-boundary
contracts, the DES log, and the checked-in E07 evidence. The worktree already
contained user-owned untracked review and instruction files; none was reset,
discarded, or committed.

E07 was captured from a dirty parent worktree, as its metadata says. This was
audited rather than assumed: applying the captured `dirty-diff.patch` to the
recorded base `9be3680b` reproduced every non-evidence file at the reviewed
commit byte-for-byte. `execution-log.json` differed only by the later GREEN and
COMMIT events, which necessarily postdate the product run. The checked-in
product output and run log agree on a zero runner exit, the exact E07 reply,
the Service at 1/1, and the VM Job at `Terminated`, `Succeeded`, exit 0.

This reviewer changed only this Markdown review artifact. No implementation,
test, expectation, DES, or evidence file was edited, and no mutation testing
was run.

## Defect counts

| Severity | Count |
|---|---:|
| Blocker | 1 |
| Critical | 0 |
| High | 4 |
| Medium | 2 |
| Low | 0 |

## Mechanical evidence

### Commit scope, evidence, and DES discipline

| Check | Result | Evidence |
|---|---|---|
| Exact parent | PASS | `45cab7ea^` is `9be3680b`. |
| Commit scope | PASS | 23 files, 6,097 insertions, 279 deletions; the large majority of added lines are captured E07 evidence. |
| Trailer | PASS | Exact `Step-Id: 02-04`. |
| Production/source whitespace | PASS | `git diff --check` passes when the raw E07 evidence directory is excluded. |
| Raw-evidence whitespace | EXPLAINED | Full `git show --check` reports blank `+ ` lines inside the verbatim dirty patch and padded table rows in `product-run.out`/`run.log`; these are faithful captured bytes, not source formatting defects. |
| Dirty evidence reproducibility | PASS | The recorded base plus `dirty-diff.patch` reproduces the reviewed non-evidence target files exactly. |
| DES order | FAIL | RED, GREEN, and COMMIT are chronological, but RED uses the forbidden `cargo test` runner and no fresh legal nextest RED exists; fresh native review also falsifies GREEN. |
| Mutation discipline | PASS | No mutation run or mutation-exclusion edit occurred in this step. |

The implementation expands into CLI serve, control-plane lifecycle, and exit
projection code. Those files are tightly related compiler/production fallout,
not unrelated scope. The exact exit-code projection is correctly limited to
`Completed` while `Terminated` and `Failed` while `Failed`; D1 is a test-fixture
lifecycle contradiction, not a request to weaken that projection.

### Contract Shape and executable mapping

**Overall: FAIL.** The roadmap owns fourteen executable identities in this
step. Their count stays below the `2 x behavior` test budget, but semantic
shape is not satisfied.

| Check | Result | Evidence |
|---|---|---|
| Five stakeholder mappings | PASS mechanically | S-GTI-01/02/03/04/07 resolve to their named Rust functions and carry the declared shape lines. |
| Pure-function declarations | PASS mechanically | P-GTI-D7-ERROR-CLOSURE and P-GTI-ILLEGAL-01/02/05 use the exact mandated declaration. |
| D7 supporting locator | **FAIL** | `D7-EXACT-RULE-HIT-WITNESS` is mapped as `unbounded-preservation`, but the named function is declared `pure-function` and checks only synthetic values. |
| S-GTI-02 preservation universe | **FAIL** | The host-veth half of the all-EtherType pre-cut universe is reduced to parsed TCP. |
| S-GTI-03 preservation universe | **FAIL** | The complete peer-wire universe is not reconstructed as byte-correct TCP streams before the unbounded plaintext search. |
| Same-tag bounded delta/complement | **FAIL** | The test proves one zero counter after two calls, not preservation of a nonzero counter/program/handle and one-guard-at-a-time ownership. |
| Outcome anchors | PASS | The transitioned stakeholder Rust scenarios retain their exact Outcome Anchor declarations. |
| Layer separation | PASS | Rust tests stay in-process; E07 alone drives the built product as a black-box expectation. |

## Findings

### D1 — required native S-GTI-01 fails because the host stops a still-running Job and expects a natural exit code

- **Severity:** Blocker
- **Dimension:** Executable acceptance, lifecycle truth, and GREEN validity
- **Locations:**
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:224-249`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:1282-1308`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:1639-1644`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:1703-1715`

After receiving the exact response, the generated guest sleeps for 45 seconds
so the host can inspect the live kTLS socket. The host finishes the D7
observation much earlier and then calls the public `stop` command before the
guest process exits. That legitimately produces an operator-stopped terminal
row with `exit_code: None`. `assert_exact_lifecycle_delta` nevertheless says
the Job "cleanly completed" and requires `Some(0)`.

The qualified native execution failed 1/1 with:

```text
assertion failed: a cleanly completed Job exposes exit code zero
left: None
right: Some(0)
```

This is one of the roadmap's five mandatory native scenarios, so a general
workspace pass and the separate E07 run cannot establish GREEN.

**Required remediation:** make the Rust journey complete by its reply-dependent
natural Job exit after the live capture is safely obtained, then observe the
real terminal result and perform public cleanup. Preserve the exact `Some(0)`
success assertion; do not paper over the contradiction by accepting the
operator-stop projection. Rerun all five native scenarios after the change.

### D2 — the host-veth pre-intercept witness ignores non-TCP and non-IPv4 guest frames

- **Severity:** High
- **Dimension:** D7/Q9 closed-world packet boundary and test honesty
- **Locations:**
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:1047-1112`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:1115-1124`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:1617-1638`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:2091-2190`

Both exact interfaces are captured with `ETH_P_ALL`, but only `tap_capture` is
filtered directly through `guest_frame_precedes_capture_ready` and required to
be empty. `guest_capture` is sent to `audit_guest_egress_boundary`, which calls
`parse_tcp_segment` and immediately continues past every non-IPv4 or non-TCP
frame. S-GTI-02 later checks only the resulting TCP segment collection.

An ARP, IPv6, VLAN/unknown-EtherType, ICMP, UDP, or other guest-originated frame
on the host-veth before `intercept-live` therefore leaves the test green. This
directly violates the ratified requirement that **both** the tap and host-veth
witness zero guest frames of every EtherType in the guarded interval.

**Required remediation:** retain and audit the complete exact-ifindex
host-veth frame collection before protocol parsing, conservatively classify
missing/ambiguous direction or timestamp as pre-cut, and require the same
all-EtherType zero-frame interval on both interfaces. Keep the later exact TCP
tuple/counter projection as a separate post-cut assertion.

### D3 — the peer-wire scanner can miss plaintext because it is not TCP reassembly

- **Severity:** High
- **Dimension:** Unbounded preservation and confidentiality oracle completeness
- **Locations:**
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:765-808`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:1542`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:1839-1885`

`scan_frames` groups payloads by directional four-tuple and appends them in
AF_PACKET dequeue order. It ignores TCP sequence numbers and the captured
`packet_type`. On loopback, outgoing and host copies of the same segment can be
placed into the same directional stream; retransmissions and out-of-order
segments have the same effect. For example, a marker split as `A`/`B` can be
materialized as `A,A,B,B`, making a search for `A+B` return zero even though the
clear marker crossed the peer wire. Gaps and overlapping conflicting bytes are
also not rejected.

The TLS application-record parser consumes the same corrupted concatenation,
so its positive observation is not a substitute for byte-correct stream
reconstruction. This violates the roadmap's explicit complete, lossless
peer-wire preservation oracle.

**Required remediation:** reconstruct each direction by TCP sequence space,
de-duplicate the loopback copies, handle retransmission consistently, and fail
closed on unresolved gaps, conflicts, truncation, or ordering ambiguity before
searching every reconstructed peer-port byte stream for both litmus values.

### D4 — strict GETRULE decoding accepts a non-multipart response

- **Severity:** High
- **Dimension:** Netlink protocol correctness and conservative error closure
- **Locations:**
  - `crates/overdrive-netlink/src/nft.rs:969-1034`
  - `crates/overdrive-netlink/src/nft.rs:1958-1986`

`decode_rule_dump_datagram` reads `nlmsg_flags` but checks only
`NLM_F_DUMP_INTR`. It never requires `NLM_F_MULTI` on `NFT_MSG_NEWRULE` data
messages. The positive property helper constructs both the rule message and
`NLMSG_DONE` with flags zero, and calls that a strict valid dump.

Consequently a single non-multipart data reply followed by a synthetic DONE is
accepted as a complete dump. The implementation cannot claim the roadmap's
strict multipart framing or rejection of every partial/malformed response.

**Required remediation:** enforce multipart flag semantics on every data
message, retain strict terminal DONE rules, and add generated malformed cases
for missing/inconsistent multipart flags as part of P-GTI-D7-ERROR-CLOSURE.

### D5 — same-tag adoption is proven only at a vacuous zero-counter baseline

- **Severity:** High
- **Dimension:** D7 adoption/replay preservation and bounded-change complement
- **Locations:**
  - `crates/overdrive-worker/tests/integration/mtls_intercept_install.rs:574-625`

The test installs a guard and immediately calls the same installer again. It
then observes one rule with `{ packets: 0, bytes: 0 }`. A defective
implementation that deletes and reinserts the rule, resets its counter, or
replaces it with a fresh byte-identical rule can satisfy every assertion. The
test does not preserve a before handle/program/userdata/counter/generation
snapshot and does not check that the rule remains owned while exactly one of
the two guards has been dropped.

The approved D7 contract expressly requires same-tag adoption to keep
accumulated counts and unchanged rule identity while establishing a fresh
baseline. Zero before and zero after cannot witness that preservation.

**Required remediation:** drive a nonzero accumulated count through the
production-owned rule, snapshot its exact handle, userdata, normalized program,
counter, and generation, then prove the second install changes none of them.
Drop each guard in turn and prove ownership remains until the final guard and
then tears down exactly once, without test-owned replacement/reset shortcuts.

### D6 — D7-EXACT-RULE-HIT-WITNESS has the wrong Contract Shape and a synthetic-only body

- **Severity:** Medium
- **Dimension:** Traceability and Contract Shape semantics
- **Locations:**
  - `docs/feature/guest-stack-transparent-mtls-intercept/deliver/roadmap.json:230-236`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:2302-2331`

The approved mapping names
`d7_exact_rule_hit_witness_is_loss_and_mutation_conservative` and declares it
`unbounded-preservation`. The implementation declares the exact same function
`pure-function` and checks one hand-built before/after pair plus one competing
count. It does not observe the unbounded capture/ruleset universe represented
by the supporting contract's name.

S-GTI-02 contains much of the native D7 machinery, but it does not repair an
exact executable mapping that resolves to the wrong semantic shape.

**Required remediation:** make the mapped D7 executable an honest
unbounded-preservation test over the complete production-path universe, with
the exact declaration, or obtain a separately reviewed roadmap remap. Keep
small synthetic decoder checks under pure-function identities rather than
using one as the full D7 witness.

### D7 — the recorded RED used the forbidden runner and the logged GREEN is not reproducible

- **Severity:** Medium
- **Dimension:** DES/TDD process integrity
- **Locations:**
  - `docs/feature/guest-stack-transparent-mtls-intercept/deliver/execution-log.json`

The only 02-04 RED event records:

```text
cargo xtask lima run -- cargo test -p overdrive-netlink ...
```

Repository policy requires nextest for Rust test execution except doctests.
The roadmap recognizes the earlier partial worktree as failing evidence but
explicitly forbids adopting it as GREEN evidence, and the repository's fresh
crafter rule requires a replacement executor to rerun and log only phases it
actually executes. No fresh legal nextest RED is present. More importantly,
fresh qualified native execution now proves the broad GREEN claim false via
D1.

**Required remediation:** after D1-D6 are corrected, execute and record an
honest RED -> GREEN -> COMMIT cycle using the mandated runner and the complete
required native selection. Do not rewrite history or claim a phase the
executing crafter did not run.

## Boundary and lifecycle audit

| Boundary | Result | Evidence |
|---|---|---|
| Exactly one built-product expectation | PASS | `verification/expectations` contains E01-E07 only; this step adds/changes only E07 and no E08/E09 exists. |
| E07 source shape | PASS | Checked-in callee is `[service]+[exec]`; caller is `[job]+[vm]`. No `[vm]+[service]` unsupported category is introduced. |
| E07 black-box purity | PASS | One outer `cargo xtask metal run --` drives the built `target/debug/overdrive`; the oracle uses public deploy/describe/stop output and exact reply only. |
| No private D7 in E07 | PASS | Capture, nft/netlink, counter, generation, kTLS, and private cleanup assertions remain outside the expectation. |
| Rust/expectation separation | PASS | Rust tests do not invoke the built Overdrive binary, `cargo test`, nextest, or an expectation runner; E07 does not invoke Rust tests or link/import product crates. |
| Checked-in example boundary | PASS | E07 runs the repository example; it does not recreate caller/callee specs inline in the expectation. |
| Legacy/no-token VM category | PASS | No missing-token, no-op, or legacy VM path is added; all exercised workloads remain mesh identities. |
| Async exit-emission release | PASS | `release_for_exit_emission(handle).await` remains awaited at both fresh and restart call sites after identity, Running persistence, and intercept installation; there is no detached spawn. |
| Exit-code projection | PASS in production, FAIL in S-GTI-01 fixture | Handlers expose Completed only for Terminated and Failed only for Failed. E07 observes natural exit 0; D1 stops the Rust Job and then incorrectly expects the same projection. |
| Public cleanup | PASS for E07 | Evidence shows public stops and removal of only E07-owned runtime materialization. |

## Independent verification

All Lima Rust executions used nextest. The native selections used the canonical
metal runner with the qualified kernel/rootfs and retained lease. The initial
metal invocation without required artifact environment correctly refused
preflight; the qualified runs below followed. The standard 23 default-suite
skips were unchanged and no new exception was needed.

| Verification | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| Lima `cargo nextest run -p overdrive-netlink` | PASS — 47/47 |
| Lima focused worker second-ensure/adoption selection | PASS — 2/2; D5 remains a static oracle defect |
| Lima focused control-plane exit-code projection | PASS — 1/1 |
| Lima clippy for netlink, worker, control-plane, and CLI, all targets with `integration-tests,kvm-tests`, `-D warnings` | PASS |
| Lima default workspace nextest suite | PASS — 2,282/2,282; 23 skipped |
| E07 source check, bash syntax, and session-lifecycle fault harness | PASS |
| Native S-GTI-02 | PASS — 1/1 in 16.206s; D2 means its all-frame complement is incomplete |
| Native M-GTI-CONCURRENT-DEPLOY | PASS — 1/1 in 28.201s |
| Native S-GTI-01 | **FAIL — 1/1 at line 1299; `None` versus `Some(0)`** |
| Native S-GTI-03/S-GTI-04/S-GTI-07 | PASS — 3/3; D3 remains a static oracle defect |
| Mutation testing | NOT RUN — repository rule reserves one run for the final DELIVER-wave gate |

## Iteration 1 verdict

**NEEDS_REVISION.** Return D1-D7 to the original step-02-04 crafter. The same
reviewer must re-review the remediation and append the next iteration to this
artifact. Do not begin step 02-05 until all findings are closed, every required
native scenario passes, a legal reproducible DES cycle exists, and the final
verdict is **APPROVED**.
