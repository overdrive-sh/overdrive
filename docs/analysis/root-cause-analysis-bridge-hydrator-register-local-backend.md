# Root-Cause Analysis — bridge writes `ServiceBackendRow` but hydrator never dispatches `RegisterLocalBackend`

- **Analyst:** Rex (Toyota 5-Whys RCA)
- **Date:** 2026-06-03
- **Branch / HEAD:** `marcus-sa/udp-support` @ `12611316`
- **Scope:** read-only investigation. No production code or tests modified.
- **Methodology:** Toyota 5-Whys, multi-causal, evidence at each level
  (`.claude/rules/debugging.md` §2/§4/§5/§6/§10/§11 applied).

---

## 1. Problem statement (scoped)

Two Tier-3 integration tests in
`crates/overdrive-control-plane/tests/integration/backend_discovery_bridge/walking_skeleton.rs`
fail, and the failure is **pre-existing on baseline `9fe1b3b4`**
(not caused by the SERVICE_MAP proto work, not leftover cgroup/XDP state):

- `submit_service_workload_tcp_round_trip_through_vip_succeeds` (S-BDB-01) — `walking_skeleton.rs:299`
- `bridge_to_hydrator_handoff_dispatches_register_local_backend` (S-BDB-19) — `walking_skeleton.rs:485`

Both panic with:

```
LOCAL_BACKEND_MAP did not receive an entry mapping <vip>:<port> -> <host_ipv4>:<port> within 5s —
bridge wrote ServiceBackendRow but hydrator did not dispatch RegisterLocalBackend (ADR-0053 classifier regression)
```

Reproduced on current HEAD (S-BDB-19):

```
panicked at .../walking_skeleton.rs:485:5:
S-BDB-19: LOCAL_BACKEND_MAP did not receive an entry mapping 10.96.0.2:8081 → 10.244.19.1:8081 within 5s
test result: FAILED. 0 passed; 1 failed; ... finished in 5.43s
```

The 5.43 s total runtime is itself evidence (debugging §11, §7): the alloc
**reached `Running` quickly** (it passed the `running.is_some()` assert at
`walking_skeleton.rs:471`, which has a 10 s budget) — the only thing that timed
out is the 5 s `LOCAL_BACKEND_MAP` poll. The producer ran; only the
`RegisterLocalBackend` dispatch is missing.

> **The panic string is the test author's guess, not the mechanism**
> (debugging §2). It blames an "ADR-0053 classifier regression." The
> confirmed mechanism is different and more specific (see Root Cause).

---

## 2. Evidence base — the surface each component actually reads/writes

### A. The test injects a **real** `EbpfDataplane` via `dataplane_override`

`test_server.rs:107-141` — `serve_with_dataplane` constructs a real
`EbpfDataplane::new_with_pin_dir(...)` on the per-test veth pair and passes it
as `dataplane_override: Some(dataplane.clone() as Arc<dyn Dataplane>)`:

```rust
// test_server.rs:107
let ebpf = EbpfDataplane::new_with_pin_dir(...).expect("EbpfDataplane::new_with_pin_dir");
...
// test_server.rs:139
dataplane_override: Some(
    dataplane.clone() as Arc<dyn overdrive_core::traits::dataplane::Dataplane>
),
```

The veth client iface carries a **real IPv4**: `10.244.19.1/24` (S-BDB-19,
`walking_skeleton.rs:447`) / `10.244.1.1/24` (S-BDB-01, `walking_skeleton.rs:211`).
The test asserts the map carries `backend_ip_host == u32::from(10.244.19.1)`
(`walking_skeleton.rs:481`).

### B. Commit `9fe1b3b4` keys `host_ipv4` resolution on `dataplane_override.is_some()`

`crates/overdrive-control-plane/src/lib.rs:1197-1200` (introduced by `9fe1b3b4`):

```rust
let host_ipv4 = if config.dataplane_override.is_some() {
    std::net::Ipv4Addr::LOCALHOST          // 127.0.0.1
} else {
    resolve_host_ipv4_from_dataplane_config(config.dataplane.as_ref())?
};
```

Because the two failing tests set `dataplane_override: Some(...)`, this branch
resolves `host_ipv4 = 127.0.0.1` — **even though the override carries a real
`EbpfDataplane` on a real veth with a real IP.** The commit's own message and
inline comment assume the override path means "SimDataplane / DST / CLI suite,
no veth provisioned, LOCALHOST is correct." That assumption is false for these
two tests, which are the only override-based tests that wire a *real* dataplane.

### C. Both bridge and hydrator are constructed with the same `host_ipv4`

