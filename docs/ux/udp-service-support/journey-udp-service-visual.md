# Journey (visual) — Submit a UDP Service and trust the wire path both ways

Persona: **Ana**, Overdrive platform engineer. Goal: submit a Service
declaring a `protocol = "udp"` listener and trust that BOTH the forward
(client→VIP→backend) AND reverse (backend→client, source-rewritten to
VIP) paths work. Job grounding: **J-OPS-004** (operator trusts the wire
signal for a Service-kind workload) + **J-PLAT-004** (SimDataplane ≡
EbpfDataplane lockstep is mechanically verified).

GH #163. The defining failure: production `EbpfDataplane::update_service`
installs REVERSE_NAT_MAP entries with `proto = Tcp` ONLY; SimDataplane
installs Tcp+Udp. The two adapters diverge silently and the
`ReverseNatLockstep` invariant runs only against Sim, so DST never pins
the divergence.

## Emotional arc — "Confidence Building" (cautious → focused → trusting)

```
Confident-but-cautious  ──►  Focused  ──►  Trusting (happy)
   "UDP declared like                          OR Relieved (sad, pre-fix):
    TCP, but reverse-path                       the gate FAILS LOUDLY at
    bugs are invisible"                         PR time, not in production
```

Apple "Form Follows Feeling": the design goal is to convert an
*invisible* asymmetry (the worst class of dataplane bug — submit
succeeds, status shows Running, but real clients time out) into a
*loud, mechanical* gate failure at PR time. Ana's anxiety is specifically
about silence; the antidote is the lockstep invariant exercising both
adapters.

## ASCII flow

```
[Trigger: UDP-bearing service]
        │
        ▼
┌─ Step 1 ─────────────┐   ┌─ Step 2 ──────────────┐   ┌─ Step 3 ───────────────┐   ┌─ Step 4 ────────────┐
│ Declare udp listener │──►│ Hydrator emits         │──►│ Forward + reverse UDP   │──►│ Lockstep cannot     │
│ + submit             │   │ update_service(udp)    │   │ path completes w/ VIP   │   │ silently regress    │
│ cmd: overdrive job   │   │ via ServiceFrontend    │   │ source                  │   │ (Tier1 Sim eq +     │
│      submit *.toml   │   │ newtype (internal)     │   │ (Tier 3 wire capture)   │   │  Tier3 Ebpf accept) │
│ feels: cautious      │   │ feels: focused         │   │ feels: trusting         │   │ feels: reassured    │
│ artifact: Listener   │   │ artifact: frontend     │   │ artifact: reverse_nat   │   │ artifact: byte-equal│
│   .protocol=Udp      │   │   .proto = Udp         │   │   _key (ip,port,udp)→vip│   │   key sets Sim≡Ebpf │
└──────────────────────┘   └────────────────────────┘   └─────────────────────────┘   └─────────────────────┘
```

## Shared artifact spine (the load-bearing tuple)

The `(ip, port, proto) → vip` REVERSE_NAT key (`BackendKey` newtype) is
THE shared artifact. Single source of truth MUST be the `ServiceFrontend`
newtype's `(vip, port, proto)`. The migration is **FROM the shipped trait
option C** (`update_service(vip: Ipv4Addr, backends)`, `dataplane.rs:101`)
→ `ServiceFrontend`; locked-A (`update_service(service_id, vip:
ServiceVip, backends)`, architecture.md §5:155) was a paper decision
never implemented. The frontend **re-absorbs `ServiceVip`** (locked-A's
typed-VIP intent) but leaves `service_id`/`correlation` on the
`Action::DataplaneUpdateService` envelope by design. Today the proto
source forks: Sim derives from a hard-coded `[Tcp, Udp]`, production
hard-codes `[Tcp]`. The feature converges both onto `frontend.proto`.

```
intent Listener.protocol (Proto)         <- SOURCE OF TRUTH for proto
        │
        ▼
ServiceMapHydrator  ──►  ServiceFrontend newtype (vip,port,proto) + backends
        │                         │   (service_id/correlation stay on Action)
        │            ┌────────────┴─────────────┐
        ▼            ▼                           ▼
   SimDataplane               EbpfDataplane Step 4b
   reverse_nat_keys_for       (today: Tcp only ✗  → after: frontend.proto ✓)
   (today: [Tcp,Udp] ✗ → after: frontend.proto ✓)
        │                           │
        └─────────► REVERSE_NAT_MAP key (ip,port,proto)→vip ◄──┘
                    MUST be byte-identical across both adapters
```

