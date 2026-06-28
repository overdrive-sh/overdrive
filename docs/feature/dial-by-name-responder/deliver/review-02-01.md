# Adversarial review — step 02-01 (`DnsResponder` IP_PKTINFO host adapter + single `FrontendAddrAllocator` wired into `run_server`)

- **Artifact:** roadmap step `02-01` — *DnsResponder host adapter (IP_PKTINFO socket loop, wildcard→per-addr fallback) + FrontendAddrAllocator + re-keyed MtlsResolve wired into run_server*
- **Commit:** `c6f4ab2a`
- **Reviewer:** `nw-software-crafter-reviewer` (Opus, adversarial "different-fox" posture). All three blocking findings independently re-traced against source/tests/log by a separate verifier subagent (not trusted from the lead reviewer's read).
- **Verdict:** **NEEDS_REVISION** — 3 blocking (D1 no mutation gate, D2 `run_server` refusal/reason-mapping untested, D3 converge-tick is aspirational-docs + unimplemented + untested), 4 non-blocking (N1 `bind_frontend` production-dead, N2 silent-deaf-responder degenerate, N3 frontend-port assumption unverified, N4 progress-tracking lag).

> The **production substrate is sound and genuinely alive** — the single-owner chain (writer 01-05 → readers 01-03/02-00 → composition 02-01) closes on the real path, the source-pin is proven at the wire level, the `by_frontend` pure-reader fail-closes on unbound `<job>`s, and the typed `DnsResponderError`→`DnsResponderBoot` surface is clean. This is **not** a dead-mechanism rejection. The three blocking issues are the same *"green suite over an unverified criterion"* class that blocked 02-00: new mutable in-process logic shipped without a mutation gate (D1), a roadmap-named criterion (BIND-03's `run_server` half) downgraded to a weaker assertion that leaves the composition-root logic untested (D2), and rustdoc + roadmap describing a converge tick the code does not implement (D3, the `development.md` § Documentation "no aspirational docs" violation).

---

## Per-criterion verdicts

| Criterion | Verdict | Defending test | Litmus |
|---|---|---|---|
| **S-DBN-BIND-01** (wildcard `0.0.0.0:53` + IP_PKTINFO; answer `F`; source-pin) | **MET** (wire-level proxy) | `wildcard_bind_answers_frontend_and_source_pins_reply` (`dns_responder_bind.rs:130`) | Asserts `reply_src.ip() == queried dst (127.0.0.2)` (`:193`) and `answered == vec![f]` (`:207`). The `reply_src` check is the *exact* mechanism a connected `getaddrinfo` socket enforces — it does **not** share `dig`'s blind spot (a missing source-pin flips it RED). Deviates from the literal "getent from ≥2 netns" (deferred to 02-02 S-DBN-WS); see N-note. |
| **S-DBN-BIND-02** (per-addr fallback **+ converge tick**) | **PARTIAL → NOT-MET** | `per_gateway_addr_fallback_binds_when_wildcard_is_held` (`:219`) | Fallback-fires half MET (forces wildcard `EADDRINUSE`, asserts the per-gateway bind is `Ok`). **Converge half (add-if-missing / drop-if-absent) is unimplemented and unasserted** — see **D3**. |
| **S-DBN-BIND-03** (Earned-Trust refuse-boot, both legs) | **PARTIAL → NOT-MET** | `probe_refuses_when_no_bindable_port` (`:265`), `probe_refuses_when_store_unreadable_at_listseed` (`:324`) | `probe()`-level half MET: both assert the `DnsResponderError` variant via `matches!` (Bind `:311`, ListSeed `:343`). **The `run_server`-refusal + reason-mapping half the criterion explicitly names is untested** — see **D2**. |
| **Pinned surface** (`DnsResponder::{new,probe,serve}`, typed `DnsResponderError`, no `Internal(String)`) | **MET** | n/a (structural) | Surface matches the pin exactly (`responder.rs:201-313`); `DnsResponderError` has Bind/ListSeed/Probe/Socket, no `Internal` (`:103-147`); `ControlPlaneError::DnsResponderBoot(#[from] …)` (`error.rs:694`) — a proper typed pass-through, not a flatten. **Praise P1/P3.** |
| **Production wiring** (ONE `FrontendAddrAllocator` injected into BOTH readers, DDN-2) | **MET** | composition-root trace | `run_server` builds one allocator (`lib.rs:1907`), injects the same `Arc`-shared handle into the re-keyed `MtlsResolve` (`:1995`), the boot rebuild (`:2178`), `AppState` (`:2052`), and the `DnsResponder` (`:2207`). The 01-05 writer (`handlers.rs:353` + `boot_rebuild.rs`) populates it — readers are **fed**, not a dead mechanism. **Praise P4.** |
| **Mutation gate ≥80%** (responder.rs + lib.rs composition) | **NOT-MET** | — | No MUTATION phase, no `mutants-02-01.md`. See **D1**. |

---

## Blocking issues

### D1 — `issue (blocking, process)`: no mutation gate ran for 02-01, despite new mutable in-process logic

The execution log records `PREPARE → RED_ACCEPTANCE → RED_UNIT → GREEN → COMMIT` for `sid=02-01` and **no `MUTATION` phase**; there is **no `mutants-02-01.md` evidence file** (only `mutants-02-00.md` exists). The roadmap's own last 02-01 criterion mandates it:

> "Mutation gate: a per-step diff-scoped run … `--file …/responder.rs` (and the lib.rs composition diff) — achieves kill-rate >= 80%, targeting the DnsResponderError variant → refusal-reason mapping … and the run_server construct+probe+spawn."

The commit added **mutation-testable, non-Tier-3 in-process logic** that this gate exists to defend — and the crafter *knew it*: the `RED_UNIT` log line itself says *"by_frontend pure-reader projection unit test (mtls_resolve_adapter.rs) — the new below-port mutable logic the Tier-3 ATs do not isolate."* The new mutable surfaces are:

- `lib.rs:2210-2223` — the 4-arm `DnsResponderError` → `dns.responder.{bind,listseed,probe,socket}` reason match + the `return Err(ControlPlaneError::DnsResponderBoot(source))` refusal (`:2231`).
- `mtls_resolve_adapter.rs:425-466` — `project_by_frontend` / `project_row_by_frontend`, the **security-critical fail-closed feeder** for `by_frontend` (the per-service `retain` evict, the `job_of` guard, the `snapshot.get(&job)` *withhold-if-unbound* branch, the `(F, backend.port, Tcp)` key build).

This is the **exact defect that blocked 02-00 (its D3)** — and 02-00 cleared it with an out-of-band run recorded in `mutants-02-00.md` (90.9%→100% diff-scoped). 02-01 repeats the omission.

**Required:** run the per-step diff-scoped mutation gate over **both** `responder.rs` *and* `mtls_resolve_adapter.rs` (the roadmap `--file` list names only `responder.rs` — it predates the 02-01 decision to move the `by_frontend` feeder into the adapter as a pure reader; the adapter projection is *new mutable logic this step introduced* and must be in scope). Record kill-rate ≥80% in `mutants-02-01.md`. Per the project's no-Tier-2-backstop note, the irreducibly-Tier-3 socket arms (IP_PKTINFO/ipi_spec_dst/fallback re-derive) are not in-process-killable and are correctly excluded — but the reason-mapping, the refusal return, and the projection are all in-process.

### D2 — `issue (blocking)`: the `run_server` boot-refusal + reason-mapping (BIND-03's composition-root half) has no killing test

S-DBN-BIND-03 reads: *"probe returns Err(Bind), **`run_server_with_obs_and_driver` REFUSES boot (returns an error, process exits non-zero), and a structured `health.startup.refused` event names the bind failure**."* The shipped tests downgrade this to a **direct `DnsResponder::probe()` call asserting the `DnsResponderError` variant** — they never boot `run_server`, so the composition-root logic the criterion names is untested:

- `grep` across all of `crates/overdrive-control-plane/tests`: `dns.responder.probe` and `dns.responder.socket` → **0 hits**; `dns.responder.bind`/`listseed` appear only in `dns_responder_bind.rs` rustdoc + `assert!` *message* strings (never asserted *values*); `DnsResponderBoot` → **0 hits anywhere in tests**.
- `dns_responder_bind.rs` contains **no `run_server` call** (only its module rustdoc, `:25`). The 18 `run_server(...)` test call-sites elsewhere are unrelated (job-stop, submit, backend-discovery, CA) and never reference `dns.responder.*`.

Consequence — two mutants **survive**:
- flattening all four reason arms to one literal (`lib.rs:2212-2221`): no test reads the emitted `reason` for these, and the BIND-03 tests assert the upstream `probe()` *variant*, independent of the lib.rs mapping;
- deleting `return Err(…DnsResponderBoot…)` (`:2231`) so boot continues instead of refusing: no test boots `run_server` through the DNS probe-refusal path.

The roadmap's claim *"a mutant flattening to one reason flips BIND-03"* is **structurally false as written** — BIND-03 tests `probe()`, which never reaches the mapping. (This compounds D1: even if a mutation run were performed today, these mutants would report **MISSED** for want of a test, not be killed.)

**Required (one of):** (a) add a `run_server`-level boot-refusal test that arms a DNS bind / List-seed failure, asserts the boot returns `Err(ControlPlaneError::DnsResponderBoot(_))`, and captures the `health.startup.refused` event with the per-variant reason (the `probe_runner_boot_gate.rs` `health.startup.refused` capture harness is the precedent to mirror); **OR** (b) if a full `run_server` boot is out of reach at this slice, surface that to the orchestrator and re-scope BIND-03's composition-root clause explicitly — do not leave the criterion claimed-MET while the mapping it names is unenforced.

### D3 — `issue (blocking)`: the per-gateway-addr "converge tick" is documented and roadmap-required but unimplemented and untested (aspirational docs)

Three rustdoc sites describe a live converge loop:

- `responder.rs:35-36` — *"On the converge tick the bound set tracks the live slot set (add-if-missing / drop-if-absent — reconcilers.md Bar-1 converge)."*
- `responder.rs:273-274` — *"the converge tick adds sockets as slots appear."*
- `responder.rs:292-293` — *"On the converge tick the bound per-gateway-addr socket set tracks the live slot set (Bar-1 converge)."*

No such tick exists. `self.slots.snapshot()` is read **exactly once**, inside `bind_per_gateway_addr` at probe time (`:277`). `serve` consumes the bound set once via `std::mem::take(&mut *self.sockets.lock())` (`:301`) and never re-reads the slot snapshot; `serve_one_socket` is a per-socket `recvmsg` loop with no slot-set re-derivation. There is no `interval`/timer/diff. This is the `.claude/rules/development.md` § Documentation violation: *"No aspirational docs. Never document behaviour that is not implemented."* — and S-DBN-BIND-02 explicitly names the converge as required (*"on the converge tick a newly-assigned slot binds a new per-addr socket (add-if-missing) and a released slot drops its socket (drop-if-absent)"*), which the single BIND-02 test (asserting only *"binds without error"*) does not cover.

**Why this is more than a doc nit (couples to N2):** in the per-gateway-addr **fallback** path — the rustdoc's own stated *"appliance-image case where a wildcard :53 holder already exists"* — `probe()` binds the snapshot *as it exists at probe time* and never converges. A node that probes before any slot is assigned binds **zero** sockets (the degenerate empty-fallback the rustdoc calls *"valid"*, `:272-274`) and, with no converge, **never binds any** — a responder that "boots successfully" and is **permanently deaf** for the process lifetime. On the real appliance (where the fallback is the production path) this is a silent-degradation footgun, exactly the failure the Earned-Trust gate exists to prevent, re-introduced one layer down.

**Required (one of):** (a) implement the converge tick (a periodic re-derive of the bound per-gateway socket set from `slots.snapshot()`, add-if-missing / drop-if-absent — the reconcilers.md Bar-1 shape the rustdoc already cites) and add the BIND-02 converge assertion; **OR** (b) if the wildcard-primary design makes the converge genuinely out of scope for v1, **correct all three rustdoc sites** to describe the actual one-shot probe-time bind, **drop the converge clause from S-DBN-BIND-02**, and surface the descoped converge to the user for a tracked issue (per CLAUDE.md § "Deferrals require GitHub issues — AND user approval BEFORE creation"). Shipping rustdoc + a roadmap AC that describe behaviour the code does not have is the rejection.

---

## Non-blocking

### N1 — `suggestion (non-blocking)`: `bind_frontend` is now production-dead (only test callers)

`BackendIndex::bind_frontend` (`mtls_resolve_adapter.rs:397`, `pub fn`) has **zero production callers** after 02-01 — the production drain feeds `by_frontend` exclusively via `project_by_frontend`/`project_row_by_frontend`. Its only callers are the 02-00 acceptance tests (`mtls_resolve_rekey.rs`, `dns_name_index.rs`). It is a legitimate *test-construction seam*, but it is also a `pub` production method no production path reaches — the deletion-discipline smell (`development.md` § "Deletion discipline"). Consider either migrating the 02-00 `classify` tests to drive `by_frontend` through the production `project_*` path (so they exercise the real feeder, not a bypass), or documenting `bind_frontend` as a test-only construction seam. Not blocking — but the longer it lives as an untrodden `pub` the more the 02-00 mutation kills against it (`bind_frontend`) certify a path production no longer uses.

### N2 — `issue (non-blocking)`: the empty-fallback "valid bind of zero sockets" is a silent-deaf-responder

Folded into D3 above. `probe()` returning `Ok(())` with an empty `sockets` vec (wildcard held + empty slot snapshot) is documented as *"degenerate but valid"* (`responder.rs:272-274`), and `serve` then spawns zero tasks and answers nothing — silently, with no event. At minimum this degenerate boot should emit a structured warning (a responder that bound zero sockets is observably degraded), so it is not indistinguishable from a healthy boot in the logs. Resolved together with D3's converge decision.

### N3 — `question (non-blocking)`: `project_row_by_frontend` keys the frontend port on `backend.addr.port()` — verify it equals the port the client dials `F` at

`project_row_by_frontend` builds the key as `(frontend_ip, backend.addr.port(), Tcp)` (`:464`). The DNS answer carries only `F` (an A record, no port); the client then connects to `F:<port>` where `<port>` is whatever the client app targets, and `classify` keys on that connect port. A HIT therefore requires `backend.addr.port() == the service listener port the client dials`. In the v1 single-replica model where the workload listens on its declared port and the backend row carries that port, this holds — but it is **unverified end-to-end until 02-02's walking skeleton** (a port mismatch would silently fail-close, not error). Confirm the assumption is pinned, or that 02-02 exercises a service whose listener port differs from any incidental backend port, so the equality is actually tested rather than coincidental.

### N4 — `nitpick (non-blocking)`: `.develop-progress.json` lags the commit

`02-01` is in `pending_step_ids` (and `current_step_id: None`) despite `COMMIT EXECUTED PASS` and the landed commit `c6f4ab2a`. Reconcile the progress file (move `02-01` to `completed_step_ids`) so the tracker matches the log + git — otherwise the next wave step reads stale state.

---

## Praise

- **P1 — `praise:` typed error surface, no flatten.** `DnsResponderError` (Bind/ListSeed/Probe/Socket, `#[source]` on the io legs, no `Internal(String)`) + `ControlPlaneError::DnsResponderBoot(#[from] …)` (`error.rs:694`) is exactly the `development.md` § "Never flatten a typed error to `Internal(String)`" shape — the CLI can `matches!` the variant for the per-leg refusal reason without `Display`-grepping.
- **P2 — `praise:` the resource-leak fix is correct and well-reasoned.** `SO_RCVTIMEO`-bounded `recvmsg` + an `AtomicBool` stop flag + `stop()`-before-`abort()` on shutdown is the right way to make an otherwise-uncancellable `spawn_blocking` syscall loop cancellable; the rustdoc explains *why* `drop(iov)` doesn't work and why the scoped-block borrow shape was chosen. The teardown ordering (`stop` then `abort` as backstop) is sound.
- **P3 — `praise:` source-pin proven at the wire level, not narrated.** BIND-01 asserts `reply_src.ip() == the queried dst` — the precise property a connected `getaddrinfo` socket enforces and `dig` does not. This honours the spike litmus's *intent* even though it predates 02-02's real getent-from-netns.
- **P4 — `praise:` the single-owner chain is genuinely closed on the production path.** ONE allocator, constructed before the `MtlsResolve` move and threaded (via self-sharing clone) into the writer (01-05 `handlers.rs:353` + `boot_rebuild.rs`), both readers (`name_index`, `by_frontend`), and `AppState`. The `by_frontend` projection correctly **withholds** an unbound `<job>` (no fabricated `F`) so a premature dial fail-closes via `classify` arm 2 — and there is an in-process test for it (`by_frontend_projection_reads_the_shared_allocator_and_never_assigns`). This is the DDN-2 invariant landed for real, not theatre.

---

## Required actions to clear NEEDS_REVISION

1. **D1** — run the per-step diff-scoped mutation gate over `responder.rs` **and** `mtls_resolve_adapter.rs` (the new `project_*` feeder); record ≥80% in `mutants-02-01.md`.
2. **D2** — add a `run_server`-level boot-refusal test (or re-scope BIND-03's composition-root clause with the orchestrator) so the reason-mapping + `DnsResponderBoot` refusal are actually killed, not MISSED.
3. **D3** — either implement the converge tick + its BIND-02 assertion, **or** correct the three rustdoc sites + drop the S-DBN-BIND-02 converge clause and surface the descope for a tracked issue (user approval first). Resolve N2 in the same pass (warn on a zero-socket bind).
4. **N1/N3/N4** — advisory; address in-pass where cheap.

> Re-review gate: per the 02-00 precedent, the corrective commit's mutation evidence (`mutants-02-01.md`) and the new `run_server` refusal test are the load-bearing artifacts a re-review will check first.

---

## Resolution (2026-06-27, orchestrator)

**Verdict: NEEDS_REVISION → APPROVED.** All three blocking findings resolved; the
production substrate the review judged "genuinely alive" is unchanged. Corrective
commits: `751a1d69` (D2/D3/N1/N2/N4) and `48bb5562` (the D1 mutation-gap test).

### D1 — mutation gate: **RESOLVED, 89.3% PASS**

Evidence: `docs/feature/dial-by-name-responder/deliver/mutants-02-01.md`. The
diff-scoped gate ran over `responder.rs` **and** `mtls_resolve_adapter.rs` (the new
`project_*` feeder) **and** the `lib.rs` composition, `--features integration-tests`,
real Lima-root.

- First run (`751a1d69`): **71.4% — 8 missed, FAIL.** 5 of the 8 were a genuine
  in-process gap on the **pure** `is_addr_in_use` EADDRINUSE predicate
  (`responder.rs:485`) — untested because in production it only sees a real `bind`
  error. Commit `48bb5562` adds a direct two-case unit test (accept `AddrInUse`,
  reject `NotFound`/`PermissionDenied`) killing all five.
- Re-run (`48bb5562`): **89.3% — 25 caught / 3 missed / 4 unviable, PASS.**
- The **3 residual misses** (`responder.rs:274` real-`EADDRINUSE` fallback-dispatch
  guard ×2; `:323` `bind_per_gateway_addr` real-socket bind) are the
  **irreducibly-Tier-3 socket arms this review itself named "correctly excluded"** —
  no Tier-2/in-process backstop (DDN-4); behaviourally covered by BIND-01 (source-pin)
  + BIND-02 (fallback-fires). The in-process surface the gate defends — the
  `DnsResponderError`→reason mapping, the `DnsResponderBoot` refusal return, and the
  `project_by_frontend` fail-closed feeder — is fully caught. (No DES `MUTATION` phase
  appended — this feature's 5-phase contract records none; the `mutants-02-01.md`
  artifact is the substantiation, as `mutants-02-00.md` was for 02-00.)

### D2 — `run_server` boot-refusal + reason-mapping: **RESOLVED** (`751a1d69`)

Split across tiers and **both mutants now killed**:
- The 4-arm reason mapping moved onto `DnsResponderError::boot_refusal_reason(&self)
  -> &'static str` (the `development.md` "label enums own their string representation"
  pattern — the only sanctioned new error surface), with a Tier-1 unit test asserting
  four distinct reasons → kills the flatten mutant in-process (no longer dependent on
  a Tier-3 path the old inline `lib.rs` match bypassed).
- A `config.dns_probe_fault: Option<String>` seam **mirroring the existing
  `mtls_probe_fault`** (the only sanctioned new config surface) drives a Tier-3
  `run_server`-level test `run_server_refuses_boot_on_dns_probe_fault_with_probe_reason`
  — boots the real `EbpfDataplane` + composed mTLS worker, asserts
  `Err(ControlPlaneError::DnsResponderBoot(Probe))` **and** captures the
  `health.startup.refused` event with `reason = dns.responder.probe` → kills the
  delete-`return Err` mutant. Proven GREEN under Lima root alongside BIND-01/02/03.

### D3 — aspirational converge-tick docs: **RESOLVED by descope** (`751a1d69`)

**User-approved descope (2026-06-27).** There is no converge tick; per the
production-path analysis (the wildcard `0.0.0.0:53` bind is the appliance path —
BIND-01 proves coexistence with systemd-resolved's specific binds; the per-gateway
fallback fires only when something *else* holds the wildcard), implementing a converge
loop for the fallback now would build a mechanism no production entry point reaches
(CLAUDE.md § "Build vertical slices"). Resolution:

- All **three** `responder.rs` rustdoc sites corrected to describe the actual
  **one-shot probe-time bind** (no converge tick), each citing the deferral.
- The S-DBN-BIND-02 **converge clause is deferred to [#247]**
  (`overdrive-sh/overdrive#247`); the **fallback-fires half remains MET**
  (`per_gateway_addr_fallback_binds_when_wildcard_is_held`). The roadmap text is left
  intact as the historical plan; this § Resolution + #247 are the authoritative record
  of what shipped vs deferred.
- **N2 folded in:** a zero-socket fallback bind now emits the structured
  `dns.responder.fallback.zero_sockets` warning (`empty_fallback_binds_zero_sockets_
  and_warns_it_is_deaf`), so a deaf boot is observable rather than silent — the interim
  mitigation #247 names.

### Non-blocking

- **N1 — RESOLVED** (`751a1d69`): `bind_frontend` documented as a test-only
  construction seam (not deleted — the 02-00 acceptance tests need it).
- **N2 — RESOLVED**: folded into D3 above (zero-socket warning).
- **N3 — carried to 02-02** (advisory): the `by_frontend` key uses
  `backend.addr.port()`; confirm 02-02's walking skeleton exercises a service whose
  listener port differs from any incidental backend port so the `F:<port>` HIT
  equality is tested rather than coincidental. No 02-01 change.
- **N4 — RESOLVED** (`751a1d69`): `.develop-progress.json` reconciled (02-01 →
  `completed_step_ids`, count 7).

[#247]: https://github.com/overdrive-sh/overdrive/issues/247

---

## Re-review (independent verification of the resolution) — **APPROVED**

- **Re-reviewed at:** HEAD `b413736e` (resolution commits `751a1d69` D2/D3 + N1/N2/N4,
  `48bb5562` D1 `is_addr_in_use` mutants).
- **Posture:** the self-stamped "review resolution — APPROVED" was **not trusted** —
  each blocking fix was independently re-traced against source / tests / the issue
  tracker (the "different-fox" discipline; CLAUDE.md § "Verify unproven claims against
  the actual evidence").
- **Verdict:** **APPROVED** — all 3 blocking issues genuinely resolved; non-blocking
  dispositions accepted.

| Issue | Resolution verified | Evidence |
|---|---|---|
| **D1** mutation gate | **CLEARED** | `mutants-02-01.md` records a real diff-scoped Lima-root run extended to `responder.rs` + `mtls_resolve_adapter.rs` + `lib.rs` (per D1's required action), **89.3% ≥ 80%**. The in-process surfaces the gate defends — `boot_refusal_reason`, the `DnsResponderBoot` refusal, and the `project_by_frontend` fail-closed feeder — are **caught**. The 3 residual misses are the `responder.rs:274/323` real-`bind()` socket arms (genuinely Tier-3, behaviourally covered by BIND-01/02). |
| **D2** refusal/reason-mapping untested | **CLEARED** | The inline 4-arm match is gone; `DnsResponderError::boot_refusal_reason()` (`responder.rs:163`) is wired at `lib.rs:2251` and killed in-process by the Tier-1 `boot_refusal_reason_maps_each_variant_to_a_distinct_reason` (`responder.rs:499`, asserts 4 distinct reasons + all-distinct). The delete-`return Err` mutant is killed by the new Tier-3 `run_server_refuses_boot_on_dns_probe_fault_with_probe_reason` (`dns_responder_bind.rs:331`) which boots the real composition via the `dns_probe_fault` seam and asserts `Err(DnsResponderBoot(Probe))` **+** the captured `health.startup.refused` `reason=dns.responder.probe`. The `dns_probe_fault` seam is a faithful mirror of the established `mtls_probe_fault` (`lib.rs:822` vs `:835`) — not an invented test-shaped surface. |
| **D3** aspirational converge-tick docs | **CLEARED** | All three `responder.rs` rustdoc sites now state "**no converge tick in v1**" and cite **#247** (verified OPEN, created 2026-06-27, correctly scoped). The deferral satisfies CLAUDE.md § "Deferrals require GitHub issues — AND user approval BEFORE creation": the resolution records user approval (2026-06-27), the issue exists, and is cited at every site. N2 folded in: `dns.responder.fallback.zero_sockets` warning + `empty_fallback_binds_zero_sockets_and_warns_it_is_deaf` make a deaf boot observable. |
| N1 / N2 / N4 | **RESOLVED** | `bind_frontend` documented as a test-only seam; zero-socket warning landed; `.develop-progress.json` reconciled (02-01 → completed). |
| N3 | **Carried to 02-02** (correct disposition) | The `by_frontend` key uses `backend.addr.port()`; the `F:<port>` HIT-equality is only end-to-end-verified at 02-02's walking skeleton — keep it visible so 02-02 exercises a service whose listener port differs from any incidental backend port. |

**Advisory (non-blocking, no action required to proceed):**
- `mutants-02-01.md`'s rationale for the `responder.rs:274` guard→`false` miss is mildly
  generous (BIND-02 *would* catch it if it ran without SKIPping), but the gate clears
  ≥80% and the in-process surfaces are caught — acceptable.
- The roadmap `S-DBN-BIND-02` text still demands the converge tick (left intact as the
  "historical plan"); this § Resolution + #247 are the authoritative shipped-vs-deferred
  record. A future roadmap reader relies on finding #247 — acceptable, slightly soft.

> **Re-review reviewer:** `nw-software-crafter-reviewer` (Opus, adversarial). Step 02-01
> is cleared to proceed to **02-02** (the walking skeleton), carrying N3 forward.
