# Independent DESIGN review — vsock Attachment Replacement (revision 4)

- Reviewer: independent `nw-solution-architect-reviewer` (Opus), 2026-10-06
- Under review: `feature-delta.md` § "vsock Attachment Replacement DESIGN" (revision 4), ADR-0145 … ADR-0164
- Provenance: the reviewer's file write failed with `ENOSPC`. This record is reconstructed by the
  orchestrator from the reviewer's returned findings list, without editorial change to the findings.

## Verdict: CHANGES_REQUESTED

BLOCKER 2 · HIGH 7 · MEDIUM 10 · LOW 6

## BLOCKER

- **B-1** (refusal code `Policy`, feature-delta ~L17171; per-flow policy ~L17700–17707): the host never
  refuses `127.0.0.0/8` or `169.254.0.0/16`. Any unprivileged guest process can open a vsock to (2, 1240)
  and request `TcpConnect` or a datagram to `127.0.0.1:port`; the owner then connects to host-loopback
  services from inside `serve`. The bridge topology dropped such packets as martians, so the exposure is new.
- **B-2** (ADR-0152 D1–2; ADR-0163 final bullet; D15 table ~L16735; G-V8): the whole guest prefix is local
  via the `lo` route. While no intake listener is bound, marked leg-S traffic and TCP probes to
  `workload_addr:port` fall through to any host listener on `0.0.0.0:port`. A remote mesh client can reach
  host sshd or the API, and probes report healthy while the guest is down. The "refused by the host kernel"
  claims are false.

## HIGH

- **H-1** (TcpConnect step 3 ~L17283; TcpAccept step 3 ~L17337): the acceptor installs its vsock socket
  before reading the 16-byte request; a route miss drops bytes, so the request is lost. The spike reads first,
  then installs (`spike-scratch/netns-density-295-v11-vip-v14/vfwd/src/owner.rs:1427-1482`,
  `bpf/src/main.rs:440-443`).
- **H-2** (guest adaptation contract; UDP slot release): the guest intake model is unpinned. Missing: reserved
  guest ports `W:15001` and `W:20000+slot` (application binds fail `EADDRINUSE`); the guest's own intake
  appearing in the D23 listener map; slot identity (per socket + destination in the proven mechanism; the
  spike's release path leaks slots); pool sizes and exhaustion outcome; all of these in the D15 table.
- **H-3** (ADR-0164 "mesh resolution unaffected"; V-6): `MtlsResolve` performs no VIP translation
  (`crates/overdrive-control-plane/src/mtls_resolve_adapter.rs:90-92,143-161`). Guest TCP to a mesh
  service's VIP is classified `NonMesh` and sent cleartext; `connect4` then rewrites it to an intercepted
  backend, rule 7 sends it to leg-C, and it fails. P-32's backend was not intercepted, so the evidence does
  not cover this case.
- **H-4** (`sweep_stale` ~L17605/17647; ownership table ~L17895): the shared `local` route survives SIGKILL,
  but `sweep_stale` only checks absence. The next boot refuses or is undefined; no converge-on-boot.
- **H-5** (ADR-0160 L39–52): host sockets toward remote destinations get no `SO_LINGER{1,0}`. On owner death
  the remote peer sees a clean FIN on a truncated stream, contradicting ADR-0160's "both ends observe failure".
- **H-6** (G-V1, G-V2, G-V4, G-V5, G-V8): missing design.md-required fields — existing evidence, affected
  state, explicitly unaffected states, ordering/budget.
- **H-7** (validation plan ~L18228/18241): V-7 (pinned 6.18) blocks only DELIVER although every proven fact
  ran on 7.0.0-29; V-10, V-12 and V-15 classification also needs reclassifying or justification.
  **User ruling 2026-10-06: ignore kernel versioning — the V-7 part of H-7 is withdrawn.** V-10/V-12/V-15
  justification remains.

## MEDIUM

- **M-1**: `sock_ops` attaches to the whole shared `serve` cgroup; the spike used an owner-only cgroup and
  identified intake children by port alone.
- **M-2**: a failed unframe attach "never quiesces forwarding", yet the audit/recovery path would treat it as
  damage; the failure record drops its cause; V-17 omits xfrm paths and interfaces owned by other software.
- **M-3**: ADR-0149 combines D5 with D5a; ADR-0158 combines D18 with D18a.
- **M-4**: Revalidate ADRs carry no amendment pointers; roadmap list lacks steps for removing nft rules 2–3 and
  `outbound_sources`, moving `managed_guest_ips` to `start_alloc`, and the `local` route.
- **M-5**: interface index is a raw `NonZeroU32` (no newtype); ports, uid, gid, mode are raw.
- **M-6**: the ≤ 2 ms listen-state lag oracle has no load profile and is not measured at density.
- **M-7**: flows admitted without a live control session; TCP probes pass while forwarding is quiesced;
  activation not serialized with `ListenState` handling.
- **M-8**: guest UDP never consults `MtlsResolve`; the design does not say whether that matches today.
- **M-9**: a CID-collision retry can be handed the same CID again.
- **M-10**: boot-time cost of the 65,536-cell pool is unmeasured.

## LOW

- **L-1**: #93 and #308–#312 not verified with `gh issue view --comments`.
- **L-2**: the delta still instructs removing two stale ADR files that are already gone.
- **L-3**: one-live-session-per-CID checks are not pinned as atomic claims.
- **L-4**: mismatched datagram request pairs have no typed outcome.
- **L-5**: two inherited outbound behaviours missing from the D15 table: `connect()` succeeds before the
  destination is reached; the host connect is capped at 3 s.
- **L-6**: `drained()` does not say it covers the parking-cell hop.

## Passed

ADRs contain no signatures or test specifications; interface contracts, wire formats, typed errors and Sim
counterparts are pinned in the feature delta; everything runs through `serve` + `deploy`; no derived state is
persisted; impact-matrix counts add up (30); every deferral cites an issue.
