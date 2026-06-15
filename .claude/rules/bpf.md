---
paths:
  - "crates/overdrive-ebpf/**/*"
  - "crates/overdrive-dataplane/**/*"
  - "crates/overdrive-worker/**/*"
---
# eBPF Optimization Discipline

How to keep the kernel-side BPF programs in `overdrive-bpf` **cheap for
the verifier and fast on the wire** — the reduction techniques, the
traps, and the budget gate.

This file is the SSOT for *making an already-correct BPF program
cheaper*. It does **not** restate:

- **Authoring / correctness idioms** — `ptr_at` bounds-checking, header
  parsing, return codes, the error-wrapper, map access, `no_std`, attach
  mode, HASH_OF_MAPS, `pinning = ByName`, and the "verifier-friendly
  idioms — what to avoid" (no unbounded loops, no recursion, no FP, no
  alloc) live in `.claude/rules/development.md` § "aya-rs XDP / TC
  kernel-side patterns". That section is about writing a program the
  verifier *accepts*; this file is about writing one it accepts
  *cheaply*.
- **The test tiers + gates** — Tier 2 (BPF unit / triptych), Tier 3
  (real-kernel matrix, per-hook coverage, Lima execution), Tier 4
  (`cargo verifier-regress` + `xdp-perf`) live in
  `.claude/rules/testing.md`. This file references the Tier-4 gate;
  it does not own its mechanics.
- **The full ranked technique catalogue + Cilium evidence** —
  `docs/research/dataplane/bpf-verifier-complexity-and-perf-optimization-research.md`.

The rules below are extracted from the Phase-2 dataplane
checksum-optimization arc (commits `e16f1972` → `62fa6be2` →
`e33d6c5d`), which cut `xdp_service_map_lookup` and
`xdp_reverse_nat_lookup` from ~151K / ~156K verified instructions to
**1,356 / 1,180** (−99%) with no loss of correctness.

---

## The verified-instruction count is a state-walk, not code size

`bpf_prog_info.verified_insns` (what `cargo verifier-regress` and aya's
`ProgramInfo::verified_instruction_count()` report) is the number of
instructions the **kernel verifier walked while validating the
program** — and the verifier **unrolls every bounded loop** in its
state walk. A single 750-iteration loop body balloons to ~150K
verified instructions while its JIT'd machine code stays compact
(<500 B). The number is a *verifier-complexity* measure, **not** binary
size and **not** per-packet runtime cost.

The lever, therefore, is almost never "write less code." It is **stop
the verifier from walking a large bounded loop** — by eliminating the
loop (Rules 1, 5), or, when a loop is unavoidable, bounding its
unrolled cost (Rule 4) or splitting the program (tail calls). Reach for
the budget gate's *number* only to confirm a reduction, never as the
thing you optimize directly.

---

## Rule 1 — Fix checksums incrementally; never recompute over the payload

**After a NAT-style header rewrite (address / port), update the L3 and
L4 checksums from the folded delta of ONLY the changed fields (RFC
1624) — an O(1) handful of instructions. Never walk the packet payload
to recompute a checksum from scratch.**

A full-payload checksum walk is a bounded loop over the L4 segment
(`for word in 0..l4_len/2`), which the verifier unrolls to ~150K
instructions at a 1500-byte MTU. The incremental delta touches only the
4-byte address and the 2-byte port that actually changed, so the
verifier sees a fixed handful of folds.

- **Codebase shape:** `csum_incremental_2_2` (IP header — two changed
  16-bit words) and `csum_incremental_3_3` (L4 — the address pair plus
  the port word) in `crates/overdrive-bpf/src/shared/csum.rs`. Both
  fold big-endian wire fields directly (see Rule 3).
- **Precedent:** `62fa6be2` replaced the full-payload recompute with
  the incremental fold: `xdp_service_map_lookup` 48,395 → **1,356**,
  `xdp_reverse_nat_lookup` 48,182 → **1,180**. The chunked
  full-recompute it deleted was `e16f1972`.
- **Cilium precedent:** production SNAT/DNAT
  (`/Users/marcus/git/cilium/cilium/bpf/lib/nat.h`) is
  `csum_diff(&old_addr, 4, &new_addr, 4, seed)` chained with the port
  delta — it **never walks the payload**.

**Symptom during review:** a `for`/`while` loop summing packet bytes on
a rewrite/NAT path. The rewrite changed a few fields; the checksum
fixup should be incremental, not a payload walk.

---

## Rule 2 — Incremental checksum needs a valid ingress base → the `tx off` invariant

**Incremental checksum update (RFC 1624) requires the incoming packet's
checksum to already be valid.** On a `veth` pair, TX-checksum-offload
delivers `CHECKSUM_PARTIAL` — the checksum is deferred, not computed —
so there is **no valid base** and the incremental delta produces a
wrong checksum that compiles, verifies, and **silently drops every
packet**.

