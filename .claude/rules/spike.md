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
  wiring, no `[[bin]]` in any workspace member's `Cargo.toml`, no new dep on a
  workspace crate. The probe is a standalone build under `spike-scratch/`.
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
- If a probe needs a helper that lives in `crates/` (a syscall wrapper, a const),
  **copy it into the spike** — never add a dependency edge that drags the
  workspace build, never edit the original.

**Why:** a spike is a *disposable validation of one assumption*, not a feature
increment. Code written into `crates/` (even reverted later) pollutes production,
drags the build chain, and blurs "what ships." **If a dispatch does pollute
`crates/`, MOVE the files into `spike-scratch/` (preserve the work) and revert the
production wiring — do not delete the work.**

## A spike may drive an existing component — never stand in for one

**A spike cannot establish component reuse or multi-component composition by
copying, simplifying, mocking, or reimplementing the components it claims to
validate.** Reuse is decided in DESIGN from the real component contracts and
production caller paths. A mechanism already validated by an existing component
does not need another spike; and a question whose answer depends on two or more
components working together is a walking-skeleton or integration-test question,
not a spike.

The valid boundary is:

- The existing production component remains unchanged and is driven through an
  already-existing production surface (binary, CLI, socket, syscall effect, or
  other real driving port). Scratch code may supply one input or observe one
  external effect; it must not substitute a parallel implementation.
- If the production component has no surface through which the one mechanism can
  be driven or observed, stop and record a **DESIGN/testability gap**. Do not add
  a production API, port, trait method, test hook, enum variant, or adapter merely
  to manufacture a spike result.
- The self-contained-copy exception above is limited to a small helper such as a
  syscall wrapper, wire constant, or struct layout needed to isolate a low-level
  kernel/library mechanism. Its verdict applies **only to that mechanism**. It is
  never evidence that the copied production component, its public contract, or
  its integration with another component works.
- A scratch-only API shape proves nothing about the production API. Exact
  aggregate, trait, method, error, and ownership surfaces remain DESIGN
  decisions; the later production walking skeleton proves their composition.

For example, “does this kernel accept this aya-rs attach shape?” can be a spike.
“Does Gateway SVID issuance → XDP backend selection → intended-peer mTLS work?”
cannot: it crosses several existing owners and must be exercised through the
production composition. If uncertainty can be decomposed honestly, use separate
single-mechanism probes such as “does a host-originated socket traverse the
existing Service frontend XDP path?” and “is the XDP-selected peer identity
observable at the existing mTLS boundary?” Each probe must drive the real
component unchanged. If it cannot, return the gap to DESIGN.

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
- A scratch `Sim*`, fake holder, parallel map hydrator, or copied proxy standing
  in for the production component named by the finding → the finding validates
  the substitute, not the component.
- One spike claiming an end-to-end verdict across multiple existing component
  owners → misclassified walking skeleton/integration test; split only the truly
  independent mechanism questions and leave composition to the production path.
- A new production port, method, hook, or adapter added only so scratch code can
  reach the hypothesised state → DESIGN/testability gap disguised as a spike
  seam; remove it and surface the gap.

## Cross-references

- `nw-spike` skill — the PROBE → PROMOTION GATE → WALKING SKELETON mechanics.
- `nw-spike-methodology` skill — one-assumption limit, when to skip an already
  proven mechanism, and the distinction between a spike and a walking skeleton.
- `nw-design` skill § "Reuse Analysis" — EXTEND/CREATE decisions come from real
  component overlap and contracts, never from a scratch substitute.
- `.claude/rules/testing.md` § "Running tests — Lima VM" — the Lima execution
  discipline + the no-Tier-2-backstop hazard for `cgroup_sock_addr`.
- `.claude/rules/development.md` § "aya-rs XDP / TC kernel-side patterns" — how to
  write aya-rs eBPF.
- Precedent: GH #236 transparent-mtls-enrollment — `spike-scratch/increment-a`
  (Probe A, aya-rs, PIVOT) and `spike-scratch/increment-b` (egress nft-TPROXY,
  Rust + `nft`, WORKS).
