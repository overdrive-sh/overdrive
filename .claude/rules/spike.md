# Spike Discipline

How throwaway spike/probe code is written, isolated, and run. Extracted from the
transparent-mtls-enrollment spike arc (GH #236), where dispatches violated each of
these before they were codified.

This governs the **PROBE phase of `/nw-spike`** and any ad-hoc throwaway probe.
The wave mechanics (probe → promotion gate → walking skeleton) live in the
`nw-spike` skill; this file is the SSOT for *how probe code is written and run*.

---

## Spike code is throwaway and ISOLATED — never in `crates/`

Probe code lives in a **gitignored** `spike-scratch/{increment-a,increment-b,…}/`
directory, **self-contained** (its own `Cargo.toml` / workspace), and **never
touches production source**:

- **NEVER** create or modify a file under `crates/`. No new modules, no `mod.rs`
  wiring, no `[[bin]]` in any workspace member's `Cargo.toml`, no new dep on a
  workspace crate. The probe is a standalone build under `spike-scratch/`.
- `spike-scratch/` is in `.gitignore` — the probe is **never committed**.
- One increment per probe attempt: `increment-a`, `increment-b`, … Preserve prior
  increments as evidence; don't overwrite.
- If a probe needs a helper that lives in `crates/` (a syscall wrapper, a const),
  **copy it into the spike** — never add a dependency edge that drags the
  workspace build, never edit the original.

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
- Record `uname -r` in the findings; the verdict is pinned to a kernel (dev Lima
  and the pinned 6.18 appliance kernel differ — ADR-0068).

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

## Symptoms during review

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
