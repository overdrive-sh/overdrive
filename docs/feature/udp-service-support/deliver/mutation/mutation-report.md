# Mutation Report — `udp-service-support`

**Feature**: udp-service-support
**Tool**: `cargo-mutants` via `cargo xtask mutants` (the project wrapper:
pins `--test-tool=nextest`, sets `OVERDRIVE_BPF_OBJECT`, enforces the
kill-rate gate).
**Mode**: diff-scoped — `cargo xtask mutants --diff origin/main --features
integration-tests`, executed through Lima (`cargo xtask lima run --`) per
`.claude/rules/testing.md` (macOS + `--features integration-tests` ⇒ Lima).
**Final HEAD at gate pass**: `gather_service_listener_facts` exclusion commit
(see "Resolution" below); implementation HEAD `80d4515b`.
**Threshold**: kill rate ≥ 80%.

## Verdict: **PASS — 100.0%**

```
4 mutants tested in 2m: 3 caught, 1 unviable
mutants: mode=diff total=4 caught=3 missed=0 timeout=0 unviable=1 kill_rate=100.0%
mutants: PASS
```

The 1 unviable mutant carries no quality signal (a synthesised replacement
body that fails to compile — the gate correctly excludes it from the
denominator). Kill rate = caught / (caught + missed) = 3 / 3 = 100%.

## Scope

`--in-diff` mutates only changed lines that overlap a mutable operator. The
feature diff is 16 production-source files / ~706 insertions, but most is
test code, comments, additive struct/enum shape, and `overdrive-sim`
adapter/invariant paths excluded by `.cargo/mutants.toml` Rule 7. That
narrowed to **5 viable mutants** initially (4 after the equivalent-mutant
exclusion below).

## Run history

| Run | HEAD | Mutants | Caught | Missed | Unviable | Kill rate | Verdict |
|-----|------|---------|--------|--------|----------|-----------|---------|
| 1 (initial) | `19216a57` | 5 | 2 | 2 | 1 | 50.0% | FAIL |
| 2 (after kill test) | `80d4515b` | 5 | 3 | 1 | 1 | 75.0% | FAIL |
| 3 (after equiv exclusion) | exclusion commit | 4 | 3 | 0 | 1 | 100.0% | **PASS** |

## Surviving mutants — resolution

### M1 — `crates/overdrive-core/src/reconcilers/service_map_hydrator.rs:396`
`replace push_register_local_backend_actions with ()` — **CAUGHT** (was MISSED in run 1).

`push_register_local_backend_actions` emits one `Action::RegisterLocalBackend`
per local IPv4 backend passing the ADR-0053 §4 `classify_backend_address`
guard (IPv6 and guard-rejected IPv4 skipped with a structured warn). Its only
prior behavioral coverage was Tier-3 (real-veth) integration tests, which the
mutants nextest run did not exercise for this mutant (`0s test`).

**Resolution (commit `80d4515b`)**: two default-lane unit tests added in a new
`#[cfg(test)] mod tests` —
`push_register_local_backend_emits_action_for_valid_local_backend` (the killer:
asserts exactly one `RegisterLocalBackend` with correct
`service_id`/`vip`/`vip_port`/`backend`/`correlation`) and
`push_register_local_backend_skips_ipv6_and_guard_rejected` (pins the two
`continue` arms). With the body no-op'd, `actions` stays empty and the
`len == 1` assertion fails. Verified empirically by hand-injecting an early
return.

### M2 — `crates/overdrive-control-plane/src/reconciler_runtime.rs:1751`
`replace || with && in gather_service_listener_facts` — **EQUIVALENT MUTANT, excluded**.

The guard `if suffix.is_empty() || suffix.contains('/') { continue; }` is a
fast-path skip of the `workloads/<id>/stop` and `workloads/<id>/kind` sub-keys
before the `from_store_bytes` decode. Flipping to `&&` makes the early-skip
never fire, but the downstream `let Ok(intent) =
WorkloadIntent::from_store_bytes(...) else { continue }` +
`WorkloadIntent::Service(..)` match rejects those sub-keys identically: `/stop`
carries an empty payload (`b""`) and `/kind` a single discriminator byte,
neither of which passes rkyv bytecheck for a full
`WorkloadIntentEnvelope::Service`. The canonical `workloads/<id>` key has a
non-empty suffix without `/`, so it is never skipped under either operator. The
returned `Vec<ListenerRow>` is byte-identical under both operators — a genuine
equivalent mutant, structurally undistinguishable by any test.

**Resolution**: an `exclude_re` entry
(`"replace \\|\\| with && in gather_service_listener_facts"`) was added to
`.cargo/mutants.toml`, with a full justification comment, alongside the
existing equivalent-mutant precedents (`evaluate_reconciler_is_pure`,
`NoopHeartbeat::hydrate`, `ProbeRunner::probe`). The source-level
`// mutants: skip` comment above the guard remains as documentation. The
`exclude_re` (not the comment) is the operative suppression mechanism: this
cargo-mutants version only honours comment-skip at function granularity on the
single line above a `fn`, so a comment above a mid-body `if` does not fire —
the same limitation the file's `ProbeRunner::probe` entry documents.

## Mutated production files in the feature diff

| Crate | Files mutated (viable surface) |
|-------|--------------------------------|
| overdrive-core | `reconcilers/service_map_hydrator.rs`, `reconcilers/mod.rs`, `dataplane/service_frontend.rs`, `traits/dataplane.rs` |
| overdrive-control-plane | `reconciler_runtime.rs`, `action_shim/dataplane_update_service.rs`, `action_shim/validate.rs` |
| overdrive-cli | `commands/job.rs` (step 01-05 Service deploy arm) |

`overdrive-sim/src/{adapters,invariants}/**` and `overdrive-dataplane/src/lib.rs`
changes are excluded by `.cargo/mutants.toml` (Rules 7 / 1-extended) — their
correctness is protected by `cargo xtask dst` (Tier 1) and Tier 3, not the
nextest mutants lane.

## Post-run safety

cargo-mutants restored the working tree itself on every normal exit (no
`git checkout` performed — the destructive-git-ops hook blocks it and the
restore is redundant per `.claude/rules/testing.md`). Working tree verified
clean of source mutations after each run.
