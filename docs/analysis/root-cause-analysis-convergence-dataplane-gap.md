# Root-Cause Analysis — "UDP Service converges but the dataplane is never programmed"

- **Investigator**: Rex (Toyota 5-Whys RCA)
- **Branch / SHA**: `marcus-sa/udp-support` @ `53124ddd` (one commit past `e9cec107`; no source delta on the convergence/dataplane path)
- **Date**: 2026-06-03
- **Method**: 5-Whys, multi-causal, evidence-required, depth 5; runtime confirmation in Lima (real `EbpfDataplane`, `ExecDriver`, `bpftool`)
- **Status**: ROOT CAUSE CONFIRMED. The reported defect is **a test-fixture port conflict**, not a convergence→dataplane wiring bug and not a UDP-specific code gap.

---

## TL;DR (the four questions)

1. **Does TCP work?** **Yes.** A TCP Service on a free port (5354) converges, the backend stays Running, and `LOCAL_BACKEND_MAP` is programmed with exactly one entry.
2. **What is the root cause?** The `dns-resolver.toml` fixture binds **UDP port 5353**, which is already owned by **`systemd-resolved`** inside the Lima VM. The `socat … UDP-RECVFROM:5353 …` backend fails `bind(): Address already in use` and exits 1 immediately. The allocation therefore never stays Running, the `BackendDiscoveryBridge` never sees a Running alloc, no `ServiceBackendRow` is written, the hydrator emits no `RegisterLocalBackend`, and the workload-lifecycle reconciler finalizes the alloc terminal and **releases the VIP**. Empty maps are the *downstream symptom* of a backend that never came up.
3. **What is the fix?** Change the fixture port off 5353 (any free UDP port, e.g. 5354/7777) — OR disable `systemd-resolved`'s stub listener in the test VM. This is a fixture/environment fix, not a production code change.
4. **Is it UDP-specific or protocol-agnostic?** **Neither in the way the problem statement framed it.** The convergence→dataplane wiring is protocol-AGNOSTIC and *works for both TCP and UDP*. The observed "UDP fails / TCP works" split was an **artifact of port choice** (UDP fixture used 5353 = occupied; TCP fixture used 5354 = free), not of the protocol. UDP on a free port programs the dataplane identically to TCP.

A secondary, independent finding: **the original probe dumped the wrong maps.** On single-node, every backend resolves to `host_ipv4` and is classified **LOCAL**, so it lands in `LOCAL_BACKEND_MAP` (+ the `cgroup_connect4` hook), *not* `SERVICE_MAP`/`REVERSE_NAT_MAP`. Those two maps being empty on single-node localhost is **expected**, not a defect.

---

## Problem definition & scope

**Symptom as reported:** a deployed `dns-resolver` UDP Service (udp/5353) shows `Allocations: 1` but `bpftool map dump name {SERVICE_MAP,REVERSE_NAT_MAP}` → `Found 0 elements`; "VIP→backend datapath is dead."

**Scope of this RCA:** the production single-node `overdrive serve` path from deploy → convergence → dataplane map programming, for UDP and TCP Services, on the Lima dev VM. Out of scope: multi-node/remote-backend steering (the `SERVICE_MAP`/`REVERSE_NAT_MAP` path), kernel-side packet forwarding correctness (covered by the passing Tier-3 `reverse_nat_udp_e2e` test).

**Initial evidence collected (this investigation, Lima, real infra):**

- `ss -ulnp` → `systemd-resolve (pid 3266620)` owns `0.0.0.0:5353` and `[::]:5353`.
- Standalone `socat -T15 UDP-RECVFROM:5353,fork PIPE` →
  `E bind(5, {AF=10 [::]:5353}, 28): Address already in use`, exit 1.
- `overdrive deploy dns-resolver.toml` (5353): `socat` process absent within 3 s; `LOCAL_BACKEND_MAP` `Found 0 elements`; serve.log shows `ReleaseServiceVip` dispatched ~+6.8 s.
- `overdrive deploy` UDP on **free port 7777**: `socat UDP4-RECVFROM:7777` RUNNING at t=4 s; `LOCAL_BACKEND_MAP` `Found 1 element` (`key … 61 1e …` = port `0x1E61` = 7777).
- `overdrive deploy` TCP on **free port 5354**: `socat TCP4-LISTEN:5354` RUNNING; `LOCAL_BACKEND_MAP` `Found 1 element`.