The invariant that makes Rule 1 correct: **`ethtool -K <iface> tx off`
on BOTH ends of every LB `veth` pair.** It is wired as a
converge-on-boot step, not left to chance:

- `VethStep::DisableClientTxOffload` / `DisableBackendTxOffload` in
  `crates/overdrive-control-plane/src/veth_provisioner.rs` — idempotent
  observe → diff → converge (observes via `ethtool -k`, acts via
  `ethtool -K tx off`, repairs drift, refuses boot on a non-benign
  failure). Commit `e33d6c5d`.
- The Tier-3 fixtures set the same `tx off` (`crates/overdrive-testing/src/netns.rs`).

**Physical-NIC ingress is already fine** — a real remote sender
computes a valid checksum on the wire, and XDP sees the raw bytes. The
`veth` `CHECKSUM_PARTIAL` short-circuit is the *only* gap, which is why
the invariant is veth-provisioner-scoped. This is standard L4-LB
practice (Cilium runs XDP on full-checksum NICs for the same reason).

This is a **platform-owned operational invariant** (we ship the
appliance OS and provision the veths), not an operator-tunable knob.

**Symptom during review:** an incremental-checksum change that lands
without a corresponding `tx off` guarantee on the interface the program
runs on. Correct under test (fixtures set it) but corrupts every packet
in production until provisioning sets it too.

---

## Rule 3 — Checksum byte-order domain: pick one, never mix

**`bpf_csum_diff` / `csum_partial` accumulate in a different byte-order
domain than manual `from_be_bytes` folding. Mixing the two in one
checksum silently byte-swaps the result** — it compiles, it verifies,
and it drops packets.

Prefer folding the big-endian wire fields **directly** (no
`bpf_csum_diff`) so the compute side and the write side
(`write_u16_be`) share one domain. That is exactly what the
`csum_incremental_*` helpers do, and it is why the incremental path
(Rule 1) needs no `bpf_csum_diff` and no final `swap_bytes`.

**The oracle: verifier-accept ≠ correct.** A wrong checksum passes the
verifier and the budget gate. The only reliable proof of checksum
correctness is a **real-packet e2e** — the Tier-3
`reverse_nat_e2e::real_tcp_connection_completes_through_vip_with_payload_echo`
and the UDP-echo tests, where a wrong checksum makes the peer drop the
segment and the echo never completes. The `e16f1972` chunked refactor
hit exactly this byte-swap trap; it was caught by the e2e, never by the
verifier.

---

## Rule 4 — Variable-length `bpf_csum_diff` is rejected → chunk by constant powers of two (a fallback, not the default)

When you genuinely must sum a **variable-length** span of packet data
through `bpf_csum_diff` (not the incremental case), the verifier
**rejects a runtime `to_size`** — it cannot track the packet pointer
across a variable-length helper read (aya-rs/aya#1562). The workaround:
process the span in **fixed power-of-two chunks** (64 → 32 → 16 → 8 → 4
bytes), so every `bpf_csum_diff` call's `to_size` is a compile-time
constant. `to_size` must be a multiple of 4; handle the trailing 1–3
bytes by hand.

**This is a fallback, not the default.** Incremental-over-changed-fields
(Rule 1) beats it whenever the rewrite touched only a few fields — which
is the NAT case. The chunked engine was *deleted* in `62fa6be2` once the
`tx off` invariant (Rule 2) unlocked the incremental path. Reach for
chunking only when you must sum the whole variable-length payload AND
genuinely cannot do it incrementally.

---

## Rule 5 — Read each header field once

Hoist repeated reads of the same packet offset into a local so each
field is read — and bounds-checked — exactly once, rather than re-read
per use. Behaviour-preserving, free, and it trims redundant verifier
state. Cilium reads each header field once into a local. (Applied
alongside Rule 1 in `62fa6be2`: the dead `ip_total_len` read — which
existed only to bound the deleted payload walk — was removed.)

---

## What does NOT help — don't reach for these reflexively

These were investigated against our programs and **rejected**; recorded
so they are not re-litigated (full analysis in the research doc, R-3 /
R-5).

- **`bpf_loop()` (kernel ≥ 5.17).** It replaces a verifier-*unrolled*
  bounded loop with a single verifier-cheap callback — but it is only
  worth it for a *genuinely large* loop. For a small loop (≤ a few
  dozen iterations) the right move is to **eliminate** the loop (e.g.
  incremental csum), not convert it. Cilium does not use `bpf_loop` in
  the hot path.
- **Cheaper modulo (Lemire fastrange / fastmod) for Maglev `% 16381`.**
  The *prime* table size is load-bearing for Maglev's distribution;
  fastrange is not a drop-in for `mod`-based Maglev; Cilium pays the
  identical prime modulo. Low leverage, high coupling.

---

## Tail-call splitting — the ceiling-escape path

When a single program approaches the verifier's 1M ceiling (or the
project's 50% / 500K target), split it into a chain of tail-called
sub-programs, each verified independently under the per-program limit
(Cilium `bpf/lib/tailcall.h`). We already do the coarse
forward/reverse split (ADR-0045 § 3). This is the structural escape
when per-program reduction (Rules 1–5) is not enough — reach for it
**before** raising the budget.

---

## The verifier-budget gate + re-baseline discipline

The gate mechanics live in `.claude/rules/testing.md` § "Tier 4 —
Verifier and Performance Gates" (per-program baselines in
`perf-baseline/main/verifier-budget/veristat-<name>.txt`, the +20% PR
delta, `cargo verifier-regress` reading `verified_insns` via aya, and
the inline `within_20pct_of_baseline` integration tests). Two
disciplines specific to *changing* a baseline:

