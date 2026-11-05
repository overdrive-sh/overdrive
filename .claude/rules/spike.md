# Spike Discipline

How throwaway spike/probe code is written, isolated, and run. Extracted from the
transparent-mtls-enrollment spike arc (GH #236), where dispatches violated each of
these before they were codified.

This governs the **PROBE phase of `/nw-spike`** and any ad-hoc throwaway probe.
The wave mechanics (probe → promotion gate → walking skeleton) live in the
`nw-spike` skill; this file is the SSOT for *how probe code is written and run*.

---

## Spike code is throwaway and ISOLATED — never in `crates/`

Probe code lives in `spike-scratch/{increment-a,increment-b,…}/`
directory, **self-contained** (its own `Cargo.toml` / workspace), and **never
touches production source**:

- **NEVER** create or modify a file under `crates/`. No new modules, no `mod.rs`
  wiring, no `[[bin]]` in any workspace member's `Cargo.toml`. The probe is a
  standalone build under `spike-scratch/`; its own `Cargo.toml` MAY depend on
  workspace crates by path (`overdrive-core = { path = "../../crates/overdrive-core" }`)
  so it can exercise real production components.
- **Probe sources, scripts and captured evidence ARE committed** and live until the
  implementation supersedes them (user ruling 2026-08-11, reversing the prior
  never-commit rule). Build output is not: `.gitignore` carries
  `spike-scratch/*/target/` and `spike-scratch/*/out/`.
  **Why the reversal:** `findings.md` quotes only extracts, so the probe and its raw
  captures are the sole record of *how* a measured claim was obtained. The
  microvm-driver spike ran 14 probes across 11 increments whose evidence repeatedly
  overturned earlier conclusions — twice catching confidently-wrong negatives and once an
  impossible positive — and none of that is reconstructible from the extracts alone. The
  probe is still **throwaway**: it is not production code, it is not a test tier, nothing
  builds or gates on it, and it is deleted when the implementation it validated lands.
- One increment per probe attempt: `increment-a`, `increment-b`, … Preserve prior
  increments as evidence; don't overwrite.
- If a probe needs a helper or component that lives in `crates/`, **link the
  crate by path** from the probe's own `Cargo.toml` — never edit the original,
  and never make the probe a member or dependency of any workspace crate.

**Why:** a spike is a *disposable validation of one assumption*, not a feature
increment. Code written into `crates/` (even reverted later) pollutes production,
drags the build chain, and blurs "what ships." **If a dispatch does pollute
`crates/`, MOVE the files into `spike-scratch/` (preserve the work) and revert the
production wiring — do not delete the work.**

## eBPF in spikes is aya-rs Rust — never C

The whole codebase is Rust; eBPF is **aya-rs** (`no_std`, `aya-ebpf` macros) —
model on `crates/overdrive-bpf/src/programs/*.rs`. **No C, no `.bpf.c`, no libbpf,
no `vmlinux.h`, no `clang`-bpf, no `bpftool gen skeleton`, no `driver.c`.**
Dataplane steering in a spike uses the same primitives production uses: BPF via
aya, nft / TPROXY via the `nft` / `ip` CLI (as `install_inbound_tproxy` does),
`IP_TRANSPARENT` + `getsockname` in Rust. Generic eBPF terminology
(`cgroup/connect4`, `bpf_sk_storage`, `SO_ORIGINAL_DST`) defaults a crafter to C
unless **aya-rs/nft Rust** is pinned in the dispatch — pin it.

## Run the probe FOR REAL — under Lima, no compile-only gate

The verdict rests on a real exercise on the real kernel, not "it compiled":

- Run inside the `overdrive` Lima VM, **as root**: `cargo xtask lima run -- …`
  (routes to root + re-injects PATH/target dir) or `limactl shell overdrive`.
- **No `--no-run` / compile-only gate** — it proves nothing about runtime
  behaviour.
- Especially load-bearing where there is **no Tier-2 `BPF_PROG_TEST_RUN`
  backstop** — `cgroup_sock_addr` / `cgroup_sockopt` programs return `ENOTSUPP`,
  and netns / nft / routing mechanisms have no synthetic harness — so **only a
  real `connect()` through a real cgroup/netns on the kernel is an honest
  signal.**
- Record `uname -r` in the findings; the verdict is pinned to a kernel (the
  pinned appliance kernel is Linux 7.0 — ADR-0068; a dev Lima or metal kernel
  of another version is not that signal).

## Findings, gate, and honesty

- Findings → `docs/feature/{id}/spike/findings{,-<name>}.md`: binary verdict
  (WORKS / DOESN'T-WORK), predicted-vs-actual evidence (paste real output, never
  narrate), edge cases, design implications, one-line gate recommendation.
- Record the promotion-gate decision (PROMOTE / DISCARD / PIVOT) in
  `docs/feature/{id}/spike/wave-decisions.md`.
- **Report DOESN'T-WORK honestly.** A negative result that kills a candidate (or
  pivots to a better one) is the spike succeeding — never contort a probe into
  green. Cross-check a surprising verdict against a production precedent (e.g. the
  Cilium source) before trusting it.