---

## The "compare populations" diagnostic (debugging.md §5) — UDP vs TCP

Capturing both populations with the *same* probe and diffing them was decisive — and it caught a confounder that the original framing would have led astray.

| Arm | Port | Port owner pre-deploy | Backend stays Running? | `LOCAL_BACKEND_MAP` | `SERVICE_MAP` / `REVERSE_NAT_MAP` |
|---|---|---|---|---|---|
| UDP (orig fixture) | **5353** | **systemd-resolved** | **No — bind EADDRINUSE, exit 1** | **0 elements** | 0 / 0 |
| TCP | 5354 | free | Yes | **1 element** | 0 / 0 |
| **UDP (control)** | **7777** | **free** | **Yes** | **1 element** | 0 / 0 |

The diff that *looked* like "UDP-specific" collapses the moment the port variable is controlled: UDP@7777 behaves identically to TCP@5354. The real differentiator is **port-already-bound**, not **protocol**. Per debugging.md §2 (error codes are taxonomy, not mechanism) and §8 (`let _`-style env drift): the "empty map" is the layer that gave up, not the cause; the cause is one hop upstream (the backend never bound).

The first probe iteration also reproduced a *second* confounder (debugging.md §3 — inspection-tool gaps look like negative evidence): `bpftool map dump name LOCAL_BACKEND_MAP` returned `Error: can't parse name` because the kernel truncates map names to 15 chars (`LOCAL_BACKEND_M`). Dumping by **map ID** instead of name surfaced the populated TCP/UDP-7777 entries that the by-name dump had hidden. Had we trusted the by-name dump, we would have falsely concluded "LOCAL_BACKEND_MAP empty for TCP too."

---

## 5-Whys — multi-causal

### Branch A — the reported defect: UDP@5353 leaves the maps empty

```
WHY 1A: SERVICE_MAP + REVERSE_NAT_MAP empty after a UDP Service "converges".
  [Evidence: bpftool map dump id <SERVICE_MAP>/<REVERSE_NAT_MAP> → Found 0 elements.]

  WHY 2A: On single-node, the converged backend is classified LOCAL, so it is
          NOT written to SERVICE_MAP/REVERSE_NAT_MAP at all — those are the
          REMOTE-steering maps. The single-node datapath is LOCAL_BACKEND_MAP.
    [Evidence: service_map_hydrator.rs:328-359 partitions backends into
     (local, remote) by `v4 == host_ipv4`; only `remote` → DataplaneUpdateService
     (SERVICE_MAP/REVERSE_NAT_MAP), `local` → RegisterLocalBackend
     (LOCAL_BACKEND_MAP). backend_discovery_bridge.rs:346-351 sets every Phase-1
     backend addr = host_ipv4:port, so on single-node every backend is LOCAL.]
    => The two dumped maps are EXPECTED-empty on single-node localhost. The probe
       dumped the wrong maps. (Inspection-tool/altitude error — debugging.md §3/§7.)

    WHY 3A: Even LOCAL_BACKEND_MAP is empty for THIS deploy — because no
            RegisterLocalBackend action was ever emitted for it.
      [Evidence: bpftool map dump id <LOCAL_BACKEND_MAP> for the 5353 deploy →
       Found 0 elements; for the 7777 UDP deploy and 5354 TCP deploy → Found 1.]

      WHY 4A: The hydrator emits RegisterLocalBackend only for a backend that
              exists in a ServiceBackendRow, and the bridge writes a
              ServiceBackendRow only from the *Running* alloc set
              (hydrate_actual filters alloc_status rows to state==Running).
              For the 5353 deploy the alloc never had a Running row to harvest.
        [Evidence: backend_discovery_bridge.rs:342-352 builds `backends` from
         `actual.actual.running`; reconciler_runtime.rs:2025-2032 populates
         `running` from `alloc_status_rows().filter(state==Running)`. With no
         Running alloc the bridge emits zero WriteServiceBackendRow, the hydrator
         desired set is empty (service_map_hydrator.rs:290 loop body never runs).]

        WHY 5A (ROOT): The socat backend for udp/5353 fails to bind and exits
                immediately, so the alloc never stays Running. Port 5353 is owned
                by systemd-resolved inside the VM.
          [Evidence: ss -ulnp → systemd-resolve owns 0.0.0.0:5353 + [::]:5353;
           standalone `socat -T15 UDP-RECVFROM:5353,fork PIPE` →
           "E bind(...:5353...): Address already in use", exit 1; under overdrive
           the socat is absent within 3 s and serve.log shows the
           workload-lifecycle finalize → ReleaseServiceVip dispatch.]
          => ROOT CAUSE A: TEST-FIXTURE / ENVIRONMENT PORT CONFLICT.
             dns-resolver.toml binds UDP 5353, which systemd-resolved already
             owns on the Lima VM. The backend can never bind; the whole
             convergence→dataplane chain is starved of a Running alloc by design.
```