## TUI mockups (material honesty — CLI feels like CLI)

### Step 1 — submit a UDP service (happy path)

```
+-- $ overdrive deploy dns-resolver.toml ---------------------------------+
| Accepted: service 'dns-resolver' (1 listener)                           |
|   listener[0]  udp/5353  -> vip 10.96.0.10:5353                          |
| Reconciling... allocation alloc-dns-resolver-0 -> Running                |
| Service 'dns-resolver' is stable                                        |
|   settled_in: 0.4s                                                       |
|   witness: startup probe #0 (udp-connect 0.0.0.0:5353)                   |
+-------------------------------------------------------------------------+
```

> `udp/5353` and `${vip}=10.96.0.10` are tracked artifacts. The proto
> token `udp` originates from `Listener.protocol` and must appear
> identically here and in the reverse-NAT key.

### Step 3 — the observable proof (Tier 3 wire capture, demo surface)

```
+-- reverse-path capture (tcpdump on client veth) ------------------------+
| # client sent: 10.244.0.5:51000 -> 10.96.0.10:5353  (to the VIP)        |
| # backend is: 10.244.0.20:5353                                          |
|                                                                         |
| 21:04:11.337  IP 10.96.0.10.5353 > 10.244.0.5.51000: UDP, length 56     |
|                  ^^^^^^^^^^^                                            |
|                  source == VIP (NOT the backend 10.244.0.20)  <-- PASS  |
+-------------------------------------------------------------------------+
```

> Pre-fix, the same capture shows `IP 10.244.0.20.5353 > ...` — the
> backend IP leaks and the client drops the response. That single
> source-address byte IS the bug.

### Multi-listener case (TCP 8080 + UDP 8081, the ServiceMapHydrator fan-out slice)

```
+-- $ overdrive deploy edge.toml -----------------------------------------+
| Accepted: service 'edge' (2 listeners)                                  |
|   listener[0]  tcp/8080  -> vip 10.96.0.11:8080                          |
|   listener[1]  udp/8081  -> vip 10.96.0.11:8081                          |
| Reconciling... allocation alloc-edge-0 -> Running                        |
| Service 'edge' is stable (settled_in: 0.5s)                             |
+-------------------------------------------------------------------------+
```

> Both listeners' forward+reverse paths must work. The hydrator emits
> one `update_service` per listener; the UDP one carries `proto=Udp`.

## Key error / sad paths

| Failure | What Ana sees | Recovery |
|---|---|---|
| Proto unsupported (e.g. `protocol = "sctp"`) | Parse-time reject at `deploy`: `error: listener[0]: unsupported protocol 'sctp' (supported: tcp, udp)` — exit 1 | Edit the spec; #164 already validates supported protos. (Confirms the boundary; no new work — but the journey acknowledges it.) |
| Reverse-path asymmetry (the #163 bug, pre-fix) | Nothing at submit time — `Accepted` + `Running` + `stable`. The bug is invisible until a real UDP client times out. | This is exactly why Step 4's lockstep gate exists: the asymmetry is converted into a **PR-time CI failure** so an operator never reaches this state. |
| Lockstep divergence reintroduced later | CI: `ReverseNatLockstep` / Tier 3 acceptance FAILS — `REVERSE_NAT key sets differ: Sim has (10.244.0.20:5353/udp), Ebpf missing it` | The author fixes the adapter before merge. Structural defense. |

## Integration checkpoints (validated in DISTILL / DESIGN)

1. **Proto is threaded end-to-end, never defaulted.** Grep the
   `update_service` call path for any `Proto::Tcp` literal that is NOT
   derived from `frontend.proto`. Zero allowed.
2. **Both adapters derive REVERSE_NAT keys from the same `ServiceFrontend`.**
   The `reverse_nat_keys_for` shape (narrowed to `frontend.proto`) and the
   production Step 4b must be provably equivalent — pinned by the
   two-pronged lockstep (Tier-1 Sim set-equality + Tier-3 Ebpf acceptance).
3. **The `ServiceFrontend` newtype is the single source of `(vip,port,proto)`.**
   No call site reconstructs the triple from separate positional args after
   the trait migration lands; `service_id`/`correlation` travelling
   separately on the `Action::DataplaneUpdateService` envelope is allowed
   by design and is NOT a violation.