- After a failed dispatch, do NOT blindly re-fire — confirm the corrected setup
  first.

## Spike evidence is not test coverage

A spike is **evidence**: it shows what a mechanism did at one commit, on one
kernel, under one setup. Cite it as evidence wherever a design claim rests on
it ("V42 shows init's binds produce no events with the hooks on the workload
cgroup"). That use is correct and needs no qualification.

A spike is **never coverage**. It does not keep checking anything; it is
throwaway and is deleted when the implementation lands. A property that needs
to keep holding gets a test (or a reasoned ruling that it needs none). The
spike can motivate that test; it cannot stand in for it.

Keep the two questions apart:

- **"Is this claim supported?"** — evidence: a spike, a source reading, a
  measurement. Citing a spike here is right.
- **"What catches a regression?"** — coverage: a test in a lane that runs, or
  a reasoned no-obligation ruling (the failure is unreachable, or it has no
  production effect). A spike is never the answer here.

When a test obligation cannot be met (no observable, no deployable fixture),
do not answer it with a spike. Decide whether the failure it guards is
reachable: if it is not, record a no-obligation ruling with that reason; if it
is, surface the missing observable or fixture as a design gap.

The opposite error is just as wrong: do not "correct" a citation that uses a
spike as evidence merely because the property also has, or lacks, a test.
Whether a property needs a test is a separate question from whether the
evidence citation is accurate.

## Symptoms during review

- An obligation, gate or test-coverage row discharged with "covered by spike
  V-N" / "proven by the spike" — a spike used as coverage. Replace with a test
  or a reasoned no-obligation ruling.
- A reviewer or agent flagging every "V-N evidence" citation as a defect —
  evidence conflated with coverage; the citation is fine.
- A `.bpf.c` / `driver.c` / `vmlinux.h` / `clang` invocation in a probe → wrong
  language; eBPF is aya-rs Rust.
- A probe file under `crates/`, or a `mod.rs` / `Cargo.toml [[bin]]` edit wiring a
  `probe_*` module into a workspace crate → isolation breach; relocate to
  `spike-scratch/`.
- A probe "verdict" from `cargo … --no-run` or a host-side compile-check → not a
  runtime signal; run it under Lima.
- A `findings.md` verdict with no pasted command/program output → narrated, not
  executed.

## Cross-references

- `nw-spike` skill — the PROBE → PROMOTION GATE → WALKING SKELETON mechanics.
- `.claude/rules/testing.md` § "Running tests — Lima VM" — the Lima execution
  discipline + the no-Tier-2-backstop hazard for `cgroup_sock_addr`.
- `.claude/rules/development.md` § "aya-rs XDP / TC kernel-side patterns" — how to
  write aya-rs eBPF.
- Precedent: GH #236 transparent-mtls-enrollment — `spike-scratch/increment-a`
  (Probe A, aya-rs, PIVOT) and `spike-scratch/increment-b` (egress nft-TPROXY,
  Rust + `nft`, WORKS).