`lib.rs:1217` and `lib.rs:1229`:

```rust
runtime.register(backend_discovery_bridge(host_ipv4, node_id.clone())).await?;
...
runtime.register(service_map_hydrator(host_ipv4)).await?;
```

### D. The bridge writes the backend address as `(host_ipv4, listener.port)`

`crates/overdrive-core/src/reconcilers/backend_discovery_bridge.rs:348`:

```rust
addr: SocketAddr::new(IpAddr::V4(self.host_ipv4), listener.port.get()),
```

So under `9fe1b3b4` the bridge writes a `ServiceBackendRow` whose backend addr
is `(127.0.0.1, 8081)`.

### E. The hydrator partitions local-vs-remote against the SAME `host_ipv4`

`service_map_hydrator.rs:304, 328-332`:

```rust
let host_ipv4 = self.host_ipv4;          // 127.0.0.1
...
let (local, remote): (Vec<&Backend>, Vec<&Backend>) =
    desired_svc.backends.iter().partition(|b| match b.addr.ip() {
        std::net::IpAddr::V4(v4) => v4 == host_ipv4,   // 127.0.0.1 == 127.0.0.1 → local
        std::net::IpAddr::V6(_) => false,
    });
```

The backend `(127.0.0.1, 8081)` is classified **local** (matches `host_ipv4`).

### F. The local-backend emitter **rejects loopback** before emitting the action

`service_map_hydrator.rs:397-431` — `push_register_local_backend_actions`
gates every local backend through `classify_backend_address`:

```rust
if let Err(reason) = classify_backend_address(*backend_v4.ip()) {
    tracing::warn!(
        name: "service_map_hydrator.register_local_backend.rejected",
        ... reason = %reason,
        "skipping RegisterLocalBackend: backend address rejected by classifier"
    );
    continue;                              // ← no Action::RegisterLocalBackend pushed
}
actions.push(Action::RegisterLocalBackend { ... });
```

And `classify_backend_address` (`service_map_hydrator.rs:211-230`) rejects
loopback first:

```rust
pub const fn classify_backend_address(addr: Ipv4Addr) -> Result<(), BackendAddressRejection> {
    if addr.is_loopback() {
        return Err(BackendAddressRejection::Loopback);   // 127.0.0.1 → rejected
    }
    ...
}
```

So the backend `(127.0.0.1, 8081)` is classified local, then immediately
dropped as loopback. **No `RegisterLocalBackend` action is emitted →
`LOCAL_BACKEND_MAP` is never populated.**

### G. `should_dispatch` is not the blocker (alternative branch ruled out)

`service_map_hydrator.rs:440-441` — `should_dispatch` returns `true` for
`None | Some(Pending)`, i.e. on first sight of a new service. The hydrator
therefore *does* enter the dispatch branch and *does* reach the partition +
emitter. The failure is specifically at the loopback guard, not at the
dispatch gate.

---

## 3. Five-Whys chain (multi-causal)