### Branch B — why it *looked* UDP-specific (the red herring)

```
WHY 1B: "UDP fails to program the dataplane but TCP succeeds" (the framing).
  [Evidence: original probe — UDP@5353 maps empty; TCP@5354 LOCAL_BACKEND_MAP=1.]

  WHY 2B: The two arms differed in MORE than protocol: UDP used port 5353, TCP
          used port 5354.
    [Evidence: dns-resolver.toml → 5353/udp; the TCP control spec → 5354/tcp.]

    WHY 3B: 5353 is occupied (systemd-resolved); 5354 is free. The protocol was
            confounded with the port.
      [Evidence: ss -ulnp shows :5353 occupied, :5354/:7777 free.]

      WHY 4B: Controlling the variable: UDP on a FREE port behaves like TCP.
        [Evidence: UDP@7777 → socat RUNNING, LOCAL_BACKEND_MAP Found 1 element
         (key encodes port 0x1E61 = 7777). Identical outcome to TCP@5354.]

        WHY 5B (ROOT): The convergence→dataplane wiring is PROTOCOL-AGNOSTIC and
                correct for both TCP and UDP. The proto threads spec → intent →
                ListenerRow → ProjectedListener → hydrator → action-shim →
                EbpfDataplane unchanged; the LOCAL classification + RegisterLocalBackend
                path does not branch on proto.
          [Evidence: hydrate_bridge_desired_listeners (reconciler_runtime.rs:1702-1714)
           carries listener.protocol verbatim; service_map_hydrator
           push_register_local_backend_actions has no proto branch; the UDP@7777
           run programmed LOCAL_BACKEND_MAP identically to TCP@5354.]
          => ROOT CAUSE B: NO UDP-SPECIFIC DEFECT EXISTS in the convergence/dataplane
             wiring. The apparent UDP/TCP split was an artifact of comparing an
             occupied UDP port against a free TCP port (a populations-comparison
             hazard — debugging.md §5: compare like-for-like or the diff lies).
```

### Branch C — the dataplane-instance "prime suspect" (cleared)

```
WHY 1C: Hypothesis — the action-shim dispatches to a DIFFERENT Dataplane instance
        than the one that attached XDP + pinned the maps at boot.
  [This was the problem statement's PRIME SUSPECT.]

  WHY 2C: There is exactly one Dataplane Arc. It is constructed once and shared.
    [Evidence: lib.rs:1089-1166 builds ONE EbpfDataplane via new_with_pin_dir →
     `Arc::new(ebpf_dataplane)`; stored as state.dataplane (lib.rs:1230);
     reconciler_runtime.rs:1155-1167 passes `state.dataplane.as_ref()` to
     action_shim::dispatch. No SimDataplane / NoopDataplane / second instance on
     the production path — dataplane_override is None in production (lib.rs:1038).]

    WHY 3C: Runtime confirmation: when the backend IS Running (UDP@7777, TCP@5354),
            the SAME pinned LOCAL_BACKEND_MAP that boot created shows the entry the
            action-shim wrote.
      [Evidence: the populated LOCAL_BACKEND_MAP entries are visible via bpftool
       against the live pinned maps the booted serve created.]
      => ROOT CAUSE C: NONE. The prime suspect is FALSIFIED. The dataplane-instance
         identity is correct; programming reaches the pinned maps whenever a
         Running backend exists.
```

