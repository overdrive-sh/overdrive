# RCA — inbound TPROXY rule intercepts no real workload traffic (`virt` wired to the agent's own leg-C port)

**Author:** Rex (Root Cause Analysis Specialist, Toyota 5 Whys)
**Date:** 2026-06-16
**Defect class:** Correctness / security — inbound transparent-mTLS is inert in production
**Scope:** `MtlsInterceptWorker::start_alloc`
(`crates/overdrive-worker/src/mtls_intercept_worker.rs:296-302`) and the
inbound dial path `server_dial_addr`
(`crates/overdrive-dataplane/src/mtls/inbound.rs:115-117`).
**Status:** Report **CONFIRMED** (both claims). Distinct from — and not
covered by — the in-flight `fix-mtls-intercept-fail-open` RCA.

---

## Problem statement

In `start_alloc`, the inbound nft-TPROXY rule's match key (`virt`) is
constructed from the **agent's own leg-C listener port**, not from the
server workload's logical address:

```rust
// crates/overdrive-worker/src/mtls_intercept_worker.rs:296-302
let agent_port = inbound_listener.local_addr().map(|a| a.port()).unwrap_or_default();
let virt = SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, agent_port);
let tproxy_guard = match install_inbound_tproxy(virt, agent_port) { ... };
```

`virt` and `agent_port` are the *same value*. The rule
`install_inbound_tproxy` emits is therefore:

```
ip daddr 127.0.0.1 tcp dport <agent_port> tproxy to 127.0.0.1:<agent_port>
```

`agent_port` is a random ephemeral port the **agent** chose for its own
leg-C listener (`bind("127.0.0.1:0")`). No client of the server workload
knows it or connects to it — clients connect to whatever address/port the
*workload* listens on. The rule therefore matches **no real inbound
workload connection**; inbound transparent mTLS is inert in production.

Both of the report's claims are confirmed:

1. **The rule intercepts no real traffic.** The match key is the agent's
   own listener port, not the address clients dial. Confirmed at
   `mtls_intercept_worker.rs:296-302` + the rule text at
   `mtls_intercept.rs:249-269`.
2. **A connection that *did* hit the rule would self-loop.**
   `server_dial_addr(orig_dst) = orig_dst` verbatim
   (`mtls/inbound.rs:115-117`); under TPROXY `getsockname` recovers
   `orig_dst = 127.0.0.1:<agent_port>` (= leg-C itself), so `enforce`
   would dial leg-C as the "server" — a proxy loop. This is latent (the
   rule cannot fire for real traffic), not the primary harm.

---

## 1. Root cause chain (5 Whys, multi-causal)

### Branch A — the install-time value is wrong

> **WHY 1A (symptom).** A deployed exec workload's inbound connections are
> never intercepted; the workload speaks/accepts cleartext while the
> platform reports it `Running` with "transparent mTLS installed."
> **Evidence:** rule key `daddr 127.0.0.1 dport <agent_port>`
> (`mtls_intercept_worker.rs:301` → `mtls_intercept.rs:255-263`);
> `agent_port` is the agent's leg-C ephemeral port (`:296`).

> **WHY 2A (mechanism).** `virt` is constructed *from* `agent_port`, not
> from the server workload's listening address. The TPROXY match key and
> the redirect target collapse to one value, so the only connection that
> could match is one already aimed at the agent's own listener.
> **Evidence:** `let virt = SocketAddrV4::new(LOCALHOST, agent_port)`
> (`:301`). Contrast the free function's own test, which drives it with a
> **distinct** virt — `virt = 127.0.0.5:18555`, `agent_port` ≠ that —
> (`mtls_intercept_install.rs:456-464, 562-564`). The free function is
> correctly designed for `virt ≠ agent_port`; the production *caller*
> violates that.

> **WHY 3A (data availability).** `start_alloc` has no server-workload
> listening address to use. `AllocationSpec` carries
> `{alloc, identity, command, args, resources, probe_descriptors}` —
> **no listen-addr / port field** (`traits/driver.rs:131-155`). The
> workload binds its own socket at runtime; the platform never enumerates
> it. So the value `virt` is *supposed* to be has no in-scope source, and
> the crafter reached for the nearest value that compiled — `agent_port`.

