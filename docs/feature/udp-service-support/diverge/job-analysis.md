# Job Analysis (brief) — udp-service-support DIVERGE

> **Scope note.** This is a SCOPED DIVERGE on ONE architectural decision
> (how to thread per-service L4 proto through `Dataplane::update_service`),
> dispatched to close review finding H3 ("simpler alternative unweighed")
> against `feature-delta.md`'s DISCUSS wave. The job is **pre-validated** —
> this section CITES the existing jobs and ODI outcomes rather than
> re-running a full JTBD extraction (per the dispatch instruction and
> `feature-delta.md` § D5). **jobs.yaml is NOT modified** — see SSOT note
> at the foot of this file.

## Raw decision under divergence

> How to thread the per-service L4 protocol (`Proto::Tcp` / `Proto::Udp`)
> through `Dataplane::update_service` so production `EbpfDataplane`
> installs `REVERSE_NAT_MAP` entries matching the declared protocol —
> fixing GH #163's TCP-only divergence between `EbpfDataplane` (installs
> only `Tcp`) and `SimDataplane` (installs both `Tcp`+`Udp` via
> `reverse_nat_keys_for`'s `[Tcp, Udp]` hardcode at
> `crates/overdrive-sim/src/adapters/dataplane.rs:277`).

## Validated jobs this DIVERGE rides (NO new job)

Per `docs/product/jobs.yaml` and `feature-delta.md` § "Story → job
traceability" — this decision serves **two existing active jobs**:

| Job | Title | Relevance to this decision |
|---|---|---|
| **J-OPS-004** | "Submit a Service-kind workload and trust the wire signal…" (operator-trust contract, served_by_phase 1, `active`) | The operator-facing outcome: a UDP service's reverse path must work both ways (response sourced from VIP, never leaking the backend IP). The `update_service` proto-threading is the mechanism that delivers this for the UDP dimension. |
| **J-PLAT-004** | "Run a reconciler I wrote against a simulated cluster and know it converges" (dataplane-correctness contract, served_by_phase 2, `active`) | The `update_service` signature is the surface the `ReverseNatLockstep` ESR invariant pins. The decision determines *whether the lockstep can be expressed against BOTH adapters as identical `(ip,port,proto)→vip` sets* — the structural defense against #163 recurring. |

UDP-as-first-class is an **extension** of these jobs along the protocol
dimension (vision.md principle 4 — "all workload types are first class"),
not a new motivation. Minting a per-protocol job would fragment J-OPS-004
(then owing a J-OPS-006 for SCTP, etc.). This is the locked D5 decision.

## Job statements (cited, not re-extracted)

**Functional (J-OPS-004 extension):** When I submit a Service-kind
workload declaring a `protocol = "udp"` listener, I want the platform to
load-balance UDP such that the backend's response is source-rewritten
back to the VIP, so I can run UDP-bearing services (DNS, QUIC edge, game
servers, syslog) and trust the connection works both ways.

**Functional (J-PLAT-004 extension):** When I change the dataplane's
service-update path, I want `SimDataplane` and production `EbpfDataplane`
to be provably equivalent on their REVERSE_NAT key sets for every
protocol, so a forward/reverse asymmetry cannot reach production
undetected.

**Emotional:** Relief from the *anxiety of silent asymmetry* — the worst
class of dataplane defect, where `deploy` succeeds, `alloc status`
shows Running, and the failure surfaces only when a real client times out.

**Social:** The dataplane author is seen as shipping a surface that future
engineers can extend without re-introducing a protocol-asymmetry incident.

## ODI outcome statements (the decision must move these)

Drawn from `feature-delta.md` § "Outcome KPIs (consolidated)" — restated
in ODI direction-metric-object-context form. These are the success
criteria the WINNING option must serve; they are the empirical anchor for
the taste-evaluation's Desirability and Speed-as-Trust scores.

| ID | ODI statement | Source KPI | Status |
|---|---|---|---|
| **O1** | Minimize the likelihood of a UDP service's reverse-path response leaking the backend IP instead of the VIP. | K1 (North Star) | Under-served (0% → 100%) |
| **O2** | Minimize the likelihood of a Sim/Ebpf REVERSE_NAT proto-fan-out divergence reaching production undetected. | K2 | Under-served (0% caught pre-merge — #163 shipped) |
| **O3** | Minimize the likelihood that the kernel reverse-NAT program fails to rewrite a `proto=17` response source. | K3 | Under-served (0% — no UDP entry to match) |
| **O4** | Minimize the number of call sites that must reconstruct the `(vip, port, proto)` triple from scattered arguments. | K5 | Under-served (proto absent from trait entirely) |
| **O5** | Minimize the effort required to extend the service-update surface to a NEW protocol (SCTP) or a multi-listener service without re-migrating call sites. | K4 + design-longevity | Under-served (single `update_service` per service collapses multi-listener) |

**Note on O4/O5 framing.** O4 and O5 are where the option study bites:
the *minimal* option (1) and the *aggregate* option (2) move O1/O2/O3
identically (they all install the right key set once proto reaches Step
4b), but they move O4 and O5 differently. O4 (scattered-args) and O5
(extension cost) are the discriminating outcomes — the taste matrix's T1
(Subtraction) and T2 (Concept Count) criteria score against exactly these.

## Gate check

- [x] Job at strategic level (J-OPS-004 operator-trust; J-PLAT-004
  dataplane-correctness) — not tactical.
- [x] ≥3 ODI outcome statements (5: O1–O5).
- [x] No feature reference inside the job statements (the *decision* is
  feature-shaped; the *jobs* are not).
- [x] No new job minted — rides existing J-OPS-004 + J-PLAT-004.

## SSOT note — jobs.yaml UNCHANGED

`docs/product/jobs.yaml` is **not modified** by this DIVERGE. J-OPS-004
and J-PLAT-004 already exist and are `active`. Per the dispatch
instruction, this DIVERGE rides the existing jobs; it does not mint a new
job nor edit jobs.yaml. (Optional, deferred to DISCUSS-revision author: a
changelog note that J-OPS-004 now spans wire-path correctness — review
finding M2, non-blocking.)