### Branch D — VIP allocation / "no VIP in alloc status" (not a defect)

```
WHY 1D: `alloc status` shows no VIP line.
  [Evidence: every probe arm renders `Allocations` + `Listeners` (e.g. `5353/udp`,
   `7777/udp`, `5354/tcp`) and no VIP line.]

  WHY 2D: A VIP IS allocated at admission and IS released on terminal — the VIP
          machinery runs; it is simply not part of the `alloc status` render surface.
    [Evidence: handlers.rs:324-326 `guard.allocate(digest_bytes)` at deploy;
     serve.log shows `release_service_vip.dispatch` for the 5353 (terminal) deploy;
     alloc.rs render surface emits Allocations + Listeners only.]
    => NOT A DEFECT (Branch D): the absent VIP line is the current intended
       `alloc status` surface, not evidence of a missing VIP. (Reported as a
       contributing symptom in the problem statement; it is not causal.)
```

---

## Cross-validation (backwards chain)

- **Root Cause A (port conflict) ⇒ symptom?** If socat can't bind 5353, the alloc
  never reaches Running ⇒ bridge `running` set empty ⇒ no ServiceBackendRow ⇒
  hydrator desired empty ⇒ no RegisterLocalBackend ⇒ LOCAL_BACKEND_MAP empty ⇒
  (and SERVICE_MAP/REVERSE_NAT_MAP empty regardless, per A/WHY-2A) ⇒ workload-lifecycle
  finalizes terminal ⇒ ReleaseServiceVip. **Every observed symptom is produced.** ✔
  Forward-verified at runtime: removing the conflict (port 7777) flips every link of
  the chain GREEN (socat Running, LOCAL_BACKEND_MAP populated, no VIP release).
- **A + B consistent?** A is the concrete cause for the 5353 deploy; B explains why
  the A-induced failure was *mis-attributed to UDP*. They reinforce, do not
  contradict: the protocol-agnostic wiring (B) is exactly why a *free-port* UDP
  deploy works, which is what isolates A as a port problem. ✔
- **A + C consistent?** C (single shared dataplane) is *required* for A's forward
  chain to terminate in a populated map on the success path — and it does. ✔
- **All symptoms explained?** "Allocations: 1" (the alloc row exists even though the
  backend died — Phase-1 records the alloc, then finalizes), "no socat", "empty
  SERVICE_MAP/REVERSE_NAT_MAP" (expected on single-node), "empty LOCAL_BACKEND_MAP"
  (backend never Running), "no VIP line" (render surface, Branch D). No residual
  unexplained symptom. ✔

---

## Is this an intended phase boundary, a bug, or an environment issue?

- The **convergence→dataplane wiring is intended behaviour and works** (Branches B, C).
  No production code defect was found on this path. The Tier-3 `reverse_nat_udp_e2e`
  test passing is consistent: it exercises the REMOTE path with a real non-host
  backend IP via `ThreeIfaceTopology`, which is why it programs REVERSE_NAT_MAP;
  production single-node uses the LOCAL path, a different (and also-working) map.
- The **defect is an environment/test-fixture issue**: the fixture's port collides
  with `systemd-resolved` on the dev VM. This is neither a phase boundary nor a
  product bug — it is a fixture that picked an occupied port.
- One genuine **product-robustness gap** is exposed but is arguably working-as-designed
  for Phase 1: a backend that exits immediately on a bind error is correctly observed,
  finalized, and its VIP released — but the operator-visible signal (`alloc status`)
  does not surface the bind-failure cause or the terminal/Failed state prominently
  (it still prints `Allocations: 1`). That is a UX/observability opportunity, not a
  datapath bug. Surface it to the user; do not treat it as the root cause.

---

## Solutions (mapped to root causes; validated against ALL root causes)