### Branch B — the dial target is the identity function

> **WHY 1B (symptom).** Even granting a hit, the agent would dial itself.
> **Evidence:** `server_dial_addr(orig_dst) = orig_dst`
> (`mtls/inbound.rs:115-117`); `orig_dst` recovered via `getsockname`
> equals the rule's daddr/dport = `127.0.0.1:<agent_port>` = leg-C.

> **WHY 2B (deferral).** `server_dial_addr` is the **named #178 deferral
> site** — the identity function until east-west service resolution lands.
> **Evidence:** `wave-decisions.md:603-620` ("the single
> production-inbound-routing site that #178 will eventually supply is
> `server_dial_addr`"); module docstring `mtls_intercept.rs:24-28`.
> This branch is *correctly deferred and documented*; it only becomes a
> self-loop because Branch A fed it a self-referential orig-dst.

### Shared root (WHY 4 / WHY 5)

> **WHY 4 (design gap).** The design specified the *model* for `virt`
> ("the server workload's logical address … the loopback addr the server
> workload listens on" — `wave-decisions.md:594-601`, D-MTLS-15 `:671`)
> but **never pinned where that value comes from at rule-install time.**
> It is the same class of missing data as the OUTBOUND peer set: both are
> east-west service-resolution facts, which is #178 (OPEN, out of v1
> scope). The design deferred the outbound half explicitly (`(3)`,
> `:603-620`) but left the inbound install-time `virt` source
> *underspecified* rather than deferred.

> **WHY 5 (process — the true root).** The two halves of the same #178 gap
> were handled **asymmetrically**. The OUTBOUND gap was handled
> *honestly*: `start_alloc` does **not** program `MTLS_REDIRECT_DEST`; the
> value is supplied only by a test-only `program_declared_peer_redirect`
> seam gated behind `#[cfg(feature = "integration-tests")]`, with the
> deferral documented and tied to #178
> (`mtls_intercept_worker.rs:37-51, 427-481`). The INBOUND gap was handled
> *dishonestly*: instead of deferring symmetrically, `start_alloc` filled
> the missing `virt` with `agent_port` and labelled it "the server
> workload's logical addr" (`:264-267, 297-301`) — a comment that
> contradicts the code. This is the CLAUDE.md "Implement to the design —
> **STOP and surface the gap**" failure mode: the design under-specified
> the value's source, and rather than returning a blocker, the
> implementation "reached for the nearest mechanism that compiles."

**Root cause:** the inbound TPROXY `virt` has no production source in v1
(it is #178 east-west service-resolution data, the same gap that defers
the outbound redirect). The outbound half was deferred behind a gated
seam; the inbound half was papered over with the agent's own port plus a
comment that misdescribes it — producing a rule that installs cleanly,
passes every test, and intercepts nothing.

---

## 2. Why the test suite and review did not catch it

This is a textbook "green suite over an inert path":

- **The free function is tested correctly, in isolation.**
  `worker_inbound_tproxy_redirect_recovers_orig_dst` and
  `worker_inbound_multi_virt_coexist_and_per_virt_teardown`
  (`mtls_intercept_install.rs:436-602`) call
  `install_inbound_tproxy(virt, agent_port)` directly with a **distinct**
  `virt` (`127.0.0.5:18555`, `127.0.0.6:18666`) and prove the rule,
  coexistence, by-handle teardown, and `getsockname` orig-dst recovery.
  All green — and all meaningful, because they supply the *correct* virt
  the production caller fails to.
- **`start_alloc`'s virt-wiring is exercised by no test.** No test drives
  a real inbound connection through `MtlsInterceptWorker::start_alloc`.
  The control-plane e2e gate `mtls_production_activation.rs` exercises
  only the **OUTBOUND** declared-peer leg (via
  `program_declared_peer_redirect`) and asserts TLS 1.3 on the
  **peer-facing leg** (`:22-29, 84-120`). The inbound rule `start_alloc`
  installs is never connected to.
- **Consequence:** the one line that collapses `virt` onto `agent_port`
  sits between a well-tested free function and a well-tested outbound e2e,
  in the one seam neither covers. This matches the recorded phase-06
  pattern ("false COMMIT-EXECUTED-PASS; do a Lima ground-truth gate run on
  phase-06 Tier-3 steps") and the note that increment-i proved the
  **outbound** composed flow — inbound production was never proven e2e.

---

## 3. Relationship to other in-flight work

- **`docs/feature/fix-mtls-intercept-fail-open/deliver/rca.md`** (untracked,
  in flight) covers a **different** defect: `start_alloc` is
  fire-and-forget and returns `()`, so intercept-install *failures* leave
  the alloc running cleartext (fail-OPEN). That RCA is about install
  *failing*; this RCA is about install *succeeding but being inert*. Both
  live in `start_alloc`; neither subsumes the other. (Verified: the
  fail-open RCA contains no mention of `virt`/`agent_port`/inertness.)
- **#178** (OPEN, "Native east-west SPIFFE-ID resolution via local
  ObservationStore") is the source of the missing data for *both* the
  outbound peer set and the inbound `virt`.

---

## 4. Recommended remediation (for user decision — not yet applied)

The honest options, strongest first. All make the inbound half **symmetric
with the already-correct outbound deferral**:

1. **Defer inbound symmetrically (recommended for v1).** Do **not** install
   the inbound TPROXY rule from production `start_alloc`. Move the
   `install_inbound_tproxy` call behind the same `#[cfg(feature =
   "integration-tests")]` seam pattern the outbound redirect uses, with the
   real `virt` supplied by the test seam — and document the deferral
   against #178 at the call site, exactly as `server_dial_addr` is. This
   removes the false comment and the inert rule; inbound transparent mTLS
   becomes an explicit, tracked v1 gap rather than a silent no-op.
2. **Thread the workload's listening address into `start_alloc`.** Add the
   server-workload logical address to `AllocationSpec` (or resolve it via
   #178) and use it as `virt`. This is the real fix but requires data v1
   does not currently produce — i.e. it *is* #178, so it is not a v1-scope
   change.

Either way: **fix the comment** (`mtls_intercept_worker.rs:264-267,
297-301`) — it currently claims `virt` is "the server workload's logical
addr" while the code assigns the agent's own port, which violates
CLAUDE.md § "No aspirational docs."

**Do not** simply add a tracking comment and leave the inert rule in place
— an installed-but-inert rule that reports success is worse than an
explicit, gated deferral, because it reads as "inbound mTLS works."

Per CLAUDE.md, any new GitHub issue or deferral language requires explicit
user approval before creation; #178 already exists and (per `gh issue view
178`) covers the east-west resolution this turns on — verify its scope with
`--comments` before citing it as the home for the inbound `virt` deferral.

---

## 5. Backward-chain validation

Reading the chain bottom-up, each link forces the one above:

- WHY 5 (asymmetric handling of the #178 gap) → WHY 4 (design left the
  inbound install-time `virt` source unspecified) → WHY 3A (no listen-addr
  in `AllocationSpec`) → WHY 2A (`virt` built from `agent_port`) → WHY 1A
  (rule matches no real traffic). ✓ each step is necessary and sufficient
  for the next.
- Branch B (self-loop via verbatim `server_dial_addr`) is real but
  *gated by* Branch A — it can only manifest if a connection reaches the
  inert rule, which real traffic never does. Fixing Branch A (a real virt,
  or an honest deferral) dissolves Branch B's reachability; #178 then
  closes `server_dial_addr` itself. ✓ the two branches share the root and
  the fix.
- Falsification check: if WHY 3A were false (i.e. `AllocationSpec` *did*
  carry the listen addr), the fix would be a one-line swap and the design
  gap (WHY 4) would not exist. It does not carry it (`traits/driver.rs:131`),
  so the gap is real. ✓

**Verdict:** the report is correct. The proximate defect is the
`virt = agent_port` wiring at `mtls_intercept_worker.rs:301`; the root
cause is an under-specified, asymmetrically-handled #178 data gap that was
papered over instead of deferred. Recommended remediation is to make the
inbound half symmetric with the outbound deferral and correct the
contradicting comment, pending user direction.
