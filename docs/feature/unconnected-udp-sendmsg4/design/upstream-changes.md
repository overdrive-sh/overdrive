# Upstream Changes (back-propagation) — unconnected-udp-sendmsg4 DESIGN

> Per the nWave back-propagation contract: when a DESIGN-wave decision
> contradicts a prior-wave (DISCUSS) assumption, the correction is recorded
> here with the **verbatim** prior wording + the new wording + the rationale,
> for the product owner to review and relay. Two corrections this wave.
>
> SSOT for the corrected design: **ADR-0053 revision 2026-06-05** (D3, D4).
> Evidence base:
> `docs/research/dataplane/recvmsg4-reply-source-rewrite-and-miss-semantics-research.md`.

---

## Change A — AC reframing: wire-layer → application-sockaddr-layer (D3 / DDD-3a)

**What changed.** US-01, US-03, and KPIs K2/K5 were written in **wire-layer**
vocabulary (`tcpdump`, "left the host", "on the wire"). The research
established (Q4, [VERIFIED-PRIMARY]) that `cgroup/recvmsg4` fires inside
`udp_recvmsg()` **after** the kernel has dequeued the skb and populated the
source sockaddr from the backend's IP/UDP headers. A `tcpdump -i lo` therefore
captures the backend-sourced reply on **every** round-trip (hit OR miss),
strictly before recvmsg4 runs — so a wire-level "no backend-IP-sourced reply"
assertion is something recvmsg4 **structurally cannot deliver**. recvmsg4's
domain is the **application sockaddr** the app reads via `recvfrom`/`msg_name`.
Wire-level no-leak is an **XDP** concern (the connected/remote REVERSE_NAT
path, out of scope for this feature). Additionally (Q1, the crux), recvmsg4
**cannot deny** — the verifier restricts its return value to exactly `[1,1]`;
a program returning 0 is rejected at load time. So "drop on miss" is
impossible at any layer; the fail-safe is a source rewrite, not a drop.

