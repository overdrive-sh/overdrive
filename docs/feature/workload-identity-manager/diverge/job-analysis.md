# Job Analysis — workload-identity-manager (GH #35 · roadmap step 2.13)

**Wave**: DIVERGE (Phase 1 of 4) · **Agent**: Flux (nw-diverger) · **Date**: 2026-06-08

> Scope note: this is a **brownfield** feature — a net-new subsystem (`IdentityMgr` +
> `SvidLifecycle`) consuming the already-shipped `Ca` port trait (#28, ADR-0063,
> `crates/overdrive-core/src/traits/ca.rs`). The capability is settled by #35; what
> DIVERGE explores is the subsystem's **internal architecture**. JTBD here exists to
> name the job precisely so options trace to it — not to re-discover an unknown problem.

---

## 1. Raw request (verbatim, GH #35 + roadmap 2.13)

> **Workload `IdentityMgr` subsystem — per-allocation SVID lifecycle + trust bundle
> store.** When an allocation reaches Running, derive its `SpiffeId`, issue an SVID
> via `Ca::issue_svid` (chain root→intermediate→leaf, exactly one URI SAN), hold
> `SvidMaterial` in a shared `Arc<IdentityMgr>`, drop on stop. `IdentityMgr` also
> holds the current CA trust bundle; exposes SVID + bundle through `Arc<IdentityMgr>`
> for sockops/gateway/telemetry consumers. Every issuance writes an
> `issued_certificates` observation row surfaced via `alloc status` (#215 O05);
> exported leaf chain verifies under `openssl verify` exit 0 (#215 E03); no silent
> issuance. Re-issue idempotent across reconciler restarts. **Rotation DEFERRED to
> #40** (a `cert_rotation` workflow) — the near-expiry branch emits NO synchronous
> re-issue. Owns SPIFFE URI assignment. Shared across sockops/gateway/telemetry;
> unified with ACME certs in 4.7. Convergence proven via
> `assert_eventually!("running allocs hold a valid SVID")`.

---

## 2. Job extraction — abstraction-layer navigation

The request *names a solution* (`IdentityMgr`, a reconciler, an `Arc`). Per the JTBD
trap, those are guesses at a mechanism. To find the job, strip the named mechanism and
ask "why?" up the layers.

| Layer | Question → answer | Job at this layer |
|---|---|---|
| **Tactical** | *How do we hold the SVID?* → "an `Arc<IdentityMgr>` with a map" | "Keep a `SvidMaterial` in a shared struct" — **a solution, not a job.** |
| **Operational** | *Why does this subsystem exist?* → "so the dataplane can present an identity for a flow" | "Make each running workload's identity reachable by the kernel-side mTLS layer." |
| **Strategic** | *Why must identity be reachable?* → "because every packet must carry forgery-proof workload identity (design principle 3), and identity is useless if minted-but-not-held" | "Ensure identity *exists, is current, and is readable* for exactly the set of workloads that are running right now." |
| **Physical** | *What is the irreducible function?* → strip the CA, the map, the reconciler | **Bind the lifetime of a credential to the lifetime of the thing it identifies, and make the live credential readable by whoever must present it.** |

**5-Why chain (condensed):**

1. *Why an `IdentityMgr`?* → To hold each workload's SVID in memory.
2. *Why hold it?* → So sockops/gateway/telemetry can read it without re-issuing per use.
3. *Why must they read it?* → To present it in the mTLS handshake / tag the flow / stamp telemetry — the consumers of design principle 3.
4. *Why bind it to "running"?* → A credential for a workload that no longer runs is dead weight and a leak risk; a running workload with no credential is unidentifiable on the wire (it fails the handshake → it has no reachability).
5. *Why must the platform own this lifecycle (not the workload)?* → Overdrive is **sidecarless** (whitepaper §7): there is no in-pod agent to fetch/hold/drop a credential. The control plane is the only actor that knows when an allocation starts and stops, so the credential's lifecycle can ONLY be driven from the allocation lifecycle the platform already owns.

**Extracted job (physical/strategic):**

> *Bind a live, chain-verifiable cryptographic credential to the exact set of
> currently-running workloads — issued when the workload starts, readable by the
> dataplane consumers that must present it, and dropped when the workload stops —
> with no in-workload agent and no manual plumbing.*

This is at the **physical/strategic boundary** (irreducible function = lifetime-binding
+ readability), not tactical (it names no struct, no map, no `Arc`). **Gate G1
abstraction-level check: PASS.**

---

## 3. The job verdict — J-SEC-001 consumer surface, or a new job?

**This is the central Phase-1 decision the dispatch demanded an explicit verdict on.**

### The two candidate framings

- **Framing A — extend J-SEC-001.** #35 is the *consumer surface* of the already-validated
  job "give every workload a forgery-proof identity the platform mints itself." The CA
  *mints*; #35 is *the same job, one activity further along the value chain* (hold + read
  + drop). Add consumer-surface ODI outcomes under J-SEC-001.

- **Framing B — mint a new job (e.g. J-SEC-002).** #35 is a *distinct* job: "every running
  workload holds a live, readable identity the dataplane can present." The actor's progress
  is different (availability/lifecycle of identity, not its existence/forgery-resistance);
  the failure mode is different (a workload runs with a stale/absent/leaked credential, vs
  a forgeable one).

### The decision: **Framing B — mint J-SEC-002.** A new, distinct job.

**Justification (three independent reasons, weighed against the dispatch's caution not to
manufacture novelty):**

1. **Different job statement at the JTBD level — different progress, different failure
   mode.** J-SEC-001's progress is *"identity is forgery-proof by construction and the
   platform mints it itself (no external PKI)."* Its failure mode is *a forgeable identity
   / having to operate SPIRE+Vault.* #35's progress is *"the identity that exists is
   live, held, readable, and lifecycle-bound to the running workload."* Its failure mode
   is *a running workload that is unidentifiable on the wire (no held SVID), a stopped
   workload whose credential leaks (not dropped), or a consumer that cannot read the
   credential it must present.* A CA can be perfect (J-SEC-001 fully satisfied) and #35's
   job entirely unmet — the SVID is mintable but never held, never readable, never
   dropped. The two jobs are **independently satisfiable and independently failable**,
   which is the JTBD test for "distinct job" (vs "same job, finer granularity").

2. **Different actor-circumstance.** J-SEC-001's circumstance is *boot / standing up a
   trust hierarchy* ("when I run a workload and have no external PKI"). #35's circumstance
   is *steady-state operation* ("when a workload transitions Running ↔ Stopped and a
   dataplane consumer needs to present its identity *right now*"). The trigger is the
   **allocation lifecycle transition**, not the **absence of a CA**. Different trigger →
   different job (JTBD: a job is progress in a *particular circumstance*).

3. **The roadmap and whitepaper already treat them as distinct primitives.** Step 2.6 =
   "Built-in CA" (J-SEC-001, #28). Step 2.13 = "Workload `IdentityMgr` subsystem" (#35) —
   a *separate roadmap line item, a separate issue, a separate phase-position*, with #35
   `Depends on #28`. Whitepaper §5/§8/§11 give `IdentityMgr` its own architectural identity
   (`Arc<IdentityMgr>` shared across sockops/gateway/telemetry, line 1227). The SSOT
   structure itself encodes the boundary; folding #35 under J-SEC-001 would make the jobs
   register *less* faithful to how the platform is actually decomposed.

**Counter-argument considered (and why it loses):** "Both are about workload identity;
one job keeps the register lean." True that both touch identity — but *lean* is not the
JTBD axis; *distinct progress in a distinct circumstance* is. The precedent the dispatch
cites (unconnected-udp-sendmsg4 elevated under existing J-OPS-004 rather than minting a
new job, 2026-06-05) is the **opposite shape**: that was the *same* reachability job
(operator trusts the wire) reached through one more protocol idiom — *same progress, same
failure mode, finer granularity*. Here the progress and failure mode genuinely differ
(forgery-resistance vs liveness/availability/lifecycle-binding). The udp precedent says
"don't fragment one job per idiom"; it does **not** say "collapse two distinct jobs."
Applying it here would over-correct.

**Therefore: J-SEC-002 is minted**, with a `relates_to: J-SEC-001` link (consumer of the
CA primitive) so traceability is preserved. The SSOT changelog records this decision and
its justification.

---

## 4. Job statements (functional + emotional + social)

**J-SEC-002 — "Keep every running workload holding a live, readable identity the
dataplane can present — and nothing held for a workload that has stopped."**

**Job statement (JTBD format):**

> *When* a workload I run transitions into (or out of) the Running state on the platform,
> *I want* the platform to bind a live, chain-verifiable SVID to that exact allocation —
> issued the moment it starts, held where my sockops/gateway/telemetry layers can read it
> the instant they need to present it, and dropped the moment the workload stops —
> *so I can* rely on "every packet carries cryptographic workload identity" being
> *operationally true for the running set*, not merely *mintable in principle*, with no
> in-workload agent to install and no credential outliving the workload it named.

| Dimension | Statement |
|---|---|
| **Functional** | Maintain an in-memory map `running-allocation → current SvidMaterial` that (a) gains an entry, chain-verifiable to the root, when an allocation reaches Running; (b) is readable by dataplane consumers (sockops/gateway/telemetry) via a shared handle without re-issuing per read; (c) loses the entry when the allocation stops; and (d) also exposes the current CA trust bundle relying parties verify against. Re-issuance is idempotent across control-plane restarts. |
| **Emotional** | Trust that the running set is *consistently* identity-bearing — no race where a workload is serving traffic before its identity is held, and no quiet leak where a stopped workload's private key lingers in memory. "Identity availability is a converged invariant, not a hope." |
| **Social** | Be able to tell a security reviewer "the live credential set is bounded to the running workload set by a convergence loop with a DST-proven `assert_eventually!` invariant — identity availability is mechanically checked, not asserted" — and have that be true. |

**Disruption check** — is there a higher-level job that makes this whole job unnecessary?
The higher job is "every packet carries forgery-proof identity" (design principle 3).
That job is *not* eliminated by anything in Phase 2 — it is the reason #35 exists. A
disruption *would* be "workloads authenticate via an external IdP / SPIRE Workload API"
— explicitly a non-goal (sidecarless, no external PKI; whitepaper §7/§8). So the job
stands; no higher job dissolves it.

---

## 5. ODI outcome statements (measurable success criteria)

ODI format: `[Direction] + [Metric] + [Object] + [Context]`. All anchor to J-SEC-002.
Forbidden words (easy/reliable/manage/…) and embedded solutions avoided.

| # | Outcome statement | Rationale / anchor |
|---|---|---|
| **O1** | Minimize the likelihood that a workload is in the Running state without a held, chain-verifiable SVID for its allocation. | The core liveness invariant — the `assert_eventually!("running allocs hold a valid SVID")` convergence target. This is the North-Star outcome: identity availability for the running set. |
| **O2** | Minimize the likelihood that `SvidMaterial` (including the leaf private key) is held in memory for an allocation that is no longer Running. | The drop-on-stop / leak-resistance outcome. A stopped workload's private key lingering is both dead weight and an attack-surface expansion (ADR-0063 redacts the key from logs; #35 must drop it from memory). |
| **O3** | Minimize the time it takes a dataplane consumer (sockops/gateway/telemetry) to read the current SVID and trust bundle for an allocation it must present identity for. | The read-surface outcome — the consumer-facing latency the whole subsystem exists to serve (whitepaper §7 in-process `Arc<IdentityMgr>`, "no gRPC, no IPC"). The mTLS handshake is on the connection hot path. |
| **O4** | Minimize the likelihood that a control-plane restart leaves a running workload with no held SVID, or re-issues a redundant SVID for one that already has a valid held credential. | The idempotent-across-restart outcome (#35's "re-issue idempotent across reconciler restarts"). Persist *inputs* (issuance facts), recompute held state on boot — per development.md "persist inputs, not derived state." |
| **O5** | Minimize the likelihood that an SVID is handed to a consumer without a corresponding `issued_certificates` audit row being observable. | The no-silent-issuance outcome (#215 O05, ADR-0063 D6). Issuance and its audit are observable together or not at all — the existing `ca_issuance::issue_and_audit` already binds these. |
| **O6** | Minimize the number of additional concurrency/storage mechanisms the subsystem introduces beyond those the reconciler runtime + `Ca` port + ObservationStore already provide. | The simplicity/cost outcome — the same shape as J-PLAT-005's O6. The subsystem should reuse the action-shim, the View store, and the observation row, not invent a parallel persistence or a bespoke async path. Directly informs the taste evaluation (Subtraction / Concept-Count). |

### Opportunity-candidate read (which outcomes are most under-served *today*)

Importance/Satisfaction here are **engineering-judgment estimates** (Phase-2 platform,
no end-user survey — same precedent as the jobs.yaml header: distilled from the
whitepaper, not interviews). Satisfaction is measured against *what exists today* (the CA
mints, but nothing holds/reads/drops).

| Outcome | Importance | Satisfaction (today) | Opportunity score | Status |
|---|---|---|---|---|
| O1 (running ⇒ held SVID) | 9.5 | 1.0 (no holder exists) | 18.0 | **Under-served** |
| O2 (stopped ⇒ dropped) | 8.5 | 1.0 (no holder ⇒ no drop) | 16.0 | **Under-served** |
| O3 (consumer read latency) | 9.0 | 2.0 (no read surface yet) | 16.0 | **Under-served** |
| O4 (restart idempotence) | 8.0 | 3.0 (`ca_issuance` re-issue exists; no held-state recompute) | 13.0 | **Under-served** |
| O5 (no silent issuance) | 8.5 | 7.0 (`issue_and_audit` already binds this) | 10.0 | Appropriately served |
| O6 (mechanism economy) | 7.5 | 5.0 (runtime/port/obs exist; subsystem could still over-build) | 10.0 | Appropriately served |

**Read:** O1, O2, O3 are the high-opportunity core — the *holding, dropping, and reading*
of identity is entirely unbuilt. O4 is moderately under-served (the re-issue *mechanism*
exists but the *held-state-recompute-on-boot* does not). O5/O6 are already
appropriately-served by the shipped CA issuance seam and the existing runtime — the
options must **reuse** them, not rebuild them. This directly shapes brainstorming: the
divergence belongs in *where identity lives, how it's read, and how its lifecycle is
wired* (O1/O2/O3/O4) — not in re-litigating issuance/audit (O5) or the runtime (O6).

---

## 6. Gate G1 evaluation

- [x] **Job at strategic/physical level** — the extracted job is the irreducible
      "bind credential lifetime to workload lifetime + make it readable"; navigation table
      shows tactical→physical. No struct/map/`Arc` in the job statement. **PASS.**
- [x] **No feature references in the job statement** — §4's statement names no
      `IdentityMgr`, no reconciler, no `Action`. **PASS.**
- [x] **≥3 ODI outcome statements** — six produced (O1–O6), each in ODI format, no
      forbidden words, no embedded solutions, no compound `and`/`or`. **PASS.**
- [x] **Job verdict explicit** — §3: mint **J-SEC-002** (distinct job), `relates_to`
      J-SEC-001, with three-reason justification and the counter-argument addressed.
      **PASS.**

**Phase 1 gate: PASS.** Ready for Phase 2 (competitive research).
