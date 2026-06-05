# DELIVER Upstream Issues — unconnected-udp-sendmsg4 (#200)

Back-propagation log: gaps/contradictions in prior-wave decisions discovered
during DELIVER implementation. Per the nw-deliver back-propagation contract,
each is documented here, surfaced to the user, and resolved before continuing
past the affected step.

---

## UI-1 (step 01-03) — D3 "rewrite-to-sentinel on reverse miss" is unworkable on recvmsg4; corrected to "rewrite-on-hit, no-op-on-miss"

**Status:** **RESOLVED 2026-06-05** — DESIGN artifacts amended to the
corrected no-op-on-miss behavior. SSOT: **ADR-0053 § D3 sub-revision
2026-06-05b** ("rewrite-on-HIT, pure NO-OP on MISS"); brief.md
§ "Unconnected-UDP sendmsg4 extension"; feature-delta.md DDD-3 + CA-3;
design/upstream-changes.md Change A § A2; research addendum "UI-1
adjudication (2026-06-05)" (verdict: crafter CORRECT, Q5 WRONG). The
corrected S-03-01 acceptance contract + K5 reframing are specified for the
acceptance-designer in feature-delta CA-3 / the research addendum
§ "S-03-01 re-scope". Implementation already lands the corrected behavior
(Tier-3 green, commit `e71ad780`).

**Affected prior-wave decisions:**
- ADR-0053 Revision 2026-06-05 § D3 (user-locked, guide mode): *"on a
  `REVERSE_LOCAL_MAP` miss, rewrite the reply source to a non-backend/non-VIP
  sentinel `192.0.2.1` (RFC 5737) + counted miss; strictly stronger than
  Cilium's pass-through-leak."*
- `docs/research/dataplane/recvmsg4-reply-source-rewrite-and-miss-semantics-research.md`
  Q5 (the recommendation that sentinel-on-miss is "strictly stronger than
  Cilium").
- DISTILL scenario **S-03-01** (step 03-01): *"reverse miss → source is
  sentinel 192.0.2.1, never the backend IP."*
- KPI **K5** / **US-03**: the no-leak guarantee, currently framed as
  "sentinel on miss."

**What was discovered:** `cgroup/recvmsg4` attaches at the cgroup ancestor
(`overdrive.slice`) and fires on **every** unconnected-UDP `recvmsg` issued by
**any** descendant process — not only service-VIP replies. A
`REVERSE_LOCAL_MAP` miss therefore does NOT mean "a service reply whose reverse
entry is missing"; it overwhelmingly means **"this datagram is not a service
reply at all"** (the backend's own `recvfrom` of the inbound query; any
unrelated same-host UDP). Rewriting those sources to the sentinel mangles the
sender address every non-service datagram's app reads — it broke the
unconnected round-trip AND the connected-UDP K4 path until caught.

**The correction (implemented, Tier-3-verified):**
- recvmsg4 on a **HIT** rewrites the reply source backend→VIP (the map hit is
  the "this is a service reply" discriminator). Unchanged from D3.
- recvmsg4 on a **MISS** is a **pure no-op** (miss-counter bump only; source
  left intact). This is what Cilium actually does; the research's
  "strictly stronger sentinel" conclusion did not account for recvmsg4 firing
  on all unconnected UDP in the cgroup, not just service replies.
- The no-leak guarantee (K5) is **preserved by a different mechanism**: the
  reverse-first dual-write guarantees every registered backend has a reverse
  entry, so a genuine service reply ALWAYS hits → always rewritten to the VIP.
  There is no backend-IP-leak path. A miss is by definition non-service
  traffic, so leaving its real source intact is correct.
- `SENTINEL_SOURCE_HOST` is retained in the code as documentation; it is not
  used on the miss path. (Whether a sentinel fail-safe has any role scoped to
  the HIT path — a service reply whose reverse entry was evicted, the
  should-never-happen-under-dual-write case — is the open question for the
  S-03-01 re-scope.)

**Resolution (DONE — all four steps complete 2026-06-05):**
1. ✅ Architect amended ADR-0053 § D3 (sub-revision 2026-06-05b):
   sentinel-on-miss → rewrite-on-hit / no-op-on-miss; recorded the
   recvmsg4-fires-on-all-cgroup-UDP rationale + the research Q5 correction;
   changelog row added.
2. ✅ Research doc carries the "Addendum — UI-1 adjudication (2026-06-05)"
   with the explicit Q5 correction (crafter CORRECT, Q5 WRONG).
3. ✅ brief.md, feature-delta.md (DDD-3, CA-3, decisions table, C4,
   tech-choices, open questions), and design/upstream-changes.md corrected.
4. ✅ **Acceptance-designer** applied the corrected S-03-01 contract +
   K5/US-03 reframing (2026-06-05) to `distill/test-scenarios.md` (S-03-01
   rewritten to the three corrected assertions + the reconciliation-note K5
   line + slice header + error-ratio label), the Tier-3 scaffold
   `crates/overdrive-dataplane/tests/integration/unconnected_udp_reply_hardening.rs`
   (S-03-01 test renamed `reverse_miss_rewrites_source_to_sentinel_not_backend_ip`
   → `non_service_unconnected_udp_reads_real_source_recvmsg4_noop_on_miss`,
   panic body + doc rewritten; stays `#[should_panic(expected = "RED scaffold")]`,
   Lima compile-check green), `distill/red-classification.md` (S-03-01 row), and
   the `deliver/roadmap.json` step 03-01 reference (name/criteria/test_file/notes
   re-scoped to no-op-on-miss; broken fn reference fixed). Three assertions:
   (a) non-service unconnected UDP reads its real sender source (unaffected —
   pure no-op on a miss); (b) a service reply always hits → VIP-sourced (no
   leak path, via the D1 reverse-first dual-write); (c) the miss counter
   increments on non-service recv but is behaviorally inert (no source rewrite).

**Why not a blocker for slices 01–02:** the round-trip (slice 01) is GREEN
with the corrected behavior; the Sim reply-mirror + equivalence invariant
(slice 02, steps 02-01/02-02) concern reply-source identity on the HIT path,
which is unchanged. The correction only affects slice 03 (the miss/hardening
scenarios) and the design artifacts.