**Intent is preserved** ("a misconfigured reply path fails clean and never
exposes the backend IP to the client app"). These are **layer/wording
corrections, not scope changes** — the guarantee is pinned to the layer
recvmsg4 can honor.

### A1 — K2 / US-01 "VIP-sourced reply"

**Prior (verbatim):**

- US-01 AC: *"Tier-3 `tcpdump` shows the reply source = the VIP, never the
  backend IP."*
- US-01 UAT scenario 2: *"Then the captured reply packet's source address is
  10.96.0.10 (the VIP) / And no reply ever leaves with the backend IP
  10.244.0.7 as its source"*.
- K2 (KPI): *"The same client … receives a reply sourced from the VIP … 100%
  of replies are VIP-sourced; 0 backend-IP-sourced replies reach a client …
  Measured By: Tier-3 `tcpdump` wire capture on the reply path."*

**New:**

- *"The source address the client application reads from `recvfrom` /
  `msg_name` is the VIP (`10.96.0.10`), not the backend IP."* Measured at the
  **application sockaddr layer** (the client app's `recvfrom` return / a
  client-side assertion), NOT via a `tcpdump -i lo` capture (which correctly
  shows the backend source pre-rewrite on every round-trip).

**Rationale:** recvmsg4 edits the already-populated sockaddr the app reads;
it never touches the wire (research Q4). The VIP-sourced guarantee is real
and Tier-3-assertable — at the app layer, which is what a source-validating
resolver actually checks.

### A2 — K5 / US-03 "no backend-IP leak on miss"

**Prior (verbatim):**

- US-03 elevator pitch: *"a `tcpdump` shows NO backend-IP-sourced reply left
  the host."*
- US-03 UAT scenario "A missing reverse entry never leaks the backend IP":
  *"Then no reply reaches the client sourced from the backend IP / And the
  miss is observable via a counter or log, not silent"*.
- US-03 AC: *"With the forward entry present and the reverse entry forced
  absent, no client-bound reply is sourced from the backend IP (Tier-3
  `tcpdump`); the miss is observable."*
- K5 (KPI): *"The reply path … fails safe when the reverse mapping is missing
  … 0 backend-IP-sourced replies leak on a reverse miss."*

**New:**

- *"On a reverse-map miss, the source address the client application reads
  from `recvfrom`/`msg_name` is a non-backend sentinel (`192.0.2.1`, RFC 5737
  — never the backend IP), and the miss is observable via a counter."* The
  `tcpdump`/"left the host" framing is **dropped** for this path (it asserts
  XDP semantics recvmsg4 cannot own; the same-host reply never leaves the
  host).

**Rationale:** recvmsg4 cannot drop (verifier `[1,1]`, research Q1), and
pass-through leaks the backend IP to the app (Cilium's behaviour). The only
no-leak-to-app option is the sentinel rewrite (research Q5). The guarantee is
pinned to the app sockaddr — the layer recvmsg4 governs.

---

## Change B — DISCUSS K4 / DD6 "0 connect4 changes / pure addition" violated (D4 / DDD-4)

**What changed.** The user overrode Morgan's Option-2 recommendation to
**Option 3 (shared helper)**: the service-key construction + `user_port`
low-16-NBO handling is factored into ONE `#[inline(always)]` kernel helper
(`build_local_service_key`) consumed by all three hooks (connect4 +
sendmsg4 + recvmsg4). The map LOOKUP and the rewrite DIRECTION stay
per-hook: connect4/sendmsg4 do a forward dest-rewrite against
`LOCAL_BACKEND_MAP`; recvmsg4 does a reverse source-rewrite against
`REVERSE_LOCAL_MAP`. A shared helper across three hooks necessarily
**refactors the shipped third hook** — `connect4`'s inline key-build body is
replaced by a call to the helper. The refactor is **behavior-preserving**
(the helper does byte-for-byte what connect4 does today) but the **diff is
non-zero**, and — because there is **no Tier-2
backstop** for `cgroup_sock_addr` (`BPF_PROG_TEST_RUN` → ENOTSUPP ≤ 6.8) —
the connect4 refactor's regression surface is **Tier-3-only**. It is
re-verified by re-running the shipped connected (TCP / connected-UDP)
round-trip acceptance against the helper-backed connect4 in the same PR.

**Prior (verbatim):**

- DD6: *"Pure addition: connect4, `LOCAL_BACKEND_MAP` forward shape, action
  variants (proto-carrying, ADR-0053 Amd 3), hydrator classifier all
  UNCHANGED. Single-cut, no shims."*
- US-01 AC: *"connect4 / forward-map shape / hydrator classifier UNCHANGED
  (pure addition)."*
- K4 (KPI): *"The dataplane author … adds the unconnected path WITHOUT
  re-migrating connect4 call sites … 0 changes to connect4 / forward-map
  shape / hydrator classifier (pure addition; diff is additive only) …
  Measured By: PR diff review: no connect4/forward-shape/classifier
  modification."*
- Carpaccio note: *"(Option 3's shared helper was explicitly NOT adopted — it
  would modify shipped connect4 code.)"*

**New (K4 restatement):**

- *"The dataplane author adds the unconnected path with **zero net-new
  connect4 behavior**. connect4's key-build body is **refactored** to call the
  shared `build_local_service_key` helper (Option 3, user-locked): the diff is
  **non-zero but behavior-preserving**, and is **Tier-3-reverified** by the
  shipped connected round-trip acceptance re-running green against the
  helper-backed connect4. The forward-map shape (`LOCAL_BACKEND_MAP`), the
  action variants, and the hydrator classifier remain UNCHANGED. Measured By:
  PR diff review confirms connect4's only change is the helper extraction
  (no behavioral delta), and the connected-round-trip Tier-3 acceptance is
  green."*

**Rationale:** the user chose a single forward-lookup + NBO + key-construction
site across three hooks (Option 3) over Option 2's three-way duplication.
That choice trades a behavior-preserving connect4 refactor (Tier-3-reverified)
for one source of truth on the NBO-hazard-prone lookup. The "pure addition"
framing was Option-2-specific; under the locked Option 3, connect4 is EXTEND.
See ADR-0053 revision 2026-06-05 § D4 for the honest no-Tier-2-backstop risk
statement.

---

## Relay summary (for the product owner)

1. **K2/K5 are now app-sockaddr assertions, not wire/`tcpdump` assertions.**
   The VIP-sourced-reply and no-backend-leak guarantees are **real and Tier-3-
   verifiable** — at the application's `recvfrom`, which is what a resolver
   source-validates against. recvmsg4 cannot make a wire-level guarantee (that
   is XDP's job, out of scope here) and cannot drop (verifier `[1,1]`); on a
   reverse miss it substitutes the sentinel `192.0.2.1` and counts the miss —
   strictly stronger than Cilium, which leaks the backend IP to the app.
2. **K4 changes from "0 connect4 changes" to "0 net-new connect4 behavior,
   non-zero diff."** The user-locked shared helper (Option 3) refactors
   shipped connect4; behavior is preserved and Tier-3-reverified, but the diff
   is real and there is no Tier-2 backstop for the refactor.
3. **One DELIVER/Tier-3 open question (no tracking issue):** confirm the
   target resolvers cleanly reject a `192.0.2.1`-sourced reply; swap the
   sentinel value if not (no design change).
