# DESIGN review (adversarial) — native-benchmark-program

Reviewer: nw-solution-architect-reviewer (Sonnet), 2026-11-17. Scope: DESIGN sections of `feature-delta.md`, ADR-0296..0309, spike findings. DDD-1..14 user-approved 2026-11-17; not re-litigated.

**Verdict: CHANGES_REQUESTED** — 0 BLOCKER, 2 HIGH, 4 MEDIUM, 3 LOW.

## Verified, no finding
- Kernel citations sampled against `vendors/linux` v7.0 match: `kernel/bpf/syscall.c` (stats fd release 5990-5996, `bpf-stats` fd 6002-6020, `CAP_SYS_ADMIN` 6030, sysctl `saved_val` 6533-6558), `fs/anon_inodes.c:76`, `kernel/time/timekeeping.c:2163`.
- `aya::sys::enable_stats` exists in aya 0.13.1.
- DDD-11 seams exist (`NodeAdapters.vmm`/`.beacon`, `vm.lifecycle.*`, `convergence.evaluation.admitted`, guest `elapsed_ms`).
- DDD-12 observer is a required constructor parameter (no defaulted port).
- V-1..V-5 claims match their findings; G-1 complete; Formal protocol model N/A sound; no untracked limits (#361, #362, #303); xtask/bin/cfg rules hold.

## Findings
1. **HIGH — relative `--results-root` resolves against the package dir.** `cargo bench` runs with cwd = package root, so `benchmarks/results` lands under `crates/overdrive-bench-families/` and the spike path doubles. Fix: resolve against `OVERDRIVE_METAL_SOURCE_ROOT` (exported at `infra/metal/bootstrap.sh:779`) or pass absolute; print repo-relative; pin in DDD-9.
2. **HIGH — DDD-14 signature unpinned.** `spawn_decrypt` (`splice.rs:220`) / `spawn_encrypt` (`:249`) take no role and serve other callers (`outbound.rs:371`, `mod.rs:1496,1503,1604`, inbound); `:281` is `spawn_async`, inbound-only; `thread::Builder::spawn` returns `io::Result`. Fix: one required `&'static str` name argument; outbound passes `mtls-out-fwd`/`mtls-out-ret`, every other caller a distinct name; spawn failure maps to the existing per-connection establish error; drop `:281`.
3. **MEDIUM — `<data_dir>` fixture not the spike-verified mechanism.** DDD-11 states 4200/0710, not `umask(0o022)`. Fix: state scoped, RAII-restored `umask(0o022)` for `<data_dir>`; keep 4200/0710 only for kit-owned dirs.
4. **MEDIUM — kit guest image read, not run.** No spike boots `GuestImageSpec::workload()` + `overdrive-guest-peer idle` under production `overdrive-init` with the console-log decorator. Fix: run it now, or reclassify as DELIVER-blocking.
5. **MEDIUM — connect-hook smoke depends on `bpftool` in the guest and unstated stats wiring.** Fix: detach through the dataplane's link handle in process; state whether `smoke` runs `StatsEnabler` or stats metrics are `absent`.
6. **MEDIUM — committed raw series unbounded** (V-2 alone: 6M queries). Fix: pin a per-record cap or histogram/run-length storage before DISTILL fixes rates.
7. **LOW — `build.opt_level` has no source** (build-script-only variable, no `build.rs`). Add one or drop the field.
8. **LOW — "pinned production settings" are test-host values** (ADR-0302; ADR-0068 defines no governor/SMT/boost). Retitle "pinned test-host settings".
9. **LOW — no error enum / `Result` alias pinned for `overdrive-bench`** (`rust.md`, CLAUDE.md). The `exec` refusal of `overdrive` is name-only.

## Resolution

**Resolved 2026-11-17.** All nine findings are closed inside DDD-1..14 (no approved decision changes). `feature-delta.md` section names below; ADR status is **Accepted**.

| # | Change |
|---|---|
| 1 | DDD-9 (Interface contracts): `bootstrap.sh` passes an absolute `--results-root $OVERDRIVE_METAL_SOURCE_ROOT/...`; the runner refuses a relative root or one outside the source root (`Error::Usage`); `OVERDRIVE_BENCH_RESULT_DIR` is printed relative to the source root. ADR-0304 matches. |
| 2 | DDD-14: `spawn_encrypt` / `spawn_decrypt` take one required `name: &'static str` and return `io::Result<PumpHandle>`; callers enumerated from the code (outbound `establish` `mtls-out-fwd` / `mtls-out-ret`; unit-test callers distinct `mtls-test-*` names); spawn failure maps to `MtlsEnforcementError::Io`, with the forward pump stopped and joined if already started; `splice.rs:281` (`spawn_async`, inbound) is out of scope. ADR-0309 matches. |
| 3 | DDD-11 (guest image and data directory): `<data_dir>` is created under a scoped RAII-restored `umask(0o022)` as `guest_listener_real_boot.rs:143-149` does; 4200 / `0710` only for kit-owned directories. "User approval" line and ADR-0306 match. |
| 4 | Validation items: new V-10 (DELIVER, blocks only the VM-lifecycle step, not contract-shaping) with hypothesis, prediction and falsification; DDD-11 heading and ADR-0306 cite it. |
| 5 | Connect-hook family: in-process detach through the kernel link commands over raw `bpf(2)` (`EbpfDataplane` has no public detach handle: links are private fields released only by `Drop`); no `bpftool`. DDD-6 / DDD-8: `smoke` holds no statistics descriptor, `bpf_stats()` is `None`, stats metrics are `absent`, no timing asserted. ADR-0303 matches. |
| 6 | DDD-5 (series cap row), DDD-6 (record `truncated`, report, retention): at most 10,000 values per series metric per replicate, first in arrival order, dropped count recorded. ADR-0300 and ADR-0301 match. |
| 7 | DDD-6 (`run_started`) and DDD-7: `debug_assertions` only; `opt_level` and `profile` dropped. ADR-0301. |
| 8 | DDD-7 and ADR-0302 retitled "pinned test-host settings" (the ADR file name is unchanged). |
| 9 | DDD-6: `overdrive_bench::Error` (`thiserror`; `Usage`, `FamilyNotDeclared`, `Refused`, `Unreadable`, `StatsEnable`, `Record`, `InvalidRecord`, `SubstrateMismatch`) and `Result<T, E = Error>`; the `exec` refusal of `overdrive` is removed (ruling 3 is enforced by the families' code and review; see the subtractive review S-8(b)). |
