# Root Cause Analysis — verifier-regress gate failure: `cgroup_connect4_service` +7.69% growth

**Date**: 2026-06-03
**Analyst**: Rex (nw-troubleshooter)
**Status**: Root cause identified — two causes, multi-causal
**Trigger**: CI `verifier-regress` gate

```
prog=cgroup_connect4_service verified_insns=28
gate failed — verifier-budget regression detected
  • cgroup_connect4_service — verified_insns: baseline=26, measured=28, growth=7.69% (threshold > 5%)
```

---

## Summary

The breach is **not a code regression**. The 2-instruction growth (26 → 28)
is the legitimate, intended cost of widening `LOCAL_BACKEND_MAP`'s key from
`(vip, port)` to `(vip, port, proto)` across roadmap steps 02-01 / 02-02. Two
independent root causes combined to surface it as a gate failure:

1. **Stale baseline** — the feature commit that added the work
   (`0876de79`, step 02-02) did not re-baseline
   `veristat-cgroup-connect4-service.txt`. The gate is correctly reporting
   "the program changed and nobody updated the baseline."
2. **Pure-relative growth gate with no absolute floor** — the 5% growth
   threshold is calibrated for the ~150K-insn XDP programs (5% ≈ 7 500 insns
   of headroom). For the ~26-insn cgroup program the same 5% is **1.3
   instructions** of headroom: a single legitimately-added instruction is
   3.8%, two is 7.7%. The gate is structurally guaranteed to trip on every
   future touch of this tiny program, regardless of correctness.

The complete fix addresses both: re-baseline to 28 (clears the breach), AND
add an absolute-delta floor to the growth gate (closes the recurring trap).

---

## Evidence

### What changed

`git diff cd5b1644 HEAD` on the program + map (baseline was recorded at
`cd5b1644`, value 26):

`crates/overdrive-bpf/src/programs/cgroup_connect4_service.rs`:
```rust
+    let proto = unsafe { (*sock_addr).protocol };
...
+    #[allow(clippy::cast_possible_truncation)]
+    let proto_byte = proto as u8;
-    let key = LocalServiceKey { vip_host, port_host, _pad: 0 };
+    let key = LocalServiceKey { vip_host, port_host, proto: proto_byte, _pad: 0 };
```

`crates/overdrive-bpf/src/maps/local_backend_map.rs` — key widened
`(vip, port)` → `(vip, port, proto)`, `_pad: u16` → `proto: u8 + _pad: u8`,
8-byte envelope preserved (`const _: () = assert!(size_of::<LocalServiceKey>() == 8)`).

These are the commits, in order:
- `cd5b1644` — added the program; baseline recorded at **26**.
- `12611316` (step 02-01), `0876de79` (step 02-02) — proto-keying;
  **+2 verified insns** (read `bpf_sock_addr.protocol`, truncate, store in key).
- Neither 02-01 nor 02-02 touched `veristat-cgroup-connect4-service.txt`.

The +2 is the proto read/truncate/store. It is correct and intended (IPVS-style
proto-keyed local backends per ADR-0053 rev 2026-06-03).

### The gate has no absolute floor

`crates/overdrive-dataplane/bin/verifier_budget.rs::evaluate`:
```rust
let growth_fraction =
    if baseline.verified_insns == 0 { 0.0 } else { (measured_f - baseline_f) / baseline_f };
if growth_fraction > policy.max_growth_fraction {   // 0.05, pure relative
    breaches.push(Breach { kind: BreachKind::GrowthExceeded { .. } });
    continue;
}
```
`max_growth_fraction: 0.05` is the only growth knob. `(28-26)/26 = 0.0769 > 0.05`.
There is no `max_growth_insns` floor. For any baseline `b`, the smallest
breaching delta is `ceil(0.05·b)+? ` — at `b=26` that is **2 instructions**.

The baseline file's own header is the tell:
> verifier complexity is essentially free (≪ 10% of the per-program ceiling)
> Recorded value: 26 instructions — 0.005% of the L1-cache-fits target

The **ceiling-proximity** gate (10% of 1M) has ~999 950 insns of headroom for
this program; the **growth** gate has 1.3. The two gates disagree by five
orders of magnitude on how much this program is allowed to move.

---

## 5 Whys

**Problem**: CI `verifier-regress` fails on `cgroup_connect4_service`, +7.69%.