| # | Addresses | Type | Action | Validates against |
|---|---|---|---|---|
| **S-A1** | Root Cause A | **Permanent fix (fixture)** | Change `crates/overdrive-cli/examples/dns-resolver.toml` (and any verification runner that deploys it for a *convergence* capture) off UDP 5353 to a port not owned by the host stub resolver (e.g. 5454/15353/7777). Keeps the udp-token threading intent intact. | Fixes A directly; B/C unaffected (proto-agnostic path already works); D irrelevant. |
| **S-A2** | Root Cause A | **Mitigation (env)** | For convergence captures that must use 5353, disable the stub listener in the test VM (`systemd-resolved` `DNSStubListener=no`, or stop the unit) before deploy. Documented one-liner in the verification harness. | Alternative to S-A1 when 5353 is load-bearing for the scenario; does not touch product code. |
| **S-A3** | Root Cause A (early detection) | **Early detection** | Have the verification runner pre-flight `ss -uln`/`ss -tln` for the fixture's listener port and fail fast with "port already bound by <pid/comm>" before deploying — turns a silent terminal-finalize into an actionable message. | Prevents the same confound from recurring in any future capture, UDP or TCP. |
| **S-B1** | Root Cause B (process) | **Prevention (test discipline)** | When comparing UDP vs TCP (or any two arms), hold every non-subject variable equal — same port-availability class. Encode in the probe/runner so a future "compare populations" run can't confound protocol with port. | Stops the mis-attribution that sent the original investigation toward "UDP path". |
| **S-A4** | Root Cause A (observability, optional) | **Permanent fix (UX)** — *needs user decision* | Make `alloc status` surface terminal/Failed state + the captured `stderr_tail` (the action-shim already propagates `stderr_tail`; `classify_driver_failure` already exists) so a bind-failure backend reads as `Failed: bind: Address already in use`, not a bare `Allocations: 1`. | Independent of A's fixture fix; improves the operator signal for *any* backend that exits on start. **Out of RCA scope to implement; surface to user.** |

**Backward-chain check on the solution set:** S-A1 (or S-A2) makes the 5353 backend bind ⇒ alloc reaches Running ⇒ bridge writes ServiceBackendRow ⇒ hydrator emits RegisterLocalBackend ⇒ LOCAL_BACKEND_MAP populated (the exact chain that already fires for 7777/5354). No solution depends on a UDP-specific code change (there is no UDP defect), and none touches the (correct, single) dataplane instance. The solution set is complete and non-contradictory across A/B/C/D.

---

## Evidence appendix — key source anchors

- Single dataplane instance + dispatch: `crates/overdrive-control-plane/src/lib.rs:1089-1166, 1230`; `crates/overdrive-control-plane/src/reconciler_runtime.rs:1155-1167`.
- LOCAL vs REMOTE backend partition: `crates/overdrive-core/src/reconcilers/service_map_hydrator.rs:328-359, 388-420`.
- Bridge backend addr = `host_ipv4:port`, Running-set source: `crates/overdrive-core/src/reconcilers/backend_discovery_bridge.rs:342-352`; `crates/overdrive-control-plane/src/reconciler_runtime.rs:2025-2032`.
- VIP allocate at admission: `crates/overdrive-control-plane/src/handlers.rs:324-326`.
- Map names + single-node LOCAL_BACKEND_MAP datapath: `crates/overdrive-dataplane/src/lib.rs:74-91, 569-580`; key struct `crates/overdrive-dataplane/src/maps/local_backend_map_handle.rs:55-62`.
- Tier-3 UDP REMOTE-path test (why it programs REVERSE_NAT_MAP, not LOCAL): `crates/overdrive-dataplane/tests/integration/reverse_nat_udp_e2e.rs:74-90`.
- Runtime confirmation (this investigation): `systemd-resolved` owns `:5353`; `socat …:5353` → `bind: Address already in use` exit 1; UDP@7777 + TCP@5354 → `LOCAL_BACKEND_MAP Found 1 element`; UDP@5353 → `Found 0 elements` + `release_service_vip.dispatch`.

*All temporary probe scripts were removed after capture; no files were left under `verification/harness/`.*
