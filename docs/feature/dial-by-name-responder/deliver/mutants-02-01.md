# Mutation-gate evidence — step 02-01 (review-02-01 D1)

**Feature:** dial-by-name-responder (ADR-0072 / #243)
**Step:** 02-01 — `DnsResponder` IP_PKTINFO host adapter + single `FrontendAddrAllocator` wired into `run_server`
**Gate at HEAD:** `48bb5562` (the `is_addr_in_use` test corrective, on top of `751a1d69` review-correctives, on top of `c6f4ab2a` feat)
**Run:** diff-scoped, Lima-routed, `--features integration-tests`, real Lima-root.
**Verdict:** **PASS — kill_rate 89.3% (≥ 80%).**

## Command

```bash
cargo xtask lima run -- cargo xtask mutants --diff origin/main \
  --features integration-tests --package overdrive-control-plane \
  --file crates/overdrive-control-plane/src/dns_responder/responder.rs \
  --file crates/overdrive-control-plane/src/mtls_resolve_adapter.rs \
  --file crates/overdrive-control-plane/src/lib.rs
```

The `--file` set extends the roadmap's `responder.rs`-only list to also cover
`mtls_resolve_adapter.rs` (the new `project_by_frontend` / `project_row_by_frontend`
pure-reader feeder this step moved into the adapter) and the `lib.rs` composition
diff — per review **D1**'s required action ("run over **both** `responder.rs` *and*
`mtls_resolve_adapter.rs`"). `--diff origin/main` scopes mutation to this PR's
changed lines, so the large `lib.rs` is not fully mutated.

## Result

```
mutants: mode=diff total=32 caught=25 missed=3 timeout=0 unviable=4 kill_rate=89.3%
mutants: PASS
Unmutated baseline: ok (22s build + 37s test) — no nft/cgroup environmental failure
32 mutants tested in 14m
```

| | count |
|---|---|
| caught | 25 |
| missed | 3 |
| unviable | 4 (don't compile — e.g. body→`Default` where the type is not `Default`; not coverage gaps) |
| **kill rate** | **89.3%** (25 / (25+3)) |

## The corrective that took it from 71.4% → 89.3%

The first run (at `751a1d69`, before this corrective) was **71.4% — 8 missed, FAIL**.
All 8 misses were in `responder.rs`; **5 of them were a genuine in-process gap** on
the pure `is_addr_in_use` EADDRINUSE predicate (`responder.rs:485`):

```
:485:5  is_addr_in_use -> bool  with true
:485:5  is_addr_in_use -> bool  with false
:485:49 ||→&&
:485:16 ==→!=
:485:71 ==→!=
```

`is_addr_in_use` is a **pure** `&io::Error → bool` predicate; in production it only
ever sees a *real* `bind` error (the Tier-3 fallback path), so it had no direct unit
test. Commit `48bb5562` adds `is_addr_in_use_accepts_eaddrinuse_and_rejects_other_kinds`
(two cases — an `AddrInUse` kind must be ACCEPTED, a `NotFound`/`PermissionDenied` kind
REJECTED) which kills all five in-process. Verified GREEN under Lima before the re-run.

## The 3 residual misses are the irreducibly-Tier-3 socket arms (sanctioned exclusion)

These are exactly the arms the review named "not in-process-killable and **correctly
excluded**" (review-02-01 D1: *"the irreducibly-Tier-3 socket arms
(IP_PKTINFO/ipi_spec_dst/fallback re-derive) are not in-process-killable"*). They each
require a **real `bind()` against a real kernel** to exercise — there is no Tier-2
`BPF_PROG_TEST_RUN` / synthetic-socket backstop (DDN-4):

| Mutant | Why Tier-3-only | Behavioural coverage |
|---|---|---|
| `responder.rs:274:25` — `match guard is_addr_in_use(&err)` → `true` | The wildcard→fallback **dispatch** fires only on a real `EADDRINUSE` from a real wildcard `bind`; no in-process seam injects a bind error at this site | `per_gateway_addr_fallback_binds_when_wildcard_is_held` (BIND-02) forces a real wildcard holder → the guard's `true` path; `wildcard_bind_answers_frontend_and_source_pins_reply` (BIND-01) takes the wildcard-success path (no error → guard not reached) |
| `responder.rs:274:25` — same guard → `false` | same | BIND-02 (above): with the guard forced `false` the responder would propagate the `EADDRINUSE` instead of falling back, and BIND-02's fallback bind would not occur |
| `responder.rs:323:9` — `bind_per_gateway_addr -> Ok(vec![])` | Binds **real OS sockets**; "bound N sockets" vs "empty vec" is only observable by a real bind + a real query/reply | BIND-02 exercises the real per-gateway bind; the empty-vec case is the **N2 deaf-responder** path, now structurally surfaced by the `dns.responder.fallback.zero_sockets` warning (`empty_fallback_binds_zero_sockets_and_warns_it_is_deaf`). The *converge*-strength assertion that would kill this mutant in-line is **descoped to #247** (review D3) |

These three are the same class the project documents elsewhere as Tier-3-killed-not-
in-process (e.g. the canonical-tproxy 02-01 gate's behaviourally-covered branch). The
in-process surface the gate exists to defend — the `DnsResponderError`→reason mapping
(`boot_refusal_reason`), the `DnsResponderBoot` refusal return, and the
`project_by_frontend` fail-closed feeder — is fully **caught**.

## Caveats / environment

- **Baseline clean this run.** The unmutated baseline passed (no `overdrive-mtls`
  nft-table parallel-execution race, no missing `overdrive.slice` cgroup) — the
  Tier-3 boot fixtures' cgroup-slice precondition (documented in the `751a1d69`
  corrective return) was satisfied in the VM at run time.
- **Guest-path note.** The Lima mutation run writes `target/xtask/mutants-summary.json`
  to the **guest** target dir; the authoritative kill-rate line is the `mutants:
  mode=diff … kill_rate=89.3% PASS` summary in the run log (this file), not the stale
  host artifact.