- **Why 1** — Why did the gate fail? Measured 28 > baseline·1.05 = 27.3.
- **Why 2** — Why is measured 28 not 26? Steps 02-01/02-02 added a proto read +
  truncate + key store (+2 insns) to proto-key the local backend map.
- **Why 3a** *(stale-baseline branch)* — Why didn't that pass the gate as
  intended work? The feature commit didn't re-baseline the file; the gate
  can't distinguish "intended growth, baseline not updated" from "regression."
  - **Why 4a** — Why wasn't the baseline updated? The DELIVER step's quality
    gate ran on macOS (`--no-run`); the verified-insns count is only produced
    by a real Lima/CI load, so the +2 was invisible at commit time and there
    was no step checklist item to re-measure + re-baseline after a BPF-program
    change.
  - **Why 5a** — **ROOT CAUSE A**: re-baselining is a manual, easily-skipped
    step with no enforcement; a kernel-side program can change without the
    baseline file being touched in the same commit.
- **Why 3b** *(gate-sensitivity branch)* — Why is a correct +2-insn change a
  gate failure at all? The growth gate is purely relative (5%).
  - **Why 4b** — Why does 5% relative fail here but not on the XDP programs?
    5% of 150K is 7 500 insns; 5% of 26 is 1.3. The threshold was sized for
    large programs and never floored for small ones.
  - **Why 5b** — **ROOT CAUSE B**: the growth gate has no absolute-delta floor,
    so its effective sensitivity scales inversely with program size — tightest
    exactly where the program is cheapest and the signal is noisiest.

---

## Root causes

**A — Stale baseline (process).** A BPF-program-affecting change landed without
re-baselining the verifier-budget file in the same commit. The breach is the
gate correctly flagging an un-acknowledged measurement change.

**B — Gate has no absolute floor (structural / meta).** The 5% relative growth
threshold, with no `max_growth_insns` floor, makes the tiny
`cgroup_connect4_service` program un-editable: any single added instruction is
≥3.8%, any two ≥7.7%. This recurs on every future touch and is a framework-level
defect in the gate's policy, not in the program.

---

## Recommended actions

### Fix A (immediate, unblocks CI) — re-baseline to 28
Update `perf-baseline/main/verifier-budget/veristat-cgroup-connect4-service.txt`:
`verified_insns=26` → `verified_insns=28`, and amend the header comment to note
the proto-key widening (ADR-0053 rev 2026-06-03, steps 02-01/02-02) as the
reason for the +2. The measured value 28 is authoritative (it is the CI
measurement). This is a known-value edit; no new Lima run is required to obtain
the number, though the gate re-run on CI confirms it.

### Fix B (structural, prevents recurrence) — add an absolute-delta floor
Add `max_growth_insns` to `BudgetPolicy` (e.g. **50**) and change the growth
condition to breach only when **both** the relative AND absolute thresholds are
exceeded:
```rust
let growth_insns = candidate.verified_insns.saturating_sub(baseline.verified_insns);
if growth_fraction > policy.max_growth_fraction
    && growth_insns > policy.max_growth_insns {
    // breach
}
```
This keeps the 5% relative gate fully effective for the 150K XDP programs
(where any breaching relative growth is thousands of insns, far above a
50-insn floor) while making sub-100-insn programs immune to noise-level deltas.
Add a unit test in `verifier_budget_gate.rs` pinning: `(baseline=26,
measured=28)` → Pass; `(baseline=150_000, measured=160_000)` → Fail.

> **Fix B is tracked in [#201](https://github.com/overdrive-sh/overdrive/issues/201)**
> (verifier-budget growth gate: add absolute-delta floor). It is a
> framework/gate-policy change deferred to that issue, not actioned in this RCA.

### Backward-chain validation
- Fix A alone: clears today's breach, but ROOT CAUSE B survives — the next +1
  insn breaches again at `(29-28)/28 = 3.6%`. Insufficient on its own.
- Fix B alone: does not clear today's breach (28 > 26 by 2 insns is still a real
  measurement change the baseline should record). Insufficient on its own.
- **A + B together**: clears the breach AND removes the recurring trap. The
  combination addresses both root causes. ✔

---

## Meta-improvement flag

ROOT CAUSE B is a **meta-improvement** (nWave / project gate-policy level): the
verifier-budget gate's growth policy should carry an absolute floor so its
sensitivity does not scale inversely with program size. Tracked in
[#201](https://github.com/overdrive-sh/overdrive/issues/201).
