# RCA — Walking-skeleton TCP round-trip fails after VIP assignment + Running

**Date**: 2026-05-21
**Investigator**: nw-troubleshooter (Rex)
**Test**: `walking_skeleton::submit_service_workload_tcp_round_trip_through_vip_succeeds`
**Symptom**: assertion at `walking_skeleton.rs:355` — round-trip returns `None` after 40 poll attempts (2s budget); 76s total wall-clock (the rest is Lima compile + Running poll + map polls + cold start).

---

## Symptom (verbatim)

```
thread '...walking_skeleton::submit_service_workload_tcp_round_trip_through_vip_succeeds'
  panicked at crates/overdrive-control-plane/tests/integration/backend_discovery_bridge/walking_skeleton.rs:355:5:
assertion `left == right` failed:
  S-BDB-01: TCP round-trip to 10.96.0.2:8080 did not echo
  b"walking-skeleton-probe\n" within 2s
  — XDP / reverse-NAT path or backend listener regression
  left: None
  right: Some([..."walking-skeleton-probe\n"...])
```

`10.96.0.2` is the allocator-issued VIP from the submit echo. The
panic fires after all three pre-TCP assertions PASS — the test
reaches line 355 only when Running, BACKEND_MAP, and SERVICE_MAP
checks all succeed.

---

## Test-body assertion altitudes — predict → probe → finding

The test (`walking_skeleton.rs:259-355`) asserts at four progressively
higher altitudes. Reproduced in Lima at `RUST_LOG=info,
overdrive_control_plane=debug` shows the panic site is the fourth
(line 355) — the three preceding `assert!` calls all pass.

### A1 — Running within 10s (`walking_skeleton.rs:259-272`)

- **Predict**: PASS — F1 fix (UI-06, on-disk uncommitted)
  dual-emits `Action::EnqueueEvaluation(bridge)` alongside
  `StartAllocation`; convergence chain is complete after UI-04 +
  UI-05 + F1.
- **Probe**: Lima run reaches line 355 — all earlier assertions
  passed. Specifically line 268 (`alloc reached Running`) did
  not fire.
- **Finding**: PASS confirmed.

### A2 — BACKEND_MAP carries `(host_ipv4=10.244.1.1, port=8080)` within 5s (`walking_skeleton.rs:287-303`)

- **Predict**: PASS — the bridge writes
  `ServiceBackendRow` with `host_ipv4 = resolve_iface_ipv4(client_iface)`
  (= `10.244.1.1`, the IP assigned to `ws-XXXX-Na` at
  `walking_skeleton.rs:213`); the hydrator dispatches
  `DataplaneUpdateService(vip, [(host_ipv4, 8080)])`; action-shim
  `dataplane_update_service::dispatch` calls
  `EbpfDataplane::update_service` which populates BACKEND_MAP.
- **Probe**: Line 298 did not fire — assertion passed.
- **Finding**: PASS confirmed. The control-plane → dataplane
  hydration chain works end-to-end.

### A3 — SERVICE_MAP resolves `(assigned_vip=10.96.0.2, port=8080)` within 2s (`walking_skeleton.rs:312-324`)

- **Predict**: PASS — same `update_service` call populates
  SERVICE_MAP outer HoM in addition to BACKEND_MAP
  (`hashofmaps_handle.set(&service_key, inner_fd)`).
- **Probe**: Line 320 did not fire — assertion passed.
- **Finding**: PASS confirmed. The HoM outer + inner slot are
  programmed for the VIP.

### A4 — TCP round-trip echoes payload within 2s (`walking_skeleton.rs:333-360`)

- **Predict**: PASS only if (a) routing reaches the iface where XDP
  is attached, AND (b) the kernel-side XDP forward + reverse-NAT
  data path works, AND (c) the Python listener actually accepts on
  `0.0.0.0:8080`.
- **Probe**: FAIL — `poll_until` returns `None` after 40 attempts.
- **Finding**: FAIL. Drill into branches below.

---

## Population comparison (per debugging.md § 5)

Sibling Tier 3 tests in `crates/overdrive-dataplane/tests/integration/`
exercise the same XDP forward + reverse-NAT data path against
synthetic backends in netns-isolated topologies:

```
$ cargo xtask lima run -- cargo nextest run -p overdrive-dataplane \
    --features integration-tests -E "test(reverse_nat_e2e)" --no-fail-fast
Summary [7.192s] 5 tests run: 5 passed, 99 skipped
```