```
PROBLEM: LOCAL_BACKEND_MAP never receives the (vip,port)→(host_ipv4,port) entry;
         the test times out after 5 s.

── BRANCH A (root) — host_ipv4 collapses to loopback on the override path ──────

WHY 1A: LOCAL_BACKEND_MAP is empty for (10.96.0.2, 8081).
  [Evidence: walking_skeleton.rs:479-485 poll returns None after 5 s; repro 5.43 s run.]

WHY 2A: The hydrator emitted no Action::RegisterLocalBackend for the service.
  [Evidence: alloc reached Running fast (passed running.is_some() @ :471, 10 s budget);
   only the 5 s map poll timed out — producer ran, dispatch missing (debugging §11).]

WHY 3A: The hydrator's local-backend emitter rejected the backend address as
        loopback and `continue`d without pushing the action.
  [Evidence: service_map_hydrator.rs:411-422 → classify_backend_address(127.0.0.1)
   = Err(Loopback) @ :214-216; the action push @ :423 is skipped.]

WHY 4A: The backend address the bridge wrote was (127.0.0.1, 8081), because the
        bridge builds it from `self.host_ipv4`, which was 127.0.0.1.
  [Evidence: backend_discovery_bridge.rs:348 addr = SocketAddr::new(host_ipv4, port);
   hydrator partitions it local because host_ipv4 also = 127.0.0.1
   (service_map_hydrator.rs:330).]

WHY 5A: `host_ipv4` resolved to Ipv4Addr::LOCALHOST because the boot path keys
        host_ipv4 resolution on `dataplane_override.is_some()`, and these two
        tests set an override — even though it carries a REAL EbpfDataplane on a
        REAL veth (10.244.x.1).
  [Evidence: lib.rs:1197 `if config.dataplane_override.is_some() { LOCALHOST }`,
   introduced by commit 9fe1b3b4; test_server.rs:139 sets the override with a
   real EbpfDataplane.]

  ROOT CAUSE A: Commit 9fe1b3b4 used `dataplane_override.is_some()` as a proxy
  for "Sim dataplane / no veth provisioned / LOCALHOST is correct." That proxy
  is wrong: it conflates "the test injects a dataplane" with "the test uses a
  SimDataplane on the loopback path." The S-BDB-01/19 walking-skeletons are the
  counterexample — real EbpfDataplane + real veth IP, injected via the same
  override knob. Collapsing their host_ipv4 to loopback makes the bridge write a
  loopback backend, which the hydrator's own (correct, intended) loopback guard
  then rejects.

── BRANCH B (compounding, would fail even without the loopback guard) ──────────

WHY 1B: Even if the loopback guard did not exist, the test's assertion filter
        would still reject the entry.
  [Evidence: walking_skeleton.rs:481 filters on
   e.backend_ip_host == u32::from(host_ipv4) where the test's local host_ipv4 =
   10.244.19.1; a map entry holding 127.0.0.1 would not match.]

WHY 2B: The test's expected host (10.244.19.1) and the production-resolved
        host (127.0.0.1) diverge.
  [Evidence: walking_skeleton.rs:446 host_ipv4 = 10.244.19.1 vs lib.rs:1197
   override→LOCALHOST.]

  → Same Root Cause A. Branch B is the same divergence observed at the
    assertion surface rather than at the emitter. It confirms Root Cause A is
    sufficient and that there is no *second independent* production bug: fix the
    host_ipv4 divergence and both the emitter (Branch A) and the filter
    (Branch B) are satisfied at once.

── BRANCH C (ruled out) — "ADR-0053 classifier regression" (the panic's guess) ─

WHY 1C: Did the local-vs-remote locality classifier itself regress?
  [Falsification probe: read the partition (service_map_hydrator.rs:328-332) and
   the unit tests push_register_local_backend_emits_action_for_valid_local_backend
   (:487, backend 192.168.1.50 → action emitted) and
   push_register_local_backend_skips_ipv6_and_guard_rejected (:593).]
  RULED OUT: the classifier is correct. With a non-loopback host_ipv4 it emits
  the action (unit test :487 passes). Loopback rejection is INTENDED design
  (:593, and ADR-0053 docstring @ :386-387). The regression is in the *input*
  (host_ipv4 = loopback), not in the classifier logic.

── BRANCH D (ruled out) — bridge never ran / never wrote the row ───────────────

WHY 1D: Did the bridge fail to write a ServiceBackendRow at all?
  [Falsification probe: alloc-Running assertion @ :471 passes; the bridge fires
   WriteServiceBackendRow on the Running transition (backend_discovery_bridge.rs:368).]
  RULED OUT: alloc reached Running (test ran 5.43 s, well under the 10 s
  Running budget). The dual-emit handoff (WriteServiceBackendRow +
  EnqueueEvaluation) is exercised; the hydrator was triggered. The empty map is
  a DOWNSTREAM symptom of the loopback rejection, not a missing producer
  (debugging §11).

── BRANCH E (ruled out) — should_dispatch gate suppressed the tick ─────────────

WHY 1E: Did should_dispatch return false, skipping the whole dispatch branch?
  [Falsification probe: should_dispatch @ :440-441 returns true for None|Pending
   (first sight of a new service).]
  RULED OUT: a freshly-submitted service has no prior hydration status, so
  should_dispatch = true; the hydrator reaches the partition + emitter every
  tick until completed.
```

### Cross-validation

- **A + B consistent:** both stem from the single `host_ipv4 = LOCALHOST`
  divergence — one observed at the emitter (loopback reject), one at the test
  filter (IP mismatch). No contradiction.
