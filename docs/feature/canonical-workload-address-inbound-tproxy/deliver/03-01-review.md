# Adversarial Implementation Review — Step 03-01

**Feature:** canonical-workload-address-inbound-tproxy (GH #241)
**Step:** `03-01` — start_alloc per-port inbound install (replace tproxy_guard = None)
**Commit under review:** `83156c56`
**Scenarios:** S-NRULES / S-DPORT / S-JOB0 (D-A1 / D-BLOCKER1 / D-TME-10)
**Reviewer:** adversarial pass (Opus), with real-kernel re-execution corroboration
**Date:** 2026-06-23

## Verdict: **APPROVED**

- **Blocking: 0**
- **Suggestion (non-blocking): 1** — stale general claim in the 03-02-owned skeleton doc.
- **Nitpick (non-blocking): 1** — `#241-deferred` issue-number imprecision on an adjacent file.
- **Praise: 2.**

This is the keystone install that closes the prior `tproxy_guard = None` deferral and the
direct corrective for the transparent-mtls #236 precedent ("`install_inbound_tproxy` was built
but no production call site fed it"). The implementation is **design-faithful on the named API
surface** — zero invented surface — and **all three Tier-3 acceptance scenarios pass on a real
kernel**, which I verified by re-running them myself (not by trusting the execution log).

---

## Evidence I reproduced (not narrated)

Ran the three scenarios under Lima as root, kernel **7.0.0-22-generic**, driven through the
production `start_alloc` call site:

```
PASS [0.237s] inbound_capture_rule_matches_declared_service_port_not_ephemeral_leg_c_port
  [03-01-dport] match dport=18777 (declared), tproxy-to=127.0.0.1:34313 (ephemeral leg-C) — distinct
PASS [0.251s] two_declared_listeners_install_exactly_two_inbound_capture_rules
PASS [0.239s] job_kind_workload_with_no_listeners_installs_no_inbound_capture_rule
  [03-01-job0] no overdrive-mtls prerouting chain after Job start_alloc (zero inbound rules)
Summary: 3 tests run: 3 passed, 178 skipped
```

The S-DPORT line is the load-bearing signal: the *live* nft rule's match key is the **declared**
service port (18777) and its `tproxy to` target is a **different**, ephemeral port (34313) —
proving the install is real and the inert self-referential `match == target` shape is structurally
absent. The execution-log GREEN is corroborated, not taken on faith.

> Kernel caveat (informational): this is dev-Lima 7.0, the inner-loop signal. The MERGE-BLOCKING
> pinned-6.18 appliance-kernel matrix (ADR-0068) is **03-02's** acceptance bar (S-WS), not 03-01's.
> Correct per the roadmap; noted so the kernel-pin obligation is not lost.

---

## Why this passes the bars that matter

### A. Design fidelity — no invented API surface (CLAUDE.md "implement to the design")
The install loop (`mtls_intercept_worker.rs:622-628`) is exactly the design's shape:

```rust
let mut inbound_tproxy_guards = Vec::new();
if let Some(workload_addr) = spec.workload_addr {
    for port in &spec.service_ports {
        let virt = SocketAddrV4::new(workload_addr, port.get());
        inbound_tproxy_guards.push(install_inbound_tproxy(virt, leg_c_addr.port())?);
    }
}
```

- `install_inbound_tproxy(virt: SocketAddrV4, agent_port: u16)` reused **as-is** — signature
  unchanged (`mtls_intercept.rs:248`). No new install primitive, no new arg.
- `leg_c_addr` is the **inline local** (`:596-599`), not `self.leg_c_addr(alloc)` — exactly as the
  design pinned.
- match `dport` = declared service port (`port.get()`); tproxy-to = ephemeral `leg_c_addr.port()`.
- `Option<TproxyInterceptGuard>` → `Vec<TproxyInterceptGuard>` as specified; threaded cleanly
  through `spawn_legs_and_record` / `record_intercept_full` and held on `AllocIntercept`
  (`:277`).

### B. Vertical slice — the install is the production call site
All three Tier-3 tests drive `worker.start_alloc(&spec)` and observe the live `overdrive-mtls`
nft ruleset. The shared harness (`inbound_tproxy_harness.rs`) **never calls
`install_inbound_tproxy` itself** — it only supplies `workload_addr` + `service_ports` on the
spec (the C3-seam channel from 01-02) and reads the dump. This is the textbook inverse of the #236
"test hand-installs the missing production call site" tell.

### C. RAII teardown is correct
`stop_alloc` does `self.intercepts.lock().remove(alloc_id)` → the `AllocIntercept` (holding the
guard `Vec`) drops → each guard's `Drop` removes its per-virt rule by handle. S-NRULES asserts
2 rules pre-teardown and **0 leftover** post-teardown; verified live.

### D. No regression to adjacent suites
`start_alloc_installs_both_tproxy.rs` and `bidirectional_walking_skeleton.rs` both build specs with
`workload_addr: None` / `service_ports: Vec::new()` → the new loop runs **zero** iterations for
them → no double-install, no conflict. The unit test at `:1933` correctly threads the new
`Vec::new()` argument.

### E. RED_UNIT skip is justified
03-01's roadmap criteria carry **no mutation gate** (unlike 01-01/01-02/02-01/02-02) — it is a
Tier-3 real-kernel I/O step. The `if let Some / for port` install loop has no pure decision branch
the three scenarios don't pin (Some→N, declared-port-key, None→0); PBT is the wrong tool for nft
I/O. The skip is consistent with the roadmap, not a dodge.

---

## Non-blocking findings

### suggestion (non-blocking) — stale GENERAL claim in `bidirectional_walking_skeleton.rs:604-606`
The `build_server_spec` doc comment states:

> `host_veth = None`: the inbound nft-TPROXY rule is the test-installed redirect to the
> production-bound leg-C (the production inbound rule is **#241-deferred — `start_alloc` installs
> none**) ...

After 03-01 this general statement is **globally false** — `start_alloc` now installs inbound
rules whenever `spec.workload_addr` is `Some`. It remains *locally* true only because this spec
sets `workload_addr: None`. This is precisely the pattern CLAUDE.md § "Behavior change must mark
stale adjacent docs" guards against, and the memory note records it recurred in the
transparent-mtls 03-01/03-02 reviews.

**Why non-blocking, not blocking:** `bidirectional_walking_skeleton.rs` is the *named target* of
the immediately-following step **03-02**, which removes the synthetic `INBOUND_VIRT_IP/PORT` virt
and the test-installed `install_inbound_tproxy` from this file wholesale (roadmap 03-02 criteria 3).
The stale comment dies with that apparatus.

**Recommendation:** either (a) tighten the comment now to "for THIS spec (`workload_addr: None`)
`start_alloc` installs none; the production per-port install lands in 03-01 and the test wiring is
removed in 03-02", or (b) explicitly confirm 03-02 deletes lines 602-621. (a) is one line and
closes the window for a reader who lands between 03-01 and 03-02.

### nitpick (non-blocking) — `#241-deferred` issue-number drift at `mtls_intercept.rs:28`
The module comment labels the `server_dial_addr` orig-dst resolution "#241-deferred". With #241's
**install** scope now landed, the residual inbound-resolution work is intended-peer pinning (#242)
/ the DNS daemon (#243). The statement "dials orig-dst verbatim … NOT touched here" is still
factually true and the file is out of 03-01's scope, so this is low-priority — but re-pointing the
tag to #242 would keep the forward reference accurate.

---

## Praise

- **praise:** The S-DPORT negative pin (`assert_ne!(target_port, SERVICE_PORT)`) is exactly the
  right adversarial test. A naive "a rule was installed" check passes the inert self-referential
  mutant (match keyed on `leg_c_addr.port()`); this assertion kills it, and the live run proves
  the two ports genuinely differ (18777 vs 34313). This is litmus, not Fixture Theater.
- **praise:** The harness's vertical-slice discipline is textbook — it carries an explicit
  module-doc contract that it "NEVER calls `install_inbound_tproxy` itself," supplies only the spec
  channel, and reads the real nft dump. Promoting it to a shared fixture (≥2 consumers, non-trivial
  cross-process kernel-state lock + shared-infra scrub) is the correct call per the shared-fixture
  rule.

---

## Acceptance-criteria ledger

| AC | Requirement | Status |
|---|---|---|
| S-NRULES | 2 declared ports → exactly 2 rules; both released on teardown | ✅ verified live |
| S-DPORT | match dport = declared port; tproxy-to = distinct ephemeral leg-C | ✅ verified live (18777 vs 34313) |
| S-JOB0 | None addr / empty ports → 0 rules, no guard retained | ✅ verified live |
| Field change | `Option<TproxyInterceptGuard>` → `Vec<TproxyInterceptGuard>` | ✅ `:277`, threaded + dropped on stop |
| Production call site | no test-installed `install_inbound_tproxy` stands in | ✅ all 3 drive `start_alloc` |

All five met. Verdict **APPROVED**; the two non-blocking findings are advisory and the first is
owned by the next step (03-02).
