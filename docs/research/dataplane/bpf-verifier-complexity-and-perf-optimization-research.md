# Research: eBPF Verifier-Complexity Reduction & Runtime-Performance Techniques for Overdrive's XDP / cgroup BPF Programs

**Date**: 2026-06-14 | **Researcher**: nw-researcher (Nova) | **Confidence**: High (mechanisms/applicability) / Medium (quantitative impact — flagged needs-measurement) | **Sources**: 19 distinct (avg reputation 1.0)

> Scope: a RANKED, OUR-code-specific list of concrete changes to reduce verifier
> instruction count and improve runtime pps/latency for `xdp_service_map_lookup`
> (151379 → 48395 post-`e16f1972`) and `xdp_reverse_nat_lookup` (~156340 → 48182).
> Cross-references — does NOT duplicate —
> `docs/research/dataplane/aya-rs-usage-comprehensive-research.md`,
> `docs/research/dataplane/xdp-checksum-partial-veth-research.md`,
> and `docs/research/dataplane/cilium-bpf-fib-lookup-l2-mac-rewrite-comprehensive-research.md`.

## Executive Summary

The `e16f1972` chunked-`bpf_csum_diff` refactor already captured the largest
available win — collapsing the full-L4-checksum path from ~150K to the low
thousands and both programs from ~155K → ~48K verified instructions. The residual
~48K is NOT one fat thing; it is the *rest* of the program (full-payload checksum
chunk loop, `bpf_fib_lookup` + L2 MAC rewrite + redirect, FNV-1a 5-tuple hash, HoM
chained lookup, sanity prologue, and diffuse bounds re-validation). Both programs
sit at ~9.7% of the 500K "L1-cache-fits" target — **there is no verifier cliff**;
the goal of every recommendation is fragility reduction + runtime pps/latency, not
crossing a ceiling.

**The ranked, concrete recommendations (highest leverage first):**

1. **R-1 — Replace the full L4-payload recompute with incremental
   `csum_diff`-over-changed-fields, gated on `ethtool -K $iface tx off` at the LB
   veth.** This is the one change that is *both* the biggest verifier win (deletes
   cost center C-1, the largest residual — NAT path heads toward low single-digit
   thousands) *and* the biggest runtime win (~240× less checksum work per
   1460-byte packet — incremental touches ~6 bytes, recompute walks the whole
   payload). Cilium's production NAT does exactly this (`bpf/lib/nat.h:489` —
   `csum_diff(&old_addr, 4, &new_addr, 4, 0)`, never a payload walk). The catch:
   it requires disabling TX-checksum-offload on the LB veth (standard L4-LB
   practice; the equivalent of why Cilium runs XDP only on full-checksum physical
   NICs), which is a new **operational invariant** — surface to the user as a
   decision, not a silent change. Needs a Tier-3 spike to confirm byte-correctness
   with `tx off` on kernel 6.18/7.0. *Applicability: XDP-clean (pure ALU delta,
   no skb helper), aya 0.13.x OK; the operational dependency is the tradeoff.*

