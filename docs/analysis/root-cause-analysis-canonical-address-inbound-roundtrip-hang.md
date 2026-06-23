# RCA — canonical-address inbound mTLS round-trip "hang" (S-WS keystone, GH #241)

- **Subject test:** `crates/overdrive-control-plane/tests/integration/canonical_address_inbound_walking_skeleton.rs::workload_reached_at_canonical_address_terminates_mtls_end_to_end`
- **Feature / step:** `canonical-workload-address-inbound-tproxy` (GH #241), step 03-02 (S-WS keystone).
- **Kernel:** `uname -r = 7.0.0-22-generic` (dev Lima; merge gate is the pinned-6.18 Tier-3 matrix, ADR-0068).
- **Verdict:** **PRODUCTION gap** (a convergence/lifecycle defect), **NOT** a test-fixture gap and **NOT** a defect in step 03-01's inbound nft-TPROXY install (which is proven correct below). The failing leg is **Leg 1 — routing/provisioning**: the server workload's per-workload netns + host-veth + both nft-TPROXY rules are **destructively torn down ~230 ms after the alloc reaches Running, by Overdrive's own convergence path, and never restored**, so the client SYN reaches a host that no longer has a route to `10.99.0.2` and no listener.
- **Investigation discipline:** every command host-`timeout`-wrapped; VM scrubbed after every run; no `| tail`/`| head` on Lima/nextest; bounded connect + file-captured diagnostics (no streaming hang).

---

## 1. The reported symptom is mis-stated: the dial does NOT hang, the connect TIMES OUT

The brief described a hang killed at nextest's 120 s slow-timeout. The actual shape, once a bounded `TcpStream::connect_timeout(10s)` replaced the unbounded `TcpStream::connect`:

```
[03-02] server canonical workload_addr = 10.99.0.2:18941
[03-02] DIAG inbound client connect 10.99.0.2:18941 FAILED after 10.014017113s: kind=TimedOut err=connection timed out
```

A **`connect()` that times out** = the SYN is sent but **no SYN-ACK ever returns**. This is unambiguously a **routing or capture** failure (Leg 1/Leg 2), not a TLS/cert failure — a handshake/trust failure would complete the TCP connect first and then error fast at the TLS layer, never block in `connect()`. The original "hang" was just the OS default SYN-retransmit window (~127 s) exceeding nextest's 120 s reap. (debugging.md §2 — the error *kind* `TimedOut` names the layer that gave up, not the mechanism; the mechanism is below.)

`tcpdump` on the client host-veth during the dial confirms the SYN leaves the client and gets retransmitted with no answer:

```
23:33:41.372412 IP 10.99.200.2.39432 > 10.99.0.2.18941: Flags [S], seq 458650516, ...
23:33:42.396026 IP 10.99.200.2.39432 > 10.99.0.2.18941: Flags [S], ...    (retransmit)
23:33:43.420249 IP 10.99.200.2.39432 > 10.99.0.2.18941: Flags [S], ...    (retransmit)
... 5 SYNs, 0 replies ...
tcpdump: ovd-hv-0000: No such device exists      <-- the workload host-veth is GONE
```

---

## 2. The decisive population-diff: the workload's netns/veth/rules vanish ~230 ms after Running

Per debugging.md §5 (compare populations) and §11 (an empty/absent surface is a downstream symptom — walk one step upstream to the producer), a 250 ms-cadence poll of the alloc row + kernel state captured the **exact transition** (full timeline pinned by bpftrace below):

```
[POLL t=0] rows=[canonical-addr-server/alloc=alloc-canonical-addr-server-0/Running/addr=Some(10.99.0.2)/reason=Some(Started)] | inbound_daddr_rule=1 egress_iif_rule=1 any_ovd_ns=1 any_ovd_hv=1 server_pid=702304,... alloc_scope=1
[POLL t=1] rows=[canonical-addr-server/alloc=alloc-canonical-addr-server-0/Running/addr=None       /reason=Some(Started)] | inbound_daddr_rule=0 egress_iif_rule=0 any_ovd_ns=0 any_ovd_hv=0 server_pid=702304,... alloc_scope=1
... (t=2 .. t=55 identical: ns=0, hv=0, rules=0, addr=None, server_pid 702304 still alive) ...
```

At **t=0** everything is correct and complete:
- alloc `Running`, canonical `workload_addr = Some(10.99.0.2)` materialised on the V2 row;
- the **PRODUCTION inbound rule is present** (`inbound_daddr_rule=1` — the `ip daddr 10.99.0.2 tcp dport 18941 tproxy to 127.0.0.1:<legC>` rule step 03-01 installs);
- the egress rule is present (`egress_iif_rule=1`);
- the per-workload netns (`ovd-ns-0000`) and host-veth (`ovd-hv-0000`) exist;
- the python echo server is `LISTEN 0.0.0.0:18941` inside the netns;
- the leg-C `IP_TRANSPARENT` listeners are bound on `127.0.0.1`.

At **t=1** (≤ 250 ms later) every one of those kernel objects is gone — netns, host-veth, both nft rules — yet the alloc row **stays `Running`** and the workload **process survives** (`server_pid=702304` stable across all 56 ticks). The netns **never returns** for the entire 14 s observation window.

This rules out a workload exit (the process never dies) and rules out a row-level terminal (the row stays `Running`, `reason=Started` — never `BackoffExhausted`/`Completed`/`Failed`).

---

## 3. The mechanism, pinned by bpftrace: Overdrive's convergence path runs `ip netns del` on a live workload

A `bpftrace` probe on `execve` of `ip` + `unlinkat` of `/run/netns/ovd*` names the deleter and its timing exactly:

```
T+6016 ms IP comm=tokio-rt-worker pid=702262 : netns add ovd-ns-0000     <-- provision (StartAllocation C3 seam)
T+6020 ms IP comm=tokio-rt-worker pid=702263 : link del ovd-hv-0000      <-- converge rebuild during provision
T+6244 ms IP comm=tokio-rt-worker pid=702361 : netns del ovd-ns-0000     <-- TEARDOWN, 228 ms after provision
T+6246 ms IP comm=tokio-rt-worker pid=702362 : link del ovd-hv-0000      <-- TEARDOWN of host-veth
                                                                          <-- NO second `netns add` — never restored
```

```
UNLINKAT pid=699137 ppid=698974 comm=ip path=/run/netns/ovd-ns-0000
```

The deleter is **`ip netns del ovd-ns-0000` immediately followed by `ip link del ovd-hv-0000`**, spawned by **`comm=tokio-rt-worker`** (`ppid` = the in-process `run_server`). That two-command sequence is exactly `veth_provisioner::teardown_workload_netns(&plan)` (`crates/overdrive-control-plane/src/veth_provisioner.rs:1758` — `ip netns del <netns>` then `ip link del <host_veth>`). It is reached, in the steady state (post-boot), **only** through:

`action_shim::teardown_and_release_netns` (`crates/overdrive-control-plane/src/action_shim/mod.rs:859 → 876`), which has exactly two call sites — the `FinalizeFailed` terminal arm (`:1070`) and the `StopAllocation` arm (`:1549`).

The boot-time orphan-GC path (`veth_provisioner::adopt_on_restart_recovery` → `teardown_workload_netns`, wired at `lib.rs:1974`) is **excluded**: it runs once, BEFORE the convergence loop spawns and BEFORE any deploy, when no workload netns exists. The teardown here fires at `T+6244 ms` — ~6 s into the run, ~230 ms after the alloc was provisioned — squarely in the steady-state convergence loop (`tokio-rt-worker`), not the boot future.

**Conclusion of the mechanism chain:** the WorkloadLifecycle reconciler / action-shim drives the just-Started alloc through a `FinalizeFailed`/`StopAllocation` terminal action **~230 ms after it reached Running**, which calls `teardown_and_release_netns` → `teardown_workload_netns` → destroys the netns + host-veth (and drops the per-alloc nft guards), **while the workload process is still alive and the row never records a terminal state**. The netns is never re-provisioned, so the canonical address `10.99.0.2` is no longer reachable on the host.

---

## 4. Why the timed-out connect follows necessarily (backward chain)

With the netns/host-veth gone, the host loses the connected `/30` route. `ip route get 10.99.0.2` flips from the provisioned state to the host default route:

```
# while provisioned (t=0):
10.99.0.2 dev ovd-hv-0000 src 10.99.0.1 uid 0
# after teardown (during the dial):
10.99.0.2 via 192.168.5.2 dev eth0 src 192.168.5.15 uid 0      <-- falls through to the VM's internet gateway
```

So when the client (sourced `10.99.200.2`, routed into the host) sends its SYN for `10.99.0.2:18941`:
- the host has **no route into a workload netns** for `10.99.0.2` — it would forward the packet out `eth0` to `192.168.5.2`, which black-holes it;
- the **inbound nft-TPROXY rule is gone** (`inbound_daddr_rule=0`), so even reaching PREROUTING there is nothing to divert it to leg-C;
- the **leg-C listener / netns / server are gone**.

⇒ no SYN-ACK ⇒ `connect()` blocks until its 10 s bound ⇒ `TimedOut`. The chain validates forward (teardown ⇒ no route/no rule/no listener ⇒ SYN unanswered ⇒ connect timeout) and is consistent at every observed layer.

---

## 5. Five-Whys, multi-causal

### Branch A — the dominant cause (the failure)

```
WHY 1A: The client's mTLS dial to 10.99.0.2:18941 never connects — connect() times out.
        [Evidence: "connect FAILED after 10.014s kind=TimedOut"; tcpdump shows 5 unanswered SYNs.]
  WHY 2A: The SYN reaches a host with no route into a workload netns for 10.99.0.2 and no
          inbound capture rule.
        [Evidence: `ip route get 10.99.0.2` = "via 192.168.5.2 dev eth0" (default route);
         nft inbound_daddr_rule count = 0 at dial time.]
    WHY 3A: The server workload's per-workload netns (ovd-ns-0000), host-veth (ovd-hv-0000),
            and both nft-TPROXY rules were destructively torn down ~230 ms after the alloc
            reached Running, and never restored.
        [Evidence: POLL t=0 (all present) → t=1 (all gone), ns/hv stay 0 for 14 s;
         bpftrace: `netns del ovd-ns-0000` @ T+6244ms, `link del ovd-hv-0000` @ T+6246ms,
         NO subsequent `netns add`.]
      WHY 4A: Overdrive's own convergence path ran teardown_workload_netns against a LIVE
              workload — a FinalizeFailed/StopAllocation terminal action fired on the alloc
              despite the workload process being alive and the row staying Running.
        [Evidence: deleter is `comm=tokio-rt-worker` (the convergence runtime, ppid=run_server),
         not the boot future; teardown_workload_netns is reached post-boot ONLY via
         action_shim::teardown_and_release_netns (mod.rs:1070 FinalizeFailed / :1549 StopAllocation);
         workload pid 702304 alive throughout; row reason=Started, never a terminal reason.]
        WHY 5A (ROOT CAUSE A): The WorkloadLifecycle convergence loop, when this Service
              workload is driven end-to-end through the REAL in-process run_server (action-shim
              StartAllocation → driver.start → Running, with per-workload netns provisioning
              live), emits a spurious terminal/stop for the freshly-Running alloc, which the
              C3 TEARDOWN SEAM honours by destroying the netns. This is a production lifecycle
              defect that no prior test exercised: the only prior inbound/bidirectional skeletons
              hand-built the netns in the TEST and called MtlsInterceptWorker::start_alloc
              DIRECTLY (bidirectional_walking_skeleton.rs:302-330), bypassing the convergence
              loop and the C3 provision/teardown seams entirely. This keystone is the FIRST to
              drive a Service workload through real run_server convergence + the composed mTLS
              worker + live per-workload netns provisioning, so it is the first to surface the
              defect.
        [Evidence: prior bidirectional skeleton stands up netns in-test and calls start_alloc
         directly; this keystone deploys via the in-process POST /v1/jobs handler and relies on
         the reconciler→action-shim→start_alloc path; the netns is provisioned by production
         (T+6016ms `netns add`) and torn down by production (T+6244ms `netns del`).]
```

### Branch B — a SECONDARY, independent observation (does NOT cause the hang, but is real)

```
WHY 1B: After the teardown, a fresh `StartAllocation` Running row is written with
        workload_addr = None (the canonical address is dropped on re-drive).
        [Evidence: POLL t=1..55 all show Running/addr=None/reason=Started, same alloc id.]
  WHY 2B: The re-driven Running write did not re-inject the canonical workload_addr, and the
          netns was NOT re-provisioned (no second `netns add` in the bpftrace timeline).
        [Evidence: only Running-writers are StartAllocation (mod.rs:1199-1200) and
         RestartAllocation (:1417-1418), both copying spec.workload_addr; a None value means the
         C3 provision seam (provision_and_inject_netns, mod.rs:802-840) did not run/complete on
         the re-drive — consistent with a teardown/re-drive race rather than a clean re-provision.]
    -> CONTRIBUTING FACTOR B: the convergence loop does not recover the workload to a
       re-provisioned, address-bearing Running state after the spurious teardown; it leaves the
       alloc Running-but-addressless with no netns. This compounds Branch A (even a single
       spurious teardown is unrecoverable for the remainder of the test).
```

### Legs explicitly ruled IN/OUT (the four lanes from the mission)

- **Leg 1 (routing) — FAILS (root cause).** At dial time the host has no route into a workload
  netns for `10.99.0.2` (falls through to `eth0`) because the netns/host-veth were torn down.
  When the netns IS present (t=0), routing is correct (`10.99.0.2 dev ovd-hv-0000`).
- **Leg 2 (capture) — works WHEN provisioned, absent at dial time.** The production inbound rule
  `ip daddr 10.99.0.2 tcp dport 18941 tproxy to 127.0.0.1:<legC>` (step 03-01) **is correctly
  installed** at t=0 (`inbound_daddr_rule=1`); it is gone at dial time only because the alloc's
  nft guards dropped during the teardown. **Step 03-01's install is NOT defective.**
- **Leg 3 (listener) — works WHEN provisioned, absent at dial time.** leg-C `IP_TRANSPARENT`
  listeners are bound on `127.0.0.1` at t=0; gone after teardown.
- **Leg 4 (handshake) — never reached.** The TCP connect never completes, so no TLS handshake
  is attempted. (A cert/trust failure would ERROR after connect, not time out in connect — ruled
  out by the `TimedOut` connect shape.)

### Cross-validation

Root Cause A and Contributing Factor B are consistent and non-contradictory: A destroys the
netns; B explains why the alloc never recovers (no re-provision, addr dropped). Together they
explain every observed symptom — the unanswered SYN, the `eth0` route fall-through, the absent
nft rule, the absent leg-C listener, the `addr=None` Running row, and the surviving workload
process. No symptom is left unexplained.

---

## 6. TEST-FIXTURE vs PRODUCTION classification

**PRODUCTION gap.** The failing behaviour is entirely in production code driven through the
production entry points:

- **Where:** the WorkloadLifecycle convergence path / action-shim drives a spurious
  terminal-or-stop action for a freshly-Running Service alloc, and the C3 TEARDOWN SEAM
  (`action_shim::teardown_and_release_netns`, `crates/overdrive-control-plane/src/action_shim/mod.rs:859`,
  called at `:1070` FinalizeFailed and `:1549` StopAllocation) honours it by running
  `veth_provisioner::teardown_workload_netns` (`crates/overdrive-control-plane/src/veth_provisioner.rs:1758`)
  against a live workload. The defect is the **emission of that terminal/stop** for an alloc
  whose process is alive and whose row is Running — i.e. a WorkloadLifecycle reconcile decision
  (`crates/overdrive-core/src/reconcilers/workload_lifecycle.rs`) and/or the exit-observer
  running-gate interaction surfaced by the in-process composition.
- **Why it is not the test fixture:** the client-source netns fixture (`ClientNetns`,
  `CLIENT_ADDR=10.99.200.2`) is correct and irrelevant to the teardown — the teardown fires
  at `T+6244 ms`, BEFORE `ClientNetns::setup()` runs, and the netns the test cares about
  (`ovd-ns-0000`) is **provisioned and destroyed entirely by production**. The fixture's only
  job is to source the SYN from outside the `/30`; it does that correctly (the SYN egresses
  `ovd-ks-cli-hv` and reaches the host, per tcpdump).
- **Why it is not step 03-01's inbound install:** the inbound `ip daddr 10.99.0.2 tcp dport 18941`
  rule **is present and correct** at t=0 (`inbound_daddr_rule=1`). 03-01 did its job; the rule
  is collateral damage of the netns teardown, not a cause.

The precise reconciler arm (FinalizeFailed vs StopAllocation, and the predicate that fires it
~230 ms after Running) is the one remaining source-level question. It could not be pinned from
tracing because the action-shim / reconciler-runtime paths emit no `info`/`debug`/`trace` logs
in this composition (the only steady-state log in the whole run is the boot-time
"adopt-on-restart §5 swept 0 rules"); it is left for the fix owner, who now has the exact
call-site map above. The leading hypothesis is a **natural-exit / early-exit misclassification
or a running-gate/exit-observer interaction** that terminalises a long-lived Service whose
process is still alive.

---

## 7. Recommended fixes (mapped to root causes)

### Root Cause A — convergence emits a spurious terminal/stop for a freshly-Running alloc

- **Immediate mitigation (restore the test's signal, NOT a production fix):** none that belongs
  in the keystone — the keystone is correctly RED and must stay RED until production stops
  tearing the netns down (its litmus depends on the production path). Do not paper over it with
  a longer connect timeout or a test-installed rule/netns (that would violate the vertical-slice
  rule and hide the production defect).
- **Permanent fix (production):** identify the WorkloadLifecycle/action-shim decision that emits
  `FinalizeFailed`/`StopAllocation` for an alloc that just reached Running with a live process,
  and stop it firing. Candidate areas, in priority order:
  1. WorkloadLifecycle natural-exit / early-exit classification
     (`crates/overdrive-core/src/reconcilers/workload_lifecycle.rs`,
     `classify_natural_exit_terminal` and the Stable/StartupProbe/EarlyExit gates) — a Service
     whose process is alive and whose row is Running must not be terminalised.
  2. The exit-observer running-gate interaction
     (`crates/overdrive-control-plane/src/worker/exit_observer.rs`) — confirm the Running-gate is
     not releasing/observing a spurious exit for a live `ExecDriver` child driven through the real
     in-process boot.
  3. The C3 TEARDOWN SEAM's precondition — `teardown_and_release_netns` should arguably refuse to
     destroy a netns whose workload still has live PIDs in its cgroup scope (defence-in-depth:
     the veth_provisioner rustdoc at `veth_provisioner.rs:1899/1930/1976-1979` already names
     "must NOT let a Running workload look like an orphan and drive a destructive `ip netns del`"
     as a known hazard for the *boot* GC; the same invariant should guard the terminal-arm
     teardown).
- **Early detection:** a DST/integration invariant — *a Running alloc with live cgroup PIDs never
  has its netns torn down* (`assert_always!`), and/or a Tier-3 assertion that the per-workload
  netns/host-veth/inbound-rule survive for the alloc's Running lifetime.

### Contributing Factor B — re-driven Running row drops the canonical address and does not re-provision

- **Permanent fix (production):** ensure that any re-drive that writes a Running row re-runs the
  C3 provision seam (re-provisioning the netns and re-injecting `workload_addr`), so a Running row
  can never carry `workload_addr = None` for a Path-A mesh alloc. (Likely subsumed by fixing A —
  if no spurious teardown fires, no addressless re-drive occurs — but worth a guard either way.)

### Scope decision — does this fit step 03-02?

**No — it exceeds 03-02's scope and warrants its own slice/issue.** Step 03-02's job is the S-WS
keystone (assert the canonical-address mTLS round-trip through the production-installed inbound
rule). The defect is a **pre-existing WorkloadLifecycle/exit-observer convergence bug** in how a
Service alloc is driven through the real in-process `run_server` — orthogonal to the inbound
TPROXY install 03-01/03-02 deliver. The keystone is the *messenger* (the first test to exercise
this path), not the *owner* of the fix. Recommendation for the orchestrator/user:

1. Keep the 03-02 keystone RED (it is a correct litmus; it must not be made green by masking).
2. Surface the convergence/teardown defect to the user and (on approval) open a tracking issue
   for the WorkloadLifecycle/exit-observer "live Running alloc terminalised → netns torn down"
   bug, with this RCA's call-site map. Do NOT create the issue without approval (CLAUDE.md
   deferral rule).
3. 03-02 cannot pass on the merge gate (pinned-6.18 Tier-3) until that fix lands; the keystone's
   own merge-blocking AC depends on it.

---

## 8. Evidence appendix — provenance

- All runs: `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane
  --features integration-tests -E 'test(workload_reached_at_canonical_address_terminates_mtls_end_to_end)'`,
  each host-`timeout`-wrapped and inner-`timeout`-wrapped, stdout/stderr captured to a guest file,
  VM scrubbed (cgroups/netns/veths/nft/bpffs) before every run.
- Kernel: `uname -r = 7.0.0-22-generic`.
- Test instrumentation added for this RCA (uncommitted, to be reverted by the crafter who
  finalizes the keystone): a bounded `TcpStream::connect_timeout(10s)` in `TestPkiHandle::dial`;
  a `diag_capture()` kernel-state dump + a `[POLL]` alloc-row/kernel-state timeline written to
  `/tmp/ks-diag.log`; a `tracing_subscriber` init; a concurrent `tcpdump` during the dial. No
  production source was modified. bpftrace probes (`/tmp/trace.bt`, `/tmp/trace2.bt`) were run
  ad-hoc in the VM, not committed.

---

## 9. Targeted predicate pin (fix-owner handoff)

This section closes the one source-level question § 6 left open — **which terminal
arm fires, what predicate emits it, and the precise fix target** — by static
code-reading alone (no new Lima run; no instrumentation added or reverted; VM left
clean). Each link in the chain is named with file:line and falsified against the §1–§5
evidence.

### 9.1 The firing arm: `FinalizeFailed` (`action_shim/mod.rs:1070`) — NOT `StopAllocation`

The deleter is reached through the **`FinalizeFailed` terminal arm**, not the
`StopAllocation` arm. Three facts pin it, and each independently matches the §2/§3
evidence that `StopAllocation` would contradict:

1. **The process survives** (`server_pid` stable across all 56 ticks, §2). The
   `FinalizeFailed` arm (`mod.rs:978`) does **NOT** call `driver.stop` — it writes the
   row, then calls only `driver.on_alloc_terminal` (probe-supervisor cleanup, no PID
   kill), `worker.stop_alloc` (mTLS-intercept teardown), and
   `teardown_and_release_netns` (`:1070`). The `StopAllocation` arm (`:1481`) DOES call
   `driver.stop(&handle)` (`:1498`) — had it fired, the python echo PID would have been
   reaped. It was not → `StopAllocation` is excluded.
2. **The row stays `Running` / `reason=Started`, never a terminal reason** (§2). The
   `FinalizeFailed` arm's GAP-9 guard (`mod.rs:1024`) sets `finalized_state =
   prior_row.state` (i.e. keeps `Running`) **specifically when `terminal` is
   `Some(TerminalCondition::Stable { .. })`** — every other terminal lands `Failed`. The
   `StopAllocation` arm unconditionally writes `AllocState::Terminated` (`:1519`). The
   observed row is `Running`, not `Terminated` → again `FinalizeFailed { Stable }`, not
   `StopAllocation`.
3. **The teardown still runs despite the row staying `Running`** (§2/§3 — netns/veth/nft
   gone at t=1). The `teardown_and_release_netns` call at `:1070` is sequenced **after**
   the `finalized_state` guard and is **NOT itself gated by the terminal kind** — it runs
   for *every* `FinalizeFailed`, including the `Stable` success-claim. This is the defect
   (§9.3).

This is fully consistent with the bpftrace deleter being `comm=tokio-rt-worker` (the
convergence runtime) and with the row never recording a terminal reason — a `Stable`
FinalizeFailed is a *success* claim that (correctly) leaves the row `Running`, so no
terminal reason is ever stamped, yet the netns is (incorrectly) destroyed.

### 9.2 The exact predicate: `service-lifecycle` reconciler, empty-startup-probes opt-out (branch a')

The `Action::FinalizeFailed { terminal: Some(Stable { .. }) }` is emitted by the
**`ServiceLifecycleReconciler`** (`crates/overdrive-core/src/service_lifecycle.rs`), in
its `reconcile` body — **branch (a'), the empty-startup-probes opt-out**
(`service_lifecycle.rs:540-558`):

```rust
// service_lifecycle.rs:540
if fact.startup_probes_empty && fact.state == AllocState::Running {
    let started = fact.started_at.unwrap_or_else(|| unreachable!(...));
    let settled_in_ms = settled_in_ms_from(tick.now_unix, started);
    let witness = ProbeWitness { probe_idx: 0, role: "startup".into(),
                                 mechanic_summary: "none (opted out)".into(), inferred: false };
    actions.push(Action::FinalizeFailed {
        alloc_id: alloc_id.clone(),
        terminal: Some(TerminalCondition::Stable { settled_in_ms, witness }),   // <-- success claim
    });
    next_view.stable_announced.insert(alloc_id.clone());
    continue;
}
```

**Tripping input state** (all three conditions hold for the keystone's server alloc on
the tick after it reaches Running):

- `fact.startup_probes_empty == true` — the keystone's `server_service_spec` declares
  `startup_probes: vec![]` (`canonical_address_inbound_walking_skeleton.rs:651`).
  Hydration maps this to `startup_probes_empty = true` via
  `spec_facts_for_service` (`reconciler_runtime.rs:1911-1918` —
  `let startup_probes_empty = svc.startup_probes.is_empty()`), stamped onto the per-alloc
  `ServiceAllocFact` at `reconciler_runtime.rs:2934`.
- `fact.state == AllocState::Running` — the alloc reached Running (§2 t=0). The fact's
  `state` is `row.state` verbatim (`reconciler_runtime.rs:2926`).
- `next_view.stable_announced` does NOT yet contain the alloc (first tick after Running),
  so the dedup short-circuit at `service_lifecycle.rs:484` does not fire.

This branch is **correct by its own spec** — ADR-0058 §4 / ADR-0059 Q5 first-Running-IS-
Stable opt-out, and the existing unit test
`empty_probes_opt_out_fires_stable_when_running`
(`crates/overdrive-core/tests/acceptance/service_lifecycle_reconcile_branches.rs:310`)
pins exactly this emission. **The reconciler is NOT the bug site.** The reconciler
emits the right action; the action-shim mishandles it (§9.3).

**Why ~230 ms after Running (one tick, not a timeout):** branch (a') has no deadline gate
— it fires on the *first* reconcile tick where the alloc is observed `Running` with
empty probes. The `service-lifecycle` reconciler gets that first tick because the
`StartAllocation` that drove the alloc Running also dual-emits
`Action::EnqueueEvaluation { reconciler: "service-lifecycle", .. }`
(`workload_lifecycle.rs:229-246`, gated on `WorkloadKind::Service` + a starting action).
So the teardown lands one convergence cadence (~one `tick_period_ms`) after Running —
matching the §3 bpftrace `netns add` @ T+6016 ms → `netns del` @ T+6244 ms (Δ ≈ 228 ms),
and ruling out a probe/backoff/deadline expiry (debugging.md §1 — the next-tick
classification, not a timer).

**WorkloadLifecycle is excluded as the emitter** for this alloc: its `reconcile_inner`
Run-branch short-circuits to `(Vec::new(), view.clone())` the moment any active alloc is
`Running` (`workload_lifecycle.rs:485-487`), so it emits no terminal for a freshly-Running
Service. Its `service_vip_release_emission` wrapper (`:891-910`) emits only
`ReleaseServiceVip` (a VIP-release, never a netns teardown) and only after a row already
carries `terminal.is_some()` — it is a *consequence* of the FinalizeFailed write, not the
teardown trigger. **The exit-observer is excluded** as the *emitter*: it writes
observation rows only and emits no `FinalizeFailed` / `StopAllocation` action itself
(`worker/exit_observer.rs` — the terminal *Action* is always a reconciler's).

### 9.3 The precise fix target + condition to change

**Fix site: `crates/overdrive-control-plane/src/action_shim/mod.rs:1070`** — the
`teardown_and_release_netns(&row.alloc_id, net_slot_allocator, mtls_worker)?` call in the
`Action::FinalizeFailed` arm.

**The condition that must change:** the teardown must be **gated on the SAME `Stable`
discriminator the `finalized_state` guard already uses at `:1024`**. A `FinalizeFailed {
Stable }` is a *success* claim that keeps the row `Running` — so it MUST NOT tear down
the alloc's netns/veth/nft. The teardown belongs only on the genuinely-terminal
`FinalizeFailed` variants (`ServiceFailed` / `BackoffExhausted` / `Completed` / `Failed`),
exactly the set that lands `finalized_state = AllocState::Failed`.

Concretely — guard the teardown by reusing the existing
`matches!(terminal, Some(TerminalCondition::Stable { .. }))` test (or, equivalently,
`finalized_state != prior_row.state` / `finalized_state == AllocState::Failed`):

```rust
// mod.rs:1024 already computes this for the row state:
let finalized_state = if matches!(terminal, Some(TerminalCondition::Stable { .. })) {
    prior_row.state            // Stable: keep Running
} else {
    AllocState::Failed
};
...
// mod.rs:1070 — gate the teardown the SAME way:
if finalized_state == AllocState::Failed {
    teardown_and_release_netns(&row.alloc_id, net_slot_allocator, mtls_worker)?;
}
```

(Equivalently: hoist a `let is_stable = matches!(terminal, Some(TerminalCondition::Stable
{ .. }));` and gate `if !is_stable { teardown_and_release_netns(..)?; }`.) The
`worker.stop_alloc` mTLS-intercept teardown at `:1063-1065` SHOULD receive the **same
gate** — a Stable alloc is still serving on leg-C, so detaching its intercept is the
same class of bug; the §1 connect-timeout is dominated by the netns teardown, but the
intercept detach would independently break the round-trip and must be gated together.
**Do NOT** widen the change to `driver.on_alloc_terminal` (`:1058`) — a Stable alloc has
indeed passed startup, and the probe-supervisor cleanup hook is benign-or-correct there;
gate only the two *destructive infrastructure* teardowns (netns + mTLS intercept).

**Defence-in-depth (secondary, optional, separate concern):** the C3 teardown seam
itself (`teardown_and_release_netns`, `mod.rs:859`) could refuse to destroy a netns whose
workload still has live PIDs in its cgroup scope — the `veth_provisioner` rustdoc already
names "must NOT let a Running workload look like an orphan and drive a destructive `ip
netns del`" as a known hazard for the *boot* GC. That is a belt-and-suspenders guard, not
the primary fix; the primary fix is the `Stable`-gate at `:1070`.

### 9.4 Recommended regression-test shape — and the Tier-1 reproducibility verdict

**Does the bug reproduce as a pure Tier-1 (reconciler-purity) test? NO.** The
`service-lifecycle` reconciler emits exactly the right `Action::FinalizeFailed { Stable }`
— that emission is *correct* and is already pinned GREEN by
`empty_probes_opt_out_fires_stable_when_running`
(`service_lifecycle_reconcile_branches.rs:310`). The defect is **downstream of the pure
reconcile function**, in the `async` action-shim `dispatch_single` `FinalizeFailed` arm,
which performs netns I/O. A `(desired, actual, view, tick) → (Vec<Action>, View)` purity
test cannot observe it — there is no wrong *action* to assert on.

**Recommended test: an action-shim dispatch acceptance test** in the existing home for
this exact seam — `crates/overdrive-control-plane/tests/integration/alloc_netns_lifecycle.rs`
(which already drives the production driving port `action_shim::dispatch` with a real
`NetSlotAllocator` + real `MtlsInterceptWorker`, and already asserts teardown-then-release
on `StopAllocation` as its sub-claim 3). Add a sub-claim:

> **Sub-claim 5 — a `FinalizeFailed { Stable }` does NOT tear down the alloc's netns.**
> Provision an alloc via the `StartAllocation` arm (slot 0 → `ovd-ns-0000`, held in the
> `NetSlotAllocator`), assert it is provisioned, then dispatch
> `Action::FinalizeFailed { alloc_id, terminal: Some(TerminalCondition::Stable {
> settled_in_ms: 0, witness: <opt-out witness> }) }` through the SAME `dispatch` with the
> real worker + allocator. Assert **the slot is STILL HELD**
> (`net_slot_allocator.snapshot().contains_key(&alloc_id)`) AND the netns survives
> (`ip netns list` still shows `ovd-ns-0000`) AND the alloc row is still `Running`.

The slot-snapshot assertion is the **in-memory observable proxy** that makes most of the
test verdict cheap and host-independent: `teardown_and_release_netns` does teardown-THEN-
`net_slot_allocator.release` (`mod.rs:876-877`), so today (bug) the slot is **released**
and `ip netns del` runs → the snapshot is empty → RED; with the `:1070` `Stable`-gate the
teardown is skipped, the slot stays held, and the netns survives → GREEN. The `ip netns
list` half needs root/CAP_NET_ADMIN (SKIP on an unprivileged runner, like the existing
sub-claims 1–3); the **slot-snapshot half runs on every host** (the `release` is a pure
in-RAM `BTreeMap::remove`, no kernel I/O), so the core RED→GREEN signal is not gated on
privilege.

This sub-claim fails RED on the current code (the slot is released / netns destroyed for a
Stable FinalizeFailed) and passes GREEN once the `:1070` teardown is `Stable`-gated.

### 9.5 Investigation discipline for this targeted pass

- **Method:** static code-reading + reasoning only — no Lima/nextest run was needed (the
  arm and predicate disambiguated from source), so the hard-timeout discipline was held
  vacuously (no test command issued). **No production-source instrumentation was added,
  therefore none to revert.** The VM was not touched and remains clean.
- **Falsification record** (debugging.md §4/§10): the hypothesis "`FinalizeFailed { Stable }`
  from branch (a'), torn down unconditionally at `:1070`" survives all three falsifiers —
  (a) the keystone *does* declare `startup_probes: vec![]` (`:651`, not non-empty); (b) the
  `FinalizeFailed` arm does *not* call `driver.stop` (`mod.rs:978-1071`, matching the
  process-survives evidence); (c) `WorkloadLifecycle` does *not* emit any terminal for a
  Running alloc (short-circuit `workload_lifecycle.rs:485`). Had any falsifier held, the
  arm/predicate would differ; none does.