**5/5 reverse_nat_e2e tests PASS in the same Lima VM, same session,
same pre-flight (no leftover XDP detected).** This proves the XDP
forward + reverse-NAT data path is functioning correctly on this
kernel + iface + bpffs configuration.

The difference between the passing reverse_nat_e2e tests and the
failing walking_skeleton is the **fixture topology**, not the
data-plane code. Compare:

| Aspect | `reverse_nat_e2e` (PASS) | `walking_skeleton` (FAIL) |
|---|---|---|
| Topology builder | `ThreeIfaceTopology::create("a")` — `crates/overdrive-dataplane/tests/integration/helpers/netns.rs:249-339` | `VethFixture::setup("10.244.1.1/24")` — `walking_skeleton.rs:85-107` |
| Network namespaces | 3 netns (client_ns, lb_ns, backend_ns) created via `unshare(CLONE_NEWNET)` | NONE — runs entirely in the host netns |
| Default route to VIP | `client_ns.add_route("default", Some(&VIP.to_string()), None)` at `netns.rs:324` — routes default via `10.0.0.1` (= VIP) | NONE — `10.96.0.x` has no route on the host |
| Backend route | `backend_ns.add_route("default", Some(&LB_BACKEND_IP.to_string()), None)` at `netns.rs:326` | N/A |
| IP forwarding | `lb_ns.sysctl("net.ipv4.ip_forward", "1")` at `netns.rs:312` | NOT set |
| rp_filter disable | Per-netns at `netns.rs:317-320` | NOT set |
| Client → XDP-ingress topology | Client packet exits client_ns on its veth, lands on lb_ns's veth ingress where XDP is attached | Test process in host netns → no ingress on `ws-XXXX-Na` (the veth is host-side, the peer `ws-XXXX-Nb` has no IP and no traffic) |
| Backend listener netns | Spawned inside `backend_ns` so its `0.0.0.0:8080` bind is reachable only via `backend_ns`'s veth | Spawned in host netns (`ExecDriver` does not enter a netns — verified via grep: zero `setns`/`unshare`/`CLONE_NEWNET` in `crates/overdrive-worker/src/`); `0.0.0.0:8080` binds on every host iface including `lo` |

The walking-skeleton was implemented from the `architecture.md`
§ 6.2 sample (lines 780-784), which references a fictional
`LimaFixture` with `client_iface` / `backend_iface` fields but does
not specify any netns/route plumbing. The implementer landed
`VethFixture` (`walking_skeleton.rs:77-128`) as a literal "veth pair
+ bpffs pin dir" interpretation, omitting the netns + route + sysctl
setup that the dataplane integration tests have always required for
real-traffic round-trip.

---

## Convergence-chain trace (all green post-UI-04 + UI-05 + F1)

Every stage between submit and BPF-map population is wired correctly
per `audit-reconciler-handoff-topology.md` and the per-altitude
assertions A1-A3 above:

| Stage | Status | Citation |
|---|---|---|
| HTTPS POST `/v1/jobs` → handler enqueues `workload-lifecycle` | OK | `handlers.rs:378/409` (per audit § E1) |
| WorkloadLifecycle reconcile emits `StartAllocation` + `EnqueueEvaluation(bridge)` (UI-06 / F1 dual-emit) | OK | `reconciler.rs` uncommitted diff lines +1391-1430 |
| Action-shim StartAllocation → driver.start (Python process spawned) → obs.write(Running) | OK | `action_shim/mod.rs:507-582` (proven by A1 pass) |
| Action-shim EnqueueEvaluation → `broker.submit(bridge)` | OK | `action_shim/mod.rs:810` |
| Bridge reconcile observes Running + emits `WriteServiceBackendRow` + `EnqueueEvaluation(hydrator)` (UI-05) | OK | `backend_discovery_bridge.rs:369, 405` (proven by A2 pass) |
| Action-shim WriteServiceBackendRow → obs.write(ServiceBackend) | OK | `action_shim/mod.rs:800` |
| Action-shim EnqueueEvaluation → `broker.submit(hydrator)` | OK | `action_shim/mod.rs:810` |
| Hydrator reconcile observes ServiceBackend + emits `DataplaneUpdateService` | OK | `reconciler.rs:2339` |
| Action-shim DataplaneUpdateService → `EbpfDataplane::update_service(vip, [(host_ipv4, 8080)])` → BACKEND_MAP + SERVICE_MAP populated | OK | `action_shim/mod.rs:753` (proven by A2 + A3 pass) |
| Python listener binds `0.0.0.0:8080` | LIKELY OK (host-netns bind succeeds; only one Service workload in the test) | `walking_skeleton.rs:170-188` |
| Kernel routes `10.96.0.2:8080` packet to client_iface ingress so XDP can intercept | **FAIL — no route exists** | `walking_skeleton.rs:85-107`; compare `netns.rs:322-326` |