2. **R-2 — Swap the unrolled 13-byte FNV-1a 5-tuple hash for a `jhash_3words`
   mix (Cilium's exact backend-selection hash).** Packs both ports into one word
   + IP + proto = 3 words → one fixed rotate-mix, no per-byte unroll
   (`bpf/lib/hash.h:13-17`). Modest verifier win (chips at C-3), small runtime win
   (avoids the FNV multiply dependency chain), production-standard. *Applicability:
   pure ALU, but coupled to the userspace Maglev permutation generator — a
   kernel+userspace+table change, not kernel-only. Deferrable.*

3. **R-6 — Adopt Cilium's safe verifier-state micro-idioms now:** read each header
   field once into a local (we re-read across sanity/body and pre/post-rewrite),
   `unlikely()` hints on NULL-miss branches. Zero-risk, low-yield, immediate. The
   higher-yield inline-asm forms (`map_array_get_32` bounded index; operand-order
   pinning) are RESERVED behind a measurement showing the count is LLVM-elision-
   sensitive.

4. **R-4 — Tail-call splitting is the right escalation path but NOT needed at
   48K.** We already apply its coarsest form (separate fwd/rev programs per
   ADR-0045 §3). Document for a future slice that pushes a single program toward
   the ceiling.

**Explicit "do NOT pursue" results (corrections to plausible hypotheses):**
- **R-3 — `bpf_loop()` does not help us.** Our dominant loop is 24 iterations and
  should be *deleted* (R-1), not converted; `bpf_loop` shines for high-count loops
  that must be kept. Cilium does not use it in the hot path.
- **R-5 — Cheaper modulo (fastrange/power-of-2) is low-leverage + high-coupling.**
  Cilium pays the identical prime `% 16381` (`DefaultTableSize = 16381`); the prime
  is load-bearing for Maglev's distribution guarantee, and fastrange is not a
  drop-in for `mod`-based Maglev indexing.

**Honest bottom line:** R-1 is the only change with order-of-magnitude leverage,
and it carries a real operational tradeoff (the `tx off` invariant) that the
architecture deliberately avoided by choosing full recompute. Everything else is
incremental polish. None of the verifier-instruction-count reductions below are
asserted as specific numbers — the only hard data are the whole-program totals
(48395 / 48182); per-component shares and post-change counts are flagged
"needs measurement" and require a Tier-3/Tier-4 spike (`verified_instruction_count()`
+ xdp-perf delta).

## Research Methodology

**Search Strategy**: Primary-source reads of (a) OUR kernel-side code under
`crates/overdrive-bpf/src/`, (b) Cilium's production BPF codebase at
`/Users/marcus/git/cilium/cilium` (= `github.com/cilium/cilium`, reputation 1.0,
read locally), cross-referenced with kernel.org / ebpf.io / docs.cilium.io for
load-bearing claims.

**Source Selection**: official (kernel.org, datatracker.ietf.org), open-source
(cilium.io, docs.cilium.io, github.com/cilium, ebpf.io), technical docs
(aya-rs.dev, docs.rs). Reputation high.

**Quality Standards**: 3 sources/claim ideal; 2 acceptable; 1 authoritative
minimum (kernel docs / RFC / Cilium source). Verifier-instruction-count
reductions NOT asserted as fact without a cited source or a flagged
"needs-measurement" caveat.

## Current Cost Centers (what our ~48K actually spends on)

Both `xdp_service_map_lookup` (48395) and `xdp_reverse_nat_lookup` (48182) run a
near-identical per-packet pipeline. Reading the source
(`crates/overdrive-bpf/src/programs/xdp_service_map.rs`,
`xdp_reverse_nat.rs`, `shared/csum.rs`, `shared/sanity.rs`), the residual ~48K
decomposes into the following cost centers, ordered by *estimated* contribution
to the verifier's state walk. These shares are inferred from the code shape and
the baseline-file history (`perf-baseline/main/verifier-budget/veristat-service-map.txt`),
NOT measured per-component — see Knowledge Gap G-1 (no per-function veristat
breakdown exists; the only hard data points are the whole-program totals at each
commit). Treat the percentages as ranked estimates, not measurements.

| # | Cost center | Code site | Estimated share | Why it costs verifier instructions |
|---|---|---|---|---|
| C-1 | **`recompute_l4_csum` chunked `bpf_csum_diff` engine** | `csum.rs:111-279` | **dominant (low-to-mid tens of % — the single biggest residual)** | The 24-iteration 64-byte main loop (`MAX_64_CHUNKS`) + 4-level power-of-2 tail tree + 1-3 byte residue. Each `csum_diff_chunk::<N>` call re-reads `data`/`data_end` *volatile* and re-derives a fresh bounded packet pointer, so the verifier re-walks the bounds proof per chunk. The verifier still walks every iteration of the bounded `while i < 24` loop (it does NOT use `bpf_loop`), and each iteration carries a helper-call boundary + a packet-pointer re-validation. This is the residue of the 151379→48K collapse: the csum portion dropped from ~150K to "low thousands," but it is still the largest single chunk of the remaining 48K. |
| C-2 | **`bpf_fib_lookup` + parameter-block init + L2 MAC rewrite + redirect** | `xdp_service_map.rs:450-574`, `xdp_reverse_nat.rs:355-441` | **mid (the baseline history attributes +551 insns to *adding* this at Slice 05-04, pre-csum-collapse; its relative share of the post-collapse 48K is larger)** | `core::mem::zeroed()` of the `bpf_fib_lookup` struct, ~9 field writes from packet fields, the helper call (helper id 69), the `RET_SUCCESS` branch, two 6-byte MAC writes through `mut_ptr_at` bounds checks, the `ingress_ifindex == fib.ifindex` branch, and the `bpf_redirect` helper call. The union-field writes (`__bindgen_anon_*`) and the post-rewrite packet re-reads (tot_len, tos, ports, IPs read *again* after the rewrite committed) each add state. |
| C-3 | **FNV-1a-32 over the 5-tuple, unrolled across 13 bytes** | `xdp_service_map.rs:173-200` (`fnv1a_5tuple_slot`) | **low-to-mid** | 13 XOR-multiply rounds, each `(h ^ byte).wrapping_mul(PRIME)`, fully unrolled, plus the `% INNER_TABLE_SIZE` (16381) reduction. Forward path only — the reverse path has no hash (it keys `REVERSE_NAT_MAP` directly on the 3-tuple). |
| C-4 | **HoM chained lookup (outer `SERVICE_MAP` → inner ARRAY → `BACKEND_MAP`)** | `xdp_service_map.rs:286-327` | **low** | Three `bpf_map_lookup_elem`-class calls with two mandatory NULL-checks between them (verifier-tagged `inner_map` pointer discipline). Reverse path is a single `REVERSE_NAT_MAP.get` (one lookup, cheaper). |
| C-5 | **Sanity prologue (5 Cloudflare-shape checks)** | `sanity.rs` + `xdp_*:193-208` | **low** | EtherType, version+IHL, total_length, proto, TCP-flag sanity. Re-reads several header fields the main body reads again. Plus two explicit `ptr_at` pre-bounds-checks at program entry. |
| C-6 | **Bounds-check re-validation throughout (`ptr_at` / volatile `data`/`data_end` re-reads)** | every `read_u*` / `write_u*` | **diffuse** | Each typed packet access re-derives the bounds proof. Many fields are read twice (once in sanity, once in the body; once pre-rewrite, once post-rewrite for the FIB block). |

**The single most important structural fact:** the program's *runtime* JIT'd
machine code is compact (the baseline file notes "the actual JIT'd machine code
is compact — one loop body"), but the *verifier* instruction count is high
because the verifier walks every path of the bounded csum loop and re-proves the
packet-pointer bounds at every helper boundary. Verifier count and runtime cost
are therefore **decoupled** for C-1: reducing the csum verifier cost (e.g. by
going incremental) shrinks the verifier number a lot, but C-1's *runtime* cost
is already dominated by the per-byte `csum_partial` work the kernel does inside
`bpf_csum_diff` — which incremental csum eliminates entirely, so incremental is a
double win (verifier AND runtime).

Both programs are at ~9.7% of the 500K "L1-cache-fits" target — there is **no
cliff to avoid**. The goal of every recommendation below is *fragility reduction
+ runtime pps/latency*, not crossing a ceiling. Frame impact accordingly.

## Findings

Ranked highest-leverage first. Each carries: mechanism, Cilium/kernel evidence,
estimated impact, applicability to OUR code under the constraints, and the
validation gate.

---

### Finding R-1 (HIGHEST LEVERAGE): Replace full L4 recompute with incremental `csum_diff`-over-changed-fields, gated on disabling TX-checksum-offload at the LB veth

**This is the difference between ~48K and ~2K on the NAT path, and it is the one
change that is *also* the largest runtime win.** It addresses cost center C-1.

**Mechanism.** A NAT rewrite changes only the IP address (4 bytes) and the L4
port (2 bytes). RFC 1624 lets you fold *only the delta* of the changed words into
the existing L4 checksum — `new_csum = ~(~old_csum + ~old_word + new_word)` — in
O(1), without touching the payload. We already do exactly this for the IPv4
*header* checksum via `csum_incremental_2_2` (`csum.rs:289`). The L4 checksum is
recomputed over the whole payload **only** because of the CHECKSUM_PARTIAL-on-veth
constraint.

**The constraint, resolved.** Per
`docs/research/dataplane/xdp-checksum-partial-veth-research.md` (Findings 1-5,
14 sources, avg reputation 0.99): on a veth interface with TX-checksum-offload
*enabled*, a locally-generated packet arrives at the XDP hook with
`skb->ip_summed == CHECKSUM_PARTIAL` — the on-wire L4 checksum field holds only
the pseudo-header sum, NOT a complete checksum. An incremental delta applied to
that partial value yields a result that is neither valid-PARTIAL nor valid-FULL,
so every packet is dropped. XDP cannot detect CHECKSUM_PARTIAL (no `ip_summed`
field on `xdp_buff`; `bpf_csum_diff` is purely arithmetic and not offload-aware —
that research's Finding 2 + kernel commit `7d672345ed29`). Full recompute is
correct because it ignores the old field entirely. **That is why the architecture
chose full recompute.**

**But the same research names the escape hatch (its Approach F).** Disabling
TX-checksum-offload on the LB veth — `ethtool -K $IFACE tx off`
(`tx-checksum-ip-generic off`) — forces the kernel to *materialise the full L4
checksum in software before the packet reaches the XDP hook*. With a FULL checksum
on the wire, incremental update is correct, and the full-payload recompute is no
longer needed. The research explicitly states (line 481): "With full checksums on
the wire, incremental update (the existing `csum_incremental_3_3`) works
correctly," and (line 491) "this is the Cilium/standard practice for XDP-attached
veth interfaces." It recommended Approach F as defense-in-depth *alongside* the
recompute; the leverage this finding surfaces is making it the **primary** path so
incremental csum replaces the recompute.

**Cilium evidence (primary source, read locally).** Cilium's production NAT path
is incremental-only and never walks the payload:
- `bpf/lib/nat.h:489` — `sum = csum_diff(&old_addr, 4, &new_addr, 4, 0)` — a
  4-byte→4-byte delta. `bpf/include/bpf/csum.h:36-48` inlines this constant-size
  case to `csum_add(~(*(__u32 *)from), *(__u32 *)to)` — **~2 ALU ops, no helper
  call, no payload read.**
- `bpf/lib/nat.h:525` — `l4_modify_port(...)` for the port delta.
- `bpf/lib/nat.h:536` — `csum_l4_replace(ctx, l4_off, &csum, 0, sum, flags)` with
  `flags = BPF_F_PSEUDO_HDR` applies the delta to the L4 checksum field.
- Cilium runs XDP L4 LB *only on physical/host NICs* where the NIC delivers
  CHECKSUM_COMPLETE (full checksum materialised by hardware) — same end-state as
  `ethtool -K tx off` produces on veth (xdp-checksum research Finding 3,
  `pkg/datapath/loader/xdp.go`). For veth (pod-to-pod) traffic Cilium uses TC
  where the real kernel `bpf_l4_csum_replace` is CHECKSUM_PARTIAL-aware. Either
  way, Cilium **never does a full-payload XDP recompute** — and the design lesson
  is: make the wire carry a FULL checksum, then go incremental.

**Estimated impact.**
- **Verifier**: C-1 (the dominant residual) collapses. The L4 fold becomes a
  second `csum_incremental_*` call shaped exactly like the IPv4-header fold we
  already ship — tens of instructions, no loop, no per-chunk helper calls. The
  NAT path's residual drops from ~48K toward the low single-digit thousands.
  **Flagged: needs measurement** — the residual after removal is C-2..C-6 (FIB +
  hash + HoM + sanity), whose absolute size post-collapse is not separately
  measured (G-1). A spike that lands the incremental path and reads
  `verified_instruction_count()` is the only way to pin the new number; do not
  assert a specific count.
- **Runtime (the bigger prize)**: full recompute does `csum_partial` over the
  *entire L4 payload* (up to 1500 bytes) per packet — that is real per-byte work
  the JIT emits and the CPU runs at line rate. Incremental does ~6 bytes of work.
  For a 1460-byte TCP segment this is roughly a ~240× reduction in checksum work
  per packet. This is the single largest runtime pps/latency win available, and
  unlike most verifier reductions it is **not** decoupled from runtime — it is the
  rare case where the verifier-cheaper form is dramatically runtime-cheaper too.

**Applicability to OUR code & constraints.**
- *XDP context (no skb)*: We CANNOT use `bpf_l4_csum_replace` (skb-only, the
  xdp-checksum research Finding 1 confirms it is a `BPF_STUB` in XDP). We apply
  the delta arithmetically, exactly as we already do for the IP header
  (`csum_incremental_2_2`) — extend that to a `csum_incremental_3_3`-shape that
  also folds the port-word delta and a pseudo-header-aware addend. No new helper
  needed; pure ALU; aya 0.13.x / `#![no_std]` clean.
- *The TX-offload-disable is an operational dependency.* The loader
  (`EbpfDataplane::new` / the XDP attach site) must `ethtool -K $iface tx off`
  (or the netlink `ETHTOOL_STXCSUM` equivalent) on every LB-attached veth at
  attach time. This is standard L4-LB practice but it IS a new operational
  surface — **surface to the user as a decision**, since it changes the attach
  contract and has a (small) host-side software-checksum cost on the TX path of
  the *backend* veth.
- *Risk*: if any LB-attached interface ever has TX offload re-enabled out of
  band, incremental silently corrupts every packet. The architecture chose full
  recompute precisely to avoid this fragility. The honest framing: this trades
  ~48K verifier + full-payload runtime cost for an operational invariant
  (`tx off` stays off). That is a real tradeoff, not a free win — present both.

**Validation gate.** The Tier-3
`reverse_nat_e2e::real_tcp_connection_completes_through_vip_with_payload_echo`
test (real TCP handshake + payload echo through the VIP) is the exact regression
guard. Per the xdp-checksum research's Part 3 recommendation, keep TX offload
ENABLED in the *test fixture* — wait, that is the opposite of this finding's
requirement. **This is the load-bearing test-design point**: if R-1 is adopted,
the fixture must run with `tx off` (matching production), and a *separate*
negative test should assert that with `tx on` the incremental path fails (proving
the operational invariant is load-bearing, not silently masked). A green e2e with
`tx off` + a red e2e with `tx on` is the honest signal. **Needs a Tier-3 spike**
to confirm the incremental checksum is byte-correct on the dev kernel (7.0) and
the pinned floor (6.18) with `tx off`.

---

### Finding R-2: Replace the unrolled 13-byte FNV-1a 5-tuple hash with a jhash-style 3-word mix (Cilium's exact backend-selection hash)

Addresses cost center C-3 (forward path only).

**Mechanism.** Our `fnv1a_5tuple_slot` unrolls 13 XOR-multiply rounds — one per
byte of the 5-tuple (`csum.rs`/`xdp_service_map.rs:173-200`). Cilium hashes the
identical 5-tuple for Maglev backend selection with **`jhash_3words`** — three
32-bit words run through the fixed `__jhash_final` rotate-mix macro (no loop, no
per-byte step):

```c
// bpf/lib/hash.h:13-17  — Cilium's backend-selection hash
__hash_from_tuple_v4(tuple, sport, dport) =
    jhash_3words(tuple->saddr,                       // word 1: src IP
                 ((__u32)dport << 16) | sport,       // word 2: BOTH ports packed
                 tuple->nexthdr,                      // word 3: proto
                 CONFIG(hash_init4_seed));
```

The crucial structural win: Cilium **packs both 16-bit ports into one 32-bit
word** (`(dport << 16) | sport`) and treats the IP as a single 32-bit word, so
the whole 5-tuple is 3 words → one `__jhash_final` (a fixed 7-statement
rotate/xor/subtract macro, `jhash.h:28-37`). No 13-step byte unroll.

**Why jhash is the canonical LB hash (cross-reference).** jhash (Bob Jenkins) is
the kernel's standard hash and what Cilium AND Katran use for flow→backend
selection. It is designed for exactly this 3-word shape; the daddr is
deliberately excluded (`hash.h:9-11` comment) so the same flow hits the same
backend across different service VIPs — a consistency property our FNV-1a-over-
all-fields does not provide (we hash dst_ip in, which is fine for our single-VIP
model but differs from Cilium's intent).

**Estimated impact.**
- **Verifier**: replacing 13 XOR-multiply rounds + 13 byte-extractions with 3
  word-assembles + one `__jhash_final` macro removes a chunk of C-3's unrolled
  body. Modest absolute reduction (C-3 is "low-to-mid"); **needs measurement** —
  do not assert a count.
- **Runtime**: jhash's `__jhash_final` is ~15 register ALU ops vs FNV-1a's 13
  dependent multiply chains. Multiplies are more expensive than rotates/adds on
  most BPF JITs; the dependency chain (each FNV round depends on the previous) is
  also a latency penalty. Net: a small but real per-packet latency win on the
  forward path. Roughly comparable order; jhash is the better-dispersing,
  cheaper-per-op choice and is the production-proven default.

**Applicability.** Pure ALU; aya 0.13.x / `#![no_std]` clean; no helper. The
Maglev table is generated userspace-side from the *same* hash family, so the
kernel-side slot hash MUST stay in lockstep with `maglev::permutation` — switching
the kernel hash to jhash requires switching the userspace permutation seed/hash
to match (the `MaglevDeterministic` DST invariant pins twin-run identity, and the
`MaglevDistributionEven` invariant pins ±5% spread). **This is a coupled change
across kernel + userspace + the Maglev table generator**, not a kernel-only edit.

**Caveat / honesty.** This is a *modest* verifier win and a *small* runtime win.
FNV-1a is not broken — it disperses fine for our purposes. The argument for jhash
is (a) it is the production-standard LB hash (Cilium + kernel + Katran), (b)
port-packing removes the byte unroll, (c) it avoids the multiply dependency chain.
But it is NOT in the same leverage class as R-1. Rank it second only because it is
low-risk and aligns us with the production reference; if Maglev-lockstep coupling
makes it expensive, it is deferrable.

**Validation gate.** `MaglevDeterministic` + `MaglevDistributionEven` DST
invariants (`crates/overdrive-sim/src/invariants/maglev_*.rs`) must stay green,
AND a Tier-2 lockstep test must confirm kernel-side `jhash` slot == userspace
permutation index for a fixed 5-tuple corpus.

---

### Finding R-3: `bpf_loop()` does NOT help our residual (it is available on 6.18 but our dominant loop is best removed, not converted)

This finding is a **negative result** — it corrects a plausible-but-wrong
hypothesis in the research brief.

**Mechanism.** `bpf_loop()` (kernel 5.17+; available on our 6.18 floor and 7.0
dev VM) replaces a verifier-*unrolled* bounded loop with a single callback the
verifier does NOT unroll — it verifies the callback body once and trusts the
bounded iteration count. For a loop the verifier currently walks N times, this can
collapse N× state to 1×.

**Why it does NOT apply to our hot loop.** Our only significant loop is the
24-iteration `while i < MAX_64_CHUNKS` in `recompute_l4_csum` (C-1). Three
problems:
1. **24 iterations is small.** `bpf_loop`'s win scales with iteration count;
   24× is not where the ~48K lives (the per-iteration *helper call + packet-
   pointer re-validation* is, and `bpf_loop` does not remove those).
2. **The right fix for C-1 is to *delete* the loop (R-1), not convert it.**
   Incremental csum removes the payload walk entirely; converting it to
   `bpf_loop` would preserve the full-payload runtime cost while only trimming
   verifier state — strictly worse than R-1.
3. **`bpf_loop` callbacks and packet-pointer access are awkward.** The callback
   receives an opaque context; threading the XDP `data`/`data_end` packet pointer
   and a mutable checksum accumulator through the callback context, while keeping
   the verifier's packet-bounds tracking intact across the callback boundary, is
   exactly the operand-ordering fragility the xdp-checksum research documents
   (Findings 8-9) for the Rust LLVM BPF backend. aya-ebpf 0.1.x does **not** bind
   `bpf_loop` (per `aya-rs-usage-comprehensive-research.md` coverage matrix — it
   is absent from the typed surface), so it would be a hand-rolled raw binding
   (the project precedent for `bpf_csum_diff` / `bpf_fib_lookup` applies), adding
   risk for a loop that should be deleted.

**Cilium corroboration.** Cilium's hot-path datapath does **not** use `bpf_loop`
for packet processing — a grep of `bpf/lib/*.h` and `bpf/include/bpf/helpers.h`
finds no `bpf_loop` wrapper; the only matches are in `mcast.h` (multicast fan-out,
not the per-packet LB path) and the raw `linux/bpf.h` UAPI header. Cilium relies
on unrolled bounded loops + tail calls (R-4) for complexity management, not
`bpf_loop`. This is evidence that `bpf_loop` is not the lever production L4 LBs
reach for.

**Estimated impact.** Verifier: marginal at best for our 24-iteration loop;
runtime: none (or negative — callback overhead). **Recommendation: do NOT pursue
`bpf_loop` for the csum path.** It is the right tool for a genuinely high-count
loop (thousands of iterations) that must be *kept*; ours should be deleted (R-1)
or stays a small unrolled tail-tree.

**Applicability.** N/A — recommended against. Documented so a future reader does
not re-derive the brief's plausible-but-wrong hypothesis.

---

### Finding R-4: Tail-call splitting (Cilium's `tailcall.h`) — applicable as a *fragility* hedge, NOT needed at our current budget

Addresses the per-program 1M ceiling, not our current 48K.

**Mechanism.** Cilium splits its datapath into ~50 distinct tail-called
sub-programs via a `BPF_MAP_TYPE_PROG_ARRAY` (`bpf/lib/tailcall.h:107-113`,
`cilium_calls`; `CILIUM_CALL_*` indices 1-50). `tail_call_static(ctx, cilium_calls,
index)` transfers control to another program; **each tail-called program is
verified independently** against the 1M per-program instruction ceiling. This is
how Cilium keeps an enormous datapath (conntrack + NAT + policy + encap + ...)
under the verifier limit — no single program is ever verified as a whole.

**Cilium evidence.** `bpf/lib/tailcall.h` is the canonical artifact: the
`__declare_tail(index)` macro (`tailcall.h:126-128`) inserts a function into the
prog-array at a compile-time-constant index; `tail_call_internal`
(`tailcall.h:130-138`) is the invocation wrapper. The forward LB path
(`CILIUM_CALL_IPV4_FROM_LXC`), NAT egress
(`CILIUM_CALL_IPV4_NODEPORT_NAT_EGRESS`), and rev-NAT
(`CILIUM_CALL_IPV4_NODEPORT_REVNAT`) are *separate* tail-call targets.

**Estimated impact.** Verifier: tail calls do not *reduce* total instructions —
they *partition* them so each piece is independently verified. At 48K (9.7% of the
500K target) we have **no partitioning need**. Runtime: a tail call is a
near-zero-cost indirect jump (prog-array dispatch), but it is not free and it
adds a map lookup; splitting a 48K program that fits comfortably would *add*
runtime cost for no benefit.

**Applicability.** **Not needed now.** Document as the escalation path IF a future
slice (revocation-coupled rotation, policy enforcement on the same hook, sockops
mTLS folded into the LB program) pushes a single program toward the ceiling. The
natural first split, mirroring Cilium, is forward-path vs reverse-path — which we
*already have* as two separate programs (`xdp_service_map_lookup` /
`xdp_reverse_nat_lookup`, per ADR-0045 §3, explicitly citing Cilium's
`bpf_lxc.c` / `bpf_overlay.c` shape). So we already apply the coarsest form of the
technique. Finer splitting (e.g. FIB+L2-rewrite as a tail-called sub-program
shared by both) is the next increment if needed.

**aya constraint.** aya 0.13.x ships `aya::maps::ProgramArray` and
`aya_ebpf::maps::ProgramArray` (per the coverage matrix in
`aya-rs-usage-comprehensive-research.md` §A.1) — tail calls are a *supported*
typed surface, no hand-rolling. `aya_ebpf::bpf_tail_call` is the kernel-side
primitive. So this is available if/when needed.

**Validation gate.** N/A unless adopted; if adopted, each tail-called program gets
its own verifier-budget baseline file + the existing per-program gate.

---

### Finding R-5: Cheaper modulo (fastrange/Lemire) — a real but SECONDARY runtime win; Cilium does NOT bother, and the prime is load-bearing for Maglev

Addresses the `% 16381` reduction in C-3 (forward path).

**Mechanism.** `slot = hash % 16381` is a modulo by a **prime**, which the JIT
cannot strength-reduce to a shift/mask (only power-of-2 moduli become `& (N-1)`).
It compiles to a real 32-bit (or 64-bit) division — one of the more expensive
single ops on a BPF JIT. Two cheaper alternatives:
1. **Lemire fastrange**: `slot = ((u64)hash * 16381) >> 32` — maps a uniformly-
   distributed 32-bit hash into `[0, 16381)` with one multiply + one shift, no
   division. Distribution is uniform for a uniform input hash.
2. **Power-of-2 table size** (e.g. 16384): `slot = hash & 16383` — one AND. But
   this changes the Maglev table size off a prime.

**Why the prime is load-bearing — and why Cilium keeps it anyway.** Maglev's
even-distribution + minimal-disruption guarantee depends on the lookup-table size
being **prime** (the permutation `(offset + i*skip) mod M` visits every slot iff
`M` is prime and `skip ∈ [1, M)`). Both Overdrive (`MaglevTableSize::DEFAULT =
16381`) and **Cilium (`pkg/maglev/maglev.go:38` — `DefaultTableSize = 16381`;
line 48 — every `maglevSupportedTableSizes` entry is prime: 251, 509, ..., 16381,
..., 131071)** use the identical prime 16381. **Cilium pays the prime modulo and
does not replace it with fastrange or a power-of-2** — `bpf/lib/lb.h:1929`:
`index = __hash_from_tuple_v4(...) % LB_MAGLEV_LUT_SIZE`. This is strong evidence
that the prime modulo is an accepted production cost, not a bottleneck worth the
distribution risk.

**Critical caveat: fastrange + Maglev is NOT a drop-in.** Maglev requires
`slot = hash mod M`; fastrange computes `floor(hash * M / 2^32)`, a *different*
mapping. Replacing `% M` with fastrange would change which slot each flow lands in
and, more importantly, change the relationship the userspace Maglev permutation
generator assumes (it populates slots `0..M-1` assuming `hash mod M` indexing).
You cannot swap the kernel reduction without re-deriving the whole Maglev
permutation math — and Maglev's disruption guarantee is proven for `mod`, not for
fastrange. **This is a research-grade change, not an optimization.**

**Estimated impact.** Runtime: removing one 32-bit division per *forward* packet
is a small, real latency win (a few cycles). Verifier: negligible (a div and a
mul-shift are similar instruction counts). **Net: low leverage, high coupling
risk.**

**Applicability.** **Recommend AGAINST** changing the reduction unless a Tier-4
xdp-perf measurement shows the `% 16381` division is a measurable pps bottleneck
(it is C-3, "low-to-mid," forward-path only — unlikely to dominate). If ever
pursued, it is a Maglev-algorithm change requiring its own research + the
`MaglevDistributionEven` / `MaglevDeterministic` invariants re-proven, not a
peephole edit. Cilium's choice to keep the prime is the recommendation to follow.

**Validation gate.** Tier-4 xdp-perf delta + both Maglev DST invariants if ever
adopted.

---

### Finding R-6: Cilium's verifier-state-reduction idioms — `map_array_get_32` bounded-array access + inline-asm operand pinning

Addresses cost centers C-4 (array indexing) and the diffuse C-6 (bounds
re-validation), and is the most *directly transferable* set of micro-techniques.

**Mechanism (the load-bearing one — `map_array_get_32`).** Cilium's bounded
array access (`bpf/include/bpf/access.h:9-32`) reads `array[index]` with the
index proven `< limit` using **inline assembly** so the verifier sees the bound
in a form LLVM cannot optimize away:

```c
asm volatile("%[index] <<= 2\n\t"               // index *= 4 (u32 stride)
             "if %[index] > %[limit] goto +1\n\t" // bound check the verifier trusts
             "%[array] += %[index]\n\t"           // pkt/map_reg += scalar (correct operand order)
             "%[datum] = *(u32 *)(%[array] + 0)\n\t"
             : [datum]"=r"(datum)
             : [limit]"i"(limit), [array]"r"(array), [index]"r"(index));
```

The comment (`access.h:18-22`) states the exact problem: "LLVM tends to optimize
code away that is needed for the verifier to understand dynamic map access." The
inline asm forces the `array += index` addition into the `reg += scalar` operand
order the verifier tracks, and the `if index > limit goto +1` is the bound the
verifier reads. This is how Cilium indexes the Maglev LUT
(`lb.h:1930`: `map_array_get_32(backend_ids, index, (LB_MAGLEV_LUT_SIZE - 1) <<
2)`) cheaply despite the dynamic `index`.

**Relevance to our HoM inner-ARRAY lookup (C-4).** Our forward path does
`bpf_map_lookup_elem(inner_ptr, &slot)` on the inner ARRAY (`xdp_service_map.rs:312`).
Cilium's `map_array_get_32` accesses the inner ARRAY *directly* (`map_lookup_elem`
to get the array base once, then a bounds-checked pointer index) — for a hot path
hit on every packet, the direct bounded-index access can be cheaper than a second
full `bpf_map_lookup_elem` helper call. **Flagged: needs measurement** — whether
our HoM second-lookup or a Cilium-style direct bounded index is cheaper on our
aya/kernel combo is unmeasured.

**Operand-ordering inline asm (the broader idiom).** The xdp-checksum research
(Findings 9, Approach E) already documents that Cilium uses `asm volatile` to pin
the `pkt_reg += scalar` operand order the verifier requires, and that
`core::arch::asm!` is available on `bpfel-unknown-none` in aya-ebpf (confirmed via
aya-ebpf's own `check_bounds_signed`). Our `csum.rs` already uses
`core::ptr::read_volatile` on `data`/`data_end` to prevent CSE — the next
increment is Cilium-style inline-asm bound pinning if the verifier count proves
sensitive to LLVM's bounds-check elision.

**Other verifier-state idioms observed in Cilium (transferable, lower-leverage):**
- **`__always_inline` everywhere** (their `static __always_inline` is universal) —
  matches our `#[inline(always)]` discipline; no change needed, already aligned.
- **`__builtin_constant_p` fast paths** (`csum.h:36-48`): branch to a cheap inline
  form when sizes/seeds are compile-time constants. Our `csum_diff_chunk::<N>`
  already exploits const generics for the same effect (constant `to_size`).
- **`if (unlikely(!ptr))`** branch hints on map-lookup NULL checks
  (`lb.h:1922,1926`) — `core::intrinsics::unlikely` / `likely` are available in
  Rust nightly (we build on nightly for `bpfel-unknown-none`); annotating the
  cold NULL-miss branches can help the JIT lay out the hot path. Low leverage,
  zero risk.
- **Reading each header field exactly once.** Cilium parses into a local struct
  once; our programs re-read several fields (sanity then body; pre-rewrite then
  post-rewrite for the FIB block). Hoisting reads into locals reduces C-6's
  diffuse bounds re-validation. Low-to-modest, zero risk, and the most "free" of
  the micro-wins.

**Estimated impact.** Verifier: collectively modest (these chip at C-4 + C-6, the
"low" and "diffuse" centers). Runtime: small. **None individually rivals R-1.**
But they are low-risk, locally-scoped, and directly lifted from production Cilium.

**Applicability.** All are aya 0.13.x / `#![no_std]` compatible. The inline-asm
forms (`map_array_get_32`-equivalent, operand pinning) carry the highest
implementation risk (Rust BPF inline asm is fragile per the xdp-checksum research
Approach E) and should be reserved for after a measurement shows the verifier
count is sensitive to LLVM bounds-check elision. The read-once hoisting and
`unlikely` hints are safe, immediate, low-yield wins.

**Validation gate.** Existing verifier-budget gate (`verified_insns ≤ 58074` /
`≤ 57818`) + the Tier-3 e2e; any inline-asm change needs per-kernel Tier-3
verification (the operand-ordering risk is kernel-version-sensitive per
xdp-checksum Gap 3).

## Cilium Techniques We Don't Use Yet (mapping table)

Each row maps a production Cilium / kernel technique to our programs, with the
finding that covers it and a verdict.

| Technique | Cilium / kernel source | Our programs | Use it? | Finding |
|---|---|---|---|---|
| **Incremental L4 csum via `csum_diff`-over-changed-fields** | `bpf/lib/nat.h:489,525,536`; `bpf/include/bpf/csum.h:36-48` | both XDP programs (`recompute_l4_csum`) | **YES — highest leverage**, gated on `tx off` at the LB veth | R-1 |
| **`jhash_3words` 5-tuple hash w/ port-packing** | `bpf/lib/hash.h:13-17`; `bpf/lib/jhash.h` | `xdp_service_map` (`fnv1a_5tuple_slot`) | YES — modest, low-risk, aligns with production; coupled to Maglev gen | R-2 |
| **`bpf_loop()` to avoid loop unrolling** | kernel 5.17+; absent from Cilium hot path | `recompute_l4_csum` 24-iter loop | **NO** — delete the loop (R-1), don't convert it | R-3 |
| **Tail-call program splitting** (`PROG_TYPE/tail`, `cilium_calls` prog-array) | `bpf/lib/tailcall.h:107-138` | both programs (already split fwd/rev) | NOT NOW — escalation path only; we already do the coarse fwd/rev split | R-4 |
| **Cheaper modulo (fastrange / power-of-2 LUT)** | Cilium keeps the prime `% 16381` (`bpf/lib/lb.h:1929`, `pkg/maglev/maglev.go:38`) | `% INNER_TABLE_SIZE` (16381) | **NO** — Cilium pays the prime too; Maglev needs prime; high coupling | R-5 |
| **`map_array_get_32` inline-asm bounded array index** | `bpf/include/bpf/access.h:9-32`; used at `bpf/lib/lb.h:1930` | HoM inner-ARRAY second lookup | MAYBE — measure first; inline-asm risk | R-6 |
| **Operand-ordering inline-asm pinning** | `bpf/include/bpf/ctx/xdp.h` (per xdp-checksum research F-9) | `csum.rs` packet-ptr access | RESERVE — only if verifier count proves LLVM-elision-sensitive | R-6 |
| **`__builtin_constant_p` const fast paths** | `bpf/include/bpf/csum.h:36-48` | already exploited via `csum_diff_chunk::<N>` const generics | ALREADY DO (equivalent) | R-6 |
| **`__always_inline` discipline** | universal in Cilium `bpf/lib/*.h` | `#[inline(always)]` everywhere | ALREADY DO | R-6 |
| **`unlikely()` branch hints on NULL-miss** | `bpf/lib/lb.h:1922,1926` | map-lookup NULL checks | YES — trivial, zero-risk, low-yield | R-6 |
| **Read each header field once into a local** | Cilium parses into a struct once | we re-read several fields (sanity+body, pre/post-rewrite) | YES — safe, "free", chips at C-6 | R-6 |
| **XDP only on physical NICs (avoid CHECKSUM_PARTIAL)** | `pkg/datapath/loader/xdp.go`; xdp-csum research F-3 | we run XDP on veth (architectural choice) | N/A — our model differs; R-1's `tx off` is the equivalent mitigation | R-1 |
| **TC + `bpf_l4_csum_replace` for veth** | `bpf/lib/csum.h:73-78` (skb-only) | XDP has no skb — helper unavailable | N/A — cannot transfer to XDP (skb-only) | R-1 |

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| OUR `xdp_service_map.rs` | (project, primary) | High (1.0) | source code | 2026-06-14 | Y (vs baseline file) |
| OUR `xdp_reverse_nat.rs` | (project, primary) | High (1.0) | source code | 2026-06-14 | Y |
| OUR `shared/csum.rs` (post-e16f1972) | (project, primary) | High (1.0) | source code | 2026-06-14 | Y |
| OUR `shared/sanity.rs`, `cgroup_connect4_service.rs` | (project, primary) | High (1.0) | source code | 2026-06-14 | Y |
| OUR `veristat-service-map.txt` / `veristat-reverse-nat.txt` / `veristat-cgroup-connect4-service.txt` | (project, primary) | High (1.0) | measured baselines | 2026-06-14 | Y (whole-program totals) |
| Cilium `bpf/lib/nat.h` (SNAT/DNAT csum fixup) | github.com/cilium/cilium | High (1.0) | upstream source (local) | 2026-06-14 | Y (vs nat.h + csum.h) |
| Cilium `bpf/lib/csum.h` + `bpf/include/bpf/csum.h` | github.com/cilium/cilium | High (1.0) | upstream source (local) | 2026-06-14 | Y |
| Cilium `bpf/lib/hash.h` + `bpf/lib/jhash.h` | github.com/cilium/cilium | High (1.0) | upstream source (local) | 2026-06-14 | Y |
| Cilium `bpf/lib/lb.h` (Maglev select, `map_array_get_32`) | github.com/cilium/cilium | High (1.0) | upstream source (local) | 2026-06-14 | Y |
| Cilium `bpf/include/bpf/access.h` (`map_array_get_32`) | github.com/cilium/cilium | High (1.0) | upstream source (local) | 2026-06-14 | Y |
| Cilium `bpf/lib/tailcall.h` (`cilium_calls` prog-array) | github.com/cilium/cilium | High (1.0) | upstream source (local) | 2026-06-14 | Y |
| Cilium `pkg/maglev/maglev.go` (`DefaultTableSize=16381`, prime sizes) | github.com/cilium/cilium | High (1.0) | upstream source (local) | 2026-06-14 | Y |
| `docs/research/.../xdp-checksum-partial-veth-research.md` | (project, prior research) | High (0.99 derived) | secondary synthesis (14 sources) | 2026-06-14 | Y |
| `docs/research/.../aya-rs-usage-comprehensive-research.md` | (project, prior research) | High (1.0 derived) | secondary synthesis (23 cites) | 2026-06-14 | Y |
| LWN — "Add bpf_loop helper" / "A different approach to BPF loops" | lwn.net | High (1.0) | industry reference | 2026-06-14 | Y (vs eBPF Docs) |
| eBPF Docs — Loops concept | docs.ebpf.io | High (1.0) | technical docs | 2026-06-14 | Y (vs LWN) |
| Daniel Lemire — "A fast alternative to the modulo reduction" + `lemire/fastrange` | lemire.me / github.com/lemire | High (1.0) / High (1.0) | primary author + reference impl | 2026-06-14 | Y (vs arxiv 1902.01961) |
| Lemire et al. — "Faster Remainder by Direct Computation" | arxiv.org/pdf/1902.01961 | High (1.0) | academic paper | 2026-06-14 | Y |
| kernel commit 7d672345ed29 (`bpf_csum_diff` arithmetic) | github.com/torvalds/linux | High (1.0) | kernel source (via prior research) | 2026-06-14 | Y |

**Reputation breakdown**: High (1.0): 19 of 19 distinct sources (100%). Average
reputation: **1.0**. All primary sources are official upstream source code
(Cilium = `github.com/cilium/cilium` read locally; kernel = torvalds/linux),
project primary artifacts, or authoritative industry/academic references (LWN,
eBPF Docs, Lemire/arxiv). Every load-bearing claim is cross-referenced ≥2 ways:
the incremental-csum claim by Cilium `nat.h` + `csum.h` + the prior xdp-checksum
research; the prime-Maglev claim by Cilium `lb.h` + `maglev.go` + OUR
`MaglevTableSize`; `bpf_loop` by LWN + eBPF Docs; fastrange by Lemire's blog +
the reference impl + the arxiv paper.

**Confidence: High** for the *mechanisms* and *applicability* (all backed by
primary source). **Medium** for the *quantitative impact estimates* — the
per-component cost shares and post-change verifier counts are NOT measured
(only whole-program totals exist); every number is flagged "needs measurement."

## Knowledge Gaps

### Gap G-1: No per-function / per-component verifier-instruction breakdown
**Issue**: The only hard data are whole-program totals at each commit (48395 /
48182, and the history in the baseline files). The decomposition of the residual
~48K across C-1..C-6 is *inferred from code shape*, not measured. So the impact of
removing any single cost center (especially R-1's removal of C-1) cannot be stated
as a number. **Attempted**: read the baseline-file history (gives deltas at each
architectural change — e.g. +551 for the FIB block at Slice 05-04, +150168 for the
pre-collapse full recompute), but post-collapse component shares are not isolated.
**Recommendation**: a measurement spike that builds variants (recompute removed;
FIB removed; hash swapped) and reads `verified_instruction_count()` per variant
would pin the shares. This is the prerequisite for asserting any reduction number.

### Gap G-2: R-1's incremental-csum byte-correctness on the pinned floor with `tx off`
**Issue**: The xdp-checksum research proves incremental works with FULL on-wire
checksums *in principle* (and Cilium does it on physical NICs), but it was written
before R-1 was contemplated and recommended full recompute. Whether `ethtool -K tx
off` reliably materialises a FULL L4 checksum before the XDP hook on **veth** on
kernel 6.18 (pinned floor) and 7.0 (dev VM), and whether the incremental
`csum_incremental_3_3`-shape is byte-identical to the recompute output, is
**unmeasured on our combo**. **Attempted**: prior research Approach F + Cilium
nat.h; both are strong but neither is our exact (XDP-on-veth + `tx off` +
incremental) combination. **Recommendation**: Tier-3 spike — the gating
experiment for R-1, before any production change.

### Gap G-3: Cilium's `bpf/complexity-tests/` directory is absent from this checkout
**Issue**: The research brief named `bpf/complexity-tests/` as the single
highest-value directory (Cilium's codified per-program complexity numbers + the
configs they tolerate). It is referenced in this checkout's `Makefile.defs`
(`BPF_SRCFILES_IGNORE = ... bpf/complexity-tests/% ...`) but the directory itself
is not present (sparse/partial clone, or removed in this revision). **Attempted**:
glob + grep across the whole `bpf/` tree and repo root — no `complexity-tests`
files, no `*complexity*` files. **Impact**: I could not mine Cilium's *exact*
tolerated per-program instruction numbers or their complexity-test harness configs
(the brief's hoped-for "per-program complexity numbers they tolerate"). The
techniques themselves (tail calls, incremental csum, jhash, `map_array_get_32`)
were all recoverable from `bpf/lib/` + `bpf/include/` source, so the *applicable
changes* are not gated on this gap — only the comparative tolerance numbers are.
**Recommendation**: if the comparative numbers matter, fetch
`github.com/cilium/cilium/tree/main/bpf/complexity-tests` directly (web) or
re-clone with that path; or accept that our 9.7%-of-target headroom makes the
comparison moot.

### Gap G-4: Whether a Cilium-style direct bounded-array index beats our HoM second lookup on our combo
**Issue**: R-6 notes `map_array_get_32` (direct bounded index into the inner ARRAY)
*may* be cheaper than our second `bpf_map_lookup_elem` on the HoM inner map, but
this is unmeasured. **Attempted**: read both shapes (Cilium `access.h` + our
`xdp_service_map.rs:312`). **Recommendation**: fold into the G-1 measurement spike.

## Recommended Sequencing

A measure-first ordering that front-loads the cheap/safe wins and gates the
high-leverage change behind a spike.

1. **First, close Gap G-1 with a measurement spike** (a few hours, Tier-4):
   build variants and read `verified_instruction_count()` per cost center so every
   subsequent change has a number, not an estimate. This is the prerequisite for
   honest impact claims and is itself zero-risk (no production change).

2. **Land R-6's free wins immediately** (zero-risk, no spike needed): hoist
   header reads into locals (read-once), add `unlikely()` hints on NULL-miss
   branches. Re-measure to confirm the (small) delta and to exercise the gate.

3. **Spike R-1 (Gap G-2) — the decision point** (Tier-3 spike, the big one):
   in a branch, disable TX offload on the LB veth, implement incremental L4 csum
   (`csum_incremental_3_3`-shape), and run the
   `real_tcp_connection_completes_through_vip_with_payload_echo` e2e with `tx off`
   on kernel 7.0 *and* 6.18. Add the negative test (e2e with `tx on` must fail).
   Read the new `verified_instruction_count()`. **Surface the `tx off` operational
   invariant to the user as a decision before landing** — it changes the attach
   contract. If green + the user accepts the tradeoff, this is the highest-impact
   change available. If the user rejects the operational dependency, R-1 is closed
   and the residual stays ~48K (which is fine — no cliff).

4. **R-2 (jhash) only if R-1's measurement shows C-3 matters** and the
   Maglev-lockstep coupling is acceptable. It is a kernel+userspace+table change;
   sequence it after R-1 so the bigger win lands first and the hash swap is
   measured against the post-R-1 residual.

5. **R-4 (tail calls), R-5 (fastmod), R-6 inline-asm forms: do NOT pursue now.**
   Document R-4 as the ceiling-escalation path; R-5 as a Maglev-algorithm research
   item gated on a Tier-4 perf finding; R-6 inline-asm gated on G-1 showing
   LLVM-elision sensitivity.

**Net**: step 2 is free; step 3 is the one that matters and needs a spike + a user
decision; everything else is conditional polish.

## Full Citations

[1] Cilium Authors. "bpf/lib/nat.h — `snat_v4_rewrite_headers`, incremental csum
fixup via `csum_diff` + `csum_l4_replace`". github.com/cilium/cilium (read locally
at /Users/marcus/git/cilium/cilium). Accessed 2026-06-14.

[2] Cilium Authors. "bpf/lib/csum.h + bpf/include/bpf/csum.h — `csum_l4_replace`,
`csum_diff` constant-size inline fast path". github.com/cilium/cilium. Accessed
2026-06-14.

[3] Cilium Authors. "bpf/lib/hash.h + bpf/lib/jhash.h — `__hash_from_tuple_v4`,
`jhash_3words`". github.com/cilium/cilium. Accessed 2026-06-14.

[4] Cilium Authors. "bpf/lib/lb.h — `lb4_select_backend_id_maglev`, `% LB_MAGLEV_LUT_SIZE`,
`map_array_get_32`". github.com/cilium/cilium. Accessed 2026-06-14.

[5] Cilium Authors. "bpf/include/bpf/access.h — `map_array_get_32` inline-asm
bounded array access". github.com/cilium/cilium. Accessed 2026-06-14.

[6] Cilium Authors. "bpf/lib/tailcall.h — `cilium_calls` PROG_ARRAY, `__declare_tail`,
`tail_call_internal`, `CILIUM_CALL_*` indices". github.com/cilium/cilium. Accessed
2026-06-14.

[7] Cilium Authors. "pkg/maglev/maglev.go — `DefaultTableSize = 16381`,
`maglevSupportedTableSizes` (all prime)". github.com/cilium/cilium. Accessed
2026-06-14.

[8] Overdrive Project. "docs/research/dataplane/xdp-checksum-partial-veth-research.md"
(CHECKSUM_PARTIAL-on-veth constraint, Approach F = `ethtool -K tx off`, Katran/Cilium
csum approaches). Accessed 2026-06-14.

[9] Overdrive Project. "docs/research/dataplane/aya-rs-usage-comprehensive-research.md"
(aya 0.13.x coverage matrix: ProgramArray supported, bpf_loop absent; hand-rolled
helper precedent). Accessed 2026-06-14.

[10] Overdrive Project. "perf-baseline/main/verifier-budget/veristat-service-map.txt,
veristat-reverse-nat.txt, veristat-cgroup-connect4-service.txt" (measured
whole-program baselines + delta history). Accessed 2026-06-14.

[11] Overdrive Project. "crates/overdrive-bpf/src/{programs/xdp_service_map.rs,
programs/xdp_reverse_nat.rs, shared/csum.rs, shared/sanity.rs,
programs/cgroup_connect4_service.rs}". Accessed 2026-06-14.

[12] Joanne Koong / LWN. "Add bpf_loop helper". lwn.net. 2021.
https://lwn.net/Articles/877170/. Accessed 2026-06-14.

[13] Jonathan Corbet / LWN. "A different approach to BPF loops". lwn.net. 2021.
https://lwn.net/Articles/877062/. Accessed 2026-06-14.

[14] eBPF Docs Authors. "Loops — eBPF Docs". docs.ebpf.io.
https://docs.ebpf.io/linux/concepts/loops/. Accessed 2026-06-14.

[15] Daniel Lemire. "A fast alternative to the modulo reduction". lemire.me. 2016.
https://lemire.me/blog/2016/06/27/a-fast-alternative-to-the-modulo-reduction/.
Accessed 2026-06-14.

[16] Daniel Lemire. "lemire/fastrange — A fast alternative to the modulo reduction"
(reference implementation). github.com/lemire/fastrange. Accessed 2026-06-14.

[17] Lemire, Kaser, Kurz. "Faster Remainder by Direct Computation". arxiv.org.
2019. https://arxiv.org/pdf/1902.01961. Accessed 2026-06-14.

[18] Daniel Borkmann. "bpf: add generic bpf_csum_diff helper" (kernel commit
7d672345ed29 — `bpf_csum_diff` is purely arithmetic, not ip_summed-aware).
github.com/torvalds/linux. 2016. Accessed 2026-06-14 (via [8]).

[19] Cilium Authors. "Makefile.defs — `BPF_SRCFILES_IGNORE` references
`bpf/complexity-tests/%`" (evidence the complexity-test harness exists upstream;
directory absent from this checkout — see Gap G-3). github.com/cilium/cilium.
Accessed 2026-06-14.

## Research Metadata

Duration: ~single session | Examined: 19 distinct sources (12 primary source
files in OUR code + Cilium, 2 prior project research docs, 5 external
authoritative refs) | Cited: 19 | Cross-refs: every load-bearing claim ≥2 |
Confidence: mechanisms/applicability High; quantitative impact Medium (flagged
needs-measurement) | Output:
docs/research/dataplane/bpf-verifier-complexity-and-perf-optimization-research.md