- **Re-baseline = append, never edit.** When a justified change moves
  the count, append a new history entry to the `veristat-<name>.txt`
  file (and update the machine-parsed `verified_insns=` line + the gate
  bound + any inline `const BASELINE`). **Never edit a prior history
  entry** — that collapses the audit trail that lets a future reader
  see how the program's cost evolved. Record the *why* and the dev-VM
  kernel version the count was measured on.
- **Counts vary across kernels — never gate on absolute numbers
  cross-kernel.** The verifier's accounting changes between kernel
  releases. The dev Lima VM (currently 7.0) and the pinned-6.18 Tier-3
  matrix produce *different* counts for the same program; the
  **pinned-6.18 matrix is the authoritative merge signal** (ADR-0068).
  A dev-VM count drifting over a baseline that was captured on a
  *different* kernel is environmental, not a regression — re-measure on
  the gating kernel before treating it as real. (Precedent: the
  151379 baseline was captured on kernel 6.8; the dev VM's advance to
  7.0 accounted the same program at 186328, which read as a "failure"
  that was pure kernel drift.)

---

## Don't raise the budget to dodge a reduction

When a verifier-budget test fails, the first question is **"can this
program be made cheaper?"** (Rules 1–5, then tail-call splitting) — not
"raise the baseline." Raising the budget is correct only for (a) a
genuine, justified, one-time architectural growth that the ACs call
for, or (b) a re-baseline against a changed *gating* kernel. The
checksum arc is the precedent: the failing 48K / 150K numbers were
driven **down** to ~1.3K, not bumped up. A budget increase in a PR
whose change did not architecturally grow the program is a review
smell.

---

## Symptoms during review

- A `for`/`while` loop summing packet bytes on a NAT / header-rewrite
  path → should be incremental (Rule 1).
- An incremental-checksum change with no `tx off` guarantee on its
  interface → silent production packet corruption (Rule 2).
- `bpf_csum_diff` mixed with `from_be_bytes` folding inside one checksum
  computation → the byte-swap trap (Rule 3).
- `bpf_csum_diff` called with a runtime (non-`const`) `to_size` → the
  verifier will reject it (Rule 4).
- A verifier-budget **baseline bump** in a PR whose change didn't grow
  the program architecturally → it should have been a reduction
  ("don't raise the budget").
- A re-baseline that **edits a prior** `veristat-<name>.txt` history
  entry instead of appending → collapses the audit trail.
- A `bpf_loop` conversion (or a fastmod swap) proposed for a small loop
  / the Maglev modulo → low-leverage; see "What does NOT help".

---

## Cross-references

- `.claude/rules/development.md` § "aya-rs XDP / TC kernel-side
  patterns" — authoring/correctness idioms; the "verifier-friendly
  idioms — what to avoid" list (the *acceptance* rules this file's
  *cheapness* rules build on).
- `.claude/rules/testing.md` § "Tier 2 — BPF Unit Tests", § "Tier 3 —
  Real-Kernel Integration", § "Tier 4 — Verifier and Performance Gates"
  — the BPF test tiers + the verifier/perf gates this file references.
- `.claude/rules/debugging.md` § "Real-kernel debugging — `pwru`" and
  "Leftover XDP attachments across runs" — when a BPF change misbehaves
  on a real kernel.
- `docs/research/dataplane/bpf-verifier-complexity-and-perf-optimization-research.md`
  — the full ranked technique catalogue, Cilium evidence, and the
  "needs measurement" gaps (G-1 no per-component verifier breakdown;
  G-2 the R-1 byte-correctness spike).
- `docs/research/dataplane/aya-rs-usage-comprehensive-research.md` —
  HASH_OF_MAPS, `prog_test_run`, and aya 0.13.x map-type coverage.
- External: RFC 1624 (incremental Internet checksum); aya-rs/aya#1562
  (variable-length `bpf_csum_diff` rejection); Cilium
  `bpf/lib/{nat,csum,hash,tailcall}.h`.
- Commits: `e16f1972` (chunked `bpf_csum_diff`, superseded), `62fa6be2`
  (incremental L4 csum + read-once), `e33d6c5d` (the `tx off`
  converge-on-boot step).
