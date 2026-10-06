# Guest-flow owner — Quint specification

This directory holds the formal model of the guest-flow owner protocol of the
vsock attachment replacement (feature `netns-density-295`, ADR-0145 to
ADR-0168). The specification is a design artifact and the DISTILL conformance
oracle (ADR-0168, decision QUINT): DISTILL drives the implementation against
these modules through quint-connect. The models encode the design text in
`docs/feature/netns-density-295/feature-delta.md` § *vsock Attachment
Replacement DESIGN*. They do not show that any implementation conforms.

Model-check results: `docs/feature/netns-density-295/spike/quint-owner-findings.md`
(first run), `quint-owner-findings-r2.md` (second) and `quint-owner-findings-r3.md`
(this revision: U-5 confirmed with K-A4, D16-PREF, D8a-READMIT,
and the accepted bounded exposures D8a-PROBE / D8a-FLUSH of 2026-10-06).

## Layout

| Path | Content |
|---|---|
| `owner_flows.qnt` | Module A: control session per CID, guest-opened flow admission, host-opened `TcpAccept` pairing, allocation lifecycle, quiescence |
| `udp_slots.qnt` | Module B: guest UDP slot pool, datagram associations, slot release |
| `cid_lease.qnt` | Module C: lease offsets → CID, next-fit, per-workload exclusion of failed offsets, `serve` restart |
| `steering.qnt` | Module D: boot steps, steering rules and shared route, intake listeners and their `intake_listeners` elements, all-or-nothing activation, where a connection lands |
| `hazard/*.qnt` | Instances of the same modules with one design rule switched off (`BUG_*`, the teeth) or an environment fault switched on (`ENV_*`) |
| `checks.toml` | Every check: spec, instance, property, backend, bound, expected verdict, CI flag |
| `evidence/` | Written only by `cargo xtask quint check --subsystem guest-flow-owner --record` (summary of the last recorded run, spec hashes, counterexample traces); checked by `cargo xtask quint verify-evidence`. Earlier runs live in git history. |

## What is modelled

| Module | State | Actions |
|---|---|---|
| A | Owner CID table (`Absent / Provisioned / Active / Retiring`), generation of the allocation leasing the CID, VM up/down, quiescence latch, the `ClaimSet` claim, control connections, guest-opened flows (queued → accepted → request read → installed → connecting → paired / closed), host-opened flows on both sides, control messages per direction | provision, VM start / stop, activate, teardown, release, quiesce, restore; session open / accept / loss; flow accept, read, install, non-blocking connect, completion, discard; `TcpAccept` open, guest accept, `Paired` / `Refused`, `Abort`, guest cleanup |
| B | Socket liveness, pending `sock_release` reports, slots (owner, association id, routed, parked frames), host associations, control messages | claim, send, associate, pair / refuse / abort, late `Paired`, pairing deadline, socket close (report or lost report), report handling, audit, idle release |
| C | Lease per offset, next-fit cursor, per-workload state and exclusion set | assign (prefer unexcluded free offsets, fall back to excluded free ones, refuse only when none is free), launch (clash on a foreign CID, or another pre-READY exit), retire and release, stop, redeploy, `serve` restart |
| D | `serve` up / boot step, steering rules, shared route, `intake_listeners` elements, allocation state, session, quiescence, guest and owner listen state, per-port listener state, activation rollback flag | boot steps (route step refuses without rules), crash, restart, foreign flush, rules repair (inside quiescence); per-allocation events; listener bring-up and take-down; revoke failure; re-admission of a resetting listener wanted again; activation begin / Ok / Err |

**Abstraction.** One CID (or one workload address) reused by two allocations;
two control connections; two guest-opened flows and one host-opened flow; two
UDP slots, two sockets, one or two destinations, two or three association
ids; three lease offsets; two guest ports, one declared. Time is abstract:
every deadline is a nondeterministic action. A connection's landing place is a
state predicate. Per-allocation serialization (M-7) is modelled as "no
per-allocation event while an item is in progress" (`rest`).

## Assumptions and what discharges them

| ID | Assumption | Discharged by |
|---|---|---|
| K-A1 | Peer-CID attribution is authentic (vhost stamps the device CID) | V-10 (P-17) |
| K-A2 | A released vhost device's connections, accept-queue entries included, are reset before the CID is reassigned (`flows_envStaleConn` drops it) | V-22 |
| K-A3 | A socket installed before its request is read hands the request to the verdict, which drops it | P-23; R5-9 |
| K-A4 | The host's close of `V_h` of an aborted or session-lost `TcpAccept` reliably tears down the guest-side connection: an accepted `V_g` observes the reset; a `V_g` still in the guest's accept queue is either removed from it or, if the guest accepts it after the close, observed as ended (the reset may race the guest's accept; it still arrives). U-5's discard rule rests on it. `flows_envGuestMissesHostClose` drops it | V-24 (feature-delta A-28) |
| A-FID | Host flow ids are not reused within one pairing window (the 2^31 id space) | design (`FlowId`) |
| K-B1 | Closing `T_g` / `Q_g` is observed by the host as the association's end | P-22 / V-2 (bounded) |
| K-B2 | `sock_release` reports the socket, or the audit finds orphan slots | R5-14 |
| K-B3 | Emptying or recreating a slot's cells discards every parked frame | R5-14 (partly) |
| K-C1 | A duplicate CID is refused at device creation; a CID is free right after VMM exit | P-16 |
| K-C2 | Foreign vhost CIDs are fixed while the model runs | environment; none |
| K-D1 | With the route and no rules, a connection reaches a wildcard host listener unless a listener bound to `workload_addr:p` exists | V-19 |
| K-D2 | With the rules: a marked connection outside `intake_listeners` is reset; other prefix traffic not diverted to leg-C is dropped / rejected | V-19 |
| K-D3 | Rules, route and elements persist across a `serve` exit; nft batches are atomic. Removal by other software is modelled (`ENV_FOREIGN_FLUSH`), not assumed away; the resulting exposure is accepted design behaviour and is checked to be bounded (`ExposureEnds`) | V-19 (persistence, U-4); V-23 (other software) |
| K-D4 | Without the route, no guest-prefix address is locally delivered | V-19 |
| K-D5 | Closing a listener resets the children in its accept queue | V-13 |

Not modelled: D25-BIND (a kernel bind fact; R5-15), D26 / V-21, V-20, framing
and unframe (D5 / D5a), VIP resolution (D24 / D24a), half-close drain, the
listen-state lag bound, D15-R3 deadlines, and the beacon beyond its claim
discipline.

## How to run

From the workspace root on the host (the Lima VM provisions Quint 0.32.0,
Apalache 0.56.1 and JDK 21 on `PATH`):

```bash
cargo xtask lima run -- cargo xtask quint typecheck
cargo xtask lima run -- cargo xtask quint check --ci --subsystem guest-flow-owner   # the CI subset
cargo xtask lima run -- cargo xtask quint check --subsystem guest-flow-owner        # every check
cargo xtask lima run -- cargo xtask quint check --subsystem guest-flow-owner --name <check>
cargo xtask lima run -- cargo xtask quint check --subsystem guest-flow-owner --record  # record evidence
cargo xtask quint verify-evidence --subsystem guest-flow-owner                          # evidence matches specs?
```

The runner (`xtask/src/quint.rs`) reads `checks.toml`, runs each check through
`quint verify`, and fails unless every outcome matches `expect`; a tool error or
timeout never counts as a result. It owns timeouts, retries, process cleanup and
parallelism. Full logs land in `target/quint/guest-flow-owner/<check>/`; only
`--record` writes `evidence/`.