---

## Root cause

**File:line — `crates/overdrive-control-plane/tests/integration/backend_discovery_bridge/walking_skeleton.rs:84-128` (`VethFixture::setup`/`Drop`).**

The walking-skeleton's `VethFixture` creates a veth pair (`ws-XXXX-Na`,
`ws-XXXX-Nb`) in the host network namespace, assigns one IP
(`10.244.1.1/24`) to the client-side veth, and does NOTHING ELSE.
There is:

1. **No route to the VIP CIDR** (`10.96.0.0/16`). When the test
   process calls `tokio::net::TcpStream::connect((10.96.0.2, 8080))`
   from the host netns (`walking_skeleton.rs:337`), the kernel
   routing table has no entry that would steer the SYN out via the
   `ws-XXXX-Na` veth (where `xdp_service_map_lookup` is attached).
   The default route — if any — sends the SYN out of the host's
   real default gateway iface (e.g., Lima's `eth0`), where Overdrive
   has attached no XDP program. The SYN either gets `EHOSTUNREACH`
   at connect-time or is forwarded into the void.

2. **No network-namespace isolation between the test process (TCP
   client) and the Python backend (TCP listener)**. Both run in the
   host netns. The Python listener binds `0.0.0.0:8080`, which means
   `127.0.0.1:8080` is reachable directly via the loopback path —
   bypassing XDP entirely. (`ExecDriver` does not enter a netns:
   verified via `grep -r "setns\|unshare\|CLONE_NEWNET" crates/
   overdrive-worker/src/` returning zero matches.) This is not a
   *correctness* failure on its own (the test connects to
   `10.96.0.2`, not `127.0.0.1`) but it means even if the route
   issue were resolved the data path would not match production
   semantics — the XDP forward and reverse-NAT would have to
   physically rewrite headers across the veth pair, not just
   short-circuit through loopback.

3. **No `net.ipv4.ip_forward=1`, no per-iface `rp_filter=0`, no
   default route on the backend side** — the sysctls and routes that
   the dataplane integration tests have always set up (`netns.rs:
   312-326`).

By contrast, the `ThreeIfaceTopology` that every passing sibling
test uses creates THREE distinct netns (client / lb / backend),
plumbs three veth pairs across them, configures rp_filter +
ip_forward, and installs a default route in `client_ns` pointing
at the VIP. The TCP SYN therefore exits the client-ns veth, hits
the lb-ns veth ingress (where XDP runs), gets rewritten to the
backend, exits the lb-ns backend-side veth, hits backend-ns ingress,
and reaches the listener. The return path traverses the
reverse-NAT XDP program back through lb-ns. This is the topology
the production XDP code was designed against.

The walking-skeleton's `VethFixture` collapses all three netns into
the host, omits the routes, and expects the kernel to do something
sensible with `connect((10.96.0.2, 8080))`. It will not.

### Comparing populations — assertion-by-assertion

The pre-TCP assertions (A1-A3) pass *despite* the missing topology
because they observe BPF-map state via direct accessor methods on
the `EbpfDataplane` Arc the test retains
(`dataplane.backend_map_entries()`,
`dataplane.service_map_contains(...)`). Map population is an
in-process side-effect of the `update_service` syscall path; it
does NOT require any packet to traverse the kernel. The TCP probe
(A4) is the first step that requires real packet traversal — and
it is the first one that fails.

### Secondary contributing factor

The architecture doc at `docs/feature/backend-discovery-bridge-service-reachability/design/architecture.md:780-784`
references `LimaFixture` with `client_iface`/`backend_iface` fields
but does not specify the netns + routing plumbing such a fixture
would need to enable the D3 in-gate TCP round-trip. The
walking-skeleton crafter implemented a literal "veth pair only"
fixture, missing the design's intent. The design itself is
under-specified at this altitude — there is no `LimaFixture`
anywhere in the tree (`grep -r "LimaFixture" crates/` returns
zero matches), and the only working precedent for real-traffic
through XDP is `ThreeIfaceTopology`, which the design does not
reference.

---

## Recommendation

**Single fix, in-scope**: replace `VethFixture` in `walking_skeleton.rs`
with a topology that mirrors `ThreeIfaceTopology` from
`crates/overdrive-dataplane/tests/integration/helpers/netns.rs`.
Specifically:

1. Create three netns (per-test, RAII).
2. Plumb the lb-side veth pair into the lb-ns, the client-side veth
   into the client-ns, the backend-side veth into the backend-ns.
3. Assign IPs (the VIP on the lb-side veth ingress, distinct
   per-test client/backend IPs).
4. Install `default → VIP` route in client-ns and `default →
   lb_backend_ip` route in backend-ns.
5. Set `ip_forward=1` + `rp_filter=0` per `netns.rs:312-320`.
6. Spawn the `TestServer` (and therefore the `EbpfDataplane`
   loader + XDP attach) inside the lb-ns via `setns(CLONE_NEWNET)`
   so XDP attaches to the lb-side veths that participate in the
   topology.
7. Spawn the Python listener inside the backend-ns. **This requires
   `ExecDriver` to accept a netns FD or path** — which is currently
   not in its public API. Either:
   - (a) extend `ExecDriver::start` to take an optional
     `netns_path: Option<PathBuf>` and call `setns(netns_fd,
     CLONE_NEWNET)` in the spawned process before `execve`, OR
   - (b) keep `ExecDriver` as-is and bind the backend listener via a
     test-side helper that runs in the backend-ns (decoupled from
     `ExecDriver`), with the test asserting the *property*
     "submit-allocate-Running-backendmap-servicemap-roundtrip"
     end-to-end without exercising `ExecDriver`'s production code
     path for the backend itself. This option fails the walking-
     skeleton's stated purpose (DWD-07 CM-A: drive everything
     through the production submit path).