- **All symptoms explained:** the empty `LOCAL_BACKEND_MAP` (A), the specific
  panic text (B's IP mismatch wording matches), and the fast-Running /
  slow-map-poll timing (D ruled out) are all accounted for by Root Cause A.
- **No second production bug:** Branches C/D/E are falsified. The locality
  classifier, the bridge producer, and the dispatch gate are all correct.

### Backwards chain validation

If `host_ipv4` resolves to `127.0.0.1` on the override-with-real-dataplane path
→ the bridge writes backend `(127.0.0.1, port)` → the hydrator classifies it
local → `classify_backend_address(127.0.0.1)` rejects loopback → no
`RegisterLocalBackend` → `LOCAL_BACKEND_MAP` stays empty → the 5 s poll times
out → panic at `:485` / `:299`. Forward trace reproduces the exact observed
symptom. ✔

---

## 4. Why this is pre-existing on `9fe1b3b4` (timeline, debugging §6)

- `9fe1b3b4` is the regression-introducing commit. Its parent `8875021a`
  resolved `host_ipv4` **unconditionally** from the configured `client_iface`
  via `getifaddrs(3)` — which, for these tests, reads the real veth IP
  (`10.244.x.1`, non-loopback) and would satisfy both the emitter and the
  filter.
- `9fe1b3b4` was a legitimate fix for a *different* failure: the 8 overdrive-cli
  integration tests (and DST) boot with a Sim override and a `client_iface`
  (`ovd-veth-cli`) that is never provisioned on the override path, so the
  unconditional `getifaddrs` resolution refused to boot
  (`DataplaneBootError::IfaceAddrResolution`). The author correctly routed
  those Sim boots to `LOCALHOST`.
- The fix's blind spot: it used `dataplane_override.is_some()` as the branch
  key. That captures the Sim suite **and** the two real-dataplane
  walking-skeletons, because both inject via the same override knob. The
  walking-skeletons provision their own veth with a real IP and *do* want the
  real-resolution path — but they cannot be distinguished by
  `dataplane_override.is_some()` alone.

> **Net:** `9fe1b3b4` traded the cli/DST boot failure for a walking-skeleton
> assertion failure. The two test classes have opposite needs from the same
> override knob, and the boolean branch cannot serve both.

---

## 5. The discarded +42-line test change (verdict on legitimacy)

The dispatch noted an unrecoverable, discarded +42-line change to
`walking_skeleton.rs`. Assessment, on the evidence:

- The tests already provision a real veth with a real IP and already declare
  `let host_ipv4 = Ipv4Addr::new(10,244,19,1)` etc. They are written for the
  **real-resolution** path. They do not need test-side host_ipv4 seeding to be
  *correct*; they need the production boot path to resolve the veth IP for them.
- A +42 test change *could* have masked the bug by switching the tests to a
  loopback expectation (filtering on `127.0.0.1`) or by stopping using the
  override — but that would be papering over a real production-config defect
  (debugging §11: don't "fix" a downstream symptom in the consumer/test).
- **Verdict:** the discarded change was most likely **not** legitimate
  now-required setup. The defect is in production (`lib.rs:1197`), not in the
  test. The right fix restores the real-resolution path for real-dataplane
  boots; the tests as they stand (asserting on the real veth IP) are the
  correct specification of the intended behaviour. If the +42 change had
  re-pointed the assertion at `127.0.0.1`, it would have been masking, and
  losing it is harmless.

---

## 6. Confirmed root cause (single, with one compounding effect)

**ROOT CAUSE A (production):** `crates/overdrive-control-plane/src/lib.rs:1197`
resolves `host_ipv4` to `Ipv4Addr::LOCALHOST` whenever
`config.dataplane_override.is_some()`. This branch key is too coarse — it
treats "an override is present" as "this is a Sim/loopback boot," but the
S-BDB-01/19 walking-skeletons inject a **real `EbpfDataplane` on a real veth**
through the same override field. The collapsed `host_ipv4 = 127.0.0.1` makes the
bridge write a loopback backend (`backend_discovery_bridge.rs:348`), which the
hydrator's intended loopback guard (`service_map_hydrator.rs:214`,
`:411`) then rejects — so no `Action::RegisterLocalBackend` is emitted and
`LOCAL_BACKEND_MAP` is never populated.

The call site that *fails to emit* the action: `service_map_hydrator.rs:421`
(`continue`), reached because `classify_backend_address(127.0.0.1)` returns
`Err(Loopback)` at `:214`. But the *cause* of the bad input is `lib.rs:1197`,
not the hydrator.

---

## 7. Proposed fix (scoped to the root cause)

The fix belongs in **production code** (`lib.rs`), not in the tests.

The branch must distinguish "Sim dataplane on a non-provisioned iface
(→ LOCALHOST)" from "real dataplane on a provisioned veth (→ resolve the
iface IP)." `dataplane_override.is_some()` cannot make that distinction; the
underlying signal is **whether the `client_iface` is actually resolvable**.

### Permanent fix — preferred (P1)

Resolve `host_ipv4` from the configured `client_iface` for *both* branches, and
fall back to `LOCALHOST` **only when iface resolution fails** (which is exactly
the Sim/cli/DST condition `9fe1b3b4` was fixing). This serves both test classes
from one rule and removes the brittle override-as-proxy:

```rust
// lib.rs (~1197), replacing the is_some() branch:
let host_ipv4 = match resolve_host_ipv4_from_dataplane_config(config.dataplane.as_ref()) {
    Ok(ip) => ip,
    // Sim/DST/cli boots inject an override and never provision client_iface;
    // its absence is the loopback signal. Real-dataplane boots (production +
    // the S-BDB walking-skeletons) provision the veth and resolve a real IP.
    Err(_) if config.dataplane_override.is_some() => std::net::Ipv4Addr::LOCALHOST,
    Err(source) => return Err(source.into()),
};
```

- Production (no override, veth provisioned): resolves the real IP, still
  refuses on absence — unchanged behaviour.
- cli/DST (override + `ovd-veth-cli` never created): resolution fails →
  `LOCALHOST` — preserves the `9fe1b3b4` fix for the 8 cli tests.
- S-BDB-01/19 (override + real veth with `10.244.x.1`): resolution **succeeds**
  → real IP → bridge writes a non-loopback backend → classifier emits
  `RegisterLocalBackend` → `LOCAL_BACKEND_MAP` populated → tests pass.

> Verify this still passes the 8 cli boots `9fe1b3b4` restored
> (`coinflip_honesty`, `cluster_and_node_commands`, `endpoint_from_config`,
> `deploy_udp_walking_skeleton`, `exec_spec_walking_skeleton`, `http_client`) —
> they should still hit the `Err(_) → LOCALHOST` arm since `ovd-veth-cli` is
> unprovisioned there.

### Alternative (only if "resolve-then-fallback" is undesirable)

Add an explicit boot flag (e.g. `ServerConfig.host_ipv4_override:
Option<Ipv4Addr>`) so a test wiring a real dataplane can state the intended
host IP, and key the loopback default on a *dedicated* "sim boot" flag rather
than on `dataplane_override`. This is more invasive (config-surface change +
all override call sites) and is **not** recommended over the preferred fix,
which needs no new surface and exactly matches the real signal
(`getifaddrs` success/failure).

### What NOT to do

- Do **not** weaken `classify_backend_address` to accept loopback. The loopback
  guard is intended (ADR-0053; unit test `:593`) and is correct for real
  single-node operation — a backend genuinely on `127.0.0.1` is not reachable
  via the `cgroup_connect4` rewrite path the map drives.
- Do **not** re-point the test assertions at `127.0.0.1` (the likely shape of
  the discarded +42 change). That masks the production defect and breaks the
  e2e TCP round-trip in S-BDB-01 (`:319-340`), which connects to the assigned
  VIP and expects the rewrite to the real backend.

---

## 8. Early detection / prevention

- **Default-lane guard (P2):** add a pure unit test in
  `overdrive-control-plane` that asserts `host_ipv4` resolution does **not**
  collapse to loopback when a real, resolvable `client_iface` is configured
  alongside a `dataplane_override`. This pins the exact conflation
  `9fe1b3b4` introduced at the lowest, fastest tier.
- **Contract note (P3):** the override-as-proxy pattern is fragile precisely
  because two test classes share one knob with opposite needs. If the
  resolve-then-fallback fix lands, document at `lib.rs:1197` that the loopback
  default is keyed on *iface-resolution failure*, never on override presence —
  so the next change does not reintroduce the proxy.
- **Population-diff lesson (debugging §5):** the passing comparison tests
  (`deploy_udp_walking_skeleton`, `exec_spec_walking_skeleton`,
  `multi_listener_tcp_udp_e2e`) all use `SimDataplane` via override and **none
  assert on `LOCAL_BACKEND_MAP`**. They are green because they never exercise
  the failing surface — not because the path works. The two failing tests are
  the *only* ones that combine (real EbpfDataplane via override) × (assert on
  `LOCAL_BACKEND_MAP` with a real veth IP). The green suite gave false comfort.

---

## 9. Verdict

- **Where the fix belongs:** production code — `lib.rs:1197` host_ipv4
  resolution. Not the test setup.
- **Was the discarded +42 test change legitimate now-required setup?** Most
  likely **no**. The defect is production-side; the tests as written correctly
  specify the real-veth-IP behaviour. If the +42 change re-pointed assertions
  at loopback, it was masking and its loss is harmless.
- **Single root cause** (A) with one compounding manifestation (B, same cause
  at the assertion filter). Branches C/D/E (classifier regression, missing
  bridge producer, suppressed dispatch gate) are falsified.