8. Issue the TCP probe **from inside the client-ns** via `setns +
   tokio::net::TcpStream::connect` so the SYN egresses the client-ns
   veth and hits the lb-ns XDP ingress.

The fix is in the test fixture, NOT in production code. The
production convergence chain (after UI-04 + UI-05 + F1) is verified
end-to-end by A1-A3 passing.

**Out-of-scope blocker**: option (a) above expands `ExecDriver`'s
public API surface — this is a real production code change that
needs explicit user approval before landing. Surface this as a
choice point to the user; do not unilaterally extend `ExecDriver`.

**Recommended sequence**:
1. Surface the topology gap + the `ExecDriver` netns-API choice
   point to the user.
2. On approval, build a `WalkingSkeletonFixture` modeled after
   `ThreeIfaceTopology` in
   `crates/overdrive-control-plane/tests/integration/backend_discovery_bridge/walking_skeleton_fixture.rs`
   (new file, test-side only).
3. Either extend `ExecDriver::start` for netns option (a), or
   decouple the backend listener from `ExecDriver` for option (b) —
   user decides.
4. Update `architecture.md` § 6.2 to reference the real
   `WalkingSkeletonFixture` (replacing the fictional `LimaFixture`
   placeholder) so the next reader is not led down the same path.

The F1 fix (UI-06 reconciler dual-emit, currently on-disk
uncommitted) IS correct and IS the right shape — it should still
land. The walking-skeleton failure is downstream of every wiring
gap F1 fixes; F1 just unblocked A1-A3, exposing the fixture-level
gap that was always there.

---

## Confidence

**High**. Evidence:
- Sibling Tier 3 tests covering the exact same XDP forward +
  reverse-NAT path (`reverse_nat_e2e` × 5 scenarios) ALL PASS in the
  same Lima VM, same session, same kernel — proves data path is OK.
- Walking-skeleton pre-TCP assertions ALL PASS — proves convergence
  chain + map population is OK.
- The single difference between the passing sibling tests and the
  failing walking-skeleton is the fixture topology, line-by-line
  comparable at `walking_skeleton.rs:85-107` vs `netns.rs:249-339`.
- The reproduction is 100% deterministic in Lima.

No further probes required to identify the defect. The "Next probe"
section is empty by design.
